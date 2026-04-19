//! Full-screen zoom view for a delegated subagent.
//!
//! Renders the child session's transcript using the same visual vocabulary
//! as the parent chat view: `┌─ tool  ──` framed tool calls, `│ preview`
//! content rows, dimmed reasoning prefixed with `├─ `, and a minimal
//! header/footer. Colors come exclusively from the `ui::palette` module —
//! no hardcoded `Color::Rgb` and no raw `Color::Blue`/`Color::Magenta`
//! that would clash with the Catppuccin theme remapping.

use crate::app::{App, Role, SubagentStatus, SubagentTask, SubagentTranscriptKind};
use crate::ui::palette;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

pub fn draw_zoom(frame: &mut Frame, area: Rect, app: &mut App, child_session_id: &str) {
    let task = app.subagents.get(child_session_id);

    let subagent_count = app
        .messages
        .iter()
        .filter(|m| matches!(m.role, Role::Subagent))
        .count();

    // Layout: header (2 rows) + body (rest) + footer (1 row).
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

    // ── Header ──
    let header_lines: Vec<Line> = match task {
        Some(t) => render_header(t, chunks[0].width),
        None => vec![
            Line::from(Span::styled(
                "  ⎇  subagent (unknown)",
                Style::default().fg(palette::DIM),
            )),
            Line::from(""),
        ],
    };
    frame.render_widget(Paragraph::new(header_lines), chunks[0]);

    // ── Body ──
    let body_lines: Vec<Line> = match task {
        Some(t) => render_body(t, chunks[1].width),
        None => vec![Line::from(Span::styled(
            "  (no events recorded yet)",
            Style::default().fg(palette::DIM),
        ))],
    };

    // Stash geometry for key handlers + compute scroll position.
    // Scroll semantics mirror the chat view: `subagent_zoom_scroll == 0`
    // means pinned to bottom (newest content visible, auto-tails as new
    // events stream in). Higher values scroll backward into history.
    let content_rows = body_lines.len() as u16;
    let viewport_rows = chunks[1].height;
    app.subagent_zoom_content_rows = content_rows;
    app.subagent_zoom_viewport_rows = viewport_rows;

    let max_scroll = content_rows.saturating_sub(viewport_rows);
    // Clamp against shrinking content (e.g. after switching subagents).
    if app.subagent_zoom_scroll > max_scroll {
        app.subagent_zoom_scroll = max_scroll;
    }
    // Ratatui's `Paragraph::scroll` is "rows-from-top". Our offset is
    // "rows-from-bottom", so subtract from max to get the render value.
    let scroll_from_top = max_scroll.saturating_sub(app.subagent_zoom_scroll);

    frame.render_widget(
        Paragraph::new(body_lines).scroll((scroll_from_top, 0)),
        chunks[1],
    );

    // ── Footer ──
    let mut footer_spans = vec![
        Span::styled("  ↑", Style::default().fg(palette::ACCENT_SYSTEM)),
        Span::styled(" parent   ", Style::default().fg(palette::DIM)),
        Span::styled("↓ PgDn", Style::default().fg(palette::ACCENT_SYSTEM)),
        Span::styled(" scroll   ", Style::default().fg(palette::DIM)),
    ];
    if subagent_count > 1 {
        footer_spans.push(Span::styled(
            "^Z",
            Style::default().fg(palette::ACCENT_SYSTEM),
        ));
        footer_spans.push(Span::styled(" next   ", Style::default().fg(palette::DIM)));
    }
    footer_spans.push(Span::styled(
        "Esc",
        Style::default().fg(palette::ACCENT_SYSTEM),
    ));
    footer_spans.push(Span::styled(" exit", Style::default().fg(palette::DIM)));
    frame.render_widget(Paragraph::new(Line::from(footer_spans)), chunks[2]);
}

// ───────────────────────── header ─────────────────────────

