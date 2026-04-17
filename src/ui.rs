use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
};

use unicode_width::UnicodeWidthStr;

use crate::app::{AgentStatus, App, ChatMessage, ModalState, Role, Screen};
use crate::ui_modal;
use crate::ui_picker;

// ─── Palette (terminal-native — inherits from your theme) ──────
mod palette {
    use ratatui::style::Color;

    pub const TEXT: Color = Color::White;
    pub const DIM: Color = Color::DarkGray;
    pub const ACCENT_USER: Color = Color::Cyan;
    pub const ACCENT_ASSISTANT: Color = Color::Magenta;
    pub const ACCENT_SYSTEM: Color = Color::Yellow;
    pub const ACCENT_THOUGHT: Color = Color::DarkGray;
    pub const SUCCESS: Color = Color::Green;
    pub const ERROR: Color = Color::Red;
    pub const CODE_FG: Color = Color::Green;
    pub const CODE_BG: Color = Color::Reset; // inherit terminal bg
    pub const BORDER: Color = Color::DarkGray;
    pub const STATUS_BG: Color = Color::DarkGray;
    pub const STATUS_FG: Color = Color::White;
    pub const QUOTE: Color = Color::Blue;
}

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
        String::new()
    } else if let Some(ref title) = app.session_title {
        format!(" > {}", truncate(title, 30))
    } else if let Some(ref sid) = app.session_id {
        let short = if sid.len() > 8 { &sid[..8] } else { sid };
        format!(" > {}", short)
    } else {
        String::new()
    };

    let status_text = match &app.status {
        AgentStatus::Idle => {
            format!(" {}{}", model, session_hint)
        }
        AgentStatus::Thinking => {
            let spinner = SPINNER[(app.tick as usize) % SPINNER.len()];
            let tool_hint = if let Some((_, name)) = app.active_tools.last() {
                format!(" {}", name)
            } else {
                " thinking…".to_string()
            };
            format!(" {}{}{}", spinner, tool_hint, session_hint)
        }
        AgentStatus::Error(e) => {
            format!(" ⚠ {}", truncate(e, 50))
        }
    };

    let style = match &app.status {
        AgentStatus::Idle => Style::default().bg(palette::STATUS_BG).fg(palette::STATUS_FG),
        AgentStatus::Thinking => Style::default().bg(palette::STATUS_BG).fg(palette::ACCENT_ASSISTANT),
        AgentStatus::Error(_) => Style::default().bg(palette::STATUS_BG).fg(palette::ERROR),
    };

    let help = if narrow { " ? " } else { " Esc quit | /help " };
    let help_display_width = help.width();
    let total_width = area.width as usize;
    let left_max = total_width.saturating_sub(help_display_width);

    // Truncate status_text to fit, using display width
    let status_display = if status_text.width() > left_max {
        let mut w = 0;
        let truncated: String = status_text
            .chars()
            .take_while(|c| {
                let cw = c.to_string().width();
                w += cw;
                w <= left_max
            })
            .collect();
        truncated
    } else {
        status_text.clone()
    };

    // Pad with spaces to fill the left side
    let pad_needed = left_max.saturating_sub(status_display.width());
    let padded_left = format!("{}{}", status_display, " ".repeat(pad_needed));

    let bar = Line::from(vec![
        Span::styled(padded_left, style),
        Span::styled(help, style.add_modifier(Modifier::DIM)),
    ]);

    frame.render_widget(Paragraph::new(bar), area);
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
        let before = all_lines.len();
        render_markdown_lines(&mut all_lines, &app.pending_response, inner_width, narrow);
        // Prepend icon to first rendered line
        if all_lines.len() > before {
            let first = &mut all_lines[before];
            let mut new_spans = vec![Span::styled(
                "  ◆ ",
                Style::default().fg(palette::ACCENT_ASSISTANT),
            )];
            new_spans.extend(first.spans.clone());
            *first = Line::from(new_spans);
        }

        // Blinking cursor at end
        if app.tick % 4 < 2 {
            if let Some(last) = all_lines.last_mut() {
                let mut spans = last.spans.clone();
                spans.push(Span::styled("█", Style::default().fg(palette::ACCENT_ASSISTANT)));
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
        .border_style(Style::default().fg(palette::BORDER))
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

        // Wrap by display width, not char count
        let mut current = String::new();
        let mut current_width = 0;
        for ch in full.chars() {
            let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            if current_width + ch_width > max_width && !current.is_empty() {
                result.push(Line::from(Span::styled(
                    std::mem::take(&mut current),
                    style,
                )));
                current_width = 0;
            }
            current.push(ch);
            current_width += ch_width;
        }
        if !current.is_empty() {
            result.push(Line::from(Span::styled(current, style)));
        }
    }
    result
}

