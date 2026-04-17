use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::event::ApprovalOption;

/// Draw the approval modal centered on screen.
pub fn draw_approval_modal(
    frame: &mut Frame,
    command: &str,
    options: &[ApprovalOption],
    selected: usize,
) {
    let area = frame.area();

    // Calculate modal size
    let modal_width = 50u16.min(area.width.saturating_sub(4));
    let modal_height = (options.len() as u16 + 6).min(area.height.saturating_sub(2));

    let modal_area = centered_rect(modal_width, modal_height, area);

    // Clear the area behind the modal
    frame.render_widget(Clear, modal_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(" Approval Required ");

    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));

    // Command preview (truncate if needed)
    let max_cmd_width = modal_width as usize - 6;
    let cmd_display = if command.len() > max_cmd_width {
        format!("  {}...", &command[..max_cmd_width.saturating_sub(3)])
    } else {
        format!("  {}", command)
    };
    lines.push(Line::from(Span::styled(
        cmd_display,
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )));

    lines.push(Line::from(""));

    // Options
    for (i, opt) in options.iter().enumerate() {
        let marker = if i == selected { "  > " } else { "    " };
        let style = if i == selected {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(Line::from(Span::styled(
            format!("{}{}", marker, opt.name),
            style,
        )));
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

/// Create a centered Rect of given size within `area`.
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length((area.height.saturating_sub(height)) / 2),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(area);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length((area.width.saturating_sub(width)) / 2),
            Constraint::Length(width),
            Constraint::Min(0),
        ])
        .split(vertical[1]);

    horizontal[1]
}
