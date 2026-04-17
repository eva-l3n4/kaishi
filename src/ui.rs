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

/// Indent prefix for message body lines — adjusted by viewport width.
fn indent(narrow: bool) -> &'static str {
    if narrow { "  " } else { "    " }
}

/// Top-level draw — dispatches to the active screen, then overlays modal.
pub fn draw(frame: &mut Frame, app: &mut App) {
    match &app.screen.clone() {
        Screen::Picker => {
            ui_picker::draw_picker(frame, &app.sessions, app.picker_selected);
        }
        Screen::Chat => {
            draw_chat(frame, app);
        }
        Screen::Disconnected(err) => {
            draw_disconnected(frame, err);
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
fn draw_chat(frame: &mut Frame, app: &mut App) {
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
    let narrow = area.width < 60;
    let model = if app.model_name.is_empty() {
        "hermes"
    } else {
        &app.model_name
    };

    let session_hint = if narrow {
        String::new() // Hide session info on narrow
    } else if let Some(ref title) = app.session_title {
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

    let help = " /help │ ^C quit ";
    // Use ratatui constraints to right-align help text instead of byte-padding
    let help_display_width = unicode_display_width(help);
    let total_width = area.width as usize;
    let left_max = total_width.saturating_sub(help_display_width);

    // Truncate status_text to fit, using display width
    let status_display = if unicode_display_width(&status_text) > left_max {
        let mut w = 0;
        let truncated: String = status_text
            .chars()
            .take_while(|c| {
                w += unicode_char_width(*c);
                w <= left_max
            })
            .collect();
        truncated
    } else {
        status_text.clone()
    };

    // Pad with spaces to fill the left side
    let pad_needed = left_max.saturating_sub(unicode_display_width(&status_display));
    let padded_left = format!("{}{}", status_display, " ".repeat(pad_needed));

    let bar = Line::from(vec![
        Span::styled(padded_left, style),
        Span::styled(help, style.add_modifier(Modifier::DIM)),
    ]);

    frame.render_widget(Paragraph::new(bar), area);
}

/// Approximate display width of a string (handles CJK, emoji, ASCII).
fn unicode_display_width(s: &str) -> usize {
    s.chars().map(unicode_char_width).sum()
}

fn unicode_char_width(c: char) -> usize {
    // CJK, emoji, and wide characters take 2 columns
    if ('\u{1100}'..='\u{115F}').contains(&c)   // Hangul Jamo
        || ('\u{2E80}'..='\u{A4CF}').contains(&c)  // CJK
        || ('\u{AC00}'..='\u{D7A3}').contains(&c)  // Hangul
        || ('\u{F900}'..='\u{FAFF}').contains(&c)  // CJK compat
        || ('\u{FE10}'..='\u{FE6F}').contains(&c)  // CJK forms
        || ('\u{FF01}'..='\u{FF60}').contains(&c)  // Fullwidth
        || ('\u{FFE0}'..='\u{FFE6}').contains(&c)  // Fullwidth signs
        || c >= '\u{1F000}' // Emoji and symbols (rough heuristic)
    {
        2
    } else {
        1
    }
}

fn draw_messages(frame: &mut Frame, app: &mut App, area: Rect) {
    let inner_width = area.width.saturating_sub(2) as usize; // borders
    let narrow = area.width < 60;

    // Invalidate cache on width or verbose change
    if app.cache_width != inner_width {
        app.line_cache.clear();
        app.cache_width = inner_width;
    }

    // Grow cache to match message count (render new messages only)
    while app.line_cache.len() < app.messages.len() {
        let idx = app.line_cache.len();
        let mut lines: Vec<Line> = Vec::new();
        render_message(&mut lines, &app.messages[idx], inner_width, app.verbose, narrow);
        lines.push(Line::from(""));
        lines = pre_wrap_lines(lines, inner_width);
        app.line_cache.push(lines);
    }

    // Build all_lines from cache
    let mut all_lines: Vec<Line> = Vec::new();

    // "Load more" indicator at top
    if app.history_loaded < app.history_total {
        let remaining = app.history_total - app.history_loaded;
        all_lines.push(Line::from(Span::styled(
            format!("    ↑ {} older messages — scroll up to load", remaining),
            Style::default().fg(Color::DarkGray).italic(),
        )));
        all_lines.push(Line::from(""));
    }

    for cached in &app.line_cache {
        all_lines.extend(cached.iter().cloned());
    }

    // Track where dynamic (uncached) content begins
    let cached_end = all_lines.len();

    // Render the in-progress streaming response
    if !app.pending_response.is_empty() {
        let label = Line::from(vec![
            Span::styled("  ◆ ", Style::default().fg(Color::Magenta)),
            Span::styled(
                "(streaming…)",
                Style::default().fg(Color::DarkGray).italic(),
            ),
        ]);
        all_lines.push(label);

        render_markdown_lines(&mut all_lines, &app.pending_response, inner_width, narrow);

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

    // Show pending thought (always show label, expand with verbose)
    if !app.pending_thought.is_empty() {
        let label = Line::from(vec![
            Span::styled("  ○ ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "thinking…",
                Style::default().fg(Color::DarkGray).italic(),
            ),
        ]);
        all_lines.push(label);
        if app.verbose {
            for line in app.pending_thought.lines() {
                all_lines.push(Line::from(Span::styled(
                    format!("{}{}", indent(narrow), line),
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }
        all_lines.push(Line::from(""));
    }

    // ── Pre-wrap: only wrap dynamic (uncached) lines ──────────────
    if all_lines.len() > cached_end {
        let dynamic = all_lines.split_off(cached_end);
        let wrapped = pre_wrap_lines(dynamic, inner_width);
        all_lines.extend(wrapped);
    }

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

fn render_message(lines: &mut Vec<Line>, msg: &ChatMessage, width: usize, verbose: bool, narrow: bool) {
    // Tool messages render as a single compact line with status icon
    if msg.role == Role::Tool {
        let (icon, color) = if msg.content.starts_with("✓") {
            ("  ✓ ", Color::Green)
        } else if msg.content.starts_with("✗") {
            ("  ✗ ", Color::Red)
        } else {
            ("  ⚙ ", Color::DarkGray)
        };
        let name = msg.content
            .trim_start_matches(['✓', '✗', '⚙', ' '])
            .to_string();
        lines.push(Line::from(vec![
            Span::styled(icon, Style::default().fg(color)),
            Span::styled(name, Style::default().fg(color)),
        ]));
        return;
    }

    let (icon, color) = match msg.role {
        Role::User => ("  ❯ ", Color::Cyan),
        Role::Assistant => ("  ◆ ", Color::Magenta),
        Role::System => ("  ● ", Color::Yellow),
        Role::Tool => unreachable!(),
        Role::Thought => ("  ○ ", Color::DarkGray),
    };

    let mut header_spans = vec![
        Span::styled(icon, Style::default().fg(color)),
    ];

    if let Some(usage) = &msg.tokens {
        header_spans.push(Span::styled(
            format!("[{}→{}]", usage.input_tokens, usage.output_tokens),
            Style::default().fg(Color::DarkGray),
        ));
    }

    lines.push(Line::from(header_spans));

    match msg.role {
        Role::Assistant => {
            render_markdown_lines(lines, &msg.content, width, narrow);
        }
        Role::Thought => {
            if verbose {
                for text_line in msg.content.lines() {
                    lines.push(Line::from(Span::styled(
                        format!("{}{}", indent(narrow), text_line),
                        Style::default().fg(Color::DarkGray).italic(),
                    )));
                }
            } else {
                let line_count = msg.content.lines().count();
                lines.push(Line::from(Span::styled(
                    format!("{}({} lines — /verbose to expand)", indent(narrow), line_count),
                    Style::default().fg(Color::DarkGray).italic(),
                )));
            }
        }
        _ => {
            for text_line in msg.content.lines() {
                lines.push(Line::from(format!("{}{}", indent(narrow), text_line)));
            }
        }
    }
}

// ─── Markdown → ratatui Lines ────────────────────────────────────────────

fn render_markdown_lines(lines: &mut Vec<Line>, text: &str, _width: usize, narrow: bool) {
    let mut in_code_block = false;
    let mut code_lang = String::new();

    for raw_line in text.lines() {
        // Fenced code blocks
        if raw_line.trim_start().starts_with("```") {
            if in_code_block {
                lines.push(Line::from(Span::styled(
                    format!("{}└─────", indent(narrow)),
                    Style::default().fg(Color::DarkGray),
                )));
                in_code_block = false;
                code_lang.clear();
            } else {
                code_lang = raw_line.trim_start().trim_start_matches('`').to_string();
                let header = if code_lang.is_empty() {
                    format!("{}┌─────", indent(narrow))
                } else {
                    format!("{}┌───── {}", indent(narrow), code_lang)
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
                format!("{}│ {}", indent(narrow), raw_line),
                Style::default().fg(Color::Green),
            )));
            continue;
        }

        let trimmed = raw_line.trim_start();

        // Headings
        if let Some(heading) = trimmed.strip_prefix("### ") {
            lines.push(Line::from(Span::styled(
                format!("{}{}", indent(narrow), heading),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )));
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix("## ") {
            lines.push(Line::from(Span::styled(
                format!("{}{}", indent(narrow), heading),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )));
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix("# ") {
            lines.push(Line::from(Span::styled(
                format!("{}{}", indent(narrow), heading),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )));
            continue;
        }

        // Horizontal rules
        if trimmed == "---" || trimmed == "***" || trimmed == "___" {
            lines.push(Line::from(Span::styled(
                format!("{}────────────────────────────────", indent(narrow)),
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
                format!("{}{}• ", indent(narrow), extra_indent),
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
                format!("{}{}{}", indent(narrow), extra_indent, &trimmed[..prefix_len]),
                Style::default().fg(Color::DarkGray),
            )];
            spans.extend(parse_inline_spans(rest));
            lines.push(Line::from(spans));
            continue;
        }

        // Blockquotes
        if let Some(quote) = trimmed.strip_prefix("> ") {
            lines.push(Line::from(vec![
                Span::styled(format!("{}▎ ", indent(narrow)), Style::default().fg(Color::Blue)),
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
            let mut spans = vec![Span::raw(indent(narrow).to_string())];
            spans.extend(parse_inline_spans(raw_line.trim_start()));
            lines.push(Line::from(spans));
        }
    }

    // Close unclosed code block
    if in_code_block {
        lines.push(Line::from(Span::styled(
            format!("{}└─────", indent(narrow)),
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
        " ❯ "
    } else {
        " ⏳ "
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(prompt_style)
        .title(title);

    // Inner width available for text (subtract 2 for borders)
    let inner_width = area.width.saturating_sub(2) as usize;

    // For multiline, calculate cursor position on the current line
    let text_before_cursor = &app.input[..app.cursor];
    let cursor_row = text_before_cursor.lines().count().saturating_sub(1)
        + if text_before_cursor.ends_with('\n') {
            1
        } else {
            0
        };
    let cursor_col = text_before_cursor
        .rsplit('\n')
        .next()
        .map(|line| line.len())
        .unwrap_or(app.cursor);

    // Horizontal scroll based on cursor column in current line
    let scroll_x = if inner_width == 0 {
        0
    } else if cursor_col > inner_width.saturating_sub(1) {
        (cursor_col - inner_width + 1) as u16
    } else {
        0
    };

    // Vertical scroll to keep cursor visible within input box
    let inner_height = area.height.saturating_sub(2) as usize; // borders
    let scroll_y = if inner_height == 0 {
        0
    } else if cursor_row >= inner_height {
        (cursor_row - inner_height + 1) as u16
    } else {
        0
    };

    // Show placeholder when input is empty
    let display_text = if app.input.is_empty() && app.status == AgentStatus::Idle {
        "Type a message… (/help for commands)"
    } else {
        &app.input
    };
    let text_style = if app.input.is_empty() && app.status == AgentStatus::Idle {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default().fg(Color::White)
    };

    let input_paragraph = Paragraph::new(display_text)
        .block(block)
        .style(text_style)
        .scroll((scroll_y, scroll_x));

    frame.render_widget(input_paragraph, area);

    if app.status == AgentStatus::Idle && !app.input.is_empty() {
        // Cursor position relative to scroll
        let visible_row = cursor_row - scroll_y as usize;
        let visible_col = cursor_col.saturating_sub(scroll_x as usize);

        let cursor_x = area.x + 1 + visible_col as u16;
        let cursor_y = area.y + 1 + visible_row as u16;
        frame.set_cursor_position((
            cursor_x.min(area.x + area.width - 2),
            cursor_y.min(area.y + area.height - 2),
        ));
    } else if app.status == AgentStatus::Idle {
        // Empty input — cursor at start
        frame.set_cursor_position((area.x + 1, area.y + 1));
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}

/// Draw the disconnected / error screen.
fn draw_disconnected(frame: &mut Frame, error: &str) {
    let area = frame.area();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .title(" 🌸 Hanami — Disconnected ");

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = vec![
        Line::from(""),
        Line::from(""),
        Line::from(Span::styled(
            "    ⚠ ACP Connection Lost",
            Style::default()
                .fg(Color::Red)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("    {}", error),
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(""),
        Line::from(Span::styled(
            "    Press Enter to reconnect, Esc to quit",
            Style::default().fg(Color::White),
        )),
    ];

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}
