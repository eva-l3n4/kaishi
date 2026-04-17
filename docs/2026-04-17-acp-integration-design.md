# Hermes TUI — ACP Integration Design

**Date:** 2026-04-17
**Status:** Approved

## Overview

Migrate hermes-tui from the HTTP-based API server gateway (`/v1/chat/completions`
on port 8664) to ACP (Agent Communication Protocol) over stdio. The TUI spawns
`hermes acp` as a child process and communicates via JSON-RPC, giving it
direct access to the full Hermes agent — tools, skills, memory, approvals,
session management — without going through the gateway middleman.

### Why ACP?

The current HTTP approach talks to the gateway's API server, which creates
isolated agent instances. This caused the TUI to run in a separate
profile/session context, disconnected from the user's primary Hermes
environment. The gateway is designed for external platforms (Discord, Telegram,
Open WebUI) — not for a native frontend.

ACP was built for exactly this: a structured protocol for native frontends
(IDEs, TUIs) to drive the Hermes agent directly. It provides:

- Structured JSON-RPC transport (no SSE parsing, no HTTP)
- Session management (create, resume, fork, list)
- Bidirectional approval flow (agent requests permission, TUI responds)
- Tool call visibility (start, progress, completion events)
- Thinking/reasoning stream
- Cancel/interrupt support
- Slash commands handled server-side

### Alternatives Considered

1. **`/v1/responses` (Responses API)** — richer than chat completions but still
   HTTP-based, still goes through the gateway, still creates isolated sessions.
   Would have required upstream contributions for approval events.

2. **Direct Python IPC** — spawn `hermes chat` and parse terminal output.
   Fragile, two competing raw-mode terminals, any CLI change breaks the parser.

3. **Stay on `/v1/chat/completions`** — the current approach. Text-only,
   no tool visibility, no structured approvals, isolated sessions.

ACP is the clear winner: structured protocol, full agent access, already
implemented in hermes-agent.

---

## Architecture

```
┌─────────────────────────────┐
│       hermes-tui (Rust)     │
│                             │
│  ┌───────────┐ ┌─────────┐ │
│  │ ui/*.rs   │ │ app.rs  │ │       JSON-RPC over stdio
│  │ (ratatui) │ │ (state) │ │    ┌──────────────────────────┐
│  └─────┬─────┘ └────┬────┘ │    │  hermes acp              │
│        │             │      │    │  (Python subprocess)     │
│  ┌─────┴─────────────┴────┐ │    │                          │
│  │       event.rs         │◄├────┤  acp_adapter/server.py   │
│  │    (AppEvent channel)  │ │    │  acp_adapter/session.py  │
│  └────────────┬───────────┘ │    │  acp_adapter/events.py   │
│               │             │    │  acp_adapter/permissions  │
│  ┌────────────┴───────────┐ │    │                          │
│  │       acp.rs           ├─├────┤  AIAgent (run_agent.py)  │
│  │  (JSON-RPC framing,   │ │    │  tools, skills, memory   │
│  │   spawn, send/recv)   │ │    │  state.db persistence    │
│  └────────────────────────┘ │    └──────────────────────────┘
└─────────────────────────────┘
         stdin ──►  ◄── stdout
                    stderr → log
```

The TUI owns the terminal (ratatui/crossterm). The ACP subprocess runs headless,
logging to stderr. Communication is bidirectional JSON-RPC: the TUI sends
requests (prompt, new_session, cancel), and the agent sends both responses and
notifications (session_update events for text, tools, thinking) plus
server-to-client requests (request_permission for approvals).

---

## Transport: JSON-RPC over stdio

### Startup sequence

1. TUI spawns: `hermes acp` (respects `HERMES_HOME` / `--profile`)
2. TUI sends `initialize` request with client info
3. Agent responds with capabilities, version, auth methods
4. TUI calls `list_sessions` to populate the session picker
5. User selects → `new_session(cwd)` or `resume_session(cwd, session_id)`
6. Chat begins — user input sent via `prompt(text, session_id)`

### Message flow during chat

```
TUI                              ACP Agent
 │                                   │
 │──── prompt(text, session_id) ────►│
 │                                   │
 │◄─── session_update ──────────────│  (agent_message_chunk: text delta)
 │◄─── session_update ──────────────│  (tool_call: started)
 │◄─── session_update ──────────────│  (tool_call_update: completed)
 │◄─── session_update ──────────────│  (agent_message_chunk: more text)
 │                                   │
 │◄─── request_permission ──────────│  (dangerous command approval)
 │──── response(allow_once) ────────►│  (user chose in modal)
 │                                   │
 │◄─── session_update ──────────────│  (agent_message_chunk: final text)
 │◄─── prompt response ─────────────│  (stop_reason, usage)
 │                                   │
```

### Cancellation

`Ctrl+C` or `/stop` sends `cancel(session_id)`. The ACP server sets the cancel
event and calls `agent.interrupt()`, stopping LLM API calls at the next loop
iteration.

---

## Launch Flow: Session Picker

