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
#[allow(dead_code)]
pub struct SessionInfo {
    pub session_id: String,
    pub cwd: String,
    pub model: String,
    pub history_len: usize,
    pub title: Option<String>,
    pub started_at: Option<f64>,
    pub last_active: Option<f64>,
    pub source: Option<String>,
}

/// Events the UI loop cares about.
#[derive(Debug)]
#[allow(dead_code)]
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
        input: Option<String>,
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

    // History replay
    HistoryLoaded(Vec<(String, String)>, usize), // (messages, total)
    HistoryFallback(String),

    // Lazy loading
    LoadMoreHistory,
    HistoryPage(Vec<(String, String)>, usize), // (older messages, total)

    // Reconnect
    ReconnectRequested,

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
