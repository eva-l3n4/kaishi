use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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

    // Input history
    pub input_history: Vec<String>,
    pub history_index: Option<usize>,
    saved_input: String,

    quit: bool,
}

impl App {
    pub fn new(sessions: Vec<SessionInfo>) -> Self {
        Self {
            screen: Screen::Picker,
            modal: ModalState::None,
            sessions,
            picker_selected: 0,
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
            input_history: Vec::new(),
            history_index: None,
            saved_input: String::new(),
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
                        let response = serde_json::json!({
                            "option_id": opt.id,
                        });
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
                        "option_id": "deny",
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
                self.scroll_offset = 0;
                true
            }
            "/new" => {
                match acp.new_session(cwd).await {
                    Ok(sid) => {
                        self.session_id = Some(sid);
                        self.messages.clear();
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
            "/model" => {
                if self.model_name.is_empty() {
                    self.sys_msg("Model: (unknown — set via ACP initialize)");
                } else {
                    self.sys_msg(format!("Model: {}", self.model_name));
                }
                true
            }
            "/verbose" | "/v" => {
                self.verbose = !self.verbose;
                self.sys_msg(format!(
                    "Verbose mode: {}",
                    if self.verbose { "on" } else { "off" }
                ));
                true
            }
            "/help" | "/h" | "/?" => {
                self.sys_msg(
                    "Commands:\n\
                     \n\
                     /new             Start a new session\n\
                     /model           Show current model\n\
                     /verbose         Toggle tool call details\n\
                     /clear           Clear the screen\n\
                     /quit            Exit\n\
                     \n\
                     Scroll: PgUp/PgDn, Ctrl+U (up 10)\n\
                     Cancel: Ctrl+C during generation\n\
                     Multiline: Ctrl+J for newline\n\
                     \n\
                     CLI: --profile, --session <id>, --cwd"
                        .to_string(),
                );
                true
            }
            _ => false,
        }
    }

    // ---- ACP event handlers -------------------------------------------------

    pub fn handle_agent_message(&mut self, text: &str) {
        self.pending_response.push_str(text);
        self.scroll_offset = 0;
    }

    pub fn handle_agent_thought(&mut self, text: &str) {
        self.pending_thought.push_str(text);
    }

    pub fn handle_tool_start(&mut self, id: &str, name: &str, _kind: Option<&str>) {
        self.active_tools.push((id.to_string(), name.to_string()));
        // Always show compact tool label
        self.messages.push(ChatMessage {
            role: Role::Tool,
            content: format!("⚙ {}", name),
            tokens: None,
        });
        self.scroll_offset = 0;
    }

    pub fn handle_tool_update(&mut self, id: &str, status: &str, content: Option<&str>) {
        if status == "completed" || status == "error" {
            self.active_tools.retain(|(tid, _)| tid != id);
        }
        if self.verbose {
            if let Some(text) = content {
                let preview = if text.len() > 200 {
                    format!("{}...", &text[..200])
                } else {
                    text.to_string()
                };
                self.messages.push(ChatMessage {
                    role: Role::Tool,
                    content: format!("[{}] {}", status, preview),
                    tokens: None,
                });
            }
        }
    }

    pub fn handle_prompt_done(&mut self, _stop_reason: &str, usage: Option<Usage>) {
        // Flush pending thought
        if !self.pending_thought.is_empty() {
            let thought = std::mem::take(&mut self.pending_thought);
            self.messages.push(ChatMessage {
                role: Role::Thought,
                content: thought,
                tokens: None,
            });
        }

        // Flush pending response
        let content = std::mem::take(&mut self.pending_response);
        if !content.is_empty() {
            self.messages.push(ChatMessage {
                role: Role::Assistant,
                content,
                tokens: usage,
            });
        }
        self.status = AgentStatus::Idle;
        self.active_tools.clear();
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
        if delta > 0 {
            self.scroll_offset = self.scroll_offset.saturating_add(delta as u16);
        } else {
            self.scroll_offset = self.scroll_offset.saturating_sub((-delta) as u16);
        }
    }

    /// Load conversation history from the server into the messages list.
    pub fn load_history(&mut self, history: Vec<(String, String)>) {
        // Clear the welcome message and "resuming" messages
        self.messages.clear();

        for (role, content) in history {
            let msg_role = match role.as_str() {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                "system" => Role::System,
                "tool" => Role::Tool,
                _ => Role::System,
            };
            self.messages.push(ChatMessage {
                role: msg_role,
                content,
                tokens: None,
            });
        }

        if self.messages.is_empty() {
            self.sys_msg("Session resumed (no history).");
        } else {
            let count = self.messages.len();
            self.sys_msg(format!("Loaded {} messages from history.", count));
        }
        self.scroll_offset = 0;
    }
}
