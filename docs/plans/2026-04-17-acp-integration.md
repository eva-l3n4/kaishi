# ACP Integration Implementation Plan

> **For Hermes:** Use subagent-driven-development skill to implement this plan
> task-by-task.

**Goal:** Replace the HTTP/SSE gateway client with ACP over stdio, adding a
session picker, modal approvals, and structured tool/thinking events.

**Architecture:** The TUI spawns `hermes acp` as a child process, communicates
via JSON-RPC on stdin/stdout. A reader task parses incoming messages and pushes
them as `AppEvent` variants into the existing event channel. The app dispatches
based on `Screen` (Picker/Chat) and `ModalState`.

**Tech Stack:** Rust 1.95, ratatui 0.29, crossterm 0.28, tokio (full), serde,
serde_json, anyhow

**Design spec:** `docs/2026-04-17-acp-integration-design.md`

---

## Phase 1: Dependencies and Types

### Task 1: Update Cargo.toml — swap HTTP deps for process-only stack

**Objective:** Remove reqwest/SSE dependencies, keep what we need.

**Files:**
- Modify: `Cargo.toml`

**Steps:**

1. Replace the current `Cargo.toml` contents with:

```toml
[package]
name = "hermes-tui"
version = "0.2.0"
edition = "2021"
description = "Terminal UI for Hermes Agent"

[dependencies]
# TUI framework
ratatui = { version = "0.29", features = ["crossterm"] }
crossterm = { version = "0.28", features = ["event-stream"] }

# Async runtime (process spawning, channels, IO)
tokio = { version = "1", features = ["full"] }

# Serialization (JSON-RPC framing)
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Misc
uuid = { version = "1", features = ["v4"] }
dirs = "6"
anyhow = "1"
```

2. Verify: `cd /home/opus/hermes-tui && cargo check 2>&1`
   - Expected: compilation errors in `api.rs` (reqwest gone). That's correct —
     we're about to replace it.

3. Commit:
```bash
git add Cargo.toml
git commit -m "chore: swap HTTP deps for ACP-only stack

Remove reqwest, reqwest-eventsource, futures, termimad.
Bump version to 0.2.0 for ACP migration."
```

---

### Task 2: Define new event types in `event.rs`

**Objective:** Replace HTTP-centric events with ACP-native variants.

**Files:**
- Rewrite: `src/event.rs`

**Steps:**

1. Replace `src/event.rs` with:

```rust
use anyhow::Result;
use crossterm::event::{Event, KeyEvent, MouseEventKind};
use std::time::Duration;
use tokio::sync::mpsc;

/// Token usage from a completed prompt.
#[derive(Debug, Clone)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// An option in an approval request.
#[derive(Debug, Clone)]
pub struct ApprovalOption {
    pub id: String,
    pub name: String,
}

/// Lightweight session info for the picker.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub session_id: String,
    pub cwd: String,
    pub model: String,
    pub history_len: usize,
    // Future upstream enrichment:
    // pub title: Option<String>,
    // pub started_at: Option<f64>,
    // pub last_active: Option<f64>,
    // pub source: Option<String>,
}

/// Events the UI loop cares about.
#[derive(Debug)]
pub enum AppEvent {
    // Terminal events
    Key(KeyEvent),
    Tick,
    MouseScroll(i16),
    Resize(u16, u16),

    // ACP agent events
    AgentMessage(String),
    AgentThought(String),
    ToolCallStart {
        id: String,
        name: String,
        kind: Option<String>,
    },
    ToolCallUpdate {
        id: String,
        status: String,
        content: Option<String>,
    },
    PromptDone {
        stop_reason: String,
        usage: Option<Usage>,
    },

    // Approval (server-to-client JSON-RPC request)
    ApprovalRequest {
        request_id: serde_json::Value,
        command: String,
        options: Vec<ApprovalOption>,
    },

    // ACP lifecycle
    AcpReady,
    AcpError(String),
    SessionsLoaded(Vec<SessionInfo>),
    SessionCreated(String),
    SessionResumed(String),

    // Slash command responses from ACP server
    SlashCommandResponse(String),
}

pub struct EventLoop {
    rx: mpsc::UnboundedReceiver<AppEvent>,
    tx: mpsc::UnboundedSender<AppEvent>,
}

impl EventLoop {
    pub fn new(tick_ms: u64) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let event_tx = tx.clone();

        tokio::spawn(async move {
            let mut reader = crossterm::event::EventStream::new();
            use futures_lite::StreamExt;
            loop {
                let tick_delay = tokio::time::sleep(Duration::from_millis(tick_ms));
                tokio::select! {
                    maybe_event = reader.next() => {
                        match maybe_event {
                            Some(Ok(Event::Key(key))) => {
                                if event_tx.send(AppEvent::Key(key)).is_err() {
                                    break;
                                }
                            }
                            Some(Ok(Event::Mouse(mouse))) => {
                                let evt = match mouse.kind {
                                    MouseEventKind::ScrollUp => Some(AppEvent::MouseScroll(3)),
                                    MouseEventKind::ScrollDown => Some(AppEvent::MouseScroll(-3)),
                                    _ => None,
                                };
                                if let Some(e) = evt {
                                    if event_tx.send(e).is_err() {
                                        break;
                                    }
                                }
                            }
                            Some(Ok(Event::Resize(w, h))) => {
                                let _ = event_tx.send(AppEvent::Resize(w, h));
                            }
                            Some(Ok(_)) => {}
                            Some(Err(_)) => break,
                            None => break,
                        }
                    }
                    _ = tick_delay => {
                        if event_tx.send(AppEvent::Tick).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        Self { rx, tx }
    }

    pub async fn next(&mut self) -> Result<AppEvent> {
        self.rx
            .recv()
            .await
            .ok_or_else(|| anyhow::anyhow!("event channel closed"))
    }

    /// Get a sender for injecting ACP events from the reader task.
    pub fn sender(&self) -> mpsc::UnboundedSender<AppEvent> {
        self.tx.clone()
    }
}
```

