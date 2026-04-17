# ACP Integration — Native Frontends for Hermes

## When to Use ACP vs the API Server

For building native frontends (TUIs, custom IDEs, desktop apps) that need full
agent access, use ACP (Agent Communication Protocol) over stdio — NOT the HTTP
API server.

**The API server creates isolated agent instances.** It's designed for external
platforms (Open WebUI, Discord, Telegram) where each platform adapter manages
its own session context. A native frontend connecting via HTTP gets a separate
agent that doesn't share sessions, memory, or approval state with the CLI or
other platforms. This is the "gateway middleman" problem — discovered when
building hermes-tui, which initially connected to /v1/chat/completions and
ended up in a completely isolated profile/session context.

**ACP solves this.** The frontend spawns `hermes acp` as a subprocess and
communicates via JSON-RPC over stdin/stdout. The agent runs directly in the
subprocess — same profile, same config, same state.db.

**Rule of thumb:** If your frontend can spawn a subprocess, use ACP. If it must
use HTTP (browser app, remote service), use the API server.

## What ACP Provides

- **Session management**: `new_session`, `resume_session`, `fork_session`,
  `list_sessions`, `cancel`
- **Structured events** via `session_update` notifications:
  - `agent_message_chunk` — streamed response text
  - `agent_thought_chunk` — reasoning/thinking text
  - `tool_call` / `tool_call_update` — tool start, progress, completion
    with name, kind (read/edit/execute/fetch), status
  - `usage_update` — token usage
  - `available_commands_update` — advertised slash commands
- **Bidirectional approvals**: `request_permission` is a server-to-client
  JSON-RPC *request* (not a notification). The agent thread blocks until
  the client responds with the chosen option (allow_once, allow_always,
  deny). No text parsing needed.
- **Slash commands** handled server-side: `/model` (show/switch), `/tools`,
  `/compact`, `/reset`, `/context`, `/version`, `/help`
- **Session persistence**: sessions save to state.db, survive process
  restarts, appear in `session_search`

## Source Layout

```
acp_adapter/
  entry.py        — CLI entry point (hermes acp), logging to stderr
  server.py       — HermesACPAgent: session management, prompt dispatch,
                    slash commands
  session.py      — SessionManager: state.db persistence, agent lifecycle
  events.py       — Callback factories bridging AIAgent → ACP notifications
  permissions.py  — Approval callback: ACP request_permission → hermes flow
  tools.py        — Tool call ID generation, ToolCallStart/Progress builders
```

## Key Details

- **Invocation**: `hermes acp` (stdio is the default transport)
- **Logging**: All logging goes to stderr; stdout is JSON-RPC only
- **Thread model**: AIAgent runs synchronously in a ThreadPoolExecutor
  (max 4 workers). Callbacks use `asyncio.run_coroutine_threadsafe()`
  to bridge worker threads → event loop.
- **Toolset**: ACP sessions use `hermes-acp` toolset by default
  (configurable via `platform_toolsets.acp` in config.yaml)
- **Profile support**: respects HERMES_HOME and --profile flags

## Known Limitations (as of April 2026)

- `list_sessions` only returns session_id, cwd, model, history_len — no
  title, timestamps, or source platform. Data exists in state.db but
  SessionManager.list_sessions() doesn't expose it.
- Missing slash commands vs CLI/gateway: `/title`, `/reasoning`, `/skills`,
  `/skill <name>`, `/yolo` are not in the ACP adapter yet. `/compact`
  exists as the equivalent of `/compress`.
- No `/approve` or `/deny` slash commands needed — the ACP protocol handles
  approvals structurally via `request_permission`.

## ACP Client Interface (what the frontend calls)

From the `acp.Client` class:

| Method | Purpose |
|--------|---------|
| `session_update(session_id, update)` | Receive streamed events |
| `request_permission(options, session_id, tool_call)` | Handle approval requests |
| `read_text_file(path, session_id)` | Agent reads a file via client |
| `write_text_file(content, path, session_id)` | Agent writes a file via client |
| `create_terminal(command, session_id)` | Agent creates a terminal |
| `terminal_output(session_id, terminal_id)` | Get terminal output |
| `kill_terminal(session_id, terminal_id)` | Kill a terminal |

## Startup Sequence

1. Spawn: `hermes acp`
2. Send `initialize` with client info
3. Receive capabilities, version, auth methods
4. Call `list_sessions` for session picker
5. `new_session(cwd)` or `resume_session(cwd, session_id)`
6. Chat via `prompt(text, session_id)` — events stream back