fn render_header(task: &SubagentTask, width: u16) -> Vec<Line<'static>> {
    // Row 1: "  ⎇  [n/N]  ● status · duration"
    // Row 2: "  <goal, dim, wrapped/truncated>"
    let (dot, dot_color, status_text) = match task.status {
        SubagentStatus::Running => ("●", palette::ACCENT_SYSTEM, "running".to_string()),
        SubagentStatus::Done => (
            "●",
            palette::SUCCESS,
            task.duration_seconds
                .map(|d| format!("done · {}", fmt_duration(d)))
                .unwrap_or_else(|| "done".to_string()),
        ),
        SubagentStatus::Failed => (
            "●",
            palette::ERROR,
            task.duration_seconds
                .map(|d| format!("failed · {}", fmt_duration(d)))
                .unwrap_or_else(|| "failed".to_string()),
        ),
    };

    let mut row1 = vec![
        Span::styled("  ", Style::default()),
        Span::styled("⎇  ", Style::default().fg(palette::ACCENT_ASSISTANT)),
    ];
    if task.task_count > 1 {
        row1.push(Span::styled(
            format!("[{}/{}]  ", task.task_index + 1, task.task_count),
            Style::default().fg(palette::DIM),
        ));
    }
    row1.push(Span::styled(dot.to_string(), Style::default().fg(dot_color)));
    row1.push(Span::styled(
        format!(" {}", status_text),
        Style::default().fg(palette::DIM),
    ));

    // Row 2: goal, dimmed. Truncate to viewport with ellipsis if needed.
    let max_goal_width = (width as usize).saturating_sub(4); // "  " prefix + room
    let goal_display = truncate_display(&task.goal, max_goal_width);
    let row2 = vec![
        Span::styled("  ", Style::default()),
        Span::styled(
            goal_display,
            Style::default().fg(palette::TEXT).add_modifier(Modifier::BOLD),
        ),
    ];

    vec![Line::from(row1), Line::from(row2)]
}

// ───────────────────────── body ─────────────────────────

fn render_body(task: &SubagentTask, width: u16) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = Vec::new();

    if task.events.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  (waiting for subagent to report)",
            Style::default().fg(palette::DIM),
        )));
        return lines;
    }

    for (i, ev) in task.events.iter().enumerate() {
        if i > 0 {
            lines.push(Line::from(""));
        }
        match &ev.kind {
            SubagentTranscriptKind::Start { .. } => {
                // Redundant with the header; skip in body.
            }
            SubagentTranscriptKind::Thinking { text } => {
                render_thinking(&mut lines, text, width);
            }
            SubagentTranscriptKind::Tool { name, preview } => {
                render_tool_box(&mut lines, name, preview.as_deref(), width);
            }
            SubagentTranscriptKind::Complete {
                status,
                summary,
                duration_seconds,
            } => {
                render_complete(&mut lines, status, summary.as_deref(), *duration_seconds, width);
            }
        }
    }

    lines
}

fn render_thinking(lines: &mut Vec<Line<'static>>, text: &str, width: u16) {
    // "  ├─ <text, dim italic, wrapped>"
    // Continuation lines align under the text, prefixed with "  │  ".
    let prefix_width = 5; // "  ├─ "
    let body_width = (width as usize).saturating_sub(prefix_width + 2);

    let wrapped = wrap_plain(text, body_width);
    for (i, row) in wrapped.iter().enumerate() {
        let prefix = if i == 0 { "  ├─ " } else { "  │  " };
        lines.push(Line::from(vec![
            Span::styled(prefix.to_string(), Style::default().fg(palette::DIM)),
            Span::styled(
                row.clone(),
                Style::default()
                    .fg(palette::DIM)
                    .add_modifier(Modifier::ITALIC),
            ),
        ]));
    }
}

fn render_tool_box(lines: &mut Vec<Line<'static>>, name: &str, preview: Option<&str>, width: u16) {
    // Matches the parent transcript's framed tool call style:
    //   ┌─ ⚙ tool_name ───────────
    //   │ preview line 1
    //   │ preview line 2
    //   └─
    // The `⚙` glyph and DIM accent mirror the in-progress tool-call
    // rendering in `ui::render_message` (the zoom view never shows a
    // completed/failed state per tool; ✓/✗ appear on the final block
    // rendered by `render_complete`).
    let accent = palette::DIM;

    // "  ┌─ ⚙ " = 2 + 3 + 2 = 7 cols before the name; reserve a bit more
    // slack so the rule never clips into the viewport edge.
    let header_visible = 7 + name.chars().count() + 1; // trailing space
    let remaining = (width as usize).saturating_sub(header_visible + 2);
    let rule = "─".repeat(remaining.min(60));

    lines.push(Line::from(vec![
        Span::styled("  ┌─ ".to_string(), Style::default().fg(palette::BORDER)),
        Span::styled("⚙ ".to_string(), Style::default().fg(accent)),
        Span::styled(
            name.to_string(),
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {}", rule),
            Style::default().fg(palette::BORDER),
        ),
    ]));

    if let Some(p) = preview {
        let p = p.trim();
        if !p.is_empty() {
            // "  │ " = 4 cols
            let max_line = (width as usize).saturating_sub(4);
            for raw_row in p.lines() {
                // Diff/patch-aware coloring — mirrors the parent transcript's
                // tool-message renderer so a `patch` or `terminal` preview
                // that contains a unified diff stays legible here too.
                let row_color = if raw_row.starts_with('+') && !raw_row.starts_with("+++") {
                    Color::Green
                } else if raw_row.starts_with('-') && !raw_row.starts_with("---") {
                    Color::Red
                } else if raw_row.starts_with("@@ ") {
                    Color::Cyan
                } else if raw_row.starts_with("---") || raw_row.starts_with("+++") {
                    Color::Yellow
                } else {
                    palette::TEXT
                };
                for wrapped in wrap_plain(raw_row, max_line) {
                    lines.push(Line::from(vec![
                        Span::styled("  │ ".to_string(), Style::default().fg(palette::BORDER)),
                        Span::styled(wrapped, Style::default().fg(row_color)),
                    ]));
                }
            }
        }
    }

    lines.push(Line::from(Span::styled(
        "  └─".to_string(),
        Style::default().fg(palette::BORDER),
    )));
}

