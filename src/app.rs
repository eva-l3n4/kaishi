use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::Line as RatLine;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::acp::AcpClient;
use crate::event::{AppEvent, ApprovalOption, SessionInfo, Usage};

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

/// What the assistant is currently doing.
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum AgentStatus {
    Idle,
    Thinking,
    Error(String),
}

/// Which screen is active.
#[derive(Debug, Clone, PartialEq)]
pub enum Screen {
    Picker,
    Chat,
    Disconnected(String), // error message
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
}

/// Application state.
pub struct App {
    pub screen: Screen,
    pub modal: ModalState,

    // Session picker
    pub sessions: Vec<SessionInfo>,
    pub picker_selected: usize,
    pub picker_scroll_offset: u16,

    // Active session
    pub session_id: Option<String>,

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
            session_id: None,
            messages: vec![ChatMessage {
                role: Role::System,
                content: "Welcome to 🌸 Hanami. Type a message or /help for commands."
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
            quit: false,
        }
    }

    pub fn should_quit(&self) -> bool {
        self.quit
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
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
        // Modal takes priority
        if let ModalState::Approval { .. } = &self.modal {
            return self.handle_modal_key(key, acp).await;
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
            }
            (_, KeyCode::Down) | (_, KeyCode::Char('j'))
                if self.picker_selected + 1 < total =>
            {
                self.picker_selected += 1;
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

                // Save to history
                if !text.starts_with('/') {
                    self.input_history.push(text.clone());
                }
                self.history_index = None;
                self.saved_input.clear();

                self.input.clear();
                self.cursor = 0;

                // Try local slash commands first
                if self.handle_local_command(&text, acp, cwd).await {
                    return Ok(());
                }

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
                                })
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

            // Scroll
            (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
                self.scroll_offset = self.scroll_offset.saturating_add(10);
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

            // Text input
            (_, KeyCode::Char(c)) => {
                self.input.insert(self.cursor, c);
                self.cursor += c.len_utf8();
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
            "/verbose" | "/v" => {
                self.verbose = !self.verbose;
                self.line_cache.clear();
                self.sys_msg(format!(
                    "Verbose mode: {}",
                    if self.verbose { "on" } else { "off" }
                ));
                true
            }
            "/help" | "/h" | "/?" => {
                self.sys_msg(
                    "Local commands:\n\
                     \n\
                     /new             Start a new session\n\
                     /clear           Clear the screen\n\
                     /verbose         Toggle tool call details\n\
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
                     \n\
                     Keys:\n\
                     \n\
                     Scroll: PgUp/PgDn, mouse wheel\n\
                     Cancel: Ctrl+C during generation\n\
                     Newline: Ctrl+J\n\
                     History: Up/Down arrows\n\
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
            "/title" => {
                // Capture title locally for status bar, then forward to server
                if let Some(title) = parts.get(1) {
                    self.session_title = Some(title.to_string());
                }
                false // fall through to send as prompt — server handles /title
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
        self.pending_response.push_str(text);
        self.scroll_offset = 0;
    }

    pub fn handle_agent_thought(&mut self, text: &str) {
        // Flush any pending response before accumulating thought
        // (handles interleaved: response → thought → response)
        if !self.pending_response.is_empty() {
            self.flush_pending_response(None);
        }
        self.pending_thought.push_str(text);
    }

    pub fn handle_tool_start(&mut self, id: &str, name: &str, _kind: Option<&str>, input: Option<&str>) {
        // Flush thought/response before tool calls
        self.flush_pending_thought();
        if !self.pending_response.is_empty() {
            self.flush_pending_response(None);
        }

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

    pub fn handle_tool_update(&mut self, id: &str, status: &str, content: Option<&str>) {
        if status == "completed" || status == "error" {
            self.active_tools.retain(|(tid, _)| tid != id);
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
        if let Some(ref u) = usage {
            self.total_input_tokens += u.input_tokens;
            self.total_output_tokens += u.output_tokens;
            self.prompt_count += 1;
        }
        self.flush_pending_response(usage);
        self.status = AgentStatus::Idle;
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
                if delta > 0 {
                    self.picker_scroll_offset = self.picker_scroll_offset.saturating_add(delta as u16);
                } else {
                    self.picker_scroll_offset = self.picker_scroll_offset.saturating_sub((-delta) as u16);
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
        _ => {
            // Generic: show first string-valued key
            if let Some(obj) = json.as_object() {
                for (k, v) in obj.iter() {
                    if let Some(s) = v.as_str() {
                        if !s.is_empty() && s.len() < 100 {
                            return Some(format!("{}: {}", k, truncate_summary(s, 60)));
                        }
                    }
                }
            }
            // Fallback: compact JSON preview
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
