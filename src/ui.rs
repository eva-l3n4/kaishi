use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
};

use crate::app::{AgentStatus, App, ChatMessage, ModalState, Role, Screen};
use crate::ui_modal;
use crate::ui_picker;

/// Spinner frames for the streaming indicator.
const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Indent prefix for message body lines.
const INDENT: &str = "    ";

/// Top-level draw — dispatches to the active screen, then overlays modal.
pub fn draw(frame: &mut Frame, app: &App) {
    match app.screen {
        Screen::Picker => {
            ui_picker::draw_picker(frame, &app.sessions, app.picker_selected);
        }
        Screen::Chat => {
            draw_chat(frame, app);
        }
    }

    // Modal overlay (drawn on top of any screen)
    if let ModalState::Approval {
        ref command,
        ref options,
        selected,
        ..
    } = app.modal
    {
        ui_modal::draw_approval_modal(frame, command, options, selected);
    }
}

/// Draw the chat view (status bar + messages + input).
fn draw_chat(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let input_lines = app.input.lines().count().max(1);
    let input_height = (input_lines as u16 + 2).clamp(3, 10);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),            // Status bar
            Constraint::Min(5),              // Messages
            Constraint::Length(input_height), // Input
        ])
        .split(area);

    draw_status_bar(frame, app, chunks[0]);
    draw_messages(frame, app, chunks[1]);
    draw_input(frame, app, chunks[2]);
}

fn draw_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let model = if app.model_name.is_empty() {
        "hermes"
    } else {
        &app.model_name
    };

    let session_hint = if let Some(ref title) = app.session_title {
        format!(" │ {}", truncate(title, 30))
    } else if let Some(ref sid) = app.session_id {
        let short = if sid.len() > 8 { &sid[..8] } else { sid };
        format!(" │ {}", short)
    } else {
        String::new()
    };

    let status_text = match &app.status {
        AgentStatus::Idle => {
            let msg_count = app.messages.iter().filter(|m| m.role == Role::User).count();
            format!(
                " 🌸 Hanami │ {} │ {} msgs{}",
                model, msg_count, session_hint,
            )
        }
        AgentStatus::Thinking => {
            let spinner = SPINNER[(app.tick as usize) % SPINNER.len()];
            let tool_hint = if let Some((_, name)) = app.active_tools.last() {
                format!(" ({})", name)
            } else {
                String::new()
            };
            format!(
                " {} thinking…{} │ {}{}",
                spinner, tool_hint, model, session_hint
            )
        }
        AgentStatus::Error(e) => {
            format!(" ⚠ {} │ {}", truncate(e, 40), model)
        }
    };

    let style = match &app.status {
        AgentStatus::Idle => Style::default().bg(Color::DarkGray).fg(Color::White),
        AgentStatus::Thinking => Style::default().bg(Color::Blue).fg(Color::White),
        AgentStatus::Error(_) => Style::default().bg(Color::Red).fg(Color::White),
    };

    let help = " /help │ Ctrl+C quit ";
    let left_width = area.width.saturating_sub(help.len() as u16) as usize;
    let padded_left = format!("{:<width$}", status_text, width = left_width);

    let bar = Line::from(vec![
        Span::styled(padded_left, style),
        Span::styled(help, style.add_modifier(Modifier::DIM)),
    ]);

    frame.render_widget(Paragraph::new(bar), area);
}

