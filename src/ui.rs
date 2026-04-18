use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
};

use unicode_width::UnicodeWidthStr;

use crate::app::{AgentStatus, App, ChatMessage, ModalState, Role, Screen};
use crate::ui_effort;
use crate::ui_file_popup;
use crate::ui_modal;
use crate::ui_palette;
use crate::ui_search;
use crate::ui_picker;

// ─── Syntax highlighting (lazy-initialized) ──────────────────────
use std::sync::OnceLock;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::SyntaxSet;

fn syntax_set() -> &'static SyntaxSet {
    static SS: OnceLock<SyntaxSet> = OnceLock::new();
    SS.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn syntax_theme() -> &'static Theme {
    static TH: OnceLock<Theme> = OnceLock::new();
    TH.get_or_init(|| {
        let ts = ThemeSet::load_defaults();
        // base16-eighties.dark is a good match for dark terminal themes
        ts.themes
            .get("base16-eighties.dark")
            .cloned()
            .unwrap_or_else(|| {
                ts.themes.values().next().cloned().unwrap()
            })
    })
}

// ─── Palette (terminal-native — inherits from your theme) ──────
pub(crate) mod palette {
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

/// Spinner glyph characters (CC-style).
const GLYPHS: &[char] = &['·', '✢', '✳', '✶', '✻', '✽'];

/// Build the bounce sequence: forward then reverse, skipping endpoints.
fn bounce_sequence() -> Vec<char> {
    let mut seq: Vec<char> = GLYPHS.to_vec();
    seq.extend(GLYPHS.iter().rev().skip(1).take(GLYPHS.len() - 2));
    seq // [· ✢ ✳ ✶ ✻ ✽ ✻ ✶ ✳ ✢]
}

/// Three-step shimmer: highlighted char, adjacent, and base.
fn shimmer_color(char_idx: usize, highlight_pos: usize) -> Color {
    let dist = char_idx.abs_diff(highlight_pos);
    match dist {
        0 => Color::White,    // highlight
        1 => Color::Gray,     // adjacent
        _ => Color::DarkGray, // base
    }
}

/// Format elapsed seconds as "Xs" or "Xm Ys".
fn format_elapsed(secs: f64) -> String {
    let s = secs as u64;
    if s < 60 {
        format!("{}s", s)
    } else {
        format!("{}m {}s", s / 60, s % 60)
    }
}

/// Format token count — compact for readability (e.g., 1.2k, 12k, 1.2M).
fn format_tokens(n: u64) -> String {
    if n < 1_000 {
        n.to_string()
    } else if n < 10_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else if n < 1_000_000 {
        format!("{}k", n / 1_000)
    } else {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    }
}

/// Render a turn completion summary as a dim divider line.
fn render_turn_summary(usage: &Usage, width: usize) -> Line<'static> {
    let in_tok = format_tokens(usage.input_tokens);
    let out_tok = format_tokens(usage.output_tokens);
    let elapsed = usage.elapsed_secs
        .map(format_elapsed)
        .unwrap_or_default();

    let content = if elapsed.is_empty() {
        format!("{} in · {} out", in_tok, out_tok)
    } else {
        format!("{} in · {} out · {}", in_tok, out_tok, elapsed)
    };

    // Center with ── dashes
    let content_width = content.len() + 2; // spaces around content
    let available = width.min(60); // cap divider width
    let dash_total = available.saturating_sub(content_width + 4); // 4 = "──" on each side
    let left_dashes = dash_total / 2;
    let right_dashes = dash_total.saturating_sub(left_dashes);

    let line_str = format!(
        "  {}── {} ──{}",
        "─".repeat(left_dashes),
        content,
        "─".repeat(right_dashes),
    );

    Line::from(Span::styled(line_str, Style::default().fg(Color::DarkGray)))
}

use crate::event::Usage;

