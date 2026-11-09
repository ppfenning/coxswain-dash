//! The Board page. A bordered block for now; the next phase draws the tree inside it.

use ratatui::{Frame, layout::Rect, widgets::Block};

use crate::app::App;

pub fn render(f: &mut Frame, area: Rect, _app: &App) {
    f.render_widget(Block::bordered().title("board"), area);
}
