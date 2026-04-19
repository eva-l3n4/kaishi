use crate::app::{App, SubagentStatus, SubagentTranscriptKind};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

pub fn draw_zoom(frame: &mut Frame, area: Rect, app: &App, child_session_id: &str) {
    let task = app.subagents.get(child_session_id);

    // Layout: header bar (3 rows) + body + footer hint (1 row)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

    // --- Header ---
    let header_text = match task {
        Some(t) => render_header_line(t),
        None => vec![Line::from(Span::styled(
            "  \u{2190} subagent (unknown)",
            Style::default().fg(Color::DarkGray),
        ))],
    };
    let header = Paragraph::new(header_text).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    frame.render_widget(header, chunks[0]);

    // --- Body ---
    let body_lines: Vec<Line> = match task {
        Some(t) => render_body_lines(t, chunks[1].width),
        None => vec![Line::from(
            "  (no events recorded yet \u{2014} waiting for subagent to report)",
        )],
    };
    let body = Paragraph::new(body_lines).scroll((app.subagent_zoom_scroll, 0));
    frame.render_widget(body, chunks[1]);

    // --- Footer ---
    let footer = Paragraph::new(Line::from(vec![
        Span::styled("  \u{2191}", Style::default().fg(Color::Yellow)),
        Span::styled(" back to parent   ", Style::default().fg(Color::DarkGray)),
        Span::styled("\u{2193}/PageDown", Style::default().fg(Color::Yellow)),
        Span::styled(" scroll   ", Style::default().fg(Color::DarkGray)),
        Span::styled("Esc", Style::default().fg(Color::Yellow)),
        Span::styled(" exit zoom", Style::default().fg(Color::DarkGray)),
    ]));
    frame.render_widget(footer, chunks[2]);
}

fn render_header_line(task: &crate::app::SubagentTask) -> Vec<Line<'static>> {
    let mut left_spans = vec![
        Span::styled("  \u{2190} ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!(
                "\u{2387} Subagent [{}/{}] ",
                task.task_index + 1,
                task.task_count
            ),
            Style::default().fg(Color::Magenta),
        ),
        Span::styled(
            task.goal.clone(),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
    ];
    let status = match task.status {
        SubagentStatus::Running => {
            Span::styled("   running", Style::default().fg(Color::Yellow))
        }
        SubagentStatus::Done => {
            let dur = task
                .duration_seconds
                .map(|d| format!(" \u{2713} {:.1}s", d))
                .unwrap_or_else(|| " \u{2713}".into());
            Span::styled(format!("  {}", dur), Style::default().fg(Color::Green))
        }
        SubagentStatus::Failed => {
            let dur = task
                .duration_seconds
                .map(|d| format!(" \u{2717} {:.1}s", d))
                .unwrap_or_else(|| " \u{2717}".into());
            Span::styled(format!("  {}", dur), Style::default().fg(Color::Red))
        }
    };
    left_spans.push(status);
    vec![Line::from(left_spans), Line::from("")]
}

fn render_body_lines(task: &crate::app::SubagentTask, _width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for ev in &task.events {
        match &ev.kind {
            SubagentTranscriptKind::Start { goal } => {
                lines.push(Line::from(vec![
                    Span::styled("\u{25b8} ", Style::default().fg(Color::Magenta)),
                    Span::styled("Started: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(goal.clone(), Style::default().fg(Color::Cyan)),
                ]));
                lines.push(Line::from(""));
            }
            SubagentTranscriptKind::Thinking { text } => {
                lines.push(Line::from(vec![
                    Span::styled("\u{1f4ad} ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        text.clone(),
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::ITALIC),
                    ),
                ]));
            }
            SubagentTranscriptKind::Tool { name, preview } => {
                let mut spans = vec![
                    Span::styled("\u{2192} ", Style::default().fg(Color::Blue)),
                    Span::styled(
                        name.clone(),
                        Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD),
                    ),
                ];
                if let Some(p) = preview {
                    spans.push(Span::styled(
                        format!("  {}", p),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                lines.push(Line::from(spans));
            }
            SubagentTranscriptKind::Complete {
                status,
                summary,
                duration_seconds,
            } => {
                lines.push(Line::from(""));
                let glyph = if status == "failed" { "\u{2717}" } else { "\u{2713}" };
                let color = if status == "failed" {
                    Color::Red
                } else {
                    Color::Green
                };
                let dur_str = duration_seconds
                    .map(|d| format!(" ({:.1}s)", d))
                    .unwrap_or_default();
                let mut spans = vec![Span::styled(
                    format!("{} Complete{}", glyph, dur_str),
                    Style::default().fg(color),
                )];
                if let Some(s) = summary {
                    spans.push(Span::styled(
                        format!("  \u{2014} {}", s),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                lines.push(Line::from(spans));
            }
        }
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (waiting for subagent to report\u{2026})",
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines
}