/// Render the animated spinner line (thinking/streaming/executing).
fn render_spinner_line(app: &App) -> Option<Line<'static>> {
    use crate::app::AgentPhase;

    if app.animation.phase == AgentPhase::Idle {
        return None;
    }

    let bounce = bounce_sequence();
    let glyph = bounce[app.animation.frame % bounce.len()];

    let label = app.animation.phase_label;
    if label.is_empty() {
        return None;
    }

    let elapsed = format_elapsed(app.animation.phase_start.elapsed().as_secs_f64());

    // Build spans
    let mut spans: Vec<Span> = Vec::new();

    // Leading indent
    spans.push(Span::raw("  "));

    // Glyph (color shifts with stall intensity: Magenta → Yellow → Red)
    let glyph_color = if app.animation.stall_intensity <= 0.0 {
        palette::ACCENT_ASSISTANT // Magenta
    } else if app.animation.stall_intensity < 0.5 {
        Color::Yellow
    } else {
        Color::Red
    };

    spans.push(Span::styled(
        glyph.to_string(),
        Style::default().fg(glyph_color),
    ));

    spans.push(Span::raw(" "));

    // Shimmer label — each char gets its own span
    // When stalled, all chars go dim (frozen shimmer)
    for (i, ch) in label.chars().enumerate() {
        let color = if app.animation.stall_intensity >= 1.0 {
            Color::DarkGray // frozen
        } else {
            shimmer_color(i, app.animation.shimmer_pos)
        };
        spans.push(Span::styled(
            ch.to_string(),
            Style::default().fg(color),
        ));
    }

    // Separator + elapsed
    spans.push(Span::styled(
        format!(" · {}", elapsed),
        Style::default().fg(Color::DarkGray),
    ));

    Some(Line::from(spans))
}

/// Indent prefix for message body lines — adjusted by viewport width.
fn indent(narrow: bool) -> &'static str {
    if narrow { "  " } else { "    " }
}