Note: We use `crossterm::event::EventStream` which needs the `event-stream`
feature (already enabled). The `futures_lite` import for `StreamExt` — we need
to add `futures-lite` as a lightweight replacement since we dropped `futures`.
Add to Cargo.toml:

```toml
futures-lite = "2"
```

2. Verify: `cargo check 2>&1` — will still error on api.rs/app.rs (expected).

3. Commit:
```bash
git add src/event.rs Cargo.toml
git commit -m "feat: ACP-native event types

Replace HTTP stream events with AgentMessage, ToolCallStart,
ApprovalRequest, SessionsLoaded, etc."
```

---

## Phase 2: ACP Client

### Task 3: Create `acp.rs` — JSON-RPC types and subprocess management

**Objective:** Build the core ACP client that spawns `hermes acp`, sends
JSON-RPC requests, and reads responses/notifications.

**Files:**
- Create: `src/acp.rs`

**Steps:**

1. Create `src/acp.rs` with the full client implementation:

```rust
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::event::{AppEvent, ApprovalOption, SessionInfo, Usage};

// -------------------------------------------------------------------
// JSON-RPC wire types
// -------------------------------------------------------------------

#[derive(Serialize)]
struct JsonRpcRequest {
    jsonrpc: &'static str,
    id: u64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(Deserialize, Debug)]
struct JsonRpcMessage {
    id: Option<Value>,
    method: Option<String>,
    params: Option<Value>,
    result: Option<Value>,
    error: Option<Value>,
}

// -------------------------------------------------------------------
// ACP Client
// -------------------------------------------------------------------

type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value>>>>>;

pub struct AcpClient {
    stdin: Arc<Mutex<tokio::process::ChildStdin>>,
    child: Arc<Mutex<Child>>,
    next_id: AtomicU64,
    pending: PendingMap,
    event_tx: mpsc::UnboundedSender<AppEvent>,
}

impl AcpClient {
    /// Spawn `hermes acp` and start the reader task.
    pub async fn spawn(
        event_tx: mpsc::UnboundedSender<AppEvent>,
        profile: Option<&str>,
    ) -> Result<Self> {
        let mut cmd = Command::new("hermes");
        cmd.arg("acp");
        if let Some(p) = profile {
            cmd.arg("--profile").arg(p);
        }
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());

        let mut child = cmd.spawn().context("Failed to spawn `hermes acp`")?;

        let stdin = child.stdin.take().context("No stdin on child")?;
        let stdout = child.stdout.take().context("No stdout on child")?;

        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let event_tx_clone = event_tx.clone();
        let pending_clone = pending.clone();

        // Reader task: parse JSON-RPC messages from stdout
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                let msg: JsonRpcMessage = match serde_json::from_str(&line) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                Self::dispatch_message(
                    msg,
                    &pending_clone,
                    &event_tx_clone,
                ).await;
            }
            // Subprocess stdout closed — signal error
            let _ = event_tx_clone.send(AppEvent::AcpError(
                "ACP subprocess exited".into(),
            ));
        });

        Ok(Self {
            stdin: Arc::new(Mutex::new(stdin)),
            child: Arc::new(Mutex::new(child)),
            next_id: AtomicU64::new(1),
            pending,
            event_tx,
        })
    }

    /// Send a JSON-RPC request and wait for the response.
    pub async fn request(&self, method: &str, params: Option<Value>) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params,
        };
        let mut payload = serde_json::to_string(&req)?;
        payload.push('\n');

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        {
            let mut stdin = self.stdin.lock().await;
            stdin.write_all(payload.as_bytes()).await?;
            stdin.flush().await?;
        }

        rx.await?.context("ACP request failed")
    }

    /// Send a JSON-RPC notification (no response expected).
    pub async fn notify(&self, method: &str, params: Option<Value>) -> Result<()> {
        // Notifications have no id
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params.unwrap_or(Value::Null),
        });
        let mut payload = serde_json::to_string(&msg)?;
        payload.push('\n');

        let mut stdin = self.stdin.lock().await;
        stdin.write_all(payload.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    /// Send a JSON-RPC response (for server-to-client requests like
    /// request_permission).
    pub async fn respond(&self, id: Value, result: Value) -> Result<()> {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        });
        let mut payload = serde_json::to_string(&msg)?;
        payload.push('\n');

        let mut stdin = self.stdin.lock().await;
        stdin.write_all(payload.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    // ---- Dispatch incoming messages ----------------------------------------

    async fn dispatch_message(
        msg: JsonRpcMessage,
        pending: &PendingMap,
        event_tx: &mpsc::UnboundedSender<AppEvent>,
    ) {
        // Response to one of our requests
        if let Some(id_val) = &msg.id {
            if msg.method.is_none() {
                // This is a response, not a request
                if let Some(id_num) = id_val.as_u64() {
                    let mut pending = pending.lock().await;
                    if let Some(tx) = pending.remove(&id_num) {
                        if let Some(err) = msg.error {
                            let _ = tx.send(Err(anyhow::anyhow!("RPC error: {}", err)));
                        } else {
                            let _ = tx.send(Ok(msg.result.unwrap_or(Value::Null)));
                        }
                    }
                }
                return;
            }
        }

        // Server-to-client request (has method + id)
        if let (Some(method), Some(id)) = (&msg.method, &msg.id) {
            if method == "request_permission" {
                Self::handle_permission_request(id.clone(), &msg.params, event_tx);
            }
            return;
        }

        // Notification (has method, no id) — session_update events
        if let Some(method) = &msg.method {
            if method == "session_update" || method == "notifications/session_update" {
                if let Some(params) = &msg.params {
                    Self::handle_session_update(params, event_tx);
                }
            }
        }
    }

    fn handle_permission_request(
        id: Value,
        params: &Option<Value>,
        event_tx: &mpsc::UnboundedSender<AppEvent>,
    ) {
        let params = match params {
            Some(p) => p,
            None => return,
        };
        let command = params
            .pointer("/tool_call/title")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown command")
            .to_string();
        let options: Vec<ApprovalOption> = params
            .get("options")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|o| {
                        Some(ApprovalOption {
                            id: o.get("option_id")?.as_str()?.to_string(),
                            name: o.get("name")?.as_str()?.to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let _ = event_tx.send(AppEvent::ApprovalRequest {
            request_id: id,
            command,
            options,
        });
    }

    fn handle_session_update(
        params: &Value,
        event_tx: &mpsc::UnboundedSender<AppEvent>,
    ) {
        let update_type = params
            .get("sessionUpdate")
            .or_else(|| params.get("session_update"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        match update_type {
            "agent_message_chunk" => {
                if let Some(text) = params
                    .pointer("/content/text")
                    .and_then(|v| v.as_str())
                {
                    let _ = event_tx.send(AppEvent::AgentMessage(text.to_string()));
                }
            }
            "agent_thought_chunk" => {
                if let Some(text) = params
                    .pointer("/content/text")
                    .and_then(|v| v.as_str())
                {
                    let _ = event_tx.send(AppEvent::AgentThought(text.to_string()));
                }
            }
            "tool_call" => {
                let id = params
                    .get("tool_call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = params
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let kind = params
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let _ = event_tx.send(AppEvent::ToolCallStart { id, name, kind });
            }
            "tool_call_update" => {
                let id = params
                    .get("tool_call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let status = params
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let content = params
                    .pointer("/content/0/text")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let _ = event_tx.send(AppEvent::ToolCallUpdate {
                    id,
                    status,
                    content,
                });
            }
            _ => {}
        }
    }

    // ---- High-level ACP operations -----------------------------------------

    pub async fn initialize(&self) -> Result<Value> {
        self.request(
            "initialize",
            Some(serde_json::json!({
                "client_info": {
                    "name": "hermes-tui",
                    "version": env!("CARGO_PKG_VERSION"),
                },
            })),
        )
        .await
    }

    pub async fn list_sessions(&self) -> Result<Vec<SessionInfo>> {
        let result = self.request("list_sessions", Some(serde_json::json!({}))).await?;
        let sessions = result
            .get("sessions")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| {
                        Some(SessionInfo {
                            session_id: s.get("session_id")?.as_str()?.to_string(),
                            cwd: s
                                .get("cwd")
                                .and_then(|v| v.as_str())
                                .unwrap_or(".")
                                .to_string(),
                            model: s
                                .get("model")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            history_len: s
                                .get("history_len")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0) as usize,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(sessions)
    }

    pub async fn new_session(&self, cwd: &str) -> Result<String> {
        let result = self
            .request(
                "new_session",
                Some(serde_json::json!({ "cwd": cwd })),
            )
            .await?;
        let session_id = result
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(session_id)
    }

    pub async fn resume_session(&self, cwd: &str, session_id: &str) -> Result<()> {
        self.request(
            "resume_session",
            Some(serde_json::json!({
                "cwd": cwd,
                "session_id": session_id,
            })),
        )
        .await?;
        Ok(())
    }

    pub async fn prompt(&self, text: &str, session_id: &str) -> Result<Value> {
        self.request(
            "prompt",
            Some(serde_json::json!({
                "session_id": session_id,
                "prompt": [{ "type": "text", "text": text }],
            })),
        )
        .await
    }

    pub async fn cancel(&self, session_id: &str) -> Result<()> {
        self.notify(
            "cancel",
            Some(serde_json::json!({ "session_id": session_id })),
        )
        .await
    }

    /// Kill the subprocess on drop.
    pub async fn shutdown(&self) {
        let mut child = self.child.lock().await;
        let _ = child.kill().await;
    }
}
```

