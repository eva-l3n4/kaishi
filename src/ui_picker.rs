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

        let detail = format!("{} msgs", session.history_len);

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