fn render_message(lines: &mut Vec<Line>, msg: &ChatMessage, width: usize, verbose: bool, narrow: bool) {
    // Tool messages render as a single compact line with status icon
    if msg.role == Role::Tool {
        let (icon, color) = if msg.content.starts_with('✓') {
            ("✓", palette::SUCCESS)
        } else if msg.content.starts_with('✗') {
            ("✗", palette::ERROR)
        } else {
            ("⚙", palette::DIM)
        };

        let rest = msg.content
            .trim_start_matches(['✓', '✗', '⚙', ' '])
            .to_string();

        // Split name from detail at first " (" or " — "
        let (name, detail) = if let Some(idx) = rest.find(" (") {
            (&rest[..idx], Some(&rest[idx..]))
        } else if let Some(idx) = rest.find(" — ") {
            (&rest[..idx], Some(&rest[idx..]))
        } else {
            (rest.as_str(), None)
        };

        let mut spans = vec![
            Span::styled(format!("  {} ", icon), Style::default().fg(color)),
            Span::styled(
                name.to_string(),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
        ];

        if let Some(d) = detail {
            spans.push(Span::styled(
                d.to_string(),
                Style::default().fg(palette::DIM),
            ));
        }

        lines.push(Line::from(spans));
        return;
    }

    let (icon, icon_color) = match msg.role {
        Role::User => ("❯ ", palette::ACCENT_USER),
        Role::Assistant => ("◆ ", palette::ACCENT_ASSISTANT),
        Role::System => ("● ", palette::ACCENT_SYSTEM),
        Role::Tool => unreachable!(),
        Role::Thought => ("○ ", palette::ACCENT_THOUGHT),
    };

    match msg.role {
        Role::Thought => {
            if verbose {
                let thought_lines: Vec<&str> = msg.content.lines().collect();
                if thought_lines.is_empty() {
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {}", icon), Style::default().fg(icon_color)),
                        Span::styled("thinking…", Style::default().fg(Color::DarkGray).italic()),
                    ]));
                } else {
                    // First line gets the icon inline
                    lines.push(Line::from(vec![
                        Span::styled(format!("  {}", icon), Style::default().fg(icon_color)),
                        Span::styled(
                            thought_lines[0].to_string(),
                            Style::default().fg(Color::DarkGray).italic(),
                        ),
                    ]));
                    // Remaining lines indented
                    for &tl in &thought_lines[1..] {
                        lines.push(Line::from(Span::styled(
                            format!("{}{}", indent(narrow), tl),
                            Style::default().fg(Color::DarkGray).italic(),
                        )));
                    }
                }
            } else {
                let line_count = msg.content.lines().count();
                lines.push(Line::from(vec![
                    Span::styled(format!("  {}", icon), Style::default().fg(icon_color)),
                    Span::styled(
                        format!("({} lines — /verbose to expand)", line_count),
                        Style::default().fg(Color::DarkGray).italic(),
                    ),
                ]));
            }
        }
        Role::Assistant => {
            // Render markdown; prepend icon to the first line
            let before = lines.len();
            render_markdown_lines(lines, &msg.content, width, narrow);
            if lines.len() > before {
                let first = &mut lines[before];
                // The first line already has indent from render_markdown_lines.
                // Strip it and replace with the icon prefix (same width).
                let mut new_spans = vec![Span::styled(
                    format!("  {}", icon),
                    Style::default().fg(icon_color),
                )];
                // Skip the leading indent span (first span is usually the indent)
                for span in first.spans.iter() {
                    let trimmed = span.content.trim_start();
                    if trimmed.is_empty() {
                        continue; // skip pure-whitespace indent spans
                    }
                    if span.content.len() != trimmed.len() {
                        // This span had leading whitespace — push only the trimmed part
                        new_spans.push(Span::styled(trimmed.to_string(), span.style));
                    } else {
                        new_spans.push(span.clone());
                    }
                }
                *first = Line::from(new_spans);
            } else {
                lines.push(Line::from(Span::styled(
                    format!("  {}", icon),
                    Style::default().fg(icon_color),
                )));
            }
            // Token usage on a subtle line at the end
            if let Some(u) = &msg.tokens {
                lines.push(Line::from(Span::styled(
                    format!("{}[{}→{} tokens]", indent(narrow), u.input_tokens, u.output_tokens),
                    Style::default().fg(Color::DarkGray),
                )));
            }
        }
        _ => {
            // User and System: inline icon with first content line
            let content_lines: Vec<&str> = msg.content.lines().collect();
            if content_lines.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("  {}", icon),
                    Style::default().fg(icon_color),
                )));
            } else {
                let mut first_spans = vec![Span::styled(
                    format!("  {}", icon),
                    Style::default().fg(icon_color),
                )];
                first_spans.extend(parse_inline_spans(content_lines[0]));
                lines.push(Line::from(first_spans));
                for &cl in &content_lines[1..] {
                    let mut spans = vec![Span::raw(indent(narrow).to_string())];
                    spans.extend(parse_inline_spans(cl));
                    lines.push(Line::from(spans));
                }
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
                Style::default().fg(palette::CODE_FG),
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
                Span::styled(
                    format!("{}▎ ", indent(narrow)),
                    Style::default().fg(palette::QUOTE),
                ),
                Span::styled(
                    quote.to_string(),
                    Style::default()
                        .fg(palette::QUOTE)
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
/// Plain text uses terminal default fg (inherits from theme).
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
                    Style::default().fg(palette::CODE_FG).bg(palette::CODE_BG),
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
        Style::default().fg(palette::ACCENT_USER)
    } else {
        Style::default().fg(palette::DIM)
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
        Style::default().fg(palette::DIM)
    } else {
        Style::default().fg(palette::TEXT)
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

fn truncate(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => &s[..byte_idx],
        None => s,
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
