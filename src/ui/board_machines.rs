//! The board's machines panel. A titled stub until its own task draws it.

use ratatui::{Frame, layout::Rect, widgets::Block};

use crate::app::App;
use crate::theme::Theme;

pub fn render(f: &mut Frame, area: Rect, _app: &App, _theme: &Theme) {
    f.render_widget(Block::bordered().title("machines"), area);
}