/// Top-level draw — dispatches to the active screen, then overlays modal.
pub fn draw(frame: &mut Frame, app: &mut App) {
    match &app.screen.clone() {
        Screen::Picker => {
            ui_picker::draw_picker(frame, &app.sessions, app.picker_selected, app.picker_scroll_offset);
        }
        Screen::Chat => {
            draw_chat(frame, app);
        }
        Screen::Disconnected(err) => {
            draw_disconnected(frame, err);
        }
    }

    // Modal overlays (drawn on top of any screen)
    match &app.modal {
        ModalState::Approval { command, options, selected, .. } => {
            ui_modal::draw_approval_modal(frame, command, options, *selected);
        }
        ModalState::CommandPalette { .. } => {
            ui_palette::draw_command_palette(frame, app);
        }
        ModalState::EffortSlider { .. } => {
            ui_effort::draw_effort_slider(frame, app);
        }
        ModalState::ReverseSearch { .. } => {
            ui_search::draw_reverse_search(frame, app);
        }
        ModalState::FileAutocomplete { .. } => {
            ui_file_popup::draw_file_popup(frame, app);
        }
        ModalState::None => {}
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
    let total_width = area.width as usize;
    let bg = palette::STATUS_BG;

    // ── Left side: model + status ──
    let model = if app.model_name.is_empty() {
        "kaishi"
    } else {
        &app.model_name
    };

    let mut left_spans: Vec<Span> = vec![
        Span::styled(format!(" {} ", model), Style::default().bg(bg).fg(palette::STATUS_FG)),
    ];

    // YOLO mode indicator
    if app.yolo_mode {
        left_spans.push(Span::styled(
            "│ ⚡yolo ",
            Style::default().bg(bg).fg(Color::Yellow),
        ));
    }

    // Effort level indicator (when not high/default)
    if app.effort_level < 2 {
        let label = match app.effort_level { 0 => "low", _ => "med" };
        left_spans.push(Span::styled(
            format!("│ ◆{} ", label),
            Style::default().bg(bg).fg(palette::DIM),
        ));
    }

    // Context window health indicator
    if !narrow && app.context_max > 0 && app.context_used > 0 {
        let pct = (app.context_used as f64 / app.context_max as f64 * 100.0) as u16;
        let filled = (pct / 10) as usize;
        let bar: String = "█".repeat(filled.min(10)) + &"░".repeat(10usize.saturating_sub(filled));
        let color = if pct > 85 { palette::ERROR }
                    else if pct > 70 { Color::Yellow }
                    else { palette::SUCCESS };
        left_spans.push(Span::styled(
            format!("│ [{bar}] {pct}% "),
            Style::default().bg(bg).fg(color),
        ));
    }

    // Activity hint
    match &app.status {
        AgentStatus::Thinking => {
            let hint = if let Some(ref tool_name) = app.animation.active_tool {
                format!("│ {} ", tool_name)
            } else {
                "│ working… ".to_string()
            };
            left_spans.push(Span::styled(hint, Style::default().bg(bg).fg(palette::ACCENT_ASSISTANT)));
        }
        AgentStatus::Error(e) => {
            left_spans.push(Span::styled(
                format!("│ ⚠ {} ", truncate(e, 30)),
                Style::default().bg(bg).fg(palette::ERROR),
            ));
        }
        AgentStatus::Idle => {}
    }

    // ── Right side: tokens + cwd ──
    let mut right_parts: Vec<String> = Vec::new();

    // Token totals (session cumulative)
    if !narrow && (app.total_input_tokens + app.total_output_tokens > 0) {
        right_parts.push(format!(
            "{}↑ {}↓",
            format_tokens(app.total_input_tokens),
            format_tokens(app.total_output_tokens),
        ));
    }

    // CWD — shortened with ~
    if !narrow && !app.cwd.is_empty() {
        let home = dirs::home_dir()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_default();
        let display_cwd = if !home.is_empty() && app.cwd.starts_with(&home) {
            format!("~{}", &app.cwd[home.len()..])
        } else {
            app.cwd.clone()
        };
        // Truncate long paths
        let max_cwd = 25;
        let short_cwd = if display_cwd.len() > max_cwd {
            format!("…{}", &display_cwd[display_cwd.len() - max_cwd + 1..])
        } else {
            display_cwd
        };
        right_parts.push(short_cwd);
    }

    let right_text = if right_parts.is_empty() {
        if narrow { " ? ".to_string() } else { " /help ".to_string() }
    } else {
        format!(" {} ", right_parts.join(" │ "))
    };

    // ── Layout: fill middle with spaces ──
    let left_width: usize = left_spans.iter().map(|s| s.content.width()).sum();
    let right_width = right_text.width();
    let pad = total_width.saturating_sub(left_width + right_width);

    let mut spans = left_spans;
    spans.push(Span::styled(" ".repeat(pad), Style::default().bg(bg)));
    spans.push(Span::styled(
        right_text,
        Style::default().bg(bg).fg(Color::DarkGray),
    ));

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
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
        render_message(&mut lines, &app.messages[idx], inner_width, app.verbose, app.show_thinking, narrow);
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
            for span in first.spans.iter() {
                let trimmed = span.content.trim_start();
                if trimmed.is_empty() { continue; }
                if span.content.len() != trimmed.len() {
                    new_spans.push(Span::styled(trimmed.to_string(), span.style));
                } else {
                    new_spans.push(span.clone());
                }
            }
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

    // Show pending thought (always show label, expand with show_thinking)
    if !app.pending_thought.is_empty() {
        let hint = if app.show_thinking { "" } else { " (ctrl+o to expand)" };
        let label = Line::from(vec![
            Span::styled("  ○ ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("thinking…{}", hint),
                Style::default().fg(Color::DarkGray).italic(),
            ),
        ]);
        all_lines.push(label);
        if app.show_thinking {
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

    // Animated spinner line (not cached — rebuilt every frame)
    if let Some(spinner_line) = render_spinner_line(app) {
        all_lines.push(spinner_line);
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
    use unicode_width::UnicodeWidthChar;

    if max_width == 0 {
        return lines;
    }
    let mut result = Vec::with_capacity(lines.len());
    for line in lines {
        if line.width() <= max_width {
            result.push(line);
            continue;
        }

        // Measure leading indent from the first span(s) to replicate on continuation lines.
        let indent_width = {
            let first_span = line.spans.first().map(|s| s.content.as_ref()).unwrap_or("");
            let has_icon = first_span.chars().any(|c| !c.is_ascii() && !c.is_whitespace());
            if has_icon {
                unicode_width::UnicodeWidthStr::width(first_span)
            } else {
                let mut w = 0usize;
                for span in &line.spans {
                    let mut all_ws = true;
                    for ch in span.content.chars() {
                        if ch.is_whitespace() {
                            w += UnicodeWidthChar::width(ch).unwrap_or(0);
                        } else {
                            all_ws = false;
                            break;
                        }
                    }
                    if !all_ws { break; }
                }
                w
            }
        };
        // Clamp indent so continuation lines still have room for content
        let cont_indent = indent_width.min(max_width / 2);
        let cont_prefix = " ".repeat(cont_indent);

        // Flatten all spans into a list of (word_or_whitespace, style) tokens.
        // A "word" is a maximal run of non-whitespace chars; whitespace is a run of spaces/tabs.
        struct Token {
            text: String,
            style: Style,
            width: usize,
            is_ws: bool,
        }
        let mut tokens: Vec<Token> = Vec::new();
        for span in line.spans.iter() {
            let style = span.style;
            let mut buf = String::new();
            let mut buf_width = 0usize;
            let mut buf_is_ws: Option<bool> = None;

            for ch in span.content.chars() {
                let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
                let ws = ch.is_whitespace();
                if buf_is_ws == Some(!ws) {
                    // Boundary: flush
                    tokens.push(Token {
                        text: std::mem::take(&mut buf),
                        style,
                        width: buf_width,
                        is_ws: buf_is_ws.unwrap(),
                    });
                    buf_width = 0;
                }
                buf.push(ch);
                buf_width += cw;
                buf_is_ws = Some(ws);
            }
            if !buf.is_empty() {
                tokens.push(Token {
                    text: buf,
                    style,
                    width: buf_width,
                    is_ws: buf_is_ws.unwrap_or(false),
                });
            }
        }

        // Build rows from tokens with word-level wrapping.
        let mut row_spans: Vec<Span<'static>> = Vec::new();
        let mut row_width: usize = 0;
        let mut is_first_row = true;

        for token in tokens {
            // If adding this token would overflow and we already have content, wrap.
            if !token.is_ws && row_width + token.width > max_width && row_width > 0 {
                // Trim trailing whitespace from current row
                if let Some(last) = row_spans.last_mut() {
                    let trimmed = last.content.trim_end().to_string();
                    if trimmed.is_empty() {
                        row_spans.pop();
                    } else {
                        *last = Span::styled(trimmed, last.style);
                    }
                }
                result.push(Line::from(std::mem::take(&mut row_spans)));
                is_first_row = false;

                // Start new row with continuation indent
                row_spans.push(Span::raw(cont_prefix.clone()));
                row_width = cont_indent;
            }

            // Skip leading whitespace on continuation lines (indent already applied)
            if token.is_ws && !is_first_row && row_width == cont_indent {
                continue;
            }

            // If a single word is wider than max_width, force-break it character-by-character
            if token.width > max_width.saturating_sub(row_width) && token.width > max_width.saturating_sub(cont_indent) {
                let style = token.style;
                let mut chunk = String::new();
                let mut chunk_width = 0usize;
                for ch in token.text.chars() {
                    let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
                    if row_width + chunk_width + cw > max_width && (row_width + chunk_width) > 0 {
                        if !chunk.is_empty() {
                            row_spans.push(Span::styled(std::mem::take(&mut chunk), style));
                            chunk_width = 0;
                        }
                        result.push(Line::from(std::mem::take(&mut row_spans)));
                        is_first_row = false;
                        row_spans.push(Span::raw(cont_prefix.clone()));
                        row_width = cont_indent;
                    }
                    chunk.push(ch);
                    chunk_width += cw;
                }
                if !chunk.is_empty() {
                    row_spans.push(Span::styled(chunk, style));
                    row_width += chunk_width;
                }
                continue;
            }

            row_spans.push(Span::styled(token.text, token.style));
            row_width += token.width;
        }

        // Flush remaining spans
        if !row_spans.is_empty() {
            result.push(Line::from(row_spans));
        }
    }
    result
}

fn render_message(
    lines: &mut Vec<Line>,
    msg: &ChatMessage,
    width: usize,
    _verbose: bool,
    show_thinking: bool,
    narrow: bool,
) {
    // Tool messages render with box-drawing frame
    if msg.role == Role::Tool {
        let (icon, color) = if msg.content.starts_with('✓') {
            ("✓", palette::SUCCESS)
        } else if msg.content.starts_with('✗') {
            ("✗", palette::ERROR)
        } else {
            ("⚙", palette::DIM)
        };

        let rest = msg
            .content
            .trim_start_matches(['✓', '✗', '⚙', ' '])
            .to_string();

        // Parse name and summary (separated by \x1f)
        let (name, summary) = if let Some(sep) = rest.find('\x1f') {
            (rest[..sep].to_string(), rest[sep + 1..].to_string())
        } else if let Some(idx) = rest.find(" (") {
            // Legacy format fallback
            (rest[..idx].to_string(), rest[idx + 2..].trim_end_matches(')').to_string())
        } else if let Some(idx) = rest.find(" — ") {
            (rest[..idx].to_string(), rest[idx + 5..].to_string())
        } else {
            (rest.clone(), String::new())
        };

        let ind = indent(narrow);

        if summary.is_empty() {
            // Simple one-liner: just icon + name
            lines.push(Line::from(vec![
                Span::styled(format!("  {} ", icon), Style::default().fg(color)),
                Span::styled(
                    name,
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
            ]));
        } else {
            // Framed tool call with summary
            // Top: ┌─ icon name ─────
            let header_text = format!("{} {} ", icon, name);
            let remaining_width = width.saturating_sub(ind.len() + 2 + header_text.len());
            let rule = "─".repeat(remaining_width.min(40));

            lines.push(Line::from(vec![
                Span::raw(format!("{}┌─", ind)),
                Span::styled(
                    format!(" {} ", icon),
                    Style::default().fg(color),
                ),
                Span::styled(
                    name.clone(),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" {}", rule),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));

            // Content: │ summary (may span multiple lines)
            let max_summary_width = width.saturating_sub(ind.len() + 4); // "│ " prefix
            for summary_line in summary.lines() {
                // Truncate long lines
                let display = if summary_line.len() > max_summary_width {
                    let end = summary_line
                        .char_indices()
                        .nth(max_summary_width.saturating_sub(1))
                        .map(|(i, _)| i)
                        .unwrap_or(summary_line.len());
                    format!("{}…", &summary_line[..end])
                } else {
                    summary_line.to_string()
                };

                let content_color = if msg.content.starts_with('✗') {
                    palette::ERROR
                } else if summary_line.starts_with('+') && !summary_line.starts_with("+++") {
                    Color::Green
                } else if summary_line.starts_with('-') && !summary_line.starts_with("---") {
                    Color::Red
                } else if summary_line.starts_with("@@ ") {
                    Color::Cyan
                } else if summary_line.starts_with("---") || summary_line.starts_with("+++") {
                    Color::Yellow
                } else {
                    Color::DarkGray
                };

                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{}│ ", ind),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(display, Style::default().fg(content_color)),
                ]));
            }

            // Bottom: └─────
            let bottom_width = width.saturating_sub(ind.len() + 1);
            lines.push(Line::from(Span::styled(
                format!("{}└{}", ind, "─".repeat(bottom_width.min(45))),
                Style::default().fg(Color::DarkGray),
            )));
        }

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
            if show_thinking {
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
                        format!("({} lines — ctrl+o to expand)", line_count),
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
            // Turn completion summary as dim divider
            if let Some(u) = &msg.tokens {
                lines.push(render_turn_summary(u, width));
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

/// Render accumulated table rows as aligned columns, then clear the buffer.
fn flush_table<'a>(rows: &mut Vec<Vec<String>>, lines: &mut Vec<Line<'a>>, narrow: bool) {
    if rows.is_empty() {
        return;
    }

    // Compute column widths
    let num_cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut col_widths = vec![0usize; num_cols];
    for row in rows.iter() {
        for (j, cell) in row.iter().enumerate() {
            if j < num_cols {
                col_widths[j] = col_widths[j].max(cell.len());
            }
        }
    }

    let ind = indent(narrow);

    for (row_idx, row) in rows.iter().enumerate() {
        let mut spans: Vec<Span> = vec![Span::raw(ind.to_string())];

        for (j, cell) in row.iter().enumerate() {
            let width = col_widths.get(j).copied().unwrap_or(cell.len());
            let padded = format!("{:<width$}", cell, width = width);

            if j > 0 {
                spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
            }

            let style = if row_idx == 0 {
                // Header row: bold
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            spans.push(Span::styled(padded, style));
        }

        lines.push(Line::from(spans));

        // Separator after header
        if row_idx == 0 {
            let sep: String = col_widths
                .iter()
                .map(|w| "─".repeat(*w))
                .collect::<Vec<_>>()
                .join("─┼─");
            lines.push(Line::from(Span::styled(
                format!("{}{}", ind, sep),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    rows.clear();
}

fn render_markdown_lines(lines: &mut Vec<Line>, text: &str, _width: usize, narrow: bool) {
    let mut in_code_block = false;
    let mut code_lang = String::new();
    let mut table_rows: Vec<Vec<String>> = Vec::new();

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
            // Syntax-highlighted code line
            let highlighted_spans = highlight_code_line(raw_line, &code_lang);
            let ind = indent(narrow);
            let mut spans = vec![
                Span::styled(format!("{}│ ", ind), Style::default().fg(Color::DarkGray)),
            ];
            spans.extend(highlighted_spans);
            lines.push(Line::from(spans));
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
            flush_table(&mut table_rows, lines, narrow);
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

        // Pipe-delimited tables: accumulate rows, render on flush
        if trimmed.starts_with('|') && trimmed.ends_with('|') {
            // Skip separator rows (| --- | --- |) but keep for detection
            let is_separator = trimmed
                .trim_matches('|')
                .split('|')
                .all(|cell| cell.trim().chars().all(|c| c == '-' || c == ':' || c == ' '));
            if !is_separator {
                let cells: Vec<String> = trimmed
                    .trim_matches('|')
                    .split('|')
                    .map(|c| c.trim().to_string())
                    .collect();
                table_rows.push(cells);
            }
            continue;
        }

        // If we were accumulating table rows but this line isn't a table line, flush
        flush_table(&mut table_rows, lines, narrow);

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

    // Flush any remaining table rows
    flush_table(&mut table_rows, lines, narrow);
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

        // Markdown links: [text](url) — render text underlined, URL in dim
        if ch == '[' {
            // Look ahead for the full [text](url) pattern
            let remaining = &text[i..];
            if let Some(close_bracket) = remaining.find(']') {
                if remaining[close_bracket..].starts_with("](") {
                    if let Some(close_paren) = remaining[close_bracket + 2..].find(')') {
                        let link_text = &remaining[1..close_bracket];
                        let url = &remaining[close_bracket + 2..close_bracket + 2 + close_paren];
                        if !link_text.is_empty() {
                            if !current.is_empty() {
                                spans.push(Span::raw(std::mem::take(&mut current)));
                            }
                            spans.push(Span::styled(
                                link_text.to_string(),
                                Style::default().add_modifier(Modifier::UNDERLINED),
                            ));
                            if !url.is_empty() {
                                spans.push(Span::styled(
                                    format!(" ({})", url),
                                    Style::default().fg(Color::DarkGray),
                                ));
                            }
                            // Advance past the entire [text](url)
                            let total_len = close_bracket + 2 + close_paren + 1;
                            for _ in 0..total_len {
                                chars.next();
                            }
                            continue;
                        }
                    }
                }
            }
        }

        chars.next();
        current.push(ch);
    }

    if !current.is_empty() {
        spans.push(Span::raw(current));
    }

    spans
}

/// Highlight a single line of code using syntect.
/// Returns styled spans; falls back to green monochrome if language is unknown.
fn highlight_code_line(line: &str, lang: &str) -> Vec<Span<'static>> {
    use syntect::easy::HighlightLines;
    use syntect::highlighting::FontStyle;

    let ss = syntax_set();
    let theme = syntax_theme();

    // Try to find syntax by language token, file extension, or fall back to plain text
    let syntax = if lang.is_empty() {
        ss.find_syntax_plain_text()
    } else {
        ss.find_syntax_by_token(lang)
            .or_else(|| ss.find_syntax_by_extension(lang))
            .unwrap_or_else(|| ss.find_syntax_plain_text())
    };

    let mut h = HighlightLines::new(syntax, theme);
    let line_with_newline = format!("{}\n", line);

    match h.highlight_line(&line_with_newline, ss) {
        Ok(ranges) => {
            ranges
                .iter()
                .map(|(style, text)| {
                    let text = text.trim_end_matches('\n').to_string();
                    let fg = Color::Rgb(
                        style.foreground.r,
                        style.foreground.g,
                        style.foreground.b,
                    );
                    let mut rat_style = Style::default().fg(fg);
                    if style.font_style.contains(FontStyle::BOLD) {
                        rat_style = rat_style.add_modifier(Modifier::BOLD);
                    }
                    if style.font_style.contains(FontStyle::ITALIC) {
                        rat_style = rat_style.add_modifier(Modifier::ITALIC);
                    }
                    Span::styled(text, rat_style)
                })
                .collect()
        }
        Err(_) => {
            // Fallback to monochrome green
            vec![Span::styled(
                line.to_string(),
                Style::default().fg(palette::CODE_FG),
            )]
        }
    }
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
    let is_idle = app.status == AgentStatus::Idle;
    let prompt_style = if is_idle {
        Style::default().fg(palette::ACCENT_USER)
    } else {
        Style::default().fg(palette::DIM)
    };

    let title = if is_idle { " ❯ " } else { " ⏳ " };

    // Key hints in bottom border (only when idle)
    let bottom_hint = if is_idle && app.input.is_empty() {
        " esc quit · / commands "
    } else if is_idle {
        " esc quit · enter send "
    } else {
        " ctrl+c cancel "
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(prompt_style)
        .title(title)
        .title_bottom(Line::from(Span::styled(
            bottom_hint,
            Style::default().fg(Color::DarkGray),
        )));

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
        "Type a message…"
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