2. Add `mod acp;` to `src/main.rs` (temporarily alongside old modules).

3. Verify: `cargo check 2>&1` — `acp.rs` should compile cleanly.

4. Commit:
```bash
git add src/acp.rs
git commit -m "feat: ACP JSON-RPC client

Spawns hermes acp subprocess, sends/receives JSON-RPC messages,
dispatches session_update notifications as AppEvents, handles
bidirectional request_permission for approval modal."
```

---

## Phase 3: UI Modules

### Task 4: Create `ui_picker.rs` — session picker screen

**Objective:** Build the full-screen session picker shown on launch.

**Files:**
- Create: `src/ui_picker.rs`

**Steps:**

1. Create `src/ui_picker.rs`:

```rust
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::event::SessionInfo;

/// Draw the session picker screen.
pub fn draw_picker(
    frame: &mut Frame,
    sessions: &[SessionInfo],
    selected: usize,
) {
    let area = frame.area();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(" 🌸 Hanami ");

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));

    // "New Session" is always first (index 0)
    let total_items = 1 + sessions.len();

    // New Session entry
    let marker = if selected == 0 { "  > " } else { "    " };
    let style = if selected == 0 {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    lines.push(Line::from(Span::styled(
        format!("{}New Session", marker),
        style,
    )));

    // Existing sessions
    for (i, session) in sessions.iter().enumerate() {
        let idx = i + 1; // offset by New Session
        let marker = if selected == idx { "  > " } else { "    " };

        let label = if session.cwd != "." {
            // Show last path component
            session
                .cwd
                .rsplit('/')
                .next()
                .unwrap_or(&session.cwd)
                .to_string()
        } else {
            session.session_id[..8.min(session.session_id.len())].to_string()
        };

        let detail = format!(
            "{} msgs",
            session.history_len,
        );

        let style = if selected == idx {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        let detail_style = if selected == idx {
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        lines.push(Line::from(vec![
            Span::styled(format!("{}{:<40}", marker, label), style),
            Span::styled(format!("  {}", detail), detail_style),
        ]));
    }

    lines.push(Line::from(""));

    let hint = Line::from(Span::styled(
        "  Enter: select  Esc: quit",
        Style::default().fg(Color::DarkGray),
    ));
    lines.push(hint);

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}
```

