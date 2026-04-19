use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::Line as RatLine;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::acp::AcpClient;
use crate::event::{AppEvent, ApprovalOption, SessionInfo, Usage};

/// All known slash commands for tab completion.
const SLASH_COMMANDS: &[&str] = &[
    "/clear", "/compact", "/context", "/effort", "/exit", "/help", "/model",
    "/new", "/quit", "/reset", "/save", "/sessions", "/title", "/tools",
    "/usage", "/verbose", "/version", "/yolo",
];

/// Visible role tag for messages in the conversation.
#[derive(Debug, Clone, PartialEq)]
pub enum Role {
    User,
    Assistant,
    System,
    Tool,
    Thought,
}

/// A single message in the conversation view.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    pub tokens: Option<Usage>,
}

/// What the assistant is currently doing (gates input acceptance).
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum AgentStatus {
    Idle,
    Thinking,
    Error(String),
}

/// What the agent is actively doing (drives spinner animation).
#[derive(Debug, Clone, PartialEq)]
pub enum AgentPhase {
    Idle,
    Thinking,
    Streaming,
    Executing,
}

/// Animation state for the spinner line — updated on AnimationTick.
pub struct AnimationState {
    /// Index into the bounce sequence.
    pub frame: usize,
    /// Sub-counter for spinner advancement (~120ms).
    pub spinner_tick: u8,
    /// Current agent phase.
    pub phase: AgentPhase,
    /// When the current phase began.
    pub phase_start: std::time::Instant,
    /// Last time content was received (future: stall detection).
    pub last_output: std::time::Instant,
    /// Shimmer highlight position (which char in the label).
    pub shimmer_pos: usize,
    /// Sub-counter for shimmer advancement (~150ms).
    pub shimmer_tick: u8,
    /// Name of currently executing tool.
    pub active_tool: Option<String>,
    /// When the current turn (prompt) started.
    pub turn_start: Option<std::time::Instant>,
    /// Stall intensity: 0.0 = normal, 0.5 = warning (yellow), 1.0 = stalled (red).
    pub stall_intensity: f32,
    /// Label for the current phase (randomly selected on entry).
    pub phase_label: &'static str,
}

impl AnimationState {
    pub fn new() -> Self {
        let now = std::time::Instant::now();
        Self {
            frame: 0,
            spinner_tick: 0,
            phase: AgentPhase::Idle,
            phase_start: now,
            last_output: now,
            shimmer_pos: 0,
            shimmer_tick: 0,
            active_tool: None,
            turn_start: None,
            stall_intensity: 0.0,
            phase_label: "",
        }
    }

    /// Transition to a new phase, resetting timers.
    pub fn set_phase(&mut self, phase: AgentPhase) {
        if self.phase != phase {
            // Pick a label from the word bank based on frame counter (pseudo-random)
            self.phase_label = match phase {
                AgentPhase::Thinking => {
                    const WORDS: &[&str] = &["thinking", "pondering", "reasoning", "considering"];
                    WORDS[self.frame % WORDS.len()]
                }
                AgentPhase::Streaming => {
                    const WORDS: &[&str] = &["streaming", "composing", "writing", "replying"];
                    WORDS[self.frame % WORDS.len()]
                }
                AgentPhase::Executing => {
                    const WORDS: &[&str] = &["executing", "running", "working", "processing"];
                    WORDS[self.frame % WORDS.len()]
                }
                AgentPhase::Idle => "",
            };
            self.phase = phase;
            self.phase_start = std::time::Instant::now();
            self.shimmer_pos = 0;
            self.shimmer_tick = 0;
            self.stall_intensity = 0.0;
        }
    }
}

/// Which screen is active.
#[derive(Debug, Clone, PartialEq)]
pub enum Screen {
    Picker,
    Chat,
    Disconnected(String), // error message
}

#[derive(Debug, Clone)]
pub struct PaletteEntry {
    pub label: String,
    pub keybind: Option<String>,
    pub action: PaletteAction,
}

#[derive(Debug, Clone)]
pub enum PaletteAction {
    SlashCommand(String),
    Keybind(String),
    SetEffort(u8),
}

/// Modal overlay state.
#[derive(Debug)]
pub enum ModalState {
    None,
    Approval {
        command: String,
        options: Vec<ApprovalOption>,
        selected: usize,
        request_id: serde_json::Value,
    },
    CommandPalette {
        query: String,
        cursor: usize,
        selected: usize,
        filtered: Vec<PaletteEntry>,
    },
    EffortSlider {
        level: u8, // 0=low, 1=medium, 2=high
    },
    ReverseSearch {
        query: String,
        cursor: usize,
        match_index: Option<usize>,
    },
    FileAutocomplete {
        query: String,
        cursor_in_input: usize,
        selected: usize,
        entries: Vec<String>,
        loading: bool,
    },
}

/// Fuzzy subsequence match (case-insensitive).
fn fuzzy_matches(query: &str, label: &str) -> bool {
    if query.is_empty() { return true; }
    let q = query.to_lowercase();
    let l = label.to_lowercase();
    let mut chars = q.chars();
    let mut current = chars.next();
    for c in l.chars() {
        if let Some(qc) = current {
            if c == qc { current = chars.next(); }
        } else { break; }
    }
    current.is_none()
}

fn scan_files(cwd: &str, query: &str, limit: usize) -> Vec<String> {
    let mut results = Vec::new();
    let walker = walkdir::WalkDir::new(cwd)
        .max_depth(4)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !name.starts_with('.') && name != "node_modules" && name != "target"
        });
    for entry in walker.flatten() {
        if entry.file_type().is_file() {
            if let Ok(rel) = entry.path().strip_prefix(cwd) {
                let s = rel.to_string_lossy().to_string();
                if query.is_empty() || fuzzy_matches(query, &s) {
                    results.push(s);
                    if results.len() >= limit { break; }
                }
            }
        }
    }
    results
}

/// Application state.
pub struct App {
    pub screen: Screen,
    pub modal: ModalState,

    // Session picker
    pub sessions: Vec<SessionInfo>,
    pub picker_selected: usize,
    pub picker_scroll_offset: u16,
    /// Last-observed height of the picker list area, stashed by the
    /// renderer so keyboard handlers can scroll-follow the cursor.
    pub picker_viewport_rows: u16,

    // Active session
    pub session_id: Option<String>,
    pub cwd: String,

    // Chat
    pub messages: Vec<ChatMessage>,
    pub input: String,
    pub cursor: usize,
    pub scroll_offset: u16,
    pub status: AgentStatus,
    pub pending_response: String,
    pub pending_thought: String,

    // Display
    pub model_name: String,
    pub session_title: Option<String>,
    pub tick: u64,
    pub verbose: bool,
    pub show_thinking: bool,
    pub animation: AnimationState,

    // Event channel for sending ACP requests
    pub event_tx: Option<mpsc::UnboundedSender<AppEvent>>,

    // Active tool calls (for status display)
    pub active_tools: Vec<(String, String)>, // (id, name)
    pub tool_msg_map: HashMap<String, usize>, // tool_call_id → message index

    // Input history
    pub input_history: Vec<String>,
    pub history_index: Option<usize>,
    saved_input: String,

    // Rendered line cache (per-message, pre-wrapped)
    pub line_cache: Vec<Vec<RatLine<'static>>>,
    pub cache_width: usize,

    // History pagination
    pub history_total: usize,
    pub history_loaded: usize,
    pub loading_more_history: bool,

    // Token tracking
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub prompt_count: u32,

    // Context window tracking
    pub context_used: u64,
    pub context_max: u64,

    // Effort level (0=low, 1=medium, 2=high)
    pub effort_level: u8,

    // Approval bypass
    pub yolo_mode: bool,

    // External editor request
    pub editor_requested: bool,

    // Undo: track message count before each prompt for rewind
    pub undo_checkpoints: Vec<usize>,
    pub last_esc: Option<std::time::Instant>,

    quit: bool,
}

impl App {
    pub fn new(sessions: Vec<SessionInfo>) -> Self {
        Self {
            screen: Screen::Picker,
            modal: ModalState::None,
            sessions,
            picker_selected: 0,
            picker_scroll_offset: 0,
            picker_viewport_rows: 0,
            session_id: None,
            cwd: String::new(),
            messages: vec![ChatMessage {
                role: Role::System,
                content: "Welcome to 懐紙 Kaishi. Type a message or /help for commands."
                    .into(),
                tokens: None,
            }],
            input: String::new(),
            cursor: 0,
            scroll_offset: 0,
            status: AgentStatus::Idle,
            pending_response: String::new(),
            pending_thought: String::new(),
            model_name: String::new(),
            session_title: None,
            tick: 0,
            verbose: false,
            show_thinking: false,
            animation: AnimationState::new(),
            event_tx: None,
            active_tools: Vec::new(),
            tool_msg_map: HashMap::new(),
            input_history: Vec::new(),
            history_index: None,
            saved_input: String::new(),
            line_cache: Vec::new(),
            cache_width: 0,
            history_total: 0,
            history_loaded: 0,
            loading_more_history: false,
            total_input_tokens: 0,
            total_output_tokens: 0,
            prompt_count: 0,
            context_used: 0,
            context_max: 200_000,  // default for Claude
            effort_level: 2,       // default: high
            yolo_mode: false,
            editor_requested: false,
            undo_checkpoints: Vec::new(),
            last_esc: None,
            quit: false,
        }
    }

