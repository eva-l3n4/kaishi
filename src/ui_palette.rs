use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use crate::app::{App, ModalState};
use crate::ui::palette;

pub fn draw_command_palette(f: &mut Frame, app: &App) {
    let (query, selected, filtered) = match &app.modal {
        ModalState::CommandPalette {
            query,
            selected,
            filtered,
            ..
        } => (query, *selected, filtered),
        _ => return,
    };

    let area = f.area();
    let modal_width = 50u16.min(area.width.saturating_sub(4));
    let modal_height = ((filtered.len() as u16).saturating_add(3))
        .min(20)
        .min(area.height.saturating_sub(4));

    if modal_width == 0 || modal_height == 0 {
        return;
    }

    let popup = centered_rect(modal_width, modal_height, area);

    f.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Command Palette ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette::ACCENT_ASSISTANT));

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);

    let prompt = Paragraph::new(Line::from(vec![
        Span::styled("❯ ", Style::default().fg(palette::ACCENT_ASSISTANT)),
        Span::raw(query.clone()),
    ]));
    f.render_widget(prompt, layout[0]);

    let sep_len = layout[1].width as usize;
    let separator = Paragraph::new(Span::styled(
        "─".repeat(sep_len),
        Style::default().fg(palette::DIM),
    ));
    f.render_widget(separator, layout[1]);

    let list_width = layout[2].width as usize;
    let items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .map(|(idx, entry)| {
            let is_selected = idx == selected;
            let selected_style = Style::default()
                .fg(Color::Black)
                .bg(palette::ACCENT_ASSISTANT);

            let line = if let Some(keybind) = &entry.keybind {
                let label = &entry.label;
                let label_len = label.chars().count();
                let key_len = keybind.chars().count();
                let spacing = list_width.saturating_sub(label_len + key_len + 1);
                let gap = if spacing == 0 {
                    " ".to_string()
                } else {
                    " ".repeat(spacing)
                };

                if is_selected {
                    Line::from(vec![
                        Span::styled(label.clone(), selected_style),
                        Span::styled(gap, selected_style),
                        Span::styled(keybind.clone(), selected_style),
                    ])
                } else {
                    Line::from(vec![
                        Span::raw(label.clone()),
                        Span::raw(gap),
                        Span::styled(keybind.clone(), Style::default().fg(palette::DIM)),
                    ])
                }
            } else if is_selected {
                Line::from(Span::styled(entry.label.clone(), selected_style))
            } else {
                Line::from(Span::raw(entry.label.clone()))
            };

            if is_selected {
                ListItem::new(line).style(selected_style)
            } else {
                ListItem::new(line)
            }
        })
        .collect();

    let mut list_state = ListState::default().with_selected(Some(selected));
    let list = List::new(items);
    f.render_stateful_widget(list, layout[2], &mut list_state);
}

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
