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
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
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

    let acp = acp::AcpClient::spawn(event_tx.clone(), profile.as_deref()).await?;

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

    // Extract model name from init result
    if let Ok(ref init) = init_result {
        if let Some(model) = init
            .get("agent_info")
            .and_then(|s| s.get("name"))
            .and_then(|m| m.as_str())
        {
            app.model_name = model.to_string();
        }
    }

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
                app.status = app::AgentStatus::Idle;
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