    pub fn close_modal(&mut self) {
        self.modal = ModalState::None;
    }

    pub fn undo_last_turn(&mut self) {
        if let Some(checkpoint) = self.undo_checkpoints.pop() {
            self.messages.truncate(checkpoint);
            self.line_cache.truncate(checkpoint);
            self.sys_msg("↩ Reverted last turn");
        } else {
            self.sys_msg("Nothing to undo");
        }
    }

    pub fn build_palette_entries() -> Vec<PaletteEntry> {
        vec![
            PaletteEntry { label: "New session".into(), keybind: None, action: PaletteAction::SlashCommand("/new".into()) },
            PaletteEntry { label: "Switch session".into(), keybind: Some("Ctrl+B".into()), action: PaletteAction::SlashCommand("/sessions".into()) },
            PaletteEntry { label: "Compact context".into(), keybind: None, action: PaletteAction::SlashCommand("/compact".into()) },
            PaletteEntry { label: "Save session".into(), keybind: None, action: PaletteAction::SlashCommand("/save".into()) },
            PaletteEntry { label: "Toggle YOLO mode".into(), keybind: Some("Shift+Tab".into()), action: PaletteAction::Keybind("toggle_yolo".into()) },
            PaletteEntry { label: "Toggle verbose".into(), keybind: None, action: PaletteAction::SlashCommand("/verbose".into()) },
            PaletteEntry { label: "Toggle thinking display".into(), keybind: Some("Ctrl+O".into()), action: PaletteAction::Keybind("toggle_thinking".into()) },
            PaletteEntry { label: "Open external editor".into(), keybind: Some("Ctrl+G".into()), action: PaletteAction::Keybind("editor".into()) },
            PaletteEntry { label: "Set effort: low".into(), keybind: None, action: PaletteAction::SetEffort(0) },
            PaletteEntry { label: "Set effort: medium".into(), keybind: None, action: PaletteAction::SetEffort(1) },
            PaletteEntry { label: "Set effort: high".into(), keybind: None, action: PaletteAction::SetEffort(2) },
            PaletteEntry { label: "Clear screen".into(), keybind: Some("Ctrl+L".into()), action: PaletteAction::Keybind("clear".into()) },
            PaletteEntry { label: "Show help".into(), keybind: None, action: PaletteAction::SlashCommand("/help".into()) },
            PaletteEntry { label: "Show usage".into(), keybind: None, action: PaletteAction::SlashCommand("/usage".into()) },
            PaletteEntry { label: "Show tools".into(), keybind: None, action: PaletteAction::SlashCommand("/tools".into()) },
            PaletteEntry { label: "Undo last turn".into(), keybind: Some("Esc Esc".into()), action: PaletteAction::Keybind("undo".into()) },
            PaletteEntry { label: "Quit".into(), keybind: Some("Ctrl+D".into()), action: PaletteAction::SlashCommand("/quit".into()) },
        ]
    }

    pub fn should_quit(&self) -> bool {
        self.quit
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    /// Advance animation counters (called every 50ms).
    pub fn handle_animation_tick(&mut self) {
        if self.animation.phase == AgentPhase::Idle {
            return;
        }

        // Stall detection: compute intensity based on silence duration
        // Executing phase is exempt — tools can legitimately take a long time
        if self.animation.phase != AgentPhase::Executing {
            let silence = self.animation.last_output.elapsed().as_secs_f32();
            self.animation.stall_intensity = if silence < 10.0 {
                0.0
            } else if silence < 20.0 {
                (silence - 10.0) / 10.0 // 0.0 → 1.0 over 10s
            } else {
                1.0
            };
        } else {
            self.animation.stall_intensity = 0.0;
        }

        // Spinner glyph: advance every ~120ms (every 2-3 ticks at 50ms)
        self.animation.spinner_tick += 1;
        if self.animation.spinner_tick >= 2 {
            self.animation.spinner_tick = 0;
            self.animation.frame = self.animation.frame.wrapping_add(1);
        }

        // Shimmer: advance every ~150ms (every 3rd tick) — freeze when stalled
        if self.animation.stall_intensity < 1.0 {
            self.animation.shimmer_tick += 1;
            if self.animation.shimmer_tick >= 3 {
                self.animation.shimmer_tick = 0;
                let label_len = self.animation.phase_label.len().max(1);
                self.animation.shimmer_pos = (self.animation.shimmer_pos + 1) % label_len;
            }
        }
    }

    /// Push a system message into the chat.
    pub fn sys_msg(&mut self, msg: impl Into<String>) {
        self.messages.push(ChatMessage {
            role: Role::System,
            content: msg.into(),
            tokens: None,
        });
        self.scroll_offset = 0;
    }

    // ---- Key dispatch -------------------------------------------------------

    pub async fn handle_key(
        &mut self,
        key: KeyEvent,
        acp: &Arc<AcpClient>,
        cwd: &str,
    ) -> Result<()> {
        // Modal overlays take priority
        match &self.modal {
            ModalState::Approval { .. } => return self.handle_modal_key(key, acp).await,
            ModalState::CommandPalette { .. } => return self.handle_palette_key(key, acp, cwd).await,
            ModalState::EffortSlider { .. } => return self.handle_effort_key(key).await,
            ModalState::ReverseSearch { .. } => return self.handle_reverse_search_key(key).await,
            ModalState::FileAutocomplete { .. } => return self.handle_file_autocomplete_key(key).await,
            ModalState::None => {}
        }

        match self.screen {
            Screen::Picker => self.handle_picker_key(key, acp, cwd).await,
            Screen::Chat => self.handle_chat_key(key, acp, cwd).await,
            Screen::Disconnected(_) => {
                // Any key quits from disconnected state
                match (key.modifiers, key.code) {
                    (_, KeyCode::Esc)
                    | (KeyModifiers::CONTROL, KeyCode::Char('c'))
                    | (KeyModifiers::CONTROL, KeyCode::Char('d')) => {
                        self.quit = true;
                    }
                    _ => {
                        // Signal reconnect request
                        if let Some(tx) = &self.event_tx {
                            let _ = tx.send(AppEvent::ReconnectRequested);
                        }
                    }
                }
                Ok(())
            }
        }
    }

    // ---- Picker key handler -------------------------------------------------

    async fn handle_picker_key(
        &mut self,
        key: KeyEvent,
        acp: &Arc<AcpClient>,
        cwd: &str,
    ) -> Result<()> {
        let total = 1 + self.sessions.len(); // New Session + existing

        match (key.modifiers, key.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('c'))
            | (_, KeyCode::Esc) => {
                self.quit = true;
            }

            (_, KeyCode::Up) | (_, KeyCode::Char('k'))
                if self.picker_selected > 0 =>
            {
                self.picker_selected -= 1;
                let rows = self.picker_viewport_rows;
                self.ensure_picker_visible(rows);
            }
            (_, KeyCode::Down) | (_, KeyCode::Char('j'))
                if self.picker_selected + 1 < total =>
            {
                self.picker_selected += 1;
                let rows = self.picker_viewport_rows;
                self.ensure_picker_visible(rows);
            }

            (_, KeyCode::Enter) => {
                if self.picker_selected == 0 {
                    // New session — switch to chat immediately, create in background
                    self.screen = Screen::Chat;
                    self.status = AgentStatus::Thinking;
                    self.sys_msg("Creating session…");

                    let acp = Arc::clone(acp);
                    let cwd = cwd.to_string();
                    let event_tx = self
                        .event_tx
                        .as_ref()
                        .expect("event_tx must be set")
                        .clone();

                    tokio::spawn(async move {
                        match acp.new_session(&cwd).await {
                            Ok(sid) => {
                                let _ = event_tx.send(AppEvent::SessionCreated(sid));
                            }
                            Err(e) => {
                                let _ = event_tx.send(AppEvent::AcpError(
                                    format!("Failed to create session: {}", e),
                                ));
                            }
                        }
                    });
                } else {
                    // Resume existing session — switch to chat immediately
                    let idx = self.picker_selected - 1;
                    if let Some(session) = self.sessions.get(idx) {
                        let sid = session.session_id.clone();
                        let _history_len = session.history_len;
                        self.session_title = session.title.clone();
                        self.model_name = if session.model.is_empty() {
                            self.model_name.clone()
                        } else {
                            session.model.clone()
                        };
                        self.screen = Screen::Chat;
                        self.status = AgentStatus::Thinking;
                        self.sys_msg("Resuming session…");

                        let acp = Arc::clone(acp);
                        let cwd = cwd.to_string();
                        let event_tx = self
                            .event_tx
                            .as_ref()
                            .expect("event_tx must be set")
                            .clone();

                        tokio::spawn(async move {
                            match acp.resume_session(&cwd, &sid).await {
                                Ok(()) => {
                                    let _ = event_tx.send(AppEvent::SessionResumed(sid));
                                }
                                Err(e) => {
                                    let _ = event_tx.send(AppEvent::AcpError(
                                        format!("Failed to resume: {}", e),
                                    ));
                                }
                            }
                        });
                    }
                }
            }

            _ => {}
        }
        Ok(())
    }