2. Verify: add `mod ui_picker;` to main.rs, `cargo check`.

3. Commit:
```bash
git add src/ui_picker.rs
git commit -m "feat: session picker UI

Full-screen picker with arrow key navigation, New Session at top."
```

---

### Task 5: Create `ui_modal.rs` — approval modal overlay

**Objective:** Build the centered modal overlay for dangerous command approvals.

**Files:**
- Create: `src/ui_modal.rs`

**Steps:**

1. Create `src/ui_modal.rs`:

```rust
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::event::ApprovalOption;

/// Draw the approval modal centered on screen.
pub fn draw_approval_modal(
    frame: &mut Frame,
    command: &str,
    options: &[ApprovalOption],
    selected: usize,
) {
    let area = frame.area();

    // Calculate modal size
    let modal_width = 50u16.min(area.width.saturating_sub(4));
    let modal_height = (options.len() as u16 + 6).min(area.height.saturating_sub(2));

    let modal_area = centered_rect(modal_width, modal_height, area);

    // Clear the area behind the modal
    frame.render_widget(Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(" Approval Required ");

    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));

    // Command preview (truncate if needed)
    let cmd_display = if command.len() > (modal_width as usize - 6) {
        format!("  {}...", &command[..modal_width as usize - 9])
    } else {
        format!("  {}", command)
    };
    lines.push(Line::from(Span::styled(
        cmd_display,
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
    )));

    lines.push(Line::from(""));

    // Options
    for (i, opt) in options.iter().enumerate() {
        let marker = if i == selected { "  > " } else { "    " };
        let style = if i == selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(Line::from(Span::styled(
            format!("{}{}", marker, opt.name),
            style,
        )));
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

/// Create a centered Rect of given size within `area`.
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length((area.height.saturating_sub(height)) / 2),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(area);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length((area.width.saturating_sub(width)) / 2),
            Constraint::Length(width),
            Constraint::Min(0),
        ])
        .split(vertical[1]);

    horizontal[1]
}
```

