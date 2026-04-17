use ratatui::{
    Frame,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::event::SessionInfo;

/// Draw the session picker screen.
pub fn draw_picker(frame: &mut Frame, sessions: &[SessionInfo], selected: usize) {
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
    let marker = if selected == 0 { "  > " } else { "    " };
    let style = if selected == 0 {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };
    lines.push(Line::from(Span::styled(
        format!("{}+ New Session", marker),
        style,
    )));

    lines.push(Line::from(""));

    // Existing sessions
    for (i, session) in sessions.iter().enumerate() {
        let idx = i + 1; // offset by New Session
        let marker = if selected == idx { "  > " } else { "    " };

        // Title or fallback to last cwd component or session_id prefix
        let label = if let Some(ref title) = session.title {
            title.clone()
        } else if session.cwd != "." {
            session
                .cwd
                .rsplit('/')
                .next()
                .unwrap_or(&session.cwd)
                .to_string()
        } else {
            session.session_id[..8.min(session.session_id.len())].to_string()
        };

        // Relative time from last_active or started_at
        let time_hint = session
            .last_active
            .or(session.started_at)
            .map(format_relative_time)
            .unwrap_or_default();

        // Source badge
        let source_badge = session
            .source
            .as_deref()
            .map(|s| match s {
                "acp" => "",
                "cli" => " [cli]",
                "discord" => " [discord]",
                "telegram" => " [telegram]",
                other => {
                    // We can't return a formatted &str, handled below
                    let _ = other;
                    ""
                }
            })
            .unwrap_or("");

        // For unknown sources, build a owned string
        let source_display = if source_badge.is_empty() {
            session
                .source
                .as_deref()
                .filter(|s| *s != "acp")
                .map(|s| format!(" [{}]", s))
                .unwrap_or_default()
        } else {
            source_badge.to_string()
        };

        let detail = format!(
            "{}{}{}",
            format_args!("{} msgs", session.history_len),
            if time_hint.is_empty() {
                String::new()
            } else {
                format!(", {}", time_hint)
            },
            source_display,
        );

        let style = if selected == idx {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        let detail_style = if selected == idx {
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        // Truncate label to fit
        let max_label = 36;
        let display_label = if label.len() > max_label {
            format!("{}…", &label[..max_label - 1])
        } else {
            label
        };

        lines.push(Line::from(vec![
            Span::styled(format!("{}{:<38}", marker, display_label), style),
            Span::styled(format!("  {}", detail), detail_style),
        ]));
    }

    lines.push(Line::from(""));

    let hint_text = if sessions.is_empty() {
        "  Connecting to Hermes…"
    } else {
        "  Enter: select  Esc: quit"
    };
    let hint = Line::from(Span::styled(
        hint_text,
        Style::default().fg(Color::DarkGray),
    ));
    lines.push(hint);

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

/// Format a UNIX timestamp as a relative time string.
fn format_relative_time(ts: f64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);

    let delta = (now - ts).max(0.0) as u64;

    if delta < 60 {
        "just now".to_string()
    } else if delta < 3600 {
        let mins = delta / 60;
        format!("{}m ago", mins)
    } else if delta < 86400 {
        let hours = delta / 3600;
        format!("{}h ago", hours)
    } else {
        let days = delta / 86400;
        format!("{}d ago", days)
    }
}