Full-screen picker shown on startup before entering chat mode.

```
 ┌─ 🌸 Hermes ──────────────────────────────────────────────┐
 │                                                           │
 │  > New Session                                            │
 │    Rust best practices review          12 msgs      2h ago│
 │    TUI scroll math debugging            8 msgs      5h ago│
 │    Deploy model router                 34 msgs      1d ago│
 │                                                           │
 └───────────────── Enter: select  Esc: quit ────────────────┘
```

**Controls:** Arrow keys / j/k to navigate, Enter to select, Esc to quit.

**Data source:** `list_sessions()` ACP call. Currently returns session_id, cwd,
model, history_len. Title, timestamps, and source platform need upstream
enrichment (see Upstream Contributions).

**State transition:** App uses a `Screen` enum — `Screen::Picker` vs
`Screen::Chat`. The picker is a separate rendering path, not a widget inside
chat.

**On selection:**
- "New Session" → `new_session(cwd)`, enter `Screen::Chat`
- Existing session → `resume_session(cwd, session_id)`, enter `Screen::Chat`

---

## Interactive Control: Modal Approvals

When the agent hits a dangerous command, the ACP server sends a
`request_permission` JSON-RPC request to the TUI.

### Payload (from agent)

```json
{
  "method": "request_permission",
  "params": {
    "session_id": "...",
    "tool_call": { "title": "rm -rf /tmp/build/*", "kind": "execute" },
    "options": [
      { "option_id": "allow_once", "kind": "allow_once", "name": "Allow once" },
      { "option_id": "allow_always", "kind": "allow_always", "name": "Allow always" },
      { "option_id": "deny", "kind": "reject_once", "name": "Deny" }
    ]
  }
}
```

### Modal overlay

```
 ┌─ Approval Required ──────────────────────────┐
 │                                               │
 │  rm -rf /tmp/build/*                          │
 │                                               │
 │  > Allow once                                 │
 │    Allow always                               │
 │    Deny                                       │
 │                                               │
 └───────────────────────────────────────────────┘
```

**Controls:** Arrow keys to navigate, Enter to confirm. Esc = Deny (safe
default).

**Behavior:**
1. `request_permission` received → push `AppEvent::ApprovalRequest`
2. App enters `ModalState::Approval { options, selected, request_id }`
3. All other input blocked — no accidental messages
4. User selects → TUI sends JSON-RPC response with chosen `option_id`
5. Modal closes, agent thread unblocks, streaming resumes

The agent thread on the server blocks on a `threading.Event` until the response
arrives, so the notification stream naturally pauses — no buffering tricks needed.

---

## Slash Commands

### TUI-local (handled in app.rs, never sent to ACP)

| Command              | Action                                      |
|----------------------|---------------------------------------------|
| `/quit`, `/exit`, `/q` | Close TUI                                 |
| `/clear`             | Clear display buffer (not session history)   |
| `/new`               | `new_session(cwd)` via ACP, reset chat view  |
| `/sessions`          | `list_sessions()` via ACP, render inline     |
| `/resume <id>`       | `resume_session(cwd, id)` via ACP            |
| `/stop`              | `cancel(session_id)` to interrupt agent      |

### ACP server-side (already implemented)

| Command    | Action                                 |
|------------|----------------------------------------|
| `/model`   | Show or switch model                   |
| `/tools`   | List available tools                   |
| `/compact`  | Compress conversation context         |
| `/reset`   | Clear conversation history             |
| `/context` | Show message counts by role            |
| `/version` | Show Hermes version                    |
| `/help`    | List available commands                |

### Tier 1 — need upstream contributions

| Command      | Notes                                           |
|--------------|-------------------------------------------------|
| `/title`     | Add to ACP slash commands, persist to SessionDB  |
| `/reasoning` | Via `set_config_option` or new slash command     |
| `/yolo`      | Toggle approval bypass via config option         |
| `/skills`    | Port skill loading from gateway to ACP adapter   |
| `/skill <n>` | Load specific skill into session                 |
| `/verbose`   | Can be TUI-local (controls tool event rendering) |

### Tier 2 — later

`/config`, `/voice`, `/personality`, `/skin`, `/background`, `/queue`,
`/branch`, `/cron`, `/tools` (enable/disable), `/toolsets`, `/browser`,
`/paste`, `/image`

---

## File Structure

```
src/
  main.rs         Spawn ACP subprocess, run picker or chat
  acp.rs          ACP JSON-RPC client: spawn, send/receive, framing
  app.rs          App state, Screen enum (Picker/Chat), event handlers
  event.rs        AppEvent variants (ACP-native), event loop
  ui.rs           Top-level draw dispatch, shared helpers, markdown rendering
  ui_picker.rs    Session picker screen
  ui_modal.rs     Approval modal overlay
```

### Rationale for split

`ui.rs` is already 539 lines with just the chat view. Adding two new screen
types (picker, modal) would push it past maintainability. Each UI module owns
one screen/overlay, with `ui.rs` retaining the chat view, markdown renderer,
and shared helpers (status bar, input box, pre-wrap logic).