2. Verify: add `mod ui_modal;` to main.rs, `cargo check`.

3. Commit:
```bash
git add src/ui_modal.rs
git commit -m "feat: approval modal overlay

Centered modal with command preview and arrow-key option selection."
```

---

## Phase 4: Core Rewrite

### Task 6: Rewrite `app.rs` — Screen, ModalState, ACP-driven handlers

**Objective:** Replace the HTTP-based app state with ACP-driven state
management, Screen enum, and ModalState.

**Files:**
- Rewrite: `src/app.rs`

**Steps:**

This is the largest task. The key changes:

- `App` no longer owns an `HermesClient` — it communicates via `AcpClient`
  (stored outside App, referenced by event_tx)
- `Screen` enum controls which view renders
- `ModalState` captures approval requests
- Chat input sends `prompt()` via a tokio task
- Slash commands are split: local ones handled in app, ACP ones forwarded
  as `prompt("/command")`

1. Rewrite `src/app.rs` — keeping the existing keybinding logic and
   chat message types, but replacing the HTTP plumbing:

Key structural changes to make:

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
        request_id: serde_json::Value,
    },
}

pub struct App {
    pub screen: Screen,
    pub modal: ModalState,

    // Session
    pub session_id: Option<String>,
    pub sessions: Vec<SessionInfo>,
    pub picker_selected: usize,

    // Chat (same as before)
    pub messages: Vec<ChatMessage>,
    pub input: String,
    pub cursor: usize,
    pub scroll_offset: u16,
    pub status: AgentStatus,
    pub pending_response: String,

    // Display
    pub model_name: String,
    pub session_title: Option<String>,
    pub tick: u64,
    pub verbose: bool,

