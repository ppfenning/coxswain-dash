//! The Spend page. For now it draws only a bordered block titled `Spend`; the table and the
//! three-way split arrive with the next task.

use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    widgets::{Block, Borders},
};

use crate::theme::Theme;

pub fn render(f: &mut Frame, area: Rect, theme: &Theme) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Spend")
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().fg(theme.fg).bg(theme.bg));
    f.render_widget(block, area);
}
