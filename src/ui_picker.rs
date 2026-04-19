use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
};

use unicode_width::UnicodeWidthStr;

use crate::event::SessionInfo;

/// Draw the session picker screen as a scrollable card list.
pub fn draw_picker(
    frame: &mut Frame,
    sessions: &[SessionInfo],
    selected: usize,
    scroll_offset: u16,
) {
    let area = frame.area();
    let narrow = area.width < 60;

    // Layout: header + session list + footer hint
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header block
            Constraint::Min(3),   // Session list (scrollable)
            Constraint::Length(2), // Footer hints
        ])
        .split(area);

    draw_header(frame, chunks[0]);
    draw_session_list(frame, sessions, selected, scroll_offset, chunks[1], narrow);
    draw_footer(frame, sessions, chunks[2]);
}

fn draw_header(frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let title = Line::from(vec![
        Span::styled(" 🌸 ", Style::default()),
        Span::styled(
            "懐紙 Kaishi",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" — Sessions", Style::default().fg(Color::DarkGray)),
    ]);

    frame.render_widget(Paragraph::new(title), inner);
}

fn draw_session_list(
    frame: &mut Frame,
    sessions: &[SessionInfo],
    selected: usize,
    scroll_offset: u16,
    area: Rect,
    narrow: bool,
) {
    let inner_width = area.width.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = Vec::new();

    // ── "New Session" card ──────────────────────────
    let is_selected = selected == 0;
    render_new_session_card(&mut lines, is_selected, inner_width, narrow);

    // ── Existing session cards ──────────────────────
    for (i, session) in sessions.iter().enumerate() {
        let idx = i + 1;
        let is_selected = selected == idx;
        render_session_card(&mut lines, session, is_selected, inner_width, narrow);
    }

    // Scrolling — scroll_offset counts from the top, 0 = top of list
    let total_lines = lines.len() as u16;
    let visible_height = area.height;
    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll_pos = scroll_offset.min(max_scroll);

    let paragraph = Paragraph::new(Text::from(lines)).scroll((scroll_pos, 0));

    frame.render_widget(paragraph, area);

    // Scrollbar
    if total_lines > visible_height {
        let mut scrollbar_state =
            ScrollbarState::new(max_scroll as usize).position(scroll_pos as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("↑"))
                .end_symbol(Some("↓")),
            area,
            &mut scrollbar_state,
        );
    }
}

fn render_new_session_card(
    lines: &mut Vec<Line<'static>>,
    selected: bool,
    _width: usize,
    _narrow: bool,
) {
    let bg = if selected {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    let marker = if selected { "▌ " } else { "  " };
    let marker_style = if selected {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    lines.push(Line::from(vec![
        Span::styled(marker, marker_style),
        Span::styled("+ New Session", bg),
    ]));

    // Separator
    lines.push(Line::from(""));
}

fn render_session_card(
    lines: &mut Vec<Line<'static>>,
    session: &SessionInfo,
    selected: bool,
    width: usize,
    narrow: bool,
) {
    let marker = if selected { "▌ " } else { "  " };
    let marker_style = if selected {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    // ── Line 1: Title ──────────────────────────────
    let title = session_title(session);
    let max_title = if narrow {
        width.saturating_sub(4)
    } else {
        width.saturating_sub(16) // leave room for time
    };

    let display_title = truncate_str(&title, max_title);
    let title_style = if selected {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    let mut title_spans = vec![
        Span::styled(marker.to_string(), marker_style),
        Span::styled(display_title.to_string(), title_style),
    ];

    // Right-aligned time on wide viewports
    if !narrow {
        let time_hint = session
            .last_active
            .or(session.started_at)
            .map(format_relative_time)
            .unwrap_or_default();

        if !time_hint.is_empty() {
            let title_display_width = marker.width() + display_title.width();
            let padding = width.saturating_sub(title_display_width + time_hint.width() + 1);
            title_spans.push(Span::raw(" ".repeat(padding)));
            title_spans.push(Span::styled(
                time_hint,
                Style::default().fg(Color::DarkGray),
            ));
        }
    }

    lines.push(Line::from(title_spans));

    // ── Line 2: Metadata (dim) ─────────────────────
    let indent = if selected { "▌ " } else { "  " };
    let mut meta_parts: Vec<String> = Vec::new();

    // Message count
    meta_parts.push(format!("{} msgs", session.history_len));

    // Source badge (skip "acp" — that's us)
    if let Some(ref source) = session.source {
        if source != "acp" {
            meta_parts.push(format!("[{}]", source));
        }
    }

    // Model (if available and different from default)
    if !session.model.is_empty() {
        let model_display = if session.model.len() > 20 {
            format!("{}…", &session.model[..19])
        } else {
            session.model.clone()
        };
        meta_parts.push(model_display);
    }

    // CWD (abbreviated)
    let cwd_hint = if session.cwd != "." {
        session
            .cwd
            .rsplit('/')
            .next()
            .unwrap_or(&session.cwd)
            .to_string()
    } else {
        String::new()
    };
    if !cwd_hint.is_empty() && !narrow {
        meta_parts.push(format!("~/{}", cwd_hint));
    }

    let meta_text = meta_parts.join(" · ");
    let max_meta = width.saturating_sub(4);
    let display_meta = truncate_str(&meta_text, max_meta);

    let meta_style = Style::default().fg(Color::DarkGray);
    lines.push(Line::from(vec![
        Span::styled(indent, marker_style),
        Span::styled(display_meta.to_string(), meta_style),
    ]));

    // Separator line (empty)
    lines.push(Line::from(""));
}

fn draw_footer(frame: &mut Frame, sessions: &[SessionInfo], area: Rect) {
    let hint_text = if sessions.is_empty() {
        " Connecting to Hermes…"
    } else {
        " ↑↓/jk: navigate  Enter: select  Esc: quit"
    };

    let line = Line::from(Span::styled(
        hint_text,
        Style::default().fg(Color::DarkGray),
    ));

    frame.render_widget(Paragraph::new(line), area);
}

// ── Helpers ────────────────────────────────────────────────

fn session_title(session: &SessionInfo) -> String {
    if let Some(ref title) = session.title {
        title.clone()
    } else if session.cwd != "." {
        session
            .cwd
            .rsplit('/')
            .next()
            .unwrap_or(&session.cwd)
            .to_string()
    } else {
        // Short session ID
        let end = session
            .session_id
            .char_indices()
            .nth(8)
            .map(|(i, _)| i)
            .unwrap_or(session.session_id.len());
        session.session_id[..end].to_string()
    }
}

/// Truncate a string by character count, adding ellipsis if needed.
fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.width() <= max_chars {
        return s.to_string();
    }
    let mut w = 0;
    let truncated: String = s
        .chars()
        .take_while(|c| {
            let cw = c.to_string().width();
            w += cw;
            w <= max_chars.saturating_sub(1)
        })
        .collect();
    format!("{}…", truncated)
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