---

## Dependency Changes

### Remove
- `reqwest` — no more HTTP client
- `reqwest-eventsource` — no more SSE parsing
- `futures` — was only used for SSE stream

### Keep
- `ratatui` + `crossterm` — TUI framework
- `tokio` — async runtime, process spawning, channels
- `serde` + `serde_json` — JSON-RPC serialization
- `termimad` — markdown rendering (evaluate if still needed vs custom renderer)
- `uuid` — session IDs (may be generated server-side now)
- `dirs` — config path resolution
- `anyhow` — error handling

### Possibly add
- Nothing new expected. `tokio::process` is already available via the `full`
  feature flag.

Net result: fewer dependencies.

---

## Event System

### AppEvent variants

```rust
pub enum AppEvent {
    // Terminal events
    Key(KeyEvent),
    Tick,
    MouseScroll(i16),
    Resize(u16, u16),

    // ACP agent events
    AgentMessage(String),          // text delta from agent
    AgentThought(String),          // reasoning/thinking text
    ToolCallStart {                // tool invocation started
        id: String,
        name: String,
        kind: Option<String>,      // read, edit, execute, fetch, etc.
    },
    ToolCallUpdate {               // tool progress or completion
        id: String,
        status: String,            // in_progress, completed, failed
        content: Option<String>,
    },
    PromptDone {                   // agent finished responding
        stop_reason: String,
        usage: Option<Usage>,
    },

    // Approval (server-to-client request)
    ApprovalRequest {
        request_id: String,        // JSON-RPC id to respond to
        command: String,
        options: Vec<ApprovalOption>,
    },

    // ACP lifecycle
    AcpReady,                      // initialize handshake complete
    AcpError(String),              // connection/protocol error
    SessionsLoaded(Vec<SessionInfo>), // list_sessions response
}
```

### Flow

1. `acp.rs` runs a reader task on the subprocess stdout
2. Parses JSON-RPC messages (responses to our requests + notifications + server requests)
3. Maps them to `AppEvent` variants and pushes to the channel
4. `event.rs` merges terminal events + ACP events into one stream
5. `app.rs` dispatches based on current `Screen` and `ModalState`

---

## App State

```rust
pub enum Screen {
    Picker,
    Chat,
}

pub enum ModalState {
    None,
    Approval {
        command: String,
        options: Vec<ApprovalOption>,
        selected: usize,
        request_id: String,
    },
}

pub struct App {
    // Screen state
    pub screen: Screen,
    pub modal: ModalState,

    // ACP connection
    pub acp: AcpClient,
    pub session_id: Option<String>,

    // Picker state
    pub sessions: Vec<SessionInfo>,
    pub picker_selected: usize,

    // Chat state
    pub messages: Vec<ChatMessage>,
    pub input: String,
    pub cursor: usize,
    pub scroll_offset: u16,
    pub status: AgentStatus,
    pub pending_response: String,
    pub pending_tools: HashMap<String, ToolCall>,

    // Display
    pub model_name: String,
    pub session_title: Option<String>,
    pub tick: u64,
    pub verbose: bool,  // controls tool event rendering detail

    quit: bool,
}
```

---

## Visual Style

- Single emoji: 🌸 (status bar header only)
- Braille spinner for streaming indicator (already implemented)
- Tool calls shown as inline text labels when not verbose, expandable
  cards with args/results when verbose
- Approval modal: bordered box, centered, highlighted selection with `>`
- Session picker: bordered box, full screen, `>` cursor
- Consistent with existing color scheme: cyan for user, magenta for assistant,
  yellow for system, dark gray for chrome

---

## Upstream Contributions Needed

These are not blockers for v1 but would enhance the experience:

1. **`list_sessions` enrichment** — add `title`, `started_at`, `last_active`,
   `source` to the ACP `ListSessionsResponse`. Currently only returns
   `session_id`, `cwd`, `model`, `history_len`. The data exists in
   `state.db` but `SessionManager.list_sessions()` doesn't expose it.

2. **`/title` slash command** — add to `_SLASH_COMMANDS` and
   `_ADVERTISED_COMMANDS` in `server.py`. Persist title to SessionDB via
   `db.rename_session()`.

3. **`/reasoning` support** — set reasoning level mid-session. Either a new
   slash command that updates the agent's reasoning parameters, or map to
   `set_config_option` with a recognized config key.

4. **`/skills` and `/skill <name>`** — port `agent/skill_commands.py` logic
   to the ACP adapter. The gateway already handles this for messaging
   platforms.

5. **`/yolo` toggle** — change approval mode mid-session. Map to
   `set_config_option` or a dedicated slash command that toggles
   `approvals.mode` between `manual` and `off`.

---

## Out of Scope (v1)

- Tool visibility beyond text labels (full collapsible cards, syntax
  highlighted output) — deferred to v2
- Image/file attachment support
- Voice input/output
- Multi-session tabs
- Config editing from within the TUI
- Cron job management
- Browser integration
