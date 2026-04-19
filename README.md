# 懐紙 Kaishi

> A terminal UI for [Hermes Agent](https://github.com/NousResearch/hermes-agent), speaking ACP over stdio.

Kaishi is a terminal UI for conversing with Hermes Agent—an AI agent that lives in your terminal. It speaks the [Agent Communication Protocol](https://agentcommunicationprotocol.dev/introduction/welcome) over stdio, rendering streaming markdown as it arrives, token by token.

Built in Rust with [ratatui](https://ratatui.rs). Syntax highlighting via syntect. No Electron, no web view, no runtime—just a 5.1M binary that starts in under 50ms. Named for the paper held beneath cherry blossoms to catch what falls.

[![Crates.io](https://img.shields.io/crates/v/kaishi)](https://crates.io/crates/kaishi)
[![License](https://img.shields.io/badge/license-MIT-blue)](./LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange)](https://www.rust-lang.org/)

## Quick Start

```bash
cargo install kaishi
```

[Hermes Agent](https://github.com/NousResearch/hermes-agent) must be installed separately—Kaishi spawns `hermes acp` as a subprocess and speaks ACP over stdin/stdout.

```bash
kaishi                       # launch with session picker
kaishi --profile <name>      # use a specific Hermes profile
kaishi --cwd <dir>           # start in a specific working directory
kaishi --session <id>        # resume a specific session
```

## Why

Kaishi spawns Hermes as a child process and speaks ACP over stdin/stdout—a JSON-RPC 2.0 wire protocol designed for agent communication. Every tool call, every thinking block, every token delta arrives as a typed event on an async channel. The TUI never blocks.

Rendering is immediate-mode via ratatui—each frame redraws from state, no retained widget tree. A dual-rate tick system drives the UI: fast ticks (50ms) for spinner animation during streaming, slow ticks (500ms) at idle. Markdown is parsed to styled spans at ingestion time and cached; the viewport only renders visible lines.

~7,900 lines of Rust. Thirteen source files. Compiles to a 5.1M binary.

## Features

- **Streaming**—tokens arrive over JSON-RPC and render as they land: thinking spinners, live markdown reflow, tool calls that update in place. No buffering.
- **Syntax highlighting**—fenced code blocks pass through syntect with the `base16-eighties.dark` theme. Rust, Python, TOML, YAML—200+ grammars, zero configuration.
- **Session continuity**—sessions are managed by the Hermes Agent backend. Kaishi connects to the running backend, lists available sessions, and resumes with full history replay.
- **File mentions**—type `@` to fuzzy-search your project tree. Selected files attach as context—no copy-paste, no leaving the conversation.
- **Command palette** (`Ctrl+P`) with fuzzy search across slash commands and keybindings.
- **Reverse history search** (`Ctrl+R`)—bash-style incremental search through input history.
- **External editor** (`Ctrl+G`) respects `$EDITOR` for composing long prompts.
- **Approval modal** for tool permissions, with a YOLO toggle (`Shift+Tab`) for trusted sessions.
- **Copy mode** (`Ctrl+Y`)—enter a text selection mode that copies to the system clipboard via OSC 52, with tmux passthrough and arboard fallback for terminals without OSC 52 support.
- **Subagent zoom** (`Ctrl+Z`)—zoom into a spawned subagent conversation with tail-follow scrolling. Actively being polished; expect rough edges.
- **Effort slider**—overlay to adjust model effort level (low/med/high) without leaving the conversation.
- **Slash commands**—`/help`, `/new`, `/sessions`, `/title`, `/save`, `/clear`, `/model`, `/compact`, `/yolo`, and more.
- **ACP reconnect** on server crash, with a dedicated disconnected screen.
- **Status bar** with token usage, context-window health bar, and mode indicators.
- **Mouse support**—scroll wheel, cursor-following selection, session picker navigation.

## Key bindings

| Key | Action |
|---|---|
| `Ctrl+P` | Command palette |
| `Ctrl+B` | Back to session picker |
| `Ctrl+R` | Reverse history search |
| `Ctrl+O` | Toggle thinking display |
| `Ctrl+G` | Open `$EDITOR` for input |
| `Ctrl+L` | Clear screen |
| `Ctrl+C` | Cancel current turn |
| `Ctrl+D` | Quit |
| `Esc Esc` | Undo last turn |
| `Shift+Tab` | Toggle YOLO (auto-approve) |
| `Ctrl+Y` | Copy mode (OSC 52 clipboard) |
| `Ctrl+Z` | Zoom into subagent view |
| `@` | File autocomplete |
| `!` | Shell escape prefix |
| `/` | Slash command (with `Tab` completion) |

## Project layout

```
src/
├─ main.rs              # terminal setup, ACP spawn, event loop dispatch
├─ acp.rs               # JSON-RPC client: subprocess management
├─ event.rs             # async event loop: keys, mouse, ticks, ACP events
├─ app.rs               # state, Screen enum, ModalState, key handlers (heavyweight)
├─ ui.rs                # top-level draw dispatch, chat view, markdown rendering (heavyweight)
├─ ui_picker.rs         # session picker screen
├─ ui_modal.rs          # approval modal overlay
├─ ui_palette.rs        # command palette (Ctrl+P)
├─ ui_effort.rs         # effort slider overlay (low/med/high)
├─ ui_copy_mode.rs      # copy mode: text selection + OSC 52 clipboard (Ctrl+Y)
├─ ui_subagent_zoom.rs  # subagent zoom view with tail-follow scrolling (Ctrl+Z)
├─ ui_search.rs         # reverse history search (Ctrl+R)
└─ ui_file_popup.rs     # @ file mention autocomplete
```

## Development

```bash
cargo build --release   # optimized binary
cargo test              # run tests
cargo clippy            # lint
```

Kaishi requires a true-color terminal. OSC 52 clipboard copy mode works on kitty, wezterm, alacritty, foot, iTerm2, and Ghostty; tmux requires passthrough enabled. The arboard fallback covers terminals without native OSC 52 support.

## License

MIT © EvaL3n4—see [LICENSE](./LICENSE).