    // ---- Chat key handler ---------------------------------------------------

    async fn handle_chat_key(
        &mut self,
        key: KeyEvent,
        acp: &Arc<AcpClient>,
        cwd: &str,
    ) -> Result<()> {
        match (key.modifiers, key.code) {
            // Ctrl+C: cancel if thinking, quit if idle
            (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                if self.status == AgentStatus::Thinking {
                    if let Some(sid) = &self.session_id {
                        let acp = Arc::clone(acp);
                        let sid = sid.clone();
                        tokio::spawn(async move {
                            let _ = acp.cancel(&sid).await;
                        });
                    }
                    self.sys_msg("Cancelled.");
                    self.status = AgentStatus::Idle;
                    let content = std::mem::take(&mut self.pending_response);
                    if !content.is_empty() {
                        self.messages.push(ChatMessage {
                            role: Role::Assistant,
                            content,
                            tokens: None,
                        });
                    }
                    self.pending_thought.clear();
                    self.active_tools.clear();
                } else {
                    self.quit = true;
                }
            }
            // Ctrl+D: always quit
            (KeyModifiers::CONTROL, KeyCode::Char('d')) => {
                self.quit = true;
            }

            // Ctrl+B: back to session picker (only when input is empty)
            (KeyModifiers::CONTROL, KeyCode::Char('b')) if self.input.is_empty() => {
                self.return_to_picker();
                // Refresh session list in background so new sessions
                // created elsewhere appear on return.
                if let Some(tx) = &self.event_tx {
                    let tx = tx.clone();
                    let acp = acp.clone();
                    tokio::spawn(async move {
                        if let Ok(sessions) = acp.list_sessions().await {
                            let _ = tx.send(crate::event::AppEvent::SessionsLoaded(sessions));
                        }
                    });
                }
            }

            // Ctrl+P: command palette
            (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
                let entries = Self::build_palette_entries();
                self.modal = ModalState::CommandPalette {
                    query: String::new(),
                    cursor: 0,
                    selected: 0,
                    filtered: entries,
                };
            }

            // Ctrl+R: reverse history search
            (KeyModifiers::CONTROL, KeyCode::Char('r')) => {
                self.modal = ModalState::ReverseSearch {
                    query: String::new(),
                    cursor: 0,
                    match_index: None,
                };
            }

            // Multiline: Shift+Enter, Alt+Enter, or Ctrl+J inserts newline
            (KeyModifiers::SHIFT, KeyCode::Enter)
            | (KeyModifiers::ALT, KeyCode::Enter) => {
                self.input.insert(self.cursor, '\n');
                self.cursor += 1;
            }
            (KeyModifiers::CONTROL, KeyCode::Char('j')) => {
                self.input.insert(self.cursor, '\n');
                self.cursor += 1;
            }

            // Submit message
            (_, KeyCode::Enter) if self.status == AgentStatus::Idle => {
                let text = self.input.trim().replace('\0', "");
                if text.is_empty() {
                    return Ok(());
                }

                // Can't send if session isn't ready yet
                if self.session_id.is_none() {
                    self.sys_msg("Session still initializing, please wait…");
                    return Ok(());
                }

                // Save to history (slash commands excluded)
                if !text.starts_with('/') {
                    self.input_history.push(text.clone());
                }
                self.history_index = None;
                self.saved_input.clear();

                self.input.clear();
                self.cursor = 0;

                // Shell escape: !command runs directly in a subprocess
                if let Some(shell_cmd) = text.strip_prefix('!') {
                    let shell_cmd = shell_cmd.trim();
                    if shell_cmd.is_empty() {
                        self.sys_msg("Usage: !<command>  (e.g. !cargo test)");
                    } else {
                        self.sys_msg(format!("$ {shell_cmd}"));
                        match std::process::Command::new("sh")
                            .args(["-c", shell_cmd])
                            .current_dir(cwd)
                            .output()
                        {
                            Ok(output) => {
                                let stdout = String::from_utf8_lossy(&output.stdout);
                                let stderr = String::from_utf8_lossy(&output.stderr);
                                let mut result = String::new();
                                if !stdout.is_empty() {
                                    result.push_str(&stdout);
                                }
                                if !stderr.is_empty() {
                                    if !result.is_empty() {
                                        result.push('\n');
                                    }
                                    result.push_str(&stderr);
                                }
                                if result.is_empty() {
                                    result.push_str("(no output)");
                                }
                                // Wrap in a code block for proper rendering
                                self.sys_msg(format!("```\n{}\n```", result.trim_end()));
                            }
                            Err(e) => {
                                self.sys_msg(format!("Shell error: {e}"));
                            }
                        }
                    }
                    self.line_cache.clear();
                    return Ok(());
                }

                // Try local slash commands first
                if self.handle_local_command(&text, acp, cwd).await {
                    return Ok(());
                }

                // Save undo checkpoint before adding the user message
                self.undo_checkpoints.push(self.messages.len());

                // Forward slash commands to ACP as prompts
                // Add user message
                self.messages.push(ChatMessage {
                    role: Role::User,
                    content: text.clone(),
                    tokens: None,
                });
                self.scroll_offset = 0;

                // Start thinking
                self.status = AgentStatus::Thinking;
                self.animation.set_phase(AgentPhase::Thinking);
                self.animation.turn_start = Some(std::time::Instant::now());
                self.pending_response.clear();
                self.pending_thought.clear();
                self.active_tools.clear();

                let session_id = self.session_id.clone().unwrap_or_default();
                let event_tx = self
                    .event_tx
                    .as_ref()
                    .expect("event_tx must be set")
                    .clone();

                // Send prompt via ACP in a background task (non-blocking!)
                let prompt_text = text;
                let acp = Arc::clone(acp);
                tokio::spawn(async move {
                    match acp.prompt(&prompt_text, &session_id).await {
                        Ok(val) => {
                            let stop_reason = val
                                .get("stop_reason")
                                .or_else(|| val.get("stopReason"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("end_turn")
                                .to_string();
                            let usage = val.get("usage").and_then(|u| {
                                Some(Usage {
                                    input_tokens: u.get("input_tokens")
                                        .or_else(|| u.get("inputTokens"))
                                        .and_then(|v| v.as_u64())?,
                                    output_tokens: u.get("output_tokens")
                                        .or_else(|| u.get("outputTokens"))
                                        .and_then(|v| v.as_u64())?,
                                    elapsed_secs: None,
                                    last_prompt_tokens: None,
                                    cache_read_tokens: None,
                                })
                            });
                            // Extract _meta fields for context gauge
                            let meta = val.get("_meta");
                            let last_prompt = meta
                                .and_then(|m| m.get("lastPromptTokens"))
                                .and_then(|v| v.as_u64());
                            let cache_read = meta
                                .and_then(|m| m.get("cacheReadTokens"))
                                .and_then(|v| v.as_u64());
                            let usage = usage.map(|mut u| {
                                u.last_prompt_tokens = last_prompt;
                                u.cache_read_tokens = cache_read;
                                u
                            });
                            let _ = event_tx.send(AppEvent::PromptDone { stop_reason, usage });
                        }
                        Err(e) => {
                            let _ = event_tx.send(AppEvent::AcpError(
                                format!("Prompt failed: {}", e),
                            ));
                        }
                    }
                });
            }

            // Clear screen (Ctrl+L)
            (KeyModifiers::CONTROL, KeyCode::Char('l')) => {
                self.messages.clear();
                self.line_cache.clear();
                self.scroll_offset = 0;
            }

            // Scroll
            (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
                self.scroll_offset = self.scroll_offset.saturating_add(10);
            }

            // Toggle thinking display (Ctrl+O — matches CC's expand pattern)
            (KeyModifiers::CONTROL, KeyCode::Char('o')) => {
                self.show_thinking = !self.show_thinking;
                // Invalidate cache — thought messages render differently
                self.line_cache.clear();
            }

            // Open external editor (Ctrl+G)
            (KeyModifiers::CONTROL, KeyCode::Char('g')) if self.status == AgentStatus::Idle => {
                self.editor_requested = true;
                return Ok(());
            }
            (_, KeyCode::PageUp) => {
                self.scroll_offset = self.scroll_offset.saturating_add(20);
            }
            (_, KeyCode::PageDown) => {
                self.scroll_offset = self.scroll_offset.saturating_sub(20);
            }

            // Input history navigation
            (_, KeyCode::Up)
                if self.status == AgentStatus::Idle
                    && !self.input_history.is_empty() =>
            {
                match self.history_index {
                    None => {
                        self.saved_input = self.input.clone();
                        let idx = self.input_history.len() - 1;
                        self.history_index = Some(idx);
                        self.input = self.input_history[idx].clone();
                        self.cursor = self.input.len();
                    }
                    Some(idx) if idx > 0 => {
                        self.history_index = Some(idx - 1);
                        self.input = self.input_history[idx - 1].clone();
                        self.cursor = self.input.len();
                    }
                    _ => {}
                }
            }
            (_, KeyCode::Down) if self.status == AgentStatus::Idle => {
                match self.history_index {
                    Some(idx) if idx + 1 < self.input_history.len() => {
                        self.history_index = Some(idx + 1);
                        self.input = self.input_history[idx + 1].clone();
                        self.cursor = self.input.len();
                    }
                    Some(_) => {
                        self.history_index = None;
                        self.input = std::mem::take(&mut self.saved_input);
                        self.cursor = self.input.len();
                    }
                    None => {}
                }
            }

            // Cursor / editing with modifiers
            (KeyModifiers::CONTROL, KeyCode::Char('a')) | (_, KeyCode::Home) => {
                self.cursor = 0;
            }
            (KeyModifiers::CONTROL, KeyCode::Char('e')) | (_, KeyCode::End) => {
                self.cursor = self.input.len();
                self.scroll_offset = self.scroll_offset.saturating_sub(10);
            }
            // Word-level cursor movement: Alt+Left / Alt+Right
            (KeyModifiers::ALT, KeyCode::Left) => {
                let before = &self.input[..self.cursor];
                let trimmed = before.trim_end();
                self.cursor = trimmed
                    .rfind(|c: char| c.is_whitespace())
                    .map(|i| i + 1)
                    .unwrap_or(0);
            }
            (KeyModifiers::ALT, KeyCode::Right) => {
                let after = &self.input[self.cursor..];
                let skip_word = after
                    .find(|c: char| c.is_whitespace())
                    .unwrap_or(after.len());
                let skip_ws = after[skip_word..]
                    .find(|c: char| !c.is_whitespace())
                    .unwrap_or(after.len() - skip_word);
                self.cursor += skip_word + skip_ws;
            }
            (KeyModifiers::CONTROL, KeyCode::Char('w')) => {
                // Delete word backward
                let before = &self.input[..self.cursor];
                let trimmed = before.trim_end();
                let new_end = trimmed
                    .rfind(|c: char| c.is_whitespace())
                    .map(|i| i + 1)
                    .unwrap_or(0);
                self.input.replace_range(new_end..self.cursor, "");
                self.cursor = new_end;
            }
            (KeyModifiers::CONTROL, KeyCode::Char('k')) => {
                // Kill to end of line
                self.input.truncate(self.cursor);
            }

            // Shift+Tab: toggle yolo mode
            (KeyModifiers::SHIFT, KeyCode::BackTab) | (_, KeyCode::BackTab) => {
                self.yolo_mode = !self.yolo_mode;
                let label = if self.yolo_mode { "on ⚡" } else { "off" };
                self.sys_msg(format!("Approval bypass: {label}"));
                self.line_cache.clear();
                // Forward /yolo to server
                if let Some(ref sid) = self.session_id {
                    let session_id = sid.clone();
                    let acp = Arc::clone(acp);
                    tokio::spawn(async move {
                        let _ = acp.prompt("/yolo", &session_id).await;
                    });
                }
                return Ok(());
            }

            // Text input
            (_, KeyCode::Tab) if self.status == AgentStatus::Idle && self.input.starts_with('/') => {
                // Tab-complete slash commands: cycle through matches
                let prefix = self.input.to_lowercase();
                let matches: Vec<&str> = SLASH_COMMANDS
                    .iter()
                    .filter(|cmd| cmd.starts_with(&prefix))
                    .copied()
                    .collect();
                if matches.len() == 1 {
                    self.input = format!("{} ", matches[0]);
                    self.cursor = self.input.len();
                } else if !matches.is_empty() {
                    // Find longest common prefix among matches
                    let mut common = matches[0].to_string();
                    for m in &matches[1..] {
                        while !m.starts_with(&common) {
                            common.pop();
                        }
                    }
                    if common.len() > self.input.len() {
                        self.input = common;
                        self.cursor = self.input.len();
                    } else {
                        // Show available completions as a system message
                        let list: Vec<&str> = matches.to_vec();
                        self.sys_msg(list.join("  "));
                    }
                }
            }
            (_, KeyCode::Char(c)) => {
                self.input.insert(self.cursor, c);
                self.cursor += c.len_utf8();

                if c == '@' {
                    let cwd = cwd.to_string();
                    let tx = self.event_tx.clone().unwrap();
                    tokio::spawn(async move {
                        let entries = scan_files(&cwd, "", 50);
                        let _ = tx.send(AppEvent::FileScanResult(entries));
                    });
                    let at_pos = self.cursor - 1;
                    self.modal = ModalState::FileAutocomplete {
                        query: String::new(),
                        cursor_in_input: at_pos,
                        selected: 0,
                        entries: vec![],
                        loading: true,
                    };
                }
            }
            (_, KeyCode::Backspace)
                if self.cursor > 0 =>
            {
                let prev = self.input[..self.cursor]
                    .char_indices()
                    .next_back()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                self.input.replace_range(prev..self.cursor, "");
                self.cursor = prev;
            }
            (_, KeyCode::Delete)
                if self.cursor < self.input.len() =>
            {
                let next = self.input[self.cursor..]
                    .char_indices()
                    .nth(1)
                    .map(|(i, _)| self.cursor + i)
                    .unwrap_or(self.input.len());
                self.input.replace_range(self.cursor..next, "");
            }
            (_, KeyCode::Left)
                if self.cursor > 0 =>
            {
                self.cursor = self.input[..self.cursor]
                    .char_indices()
                    .next_back()
                    .map(|(i, _)| i)
                    .unwrap_or(0);
            }
            (_, KeyCode::Right)
                if self.cursor < self.input.len() =>
            {
                self.cursor = self.input[self.cursor..]
                    .char_indices()
                    .nth(1)
                    .map(|(i, _)| self.cursor + i)
                    .unwrap_or(self.input.len());
            }

            // Esc: double-tap to undo, single for future use
            (_, KeyCode::Esc) => {
                let now = std::time::Instant::now();
                if let Some(last) = self.last_esc {
                    if now.duration_since(last).as_millis() < 500 {
                        self.last_esc = None;
                        if self.status == AgentStatus::Idle {
                            self.undo_last_turn();
                        }
                        return Ok(());
                    }
                }
                self.last_esc = Some(now);
            }

            _ => {}
        }

        Ok(())
    }

    // ---- Modal key handler --------------------------------------------------

    async fn handle_modal_key(&mut self, key: KeyEvent, acp: &Arc<AcpClient>) -> Result<()> {
        let (options_len, _selected) = if let ModalState::Approval {
            ref options,
            selected,
            ..
        } = self.modal
        {
            (options.len(), selected)
        } else {
            return Ok(());
        };

        match (key.modifiers, key.code) {
            (_, KeyCode::Up) | (_, KeyCode::Char('k')) => {
                if let ModalState::Approval {
                    ref mut selected, ..
                } = self.modal
                {
                    if *selected > 0 {
                        *selected -= 1;
                    }
                }
            }
            (_, KeyCode::Down) | (_, KeyCode::Char('j')) => {
                if let ModalState::Approval {
                    ref mut selected, ..
                } = self.modal
                {
                    if *selected + 1 < options_len {
                        *selected += 1;
                    }
                }
            }

            (_, KeyCode::Enter) => {
                if let ModalState::Approval {
                    ref options,
                    selected,
                    ref request_id,
                    ..
                } = self.modal
                {
                    if let Some(opt) = options.get(selected) {
                        let response = if opt.id == "deny" {
                            serde_json::json!({
                                "outcome": {
                                    "outcome": "rejected",
                                }
                            })
                        } else {
                            serde_json::json!({
                                "outcome": {
                                    "optionId": opt.id,
                                    "outcome": "selected",
                                }
                            })
                        };
                        let _ = acp.respond(request_id.clone(), response).await;
                        self.sys_msg(format!("Approval: {}", opt.name));
                    }
                }
                self.modal = ModalState::None;
            }

            (_, KeyCode::Esc) | (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                // Esc = deny
                if let ModalState::Approval {
                    ref request_id, ..
                } = self.modal
                {
                    let response = serde_json::json!({
                        "outcome": {
                            "outcome": "rejected",
                        }
                    });
                    let _ = acp.respond(request_id.clone(), response).await;
                    self.sys_msg("Approval: Denied");
                }
                self.modal = ModalState::None;
            }

            _ => {}
        }
        Ok(())
    }

    // ---- Command palette key handler -----------------------------------------

    async fn handle_palette_key(
        &mut self,
        key: KeyEvent,
        acp: &Arc<AcpClient>,
        cwd: &str,
    ) -> Result<()> {
        if let ModalState::CommandPalette { query, cursor, selected, filtered } = &mut self.modal {
            match (key.code, key.modifiers) {
                (KeyCode::Esc, _) => self.close_modal(),
                (KeyCode::Enter, _) => {
                    if let Some(entry) = filtered.get(*selected).cloned() {
                        self.close_modal();
                        self.execute_palette_action(entry.action, acp, cwd).await?;
                    }
                }
                (KeyCode::Up, _) => {
                    *selected = selected.saturating_sub(1);
                }
                (KeyCode::Down, _)
                    if *selected + 1 < filtered.len() => { *selected += 1; }
                (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                    query.insert(*cursor, c);
                    *cursor += 1;
                    *selected = 0;
                    let all = Self::build_palette_entries();
                    *filtered = all.into_iter().filter(|e| fuzzy_matches(query, &e.label)).collect();
                }
                (KeyCode::Backspace, _)
                    if *cursor > 0 => {
                        *cursor -= 1;
                        query.remove(*cursor);
                        *selected = 0;
                        let all = Self::build_palette_entries();
                        *filtered = all.into_iter().filter(|e| fuzzy_matches(query, &e.label)).collect();
                    }
                _ => {}
            }
        }
        Ok(())
    }

    async fn execute_palette_action(
        &mut self,
        action: PaletteAction,
        acp: &Arc<AcpClient>,
        cwd: &str,
    ) -> Result<()> {
        match action {
            PaletteAction::SlashCommand(cmd) => {
                self.handle_local_command(&cmd, acp, cwd).await;
            }
            PaletteAction::Keybind(name) => match name.as_str() {
                "toggle_yolo" => {
                    self.yolo_mode = !self.yolo_mode;
                    let label = if self.yolo_mode { "on" } else { "off" };
                    self.sys_msg(format!("Bypass: {label}"));
                    if let Some(ref sid) = self.session_id {
                        let sid = sid.clone();
                        let acp = Arc::clone(acp);
                        tokio::spawn(async move { let _ = acp.prompt("/yolo", &sid).await; });
                    }
                }
                "toggle_thinking" => {
                    self.show_thinking = !self.show_thinking;
                    self.line_cache.clear();
                }
                "editor" => { self.editor_requested = true; }
                "clear" => {
                    self.messages.clear();
                    self.line_cache.clear();
                }
                "undo" => { self.undo_last_turn(); }
                _ => {}
            },
            PaletteAction::SetEffort(level) => {
                self.effort_level = level;
                let name = match level { 0 => "low", 1 => "medium", _ => "high" };
                self.sys_msg(format!("Effort set to {name}"));
            }
        }
        Ok(())
    }

    // ---- Effort slider key handler -------------------------------------------

    async fn handle_effort_key(&mut self, key: KeyEvent) -> Result<()> {
        if let ModalState::EffortSlider { level } = &mut self.modal {
            match key.code {
                KeyCode::Left => { *level = level.saturating_sub(1); }
                KeyCode::Right => { *level = (*level + 1).min(2); }
                KeyCode::Enter => {
                    self.effort_level = *level;
                    let name = match *level { 0 => "low", 1 => "medium", _ => "high" };
                    self.close_modal();
                    self.sys_msg(format!("Effort set to {name}"));
                }
                KeyCode::Esc => { self.close_modal(); }
                _ => {}
            }
        }
        Ok(())
    }

    // ---- Reverse search key handler ------------------------------------------

    async fn handle_reverse_search_key(&mut self, key: KeyEvent) -> Result<()> {
        if let ModalState::ReverseSearch { query, cursor, match_index } = &mut self.modal {
            match (key.code, key.modifiers) {
                (KeyCode::Esc, _) => { self.close_modal(); }
                (KeyCode::Enter, _) => {
                    if let Some(idx) = match_index {
                        if let Some(entry) = self.input_history.get(*idx) {
                            self.input = entry.clone();
                            self.cursor = self.input.len();
                        }
                    }
                    self.close_modal();
                }
                (KeyCode::Char('r'), KeyModifiers::CONTROL) => {
                    // Ctrl+R again: search next (older) match
                    let start = match_index.map(|i| i + 1).unwrap_or(0);
                    *match_index = self.input_history.iter().skip(start)
                        .position(|h| h.to_lowercase().contains(&query.to_lowercase()))
                        .map(|i| i + start);
                }
                (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                    query.insert(*cursor, c);
                    *cursor += 1;
                    *match_index = self.input_history.iter()
                        .position(|h| h.to_lowercase().contains(&query.to_lowercase()));
                }
                (KeyCode::Backspace, _)
                    if *cursor > 0 => {
                        *cursor -= 1;
                        query.remove(*cursor);
                        *match_index = self.input_history.iter()
                            .position(|h| h.to_lowercase().contains(&query.to_lowercase()));
                    }
                _ => {}
            }
        }
        Ok(())
    }

    // ---- File autocomplete key handler (placeholder) -------------------------

    async fn handle_file_autocomplete_key(&mut self, key: KeyEvent) -> Result<()> {
        if let ModalState::FileAutocomplete { query, cursor_in_input, selected, entries, .. } = &mut self.modal {
            match key.code {
                KeyCode::Esc => { self.close_modal(); }
                KeyCode::Enter => {
                    if let Some(path) = entries.get(*selected).cloned() {
                        // Replace @query with the file path
                        let at_start = *cursor_in_input;
                        let at_end = at_start + 1 + query.len();
                        let safe_end = at_end.min(self.input.len());
                        self.input.replace_range(at_start..safe_end, &path);
                        self.cursor = at_start + path.len();
                    }
                    self.close_modal();
                }
                KeyCode::Up => {
                    *selected = selected.saturating_sub(1);
                }
                KeyCode::Down
                    if *selected + 1 < entries.len() => { *selected += 1; }
                KeyCode::Char(c) => {
                    query.push(c);
                    *selected = 0;
                    let q = query.clone();
                    entries.retain(|e| fuzzy_matches(&q, e));
                }
                KeyCode::Backspace => {
                    if !query.is_empty() {
                        query.pop();
                        *selected = 0;
                    } else {
                        self.close_modal();
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    // ---- Local slash commands ------------------------------------------------

    async fn handle_local_command(&mut self, text: &str, acp: &Arc<AcpClient>, cwd: &str) -> bool {
        let parts: Vec<&str> = text.splitn(2, ' ').collect();
        let cmd = parts[0].to_lowercase();

        match cmd.as_str() {
            "/quit" | "/exit" | "/q" => {
                self.quit = true;
                true
            }
            "/clear" => {
                self.messages.clear();
                self.line_cache.clear();
                self.scroll_offset = 0;
                true
            }
            "/new" => {
                match acp.new_session(cwd).await {
                    Ok(sid) => {
                        self.session_id = Some(sid);
                        self.messages.clear();
                        self.line_cache.clear();
                        self.session_title = None;
                        self.scroll_offset = 0;
                        self.sys_msg("New session started.");
                    }
                    Err(e) => {
                        self.sys_msg(format!("Failed to create session: {}", e));
                    }
                }
                true
            }
            "/sessions" | "/switch" => {
                self.return_to_picker();
                if let Some(tx) = &self.event_tx {
                    let tx = tx.clone();
                    let acp_cloned = acp.clone();
                    tokio::spawn(async move {
                        if let Ok(sessions) = acp_cloned.list_sessions().await {
                            let _ = tx.send(crate::event::AppEvent::SessionsLoaded(sessions));
                        }
                    });
                }
                true
            }
            "/verbose" | "/v" => {
                self.verbose = !self.verbose;
                self.line_cache.clear();
                self.sys_msg(format!(
                    "Verbose mode: {}",
                    if self.verbose { "on" } else { "off" }
                ));
                true
            }
            "/effort" => {
                self.modal = ModalState::EffortSlider { level: self.effort_level };
                true
            }
            "/help" | "/h" | "/?" => {
                self.sys_msg(
                    "Local commands:\n\
                     \n\
                     /new             Start a new session\n\
                     /sessions        Return to session picker\n\
                     /clear           Clear the screen\n\
                     /verbose         Toggle tool call details\n\
                     /save [path]     Export session to markdown\n\
                     /usage           Show token usage\n\
                     /quit            Exit (also Ctrl+D)\n\
                     \n\
                     Server commands:\n\
                     \n\
                     /model [name]    Show or switch model\n\
                     /tools           List available tools\n\
                     /context         Show conversation stats\n\
                     /compact         Compress conversation context\n\
                     /reset           Clear conversation history\n\
                     /title [name]    Set or show session title\n\
                     /version         Show Hermes version\n\
                     /yolo            Toggle approval bypass\n\
                     /effort          Set reasoning effort\n\
                     \n\
                     Keys:\n\
                     \n\
                     Ctrl+P:   Command palette\n\
                     Ctrl+B:   Back to session picker\n\
                     Ctrl+R:   Reverse history search\n\
                     Esc Esc:  Undo last turn\n\
                     Scroll:   PgUp/PgDn, Ctrl+U, mouse wheel\n\
                     Cancel:   Ctrl+C during generation\n\
                     Clear:    Ctrl+L\n\
                     Newline: Ctrl+J or Shift+Enter\n\
                     History: Up/Down arrows\n\
                     Word jump: Alt+Left/Right\n\
                     Thinking: Ctrl+O toggle expand\n\
                     YOLO:    Shift+Tab toggle bypass\n\
                     Editor:  Ctrl+G open $EDITOR\n\
                     Tab: complete /commands\n\
                     \n\
                     Shell escape:\n\
                     \n\
                     !<cmd>   Run a shell command (e.g. !cargo test)\n\
                     \n\
                     Unrecognized /commands are forwarded to the server."
                        .to_string(),
                );
                true
            }
            "/usage" | "/u" => {
                let total = self.total_input_tokens + self.total_output_tokens;
                if self.prompt_count == 0 {
                    self.sys_msg("No usage data yet.".to_string());
                } else {
                    self.sys_msg(format!(
                        "Session usage ({} prompt{}):\n  Input:  {} tokens\n  Output: {} tokens\n  Total:  {} tokens",
                        self.prompt_count,
                        if self.prompt_count == 1 { "" } else { "s" },
                        self.total_input_tokens,
                        self.total_output_tokens,
                        total,
                    ));
                }
                true
            }
            "/reset" => {
                // Clear local display, then forward to server to clear server-side history
                self.messages.clear();
                self.line_cache.clear();
                self.scroll_offset = 0;
                false // fall through to send as prompt — server handles /reset
            }
            "/compact" => {
                if parts.len() > 1 {
                    self.sys_msg(format!("Compressing context (focus: {})…", parts[1]));
                } else {
                    self.sys_msg("Compressing context…");
                }
                false // fall through to send as prompt — server handles /compact
            }
            "/title" => {
                // Capture title locally for status bar, then forward to server
                if parts.len() > 1 {
                    self.session_title = Some(parts[1..].join(" "));
                }
                false // fall through to send as prompt — server handles /title
            }
            "/yolo" => {
                self.yolo_mode = !self.yolo_mode;
                self.sys_msg(format!(
                    "Approval bypass: {}",
                    if self.yolo_mode { "on ⚡" } else { "off" }
                ));
                false // fall through to send as prompt — server handles /yolo
            }
            "/save" => {
                let path = if parts.len() > 1 {
                    std::path::PathBuf::from(parts[1])
                } else {
                    let secs = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    dirs::home_dir()
                        .unwrap_or_default()
                        .join(format!("kaishi-export-{secs}.md"))
                };
                let mut content = String::new();
                for msg in &self.messages {
                    let label = match msg.role {
                        Role::User => "## User",
                        Role::Assistant => "## Assistant",
                        Role::System => "## System",
                        Role::Thought => "## Thought",
                        Role::Tool => "## Tool",
                    };
                    content.push_str(label);
                    content.push_str("\n\n");
                    content.push_str(&msg.content);
                    content.push_str("\n\n---\n\n");
                }
                match std::fs::write(&path, &content) {
                    Ok(()) => self.sys_msg(format!(
                        "Saved {} messages to {}",
                        self.messages.len(),
                        path.display()
                    )),
                    Err(e) => self.sys_msg(format!("Save failed: {e}")),
                }
                true
            }
            _ => false,
        }
    }

    // ---- ACP event handlers -------------------------------------------------

    /// Flush accumulated reasoning text into a Thought message.
    fn flush_pending_thought(&mut self) {
        if !self.pending_thought.is_empty() {
            let thought = std::mem::take(&mut self.pending_thought);
            self.messages.push(ChatMessage {
                role: Role::Thought,
                content: thought,
                tokens: None,
            });
        }
    }

    /// Flush accumulated streaming response into an Assistant message.
    fn flush_pending_response(&mut self, usage: Option<Usage>) {
        // Always flush thought before response (reasoning precedes the answer)
        self.flush_pending_thought();

        let content = std::mem::take(&mut self.pending_response);
        if !content.is_empty() {
            self.messages.push(ChatMessage {
                role: Role::Assistant,
                content,
                tokens: usage,
            });
        }
    }

    pub fn handle_agent_message(&mut self, text: &str) {
        // Flush any accumulated thought before streaming response text
        self.flush_pending_thought();
        if self.animation.phase != AgentPhase::Streaming {
            self.animation.set_phase(AgentPhase::Streaming);
        }
        self.animation.last_output = std::time::Instant::now();
        self.pending_response.push_str(text);
        self.scroll_offset = 0;
    }

    pub fn handle_agent_thought(&mut self, text: &str) {
        // Flush any pending response before accumulating thought
        // (handles interleaved: response → thought → response)
        if !self.pending_response.is_empty() {
            self.flush_pending_response(None);
        }
        if self.animation.phase == AgentPhase::Idle {
            self.animation.set_phase(AgentPhase::Thinking);
        }
        self.animation.last_output = std::time::Instant::now();
        self.pending_thought.push_str(text);
    }

    pub fn handle_tool_start(&mut self, id: &str, name: &str, _kind: Option<&str>, input: Option<&str>) {
        // Flush thought/response before tool calls
        self.flush_pending_thought();
        if !self.pending_response.is_empty() {
            self.flush_pending_response(None);
        }

        self.animation.set_phase(AgentPhase::Executing);
        self.animation.active_tool = Some(name.to_string());

        self.active_tools.push((id.to_string(), name.to_string()));

        // Parse input into a human-readable summary
        let summary = input
            .and_then(|s| summarize_tool_input(name, s))
            .unwrap_or_default();

        let idx = self.messages.len();
        self.messages.push(ChatMessage {
            role: Role::Tool,
            content: format!("⚙ {}\x1f{}", name, summary),
            tokens: None,
        });
        self.tool_msg_map.insert(id.to_string(), idx);
        self.scroll_offset = 0;
    }

    /// Check if text looks like a unified diff.
    fn looks_like_diff(text: &str) -> bool {
        let first = text.lines().next().unwrap_or("");

        // Explicit diff headers
        if first.starts_with("--- ")
            || first.starts_with("diff --git")
            || first.starts_with("*** Begin Patch")
        {
            return true;
        }

        // Heuristic: has unified diff markers
        if text.contains("\n@@ ") && (text.contains("\n+") || text.contains("\n-")) {
            return true;
        }

        // Heuristic: has enough +/- lines to look like a diff (at least 2 of each)
        let plus_lines = text
            .lines()
            .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
            .count();
        let minus_lines = text
            .lines()
            .filter(|l| l.starts_with('-') && !l.starts_with("---"))
            .count();
        if plus_lines >= 2 && minus_lines >= 1 {
            return true;
        }
        if minus_lines >= 2 && plus_lines >= 1 {
            return true;
        }

        false
    }

    pub fn handle_tool_update(&mut self, id: &str, status: &str, content: Option<&str>) {
        if status == "completed" || status == "error" {
            self.active_tools.retain(|(tid, _)| tid != id);

            // When all tools finish, go back to Thinking (model processes result)
            if self.active_tools.is_empty() {
                self.animation.set_phase(AgentPhase::Thinking);
                self.animation.active_tool = None;
            }
        }

        // Update the existing tool message in-place
        if let Some(&msg_idx) = self.tool_msg_map.get(id) {
            if msg_idx < self.messages.len() {
                // Extract name and summary from existing content (separated by \x1f)
                let existing = &self.messages[msg_idx].content;
                let rest = existing
                    .trim_start_matches(['✓', '✗', '⚙', ' '])
                    .to_string();
                let (name, summary) = if let Some(sep) = rest.find('\x1f') {
                    (rest[..sep].to_string(), rest[sep + 1..].to_string())
                } else {
                    // Fallback: split at first space/paren
                    let n = rest.split([' ', '(']).next().unwrap_or("").to_string();
                    (n, String::new())
                };

                let status_icon = match status {
                    "completed" => "✓",
                    "error" => "✗",
                    _ => "⚙",
                };

                // For errors, append error detail to summary
                let final_summary = if status == "error" {
                    let detail = content
                        .map(|t| {
                            let preview: String = t.chars().take(80).collect();
                            if summary.is_empty() {
                                preview
                            } else {
                                format!("{} — {}", summary, preview)
                            }
                        })
                        .unwrap_or(summary);
                    detail
                } else if status == "completed" {
                    // For completed tools, check if content contains a diff
                    if let Some(text) = content {
                        if Self::looks_like_diff(text) {
                            // Append diff to summary for rendering
                            if summary.is_empty() {
                                text.to_string()
                            } else {
                                format!("{}\n{}", summary, text)
                            }
                        } else {
                            summary
                        }
                    } else {
                        summary
                    }
                } else {
                    summary
                };

                self.messages[msg_idx].content =
                    format!("{} {}\x1f{}", status_icon, name, final_summary);

                // Invalidate cached rendering for this message
                if msg_idx < self.line_cache.len() {
                    self.line_cache.truncate(msg_idx);
                }
            }

            if status == "completed" || status == "error" {
                self.tool_msg_map.remove(id);
            }
        }
    }

    pub fn handle_prompt_done(&mut self, _stop_reason: &str, usage: Option<Usage>) {
        // Terminal bell — notifies backgrounded terminals that a turn finished
        print!("\x07");
        // Compute elapsed time for turn summary
        let elapsed = self.animation.turn_start
            .map(|t| t.elapsed().as_secs_f64());

        // The server sends SESSION-CUMULATIVE token counts (sum of all API calls).
        // Compute per-turn deltas for display by subtracting previous totals.
        let turn_usage = usage.map(|mut u| {
            let turn_in = u.input_tokens.saturating_sub(self.total_input_tokens);
            let turn_out = u.output_tokens.saturating_sub(self.total_output_tokens);

            // Update running totals to the new cumulative values
            self.total_input_tokens = u.input_tokens;
            self.total_output_tokens = u.output_tokens;
            self.prompt_count += 1;

            // Overwrite with per-turn deltas for display
            u.input_tokens = turn_in;
            u.output_tokens = turn_out;
            u.elapsed_secs = elapsed;
            u
        });

        // Track context window usage from the latest prompt.
        // Use last_prompt_tokens from _meta (the actual context window fill
        // for the most recent API call) instead of cumulative totals.
        if let Some(ref u) = turn_usage {
            if let Some(lp) = u.last_prompt_tokens {
                self.context_used = lp;
            } else {
                // Fallback: per-turn delta is better than session cumulative
                self.context_used = u.input_tokens;
            }
        }

        self.flush_pending_response(turn_usage);
        self.status = AgentStatus::Idle;
        self.animation.set_phase(AgentPhase::Idle);
        self.animation.active_tool = None;
        self.animation.turn_start = None;
        self.active_tools.clear();
        self.tool_msg_map.clear();
        self.scroll_offset = 0;
    }

    pub fn show_approval_modal(
        &mut self,
        request_id: serde_json::Value,
        command: String,
        options: Vec<ApprovalOption>,
    ) {
        self.modal = ModalState::Approval {
            command,
            options,
            selected: 0,
            request_id,
        };
    }

    /// Handle mouse scroll: positive = scroll up, negative = scroll down.
    pub fn handle_scroll(&mut self, delta: i16) {
        match self.screen {
            Screen::Picker => {
                // delta > 0 = wheel up → scroll toward top (decrease offset).
                // delta < 0 = wheel down → scroll toward bottom (increase offset).
                if delta > 0 {
                    self.picker_scroll_offset = self.picker_scroll_offset.saturating_sub(delta as u16);
                } else {
                    self.picker_scroll_offset = self.picker_scroll_offset.saturating_add((-delta) as u16);
                }
            }
            Screen::Chat => {
                if delta > 0 {
                    self.scroll_offset = self.scroll_offset.saturating_add(delta as u16);

                    // Trigger lazy load when scrolled near the top
                    if self.history_loaded < self.history_total
                        && !self.loading_more_history
                    {
                        let total_cached_lines: usize = self.line_cache.iter().map(|c| c.len()).sum();
                        if self.scroll_offset as usize + 20 >= total_cached_lines {
                            self.loading_more_history = true;
                            if let Some(tx) = &self.event_tx {
                                let _ = tx.send(crate::event::AppEvent::LoadMoreHistory);
                            }
                        }
                    }
                } else {
                    self.scroll_offset = self.scroll_offset.saturating_sub((-delta) as u16);
                }
            }
            _ => {}
        }
    }

    /// Load conversation history from the server into the messages list.
    pub fn load_history(&mut self, history: Vec<(String, String)>, total: usize, prepend: bool) {
        self.history_total = total;

        if history.is_empty() && !prepend {
            self.sys_msg("Session resumed (no history available).");
            return;
        }

        let new_msgs: Vec<ChatMessage> = history
            .iter()
            .map(|(role, content)| ChatMessage {
                role: match role.as_str() {
                    "user" => Role::User,
                    "assistant" => Role::Assistant,
                    "system" => Role::System,
                    "tool" => Role::Tool,
                    _ => Role::System,
                },
                content: content.clone(),
                tokens: None,
            })
            .collect();

        let added = new_msgs.len();

        if prepend {
            // Insert older messages at the beginning
            let old_messages = std::mem::take(&mut self.messages);
            self.messages = new_msgs;
            self.messages.extend(old_messages);
            self.line_cache.clear(); // Must rebuild — indices shifted
        } else {
            // Initial load — clear welcome messages
            self.messages.clear();
            self.line_cache.clear();
            self.messages = new_msgs;
        }

        self.history_loaded = self
            .messages
            .iter()
            .filter(|m| m.role == Role::User || m.role == Role::Assistant)
            .count();

        self.loading_more_history = false;

        if !prepend {
            if self.history_loaded < self.history_total {
                self.sys_msg(format!(
                    "Loaded {} of {} messages.",
                    self.history_loaded, self.history_total
                ));
            } else {
                self.sys_msg(format!("Loaded {} messages from history.", added));
            }
        }

        self.scroll_offset = 0;
    }

    /// Exit the current chat session and return to the picker.
    /// Clears per-session state; keeps app-level settings (effort, yolo,
    /// verbose, input history). Caller is responsible for triggering a
    /// session list refresh via `AppEvent::SessionsLoaded` if desired.
    pub fn return_to_picker(&mut self) {
        self.screen = Screen::Picker;
        self.modal = ModalState::None;

        // Clear per-session chat state
        self.session_id = None;
        self.session_title = None;
        self.messages.clear();
        self.line_cache.clear();
        self.pending_response.clear();
        self.pending_thought.clear();
        self.scroll_offset = 0;
        self.status = AgentStatus::Idle;
        self.active_tools.clear();
        self.tool_msg_map.clear();

        // Reset per-session counters
        self.total_input_tokens = 0;
        self.total_output_tokens = 0;
        self.prompt_count = 0;
        self.context_used = 0;
        self.undo_checkpoints.clear();

        // Reset picker scroll to top
        self.picker_selected = 0;
        self.picker_scroll_offset = 0;

        // Clear history pagination state
        self.history_total = 0;
        self.history_loaded = 0;
        self.loading_more_history = false;
    }

    /// Adjust `picker_scroll_offset` so the currently selected card is
    /// within the visible viewport. `visible_rows` is the terminal-row
    /// height of the picker list area (not including header/footer).
    pub fn ensure_picker_visible(&mut self, visible_rows: u16) {
        use crate::ui_picker::CARD_HEIGHT;
        if visible_rows == 0 {
            return;
        }
        let top_row = (self.picker_selected as u16).saturating_mul(CARD_HEIGHT);
        let bot_row = top_row.saturating_add(CARD_HEIGHT);

        // Selected card is above viewport
        if top_row < self.picker_scroll_offset {
            self.picker_scroll_offset = top_row;
            return;
        }
        // Selected card is below viewport
        let viewport_bottom = self.picker_scroll_offset.saturating_add(visible_rows);
        if bot_row > viewport_bottom {
            self.picker_scroll_offset = bot_row.saturating_sub(visible_rows);
        }
    }
}

/// Produce a short, readable summary of tool input by tool name.
/// Returns None if input is empty or unparseable.
fn summarize_tool_input(tool_name: &str, raw_input: &str) -> Option<String> {
    let trimmed = raw_input.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Try parsing as JSON
    let json: serde_json::Value = serde_json::from_str(trimmed).ok()?;

    let summary = match tool_name {
        "terminal" => {
            let cmd = json.get("command").and_then(|v| v.as_str())?;
            truncate_summary(cmd, 120)
        }
        "read_file" => {
            let path = json.get("path").and_then(|v| v.as_str())?;
            let offset = json.get("offset").and_then(|v| v.as_u64());
            let limit = json.get("limit").and_then(|v| v.as_u64());
            match (offset, limit) {
                (Some(o), Some(l)) => format!("{} ({}–{})", path, o, o + l),
                (Some(o), None) => format!("{} (from {})", path, o),
                _ => path.to_string(),
            }
        }
        "write_file" => {
            let path = json.get("path").and_then(|v| v.as_str())?;
            let content = json.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let lines = content.lines().count();
            format!("{} ({} lines)", path, lines)
        }
        "patch" => {
            let path = json.get("path").and_then(|v| v.as_str()).unwrap_or("(patch)");
            let mode = json.get("mode").and_then(|v| v.as_str()).unwrap_or("replace");
            if mode == "patch" {
                "multi-file patch".to_string()
            } else {
                path.to_string()
            }
        }
        "search_files" => {
            let pattern = json.get("pattern").and_then(|v| v.as_str()).unwrap_or("?");
            let target = json.get("target").and_then(|v| v.as_str()).unwrap_or("content");
            let path = json.get("path").and_then(|v| v.as_str()).unwrap_or(".");
            if target == "files" {
                format!("files matching {} in {}", pattern, path)
            } else {
                format!("\"{}\" in {}", pattern, path)
            }
        }
        "web_search" => {
            let q = json.get("query").and_then(|v| v.as_str())?;
            truncate_summary(q, 80)
        }
        "web_extract" => {
            let urls = json.get("urls").and_then(|v| v.as_array())?;
            if urls.len() == 1 {
                urls[0].as_str().unwrap_or("url").to_string()
            } else {
                format!("{} URLs", urls.len())
            }
        }
        "browser_navigate" => {
            let url = json.get("url").and_then(|v| v.as_str())?;
            truncate_summary(url, 80)
        }
        "browser_click" | "browser_type" => {
            let r = json.get("ref").and_then(|v| v.as_str()).unwrap_or("?");
            let text = json.get("text").and_then(|v| v.as_str());
            match text {
                Some(t) => format!("{} → {}", r, truncate_summary(t, 60)),
                None => r.to_string(),
            }
        }
        "skill_view" | "skill_manage" => {
            let name = json.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            name.to_string()
        }
        "memory" | "hindsight_retain" | "hindsight_recall" => {
            let content = json.get("content")
                .or_else(|| json.get("query"))
                .and_then(|v| v.as_str())
                .unwrap_or("…");
            truncate_summary(content, 60)
        }
        "delegate_task" => {
            let goal = json.get("goal").and_then(|v| v.as_str());
            let tasks = json.get("tasks").and_then(|v| v.as_array());
            match (goal, tasks) {
                (Some(g), _) => truncate_summary(g, 60),
                (_, Some(t)) => format!("{} parallel tasks", t.len()),
                _ => "task".to_string(),
            }
        }
        "vision_analyze" | "browser_vision" => {
            let q = json.get("question").and_then(|v| v.as_str()).unwrap_or("analyze");
            truncate_summary(q, 60)
        }
        "todo" => {
            let todos = json.get("todos").and_then(|v| v.as_array());
            match todos {
                Some(t) => format!("{} items", t.len()),
                None => "read list".to_string(),
            }
        }
        "session_search" => {
            let q = json.get("query").and_then(|v| v.as_str());
            match q {
                Some(q) => truncate_summary(q, 60),
                None => "recent sessions".to_string(),
            }
        }
        "execute_code" => {
            let code = json.get("code").and_then(|v| v.as_str()).unwrap_or("");
            let first_line = code.lines().next().unwrap_or("script");
            let line_count = code.lines().count();
            format!("{} ({} lines)", truncate_summary(first_line, 40), line_count)
        }
        _ => {
            if let Some(obj) = json.as_object() {
                // Priority 1: look for common meaningful keys
                let meaningful_keys = ["path", "name", "query", "url", "command", "goal", "file", "ref"];
                for key in meaningful_keys {
                    if let Some(s) = obj.get(key).and_then(|v| v.as_str()) {
                        if !s.is_empty() && s.len() < 120 {
                            return Some(truncate_summary(s, 80));
                        }
                    }
                }

                // Priority 2: first short string value
                for (_k, v) in obj.iter() {
                    if let Some(s) = v.as_str() {
                        if !s.is_empty() && s.len() < 100 {
                            return Some(truncate_summary(s, 60));
                        }
                    }
                }

                // Priority 3: show key names only (not values)
                let keys: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
                if !keys.is_empty() {
                    return Some(truncate_summary(&keys.join(", "), 60));
                }
            }

            // Final fallback: compact JSON preview
            let compact = trimmed.replace('\n', " ");
            truncate_summary(&compact, 60)
        }
    };

    Some(summary)
}

fn truncate_summary(s: &str, max: usize) -> String {
    // Single-line it
    let clean: String = s.lines().next().unwrap_or(s).to_string();
    if clean.len() <= max {
        return clean;
    }
    let end = clean
        .char_indices()
        .nth(max.saturating_sub(1))
        .map(|(i, _)| i)
        .unwrap_or(clean.len());
    format!("{}…", &clean[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat_app_fixture() -> App {
        let mut app = App::new(vec![]);
        app.screen = Screen::Chat;
        app.session_id = Some("sess-abc".into());
        app.session_title = Some("Old title".into());
        app.messages.push(ChatMessage {
            role: Role::User,
            content: "hello".into(),
            tokens: None,
        });
        app.pending_response = "streaming…".into();
        app.pending_thought = "thinking…".into();
        app.scroll_offset = 42;
        app.total_input_tokens = 100;
        app.total_output_tokens = 50;
        app.prompt_count = 3;
        app.context_used = 1234;
        app.undo_checkpoints.push(1);
        app
    }

    #[test]
    fn return_to_picker_switches_screen() {
        let mut app = chat_app_fixture();
        app.return_to_picker();
        assert_eq!(app.screen, Screen::Picker);
    }

    #[test]
    fn return_to_picker_clears_session_state() {
        let mut app = chat_app_fixture();
        app.return_to_picker();
        assert!(app.session_id.is_none());
        assert!(app.session_title.is_none());
        assert!(app.messages.is_empty());
        assert_eq!(app.pending_response, "");
        assert_eq!(app.pending_thought, "");
        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn return_to_picker_resets_counters() {
        let mut app = chat_app_fixture();
        app.return_to_picker();
        assert_eq!(app.total_input_tokens, 0);
        assert_eq!(app.total_output_tokens, 0);
        assert_eq!(app.prompt_count, 0);
        assert_eq!(app.context_used, 0);
        assert!(app.undo_checkpoints.is_empty());
    }

    #[test]
    fn return_to_picker_resets_picker_selection() {
        let mut app = chat_app_fixture();
        app.picker_selected = 5;
        app.picker_scroll_offset = 100;
        app.return_to_picker();
        assert_eq!(app.picker_selected, 0);
        assert_eq!(app.picker_scroll_offset, 0);
    }

    #[test]
    fn return_to_picker_is_idempotent() {
        let mut app = chat_app_fixture();
        app.return_to_picker();
        app.return_to_picker();
        assert_eq!(app.screen, Screen::Picker);
    }

    #[test]
    fn picker_scroll_follows_cursor_down() {
        let mut app = App::new(vec![]);
        // Card 5 occupies rows 15..18 (CARD_HEIGHT = 3).
        // Viewport 10 rows tall starting at offset 0 → card 5 is off-screen.
        app.picker_selected = 5;
        app.picker_scroll_offset = 0;
        app.ensure_picker_visible(10);
        // Offset should be nudged so row 18 fits: offset ≥ 18 - 10 = 8.
        assert!(
            app.picker_scroll_offset >= 8,
            "offset={} should be ≥ 8",
            app.picker_scroll_offset
        );
        // And not overshoot past the top of the card (row 15).
        assert!(
            app.picker_scroll_offset <= 15,
            "offset={} should be ≤ 15",
            app.picker_scroll_offset
        );
    }

    #[test]
    fn picker_scroll_follows_cursor_up() {
        let mut app = App::new(vec![]);
        app.picker_scroll_offset = 30;
        app.picker_selected = 2; // top row 6 → above offset 30
        app.ensure_picker_visible(10);
        assert!(
            app.picker_scroll_offset <= 6,
            "offset={} should be ≤ 6",
            app.picker_scroll_offset
        );
    }

    #[test]
    fn picker_scroll_no_change_when_already_visible() {
        let mut app = App::new(vec![]);
        app.picker_scroll_offset = 5;
        app.picker_selected = 3; // rows 9..12, viewport 5..15 → in view
        app.ensure_picker_visible(10);
        assert_eq!(app.picker_scroll_offset, 5);
    }

    #[test]
    fn picker_scroll_ignores_zero_viewport() {
        // Before any frame is drawn, viewport_rows is 0. Should no-op.
        let mut app = App::new(vec![]);
        app.picker_scroll_offset = 7;
        app.picker_selected = 20;
        app.ensure_picker_visible(0);
        assert_eq!(app.picker_scroll_offset, 7);
    }
}