    // Event channel for sending ACP requests
    pub event_tx: Option<mpsc::UnboundedSender<AppEvent>>,

    quit: bool,
}
```

The `handle_key` method needs to dispatch based on `screen` and `modal`:

```rust
pub async fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
    // Modal takes priority
    if let ModalState::Approval { .. } = &self.modal {
        return self.handle_modal_key(key).await;
    }

    match self.screen {
        Screen::Picker => self.handle_picker_key(key).await,
        Screen::Chat => self.handle_chat_key(key).await,
    }
}
```

Picker key handler: arrow up/down/j/k moves `picker_selected`, Enter sends
`SessionCreated` or `SessionResumed` event, Esc quits.

Chat key handler: same as current `handle_key` minus the HTTP streaming —
on Enter, push user message and send `prompt()` via tokio task.

Modal key handler: arrow up/down moves `selected`, Enter sends
`respond()` via tokio task, Esc sends deny.

Stream event handlers (`handle_stream_delta`, etc.) become ACP event
handlers (`handle_agent_message`, `handle_tool_call_start`, etc.).

2. Verify: `cargo check 2>&1`

3. Commit:
```bash
git add src/app.rs
git commit -m "feat: ACP-driven app state

Screen enum (Picker/Chat), ModalState for approvals,
ACP event handlers replacing HTTP stream handlers."
```

---

### Task 7: Update `ui.rs` — top-level dispatch and shared helpers

**Objective:** Make `ui.rs` dispatch to the correct screen renderer, keep chat
view and markdown rendering, remove assumptions about single-screen mode.

**Files:**
- Modify: `src/ui.rs`

**Steps:**

1. Add a top-level `draw` function that dispatches:

```rust
pub fn draw(frame: &mut Frame, app: &App) {
    match app.screen {
        Screen::Picker => {
            ui_picker::draw_picker(frame, &app.sessions, app.picker_selected);
        }
        Screen::Chat => {
            draw_chat(frame, app);
        }
    }

    // Modal overlay (drawn on top of any screen)
    if let ModalState::Approval {
        ref command,
        ref options,
        selected,
        ..
    } = app.modal
    {
        ui_modal::draw_approval_modal(frame, command, options, selected);
    }
}
```

2. Rename the existing `draw` function to `draw_chat` (the main chat view).
   Keep all the markdown rendering, status bar, input box, pre-wrap logic
   as-is — it all still works.

3. Update the status bar to use "🌸 Hanami" instead of "🌸 Hermes".

4. Verify: `cargo check`

5. Commit:
```bash
git add src/ui.rs
git commit -m "refactor: ui.rs dispatches to picker/chat/modal

Top-level draw() routes by Screen enum, modal overlays on top.
Rename draw -> draw_chat for the chat view."
```

---

### Task 8: Rewrite `main.rs` — ACP startup flow

**Objective:** Wire everything together: spawn ACP, initialize, show picker,
run event loop.

**Files:**
- Rewrite: `src/main.rs`

**Steps:**

1. Rewrite `src/main.rs`:

```rust
mod acp;
mod app;
mod event;
mod ui;
mod ui_modal;
mod ui_picker;

use anyhow::Result;
use app::App;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{
        EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    },
};
use event::EventLoop;
use ratatui::prelude::*;
use std::io;

#[tokio::main]
async fn main() -> Result<()> {
    // Parse args
    let profile = std::env::var("HERMES_PROFILE").ok();
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());

    // Terminal setup
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // Event loop + ACP client
    let mut events = EventLoop::new(250);
    let event_tx = events.sender();

    let acp = acp::AcpClient::spawn(
        event_tx.clone(),
        profile.as_deref(),
    )
    .await?;

    // Initialize ACP handshake
    let init_result = acp.initialize().await;
    if let Err(e) = &init_result {
        eprintln!("ACP initialize failed: {}", e);
    }

    // Fetch sessions for the picker
    let sessions = acp.list_sessions().await.unwrap_or_default();

    // Create app
    let mut app = App::new(sessions);
    app.event_tx = Some(event_tx);

    let result = run(&mut terminal, &mut app, &mut events, &acp, &cwd).await;

    // Cleanup
    acp.shutdown().await;
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

async fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    events: &mut EventLoop,
    acp: &acp::AcpClient,
    cwd: &str,
) -> Result<()> {
    loop {
        terminal.draw(|frame| ui::draw(frame, app))?;

        match events.next().await? {
            event::AppEvent::Key(key) => {
                app.handle_key(key, acp, cwd).await?;
            }
            event::AppEvent::Tick => {
                app.tick();
            }
            event::AppEvent::MouseScroll(delta) => {
                app.handle_scroll(delta);
            }
            event::AppEvent::Resize(_, _) => {}

            // ACP events
            event::AppEvent::AgentMessage(text) => {
                app.handle_agent_message(&text);
            }
            event::AppEvent::AgentThought(text) => {
                app.handle_agent_thought(&text);
            }
            event::AppEvent::ToolCallStart { id, name, kind } => {
                app.handle_tool_start(&id, &name, kind.as_deref());
            }
            event::AppEvent::ToolCallUpdate {
                id,
                status,
                content,
            } => {
                app.handle_tool_update(&id, &status, content.as_deref());
            }
            event::AppEvent::PromptDone { stop_reason, usage } => {
                app.handle_prompt_done(&stop_reason, usage);
            }
            event::AppEvent::ApprovalRequest {
                request_id,
                command,
                options,
            } => {
                app.show_approval_modal(request_id, command, options);
            }
            event::AppEvent::AcpError(err) => {
                app.sys_msg(format!("ACP error: {}", err));
            }
            event::AppEvent::SlashCommandResponse(text) => {
                app.sys_msg(text);
            }
            _ => {}
        }

        if app.should_quit() {
            return Ok(());
        }
    }
}
```

2. Verify: `cargo check`

3. Commit:
```bash
git add src/main.rs
git commit -m "feat: ACP startup flow in main.rs

Spawn hermes acp, initialize, fetch sessions, show picker,
dispatch ACP events in the main loop."
```

---

## Phase 5: Cleanup

### Task 9: Remove old `api.rs`

**Objective:** Delete the HTTP client module now that ACP replaces it.

**Files:**
- Delete: `src/api.rs`

**Steps:**

1. Remove `src/api.rs`
2. Remove `mod api;` from any file that still references it
3. Verify: `cargo check` — clean build, no warnings about dead code
4. Commit:
```bash
git rm src/api.rs
git commit -m "chore: remove old HTTP API client

Replaced by ACP JSON-RPC client in acp.rs."
```

---

### Task 10: Full build + clippy + manual test

**Objective:** Verify the complete build, fix warnings, and do a smoke test.

**Files:**
- Potentially any file (clippy fixes)

**Steps:**

1. Run full build:
```bash
cargo build --release 2>&1
```

2. Run clippy:
```bash
cargo clippy --all-targets -- -D warnings 2>&1
```

3. Fix any warnings/errors.

4. Smoke test — run the TUI:
```bash
HERMES_PROFILE=hanami ./target/release/hermes-tui
```

Expected behavior:
- Session picker appears with "🌸 Hanami" header
- Arrow keys navigate, Enter selects "New Session"
- Chat mode enters, can type a message
- Agent responds with streamed text
- `Ctrl+C` exits cleanly

5. Final commit:
```bash
git add -A
git commit -m "feat: hermes-tui v0.2.0 — ACP integration

Complete migration from HTTP gateway to ACP over stdio.
Session picker, modal approvals, structured events."
git tag v0.2.0
```

6. Push:
```bash
git push origin main --tags
```

---

## Task Summary

| # | Task | Phase | Files |
|---|------|-------|-------|
| 1 | Update Cargo.toml | Deps | Cargo.toml |
| 2 | New event types | Deps | event.rs |
| 3 | ACP JSON-RPC client | ACP | acp.rs (new) |
| 4 | Session picker UI | UI | ui_picker.rs (new) |
| 5 | Approval modal UI | UI | ui_modal.rs (new) |
| 6 | Rewrite app.rs | Core | app.rs |
| 7 | Update ui.rs dispatch | Core | ui.rs |
| 8 | Rewrite main.rs | Core | main.rs |
| 9 | Remove old api.rs | Cleanup | api.rs (delete) |
| 10 | Build + clippy + test | Verify | any |

**Estimated total:** 10 tasks, roughly 2-3 hours of focused work.
Tasks 1-5 can be done with cargo check passing incrementally.
Tasks 6-8 are the core rewrite and may need iteration.
Tasks 9-10 are cleanup and verification.