fn render_complete(
    lines: &mut Vec<Line<'static>>,
    status: &str,
    summary: Option<&str>,
    duration_seconds: Option<f64>,
    width: u16,
) {
    let (glyph, color, label) = if status == "failed" {
        ("✗", palette::ERROR, "failed")
    } else {
        ("✓", palette::SUCCESS, "complete")
    };

    // Soft divider — a row of middle-dots in DIM — sets the completion
    // block apart from the last tool call, so the final summary reads as
    // a capstone instead of just another event in the stream.
    let divider_width = (width as usize).saturating_sub(4).min(48);
    lines.push(Line::from(Span::styled(
        format!("  {}", "·".repeat(divider_width)),
        Style::default().fg(palette::DIM),
    )));

    let mut spans = vec![
        Span::styled("  ", Style::default()),
        Span::styled(
            format!("{} {}", glyph, label),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(d) = duration_seconds {
        spans.push(Span::styled(
            format!(" · {}", fmt_duration(d)),
            Style::default().fg(palette::DIM),
        ));
    }
    lines.push(Line::from(spans));

    if let Some(s) = summary.map(str::trim).filter(|s| !s.is_empty()) {
        // Wrap the summary to the viewport so long multi-line summaries
        // from the subagent don't clip off the right edge.
        let max_line = (width as usize).saturating_sub(4); // "  " indent + slack
        for row in wrap_plain(s, max_line) {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(row, Style::default().fg(palette::TEXT)),
            ]));
        }
    }
}

// ───────────────────────── helpers ─────────────────────────

fn fmt_duration(secs: f64) -> String {
    if secs < 10.0 {
        format!("{:.1}s", secs)
    } else if secs < 60.0 {
        format!("{:.0}s", secs)
    } else {
        let m = (secs / 60.0) as u64;
        let s = (secs as u64) % 60;
        format!("{}m{:02}s", m, s)
    }
}

/// Truncate by display width (not byte length); appends a single-character
/// horizontal ellipsis when clipped.
fn truncate_display(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    let keep: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{keep}…")
}

/// Dumb word-wrap by characters, preserving existing newlines.
/// Keeps it simple — no soft-break heuristics, just fits words to width.
fn wrap_plain(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    for raw_line in text.lines() {
        let mut current = String::new();
        let mut current_width = 0usize;
        for word in raw_line.split_whitespace() {
            let w_width = word.chars().count();
            if current_width == 0 {
                if w_width > width {
                    // force-break very long token
                    let mut buf = String::new();
                    for ch in word.chars() {
                        if buf.chars().count() >= width {
                            out.push(std::mem::take(&mut buf));
                        }
                        buf.push(ch);
                    }
                    current = buf;
                    current_width = current.chars().count();
                } else {
                    current.push_str(word);
                    current_width = w_width;
                }
            } else if current_width + 1 + w_width <= width {
                current.push(' ');
                current.push_str(word);
                current_width += 1 + w_width;
            } else {
                out.push(std::mem::take(&mut current));
                current.push_str(word);
                current_width = w_width;
            }
        }
        out.push(current);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_shorter_than_max_is_unchanged() {
        assert_eq!(truncate_display("hello", 10), "hello");
    }

    #[test]
    fn truncate_appends_ellipsis() {
        assert_eq!(truncate_display("hello world", 6), "hello…");
    }

    #[test]
    fn truncate_zero_max_is_empty() {
        assert_eq!(truncate_display("hello", 0), "");
    }

    #[test]
    fn wrap_short_text_is_single_row() {
        assert_eq!(wrap_plain("hi there", 80), vec!["hi there".to_string()]);
    }

    #[test]
    fn wrap_splits_on_word_boundaries() {
        let rows = wrap_plain("one two three four five", 9);
        assert_eq!(rows, vec!["one two".to_string(), "three".to_string(), "four five".to_string()]);
    }

    #[test]
    fn fmt_duration_scales() {
        assert_eq!(fmt_duration(2.5), "2.5s");
        assert_eq!(fmt_duration(42.0), "42s");
        assert_eq!(fmt_duration(125.0), "2m05s");
    }
}