fn draw_messages(frame: &mut Frame, app: &App, area: Rect) {
    let inner_width = area.width.saturating_sub(2) as usize; // borders

    let mut all_lines: Vec<Line> = Vec::new();

    for msg in &app.messages {
        render_message(&mut all_lines, msg, inner_width);
        all_lines.push(Line::from(""));
    }

    // Render the in-progress streaming response
    if !app.pending_response.is_empty() {
        let label = Line::from(vec![
            Span::styled("  ◆ ", Style::default().fg(Color::Magenta)),
            Span::styled("assistant", Style::default().fg(Color::Magenta).bold()),
            Span::styled(
                " (streaming…)",
                Style::default().fg(Color::DarkGray).italic(),
            ),
        ]);
        all_lines.push(label);

        render_markdown_lines(&mut all_lines, &app.pending_response, inner_width);

        // Blinking cursor at end
        if app.tick % 4 < 2 {
            if let Some(last) = all_lines.last_mut() {
                let mut spans = last.spans.clone();
                spans.push(Span::styled("█", Style::default().fg(Color::Magenta)));
                *last = Line::from(spans);
            }
        }
        all_lines.push(Line::from(""));
    }

    // Show pending thought if verbose
    if app.verbose && !app.pending_thought.is_empty() {
        let label = Line::from(vec![
            Span::styled("  ○ ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "thinking…",
                Style::default().fg(Color::DarkGray).italic(),
            ),
        ]);
        all_lines.push(label);
        for line in app.pending_thought.lines() {
            all_lines.push(Line::from(Span::styled(
                format!("{}{}", INDENT, line),
                Style::default().fg(Color::DarkGray),
            )));
        }
        all_lines.push(Line::from(""));
    }

    // ── Pre-wrap: split any line wider than inner_width ──────────────
    all_lines = pre_wrap_lines(all_lines, inner_width);

    let total_lines = all_lines.len() as u16;
    let visible_height = area.height.saturating_sub(2);
    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll_pos = max_scroll.saturating_sub(app.scroll_offset.min(max_scroll));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title_bottom(if app.scroll_offset > 0 {
            format!(" ↑ {} lines above ", app.scroll_offset)
        } else {
            String::new()
        });

    let paragraph = Paragraph::new(Text::from(all_lines))
        .block(block)
        .scroll((scroll_pos, 0));

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

/// Split any Line wider than `max_width` into multiple Lines.
fn pre_wrap_lines(lines: Vec<Line<'static>>, max_width: usize) -> Vec<Line<'static>> {
    if max_width == 0 {
        return lines;
    }
    let mut result = Vec::with_capacity(lines.len());
    for line in lines {
        if line.width() <= max_width {
            result.push(line);
            continue;
        }
        let style = line.spans.first().map(|s| s.style).unwrap_or_default();
        let full: String = line.spans.iter().map(|s| s.content.as_ref()).collect();

        let mut chars = full.chars().peekable();
        while chars.peek().is_some() {
            let chunk: String = chars.by_ref().take(max_width).collect();
            result.push(Line::from(Span::styled(chunk, style)));
        }
    }
    result
}

fn render_message(lines: &mut Vec<Line>, msg: &ChatMessage, width: usize) {
    let (icon, color, label) = match msg.role {
        Role::User => ("  ❯ ", Color::Cyan, "you"),
        Role::Assistant => ("  ◆ ", Color::Magenta, "assistant"),
        Role::System => ("  ● ", Color::Yellow, "system"),
        Role::Tool => ("  ⚙ ", Color::DarkGray, "tool"),
        Role::Thought => ("  ○ ", Color::DarkGray, "thought"),
    };

    let mut header_spans = vec![
        Span::styled(icon, Style::default().fg(color)),
        Span::styled(label, Style::default().fg(color).bold()),
    ];

    if let Some(usage) = &msg.tokens {
        header_spans.push(Span::styled(
            format!("  [{}→{}]", usage.input_tokens, usage.output_tokens),
            Style::default().fg(Color::DarkGray),
        ));
    }

    lines.push(Line::from(header_spans));

    match msg.role {
        Role::Assistant => {
            render_markdown_lines(lines, &msg.content, width);
        }
        Role::Thought => {
            for text_line in msg.content.lines() {
                lines.push(Line::from(Span::styled(
                    format!("{}{}", INDENT, text_line),
                    Style::default().fg(Color::DarkGray).italic(),
                )));
            }
        }
        _ => {
            for text_line in msg.content.lines() {
                lines.push(Line::from(format!("{}{}", INDENT, text_line)));
            }
        }
    }
}

// ─── Markdown → ratatui Lines ────────────────────────────────────────────

fn render_markdown_lines(lines: &mut Vec<Line>, text: &str, _width: usize) {
    let mut in_code_block = false;
    let mut code_lang = String::new();

    for raw_line in text.lines() {
        // Fenced code blocks
        if raw_line.trim_start().starts_with("```") {
            if in_code_block {
                lines.push(Line::from(Span::styled(
                    format!("{}└─────", INDENT),
                    Style::default().fg(Color::DarkGray),
                )));
                in_code_block = false;
                code_lang.clear();
            } else {
                code_lang = raw_line.trim_start().trim_start_matches('`').to_string();
                let header = if code_lang.is_empty() {
                    format!("{}┌─────", INDENT)
                } else {
                    format!("{}┌───── {}", INDENT, code_lang)
                };
                lines.push(Line::from(Span::styled(
                    header,
                    Style::default().fg(Color::DarkGray),
                )));
                in_code_block = true;
            }
            continue;
        }

        if in_code_block {
            lines.push(Line::from(Span::styled(
                format!("{}│ {}", INDENT, raw_line),
                Style::default().fg(Color::Green),
            )));
            continue;
        }

        let trimmed = raw_line.trim_start();

        // Headings
        if let Some(heading) = trimmed.strip_prefix("### ") {
            lines.push(Line::from(Span::styled(
                format!("{}{}", INDENT, heading),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )));
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix("## ") {
            lines.push(Line::from(Span::styled(
                format!("{}{}", INDENT, heading),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )));
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix("# ") {
            lines.push(Line::from(Span::styled(
                format!("{}{}", INDENT, heading),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )));
            continue;
        }

        // Horizontal rules
        if trimmed == "---" || trimmed == "***" || trimmed == "___" {
            lines.push(Line::from(Span::styled(
                format!("{}────────────────────────────────", INDENT),
                Style::default().fg(Color::DarkGray),
            )));
            continue;
        }

        // Bullet lists
        if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            let indent_level = raw_line.len() - trimmed.len();
            let extra_indent = " ".repeat(indent_level);
            let body = &trimmed[2..];
            let mut spans = vec![Span::styled(
                format!("{}{}• ", INDENT, extra_indent),
                Style::default().fg(Color::DarkGray),
            )];
            spans.extend(parse_inline_spans(body));
            lines.push(Line::from(spans));
            continue;
        }

        // Numbered lists
        if let Some(rest) = strip_numbered_prefix(trimmed) {
            let indent_level = raw_line.len() - trimmed.len();
            let extra_indent = " ".repeat(indent_level);
            let prefix_len = trimmed.len() - rest.len();
            let mut spans = vec![Span::styled(
                format!("{}{}{}", INDENT, extra_indent, &trimmed[..prefix_len]),
                Style::default().fg(Color::DarkGray),
            )];
            spans.extend(parse_inline_spans(rest));
            lines.push(Line::from(spans));
            continue;
        }

        // Blockquotes
        if let Some(quote) = trimmed.strip_prefix("> ") {
            lines.push(Line::from(vec![
                Span::styled(format!("{}▎ ", INDENT), Style::default().fg(Color::Blue)),
                Span::styled(
                    quote.to_string(),
                    Style::default()
                        .fg(Color::Blue)
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
            continue;
        }

        // Regular paragraph
        if trimmed.is_empty() {
            lines.push(Line::from(""));
        } else {
            let mut spans = vec![Span::raw(INDENT.to_string())];
            spans.extend(parse_inline_spans(raw_line.trim_start()));
            lines.push(Line::from(spans));
        }
    }

    // Close unclosed code block
    if in_code_block {
        lines.push(Line::from(Span::styled(
            format!("{}└─────", INDENT),
            Style::default().fg(Color::DarkGray),
        )));
    }
}

/// Parse inline markdown: **bold**, *italic*, `code`
fn parse_inline_spans(text: &str) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut chars = text.char_indices().peekable();
    let mut current = String::new();

    while let Some(&(i, ch)) = chars.peek() {
        // Inline code: `...`
        if ch == '`' {
            if !current.is_empty() {
                spans.push(Span::raw(std::mem::take(&mut current)));
            }
            chars.next();
            let mut code = String::new();
            let mut closed = false;
            while let Some(&(_, c)) = chars.peek() {
                chars.next();
                if c == '`' {
                    closed = true;
                    break;
                }
                code.push(c);
            }
            if closed {
                spans.push(Span::styled(
                    code,
                    Style::default().fg(Color::Green).bg(Color::Rgb(40, 40, 40)),
                ));
            } else {
                current.push('`');
                current.push_str(&code);
            }
            continue;
        }

        // Bold: **...**
        if ch == '*' && text[i..].starts_with("**") {
            if !current.is_empty() {
                spans.push(Span::raw(std::mem::take(&mut current)));
            }
            chars.next();
            chars.next();
            let mut bold_text = String::new();
            let mut closed = false;
            while let Some(&(j, c)) = chars.peek() {
                if c == '*' && text[j..].starts_with("**") {
                    chars.next();
                    chars.next();
                    closed = true;
                    break;
                }
                chars.next();
                bold_text.push(c);
            }
            if closed {
                spans.push(Span::styled(
                    bold_text,
                    Style::default().add_modifier(Modifier::BOLD),
                ));
            } else {
                current.push_str("**");
                current.push_str(&bold_text);
            }
            continue;
        }

        // Italic: *...*
        if ch == '*' {
            if !current.is_empty() {
                spans.push(Span::raw(std::mem::take(&mut current)));
            }
            chars.next();
            let mut italic_text = String::new();
            let mut closed = false;
            while let Some(&(_, c)) = chars.peek() {
                if c == '*' {
                    chars.next();
                    closed = true;
                    break;
                }
                chars.next();
                italic_text.push(c);
            }
            if closed {
                spans.push(Span::styled(
                    italic_text,
                    Style::default().add_modifier(Modifier::ITALIC),
                ));
            } else {
                current.push('*');
                current.push_str(&italic_text);
            }
            continue;
        }

        chars.next();
        current.push(ch);
    }

    if !current.is_empty() {
        spans.push(Span::raw(current));
    }

    spans
}

/// Strip a numbered list prefix like "1. " and return the rest.
fn strip_numbered_prefix(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 || i >= bytes.len() {
        return None;
    }
    if bytes[i] == b'.' && i + 1 < bytes.len() && bytes[i + 1] == b' ' {
        Some(&s[i + 2..])
    } else {
        None
    }
}

fn draw_input(frame: &mut Frame, app: &App, area: Rect) {
    let prompt_style = if app.status == AgentStatus::Idle {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let title = if app.status == AgentStatus::Idle {
        " ❯ message "
    } else {
        " ⏳ waiting… "
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(prompt_style)
        .title(title);

    // Inner width available for text (subtract 2 for borders)
    let inner_width = area.width.saturating_sub(2) as usize;

    // Calculate horizontal scroll to keep cursor visible
    let scroll_x = if inner_width == 0 {
        0
    } else if app.cursor > inner_width.saturating_sub(1) {
        (app.cursor - inner_width + 1) as u16
    } else {
        0
    };

    let input_paragraph = Paragraph::new(app.input.as_str())
        .block(block)
        .style(Style::default().fg(Color::White))
        .scroll((0, scroll_x));

    frame.render_widget(input_paragraph, area);

    if app.status == AgentStatus::Idle {
        let cursor_x = area.x + 1 + (app.cursor as u16).saturating_sub(scroll_x);
        let cursor_y = area.y + 1;
        frame.set_cursor_position((cursor_x.min(area.x + area.width - 2), cursor_y));
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}
