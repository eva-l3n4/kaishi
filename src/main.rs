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
use std::sync::Arc;

/// CLI argument parsing (no dependency needed for this).
struct CliArgs {
    profile: Option<String>,
    cwd: String,
    session: Option<String>,
}

fn parse_args() -> CliArgs {
    let mut args = std::env::args().skip(1);
    let mut profile: Option<String> = std::env::var("HERMES_PROFILE").ok();
    let mut cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string());
    let mut session: Option<String> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--profile" | "-p" => {
                profile = args.next();
            }
            "--cwd" | "-C" => {
                if let Some(val) = args.next() {
                    cwd = val;
                }
            }
            "--session" | "-s" => {
                session = args.next();
            }
            "--help" | "-h" => {
                eprintln!(
                    "hermes-tui {}\n\n\
                     Usage: hermes-tui [OPTIONS]\n\n\
                     Options:\n  \
                       --profile, -p <name>   Hermes profile to use (env: HERMES_PROFILE)\n  \
                       --cwd, -C <path>       Working directory for sessions\n  \
                       --session, -s <id>     Resume a session directly (skip picker)\n  \
                       --help, -h             Show this help",
                    env!("CARGO_PKG_VERSION")
                );
                std::process::exit(0);
            }
            _ => {
                eprintln!("Unknown argument: {}. Use --help for usage.", arg);
                std::process::exit(1);
            }
        }
    }

    CliArgs {
        profile,
        cwd,
        session,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = parse_args();

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

    let acp = Arc::new(acp::AcpClient::spawn(event_tx.clone(), cli.profile.as_deref()).await?);

    // Create app immediately — show picker with "Connecting..." while ACP initializes
    let mut app = App::new(vec![]);
    app.event_tx = Some(event_tx.clone());

    // If --session was provided, skip picker and go straight to chat
    let direct_session = cli.session.clone();
    if direct_session.is_some() {
        app.screen = app::Screen::Chat;
        app.status = app::AgentStatus::Thinking;
        app.sys_msg("Resuming session…");
    }

    // Initialize ACP + fetch sessions in background
    let acp_init = Arc::clone(&acp);
    let event_tx_init = event_tx.clone();
    let direct_sid = direct_session.clone();
    let direct_cwd = cli.cwd.clone();
    tokio::spawn(async move {
        // Initialize handshake
        match acp_init.initialize().await {
            Ok(init) => {
                if let Some(model) = init
                    .get("agentInfo")
                    .or_else(|| init.get("agent_info"))
                    .and_then(|s| s.get("name"))
                    .and_then(|m| m.as_str())
                {
                    let _ = event_tx_init.send(event::AppEvent::SlashCommandResponse(
                        format!("__model_name:{}", model),
                    ));
                }
            }
            Err(e) => {
                let _ = event_tx_init.send(event::AppEvent::AcpError(
                    format!("ACP initialize failed: {}", e),
                ));
            }
        }

        // If direct session requested, resume it immediately
        if let Some(sid) = direct_sid {
            match acp_init.resume_session(&direct_cwd, &sid).await {
                Ok(()) => {
                    let _ = event_tx_init.send(event::AppEvent::SessionResumed(sid));
                }
                Err(e) => {
                    let _ = event_tx_init.send(event::AppEvent::AcpError(
                        format!("Failed to resume session: {}", e),
                    ));
                }
            }
        } else {
            // Fetch sessions for the picker
            match acp_init.list_sessions().await {
                Ok(sessions) => {
                    let _ = event_tx_init.send(event::AppEvent::SessionsLoaded(sessions));
                }
                Err(e) => {
                    let _ = event_tx_init.send(event::AppEvent::AcpError(
                        format!("Failed to list sessions: {}", e),
                    ));
                }
            }
        }

        let _ = event_tx_init.send(event::AppEvent::AcpReady);
    });

    let result = run(&mut terminal, &mut app, &mut events, acp.clone(), &cli.cwd, cli.profile.as_deref()).await;

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
    mut acp: Arc<acp::AcpClient>,
    cwd: &str,
    profile: Option<&str>,
) -> Result<()> {
    loop {
        terminal.draw(|frame| ui::draw(frame, app))?;

        match events.next().await? {
            event::AppEvent::Key(key) => {
                app.handle_key(key, &acp, cwd).await?;
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
                if err.contains("subprocess exited") || err.contains("ACP subprocess") {
                    // ACP process died — show disconnected screen
                    app.screen = app::Screen::Disconnected(err);
                    app.status = app::AgentStatus::Idle;
                } else {
                    app.sys_msg(format!("ACP error: {}", err));
                    app.status = app::AgentStatus::Idle;
                }
            }
            event::AppEvent::SessionCreated(sid) => {
                app.session_id = Some(sid);
                app.status = app::AgentStatus::Idle;
                app.sys_msg("Session ready.");
            }
            event::AppEvent::SessionResumed(sid) => {
                app.session_id = Some(sid.clone());
                app.sys_msg("Session resumed. Loading history…");

                // Fetch session history via the extension method (replaces /context hack)
                let acp_hist = acp.clone();
                let event_tx_hist = app.event_tx.as_ref().unwrap().clone();
                tokio::spawn(async move {
                    match acp_hist.get_session_history(&sid, 50, 0).await {
                        Ok((messages, total)) => {
                            let _ = event_tx_hist
                                .send(event::AppEvent::HistoryLoaded(messages, total));
                        }
                        Err(_) => {
                            // Extension method not available — fallback to /context
                            let _ = event_tx_hist.send(event::AppEvent::HistoryFallback(sid));
                        }
                    }
                });
            }
            event::AppEvent::HistoryLoaded(messages, total) => {
                app.load_history(messages, total, false);
                app.status = app::AgentStatus::Idle;
            }
            event::AppEvent::HistoryFallback(sid) => {
                // Fallback: send /context like before
                app.sys_msg("History unavailable, fetching context…");
                let acp_ctx = acp.clone();
                let event_tx_ctx = app.event_tx.as_ref().unwrap().clone();
                tokio::spawn(async move {
                    if let Ok(val) = acp_ctx.prompt("/context", &sid).await {
                        let stop_reason = val
                            .get("stopReason")
                            .or_else(|| val.get("stop_reason"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("end_turn")
                            .to_string();
                        let _ = event_tx_ctx.send(
                            event::AppEvent::PromptDone { stop_reason, usage: None },
                        );
                    }
                });
            }
            event::AppEvent::SessionsLoaded(sessions) => {
                app.sessions = sessions;
            }
            event::AppEvent::LoadMoreHistory => {
                if let Some(sid) = &app.session_id {
                    let sid = sid.clone();
                    let offset = app.history_loaded;
                    let acp_h = acp.clone();
                    let tx = app.event_tx.as_ref().unwrap().clone();
                    tokio::spawn(async move {
                        match acp_h.get_session_history(&sid, 50, offset).await {
                            Ok((msgs, total)) => {
                                let _ = tx.send(event::AppEvent::HistoryPage(msgs, total));
                            }
                            Err(_) => {
                                // Silently fail — user can try scrolling again
                            }
                        }
                    });
                }
            }
            event::AppEvent::HistoryPage(messages, total) => {
                app.load_history(messages, total, true);
            }
            event::AppEvent::AcpReady => {
                // ACP is ready — picker can now accept Enter
            }
            event::AppEvent::SlashCommandResponse(text) => {
                // Hack: model name arrives via this channel from init
                if let Some(model) = text.strip_prefix("__model_name:") {
                    app.model_name = model.to_string();
                } else {
                    app.sys_msg(text);
                }
            }
            event::AppEvent::ReconnectRequested => {
                // Respawn the ACP subprocess
                app.sys_msg("Reconnecting…");
                app.screen = app::Screen::Picker;
                app.sessions.clear();
                app.session_id = None;
                app.messages.clear();
                app.messages.push(app::ChatMessage {
                    role: app::Role::System,
                    content: "Reconnecting to Hermes…".into(),
                    tokens: None,
                });
                app.status = app::AgentStatus::Idle;
                app.picker_selected = 0;

                let event_tx = events.sender();
                match acp::AcpClient::spawn(event_tx.clone(), profile).await {
                    Ok(new_client) => {
                        acp = Arc::new(new_client);
                        // Re-initialize
                        let acp_init = Arc::clone(&acp);
                        let event_tx_init = event_tx.clone();
                        tokio::spawn(async move {
                            if let Ok(init) = acp_init.initialize().await {
                                if let Some(model) = init
                                    .get("agentInfo")
                                    .or_else(|| init.get("agent_info"))
                                    .and_then(|s| s.get("name"))
                                    .and_then(|m| m.as_str())
                                {
                                    let _ = event_tx_init.send(event::AppEvent::SlashCommandResponse(
                                        format!("__model_name:{}", model),
                                    ));
                                }
                            }
                            if let Ok(sessions) = acp_init.list_sessions().await {
                                let _ = event_tx_init.send(event::AppEvent::SessionsLoaded(sessions));
                            }
                            let _ = event_tx_init.send(event::AppEvent::AcpReady);
                        });
                    }
                    Err(e) => {
                        app.screen = app::Screen::Disconnected(
                            format!("Failed to restart: {}", e),
                        );
                    }
                }
            }
        }

        if app.should_quit() {
            return Ok(());
        }
    }
}
