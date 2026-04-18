use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem};

use crate::app::{App, ModalState};
use crate::ui::palette;

pub fn draw_file_popup(f: &mut Frame, app: &App) {
    let ModalState::FileAutocomplete {
        selected,
        entries,
        loading,
        ..
    } = &app.modal
    else {
        return;
    };

    let area = f.area();
    // Position: bottom of screen, above input, full width
    let max_items = entries.len().min(8) as u16;
    let height = max_items + 2; // borders
    let width = area.width.min(60);
    let y = area.height.saturating_sub(height + 4); // above input
    let x = 2;
    let rect = Rect::new(x, y, width, height);

    f.render_widget(Clear, rect);

    let title = if *loading { " Scanning… " } else { " Files " };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(palette::ACCENT_ASSISTANT));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let items: Vec<ListItem> = entries
        .iter()
        .enumerate()
        .take(8)
        .map(|(i, path)| {
            let style = if i == *selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(palette::ACCENT_ASSISTANT)
            } else {
                Style::default()
            };
            ListItem::new(format!(" {}", path)).style(style)
        })
        .collect();

    f.render_widget(List::new(items), inner);
}
