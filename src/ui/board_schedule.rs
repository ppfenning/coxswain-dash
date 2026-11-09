//! The board's schedule panel for one host. A titled stub until its own task draws it.

use ratatui::{Frame, layout::Rect, widgets::Block};

use crate::app::App;
use crate::theme::Theme;

/// Drawn by `board_machines` in a rectangle inside each host panel.
#[allow(dead_code)] // board_machines calls this once its task lands.
pub fn render_host(f: &mut Frame, area: Rect, _app: &App, _theme: &Theme, _host: &str) {
    f.render_widget(Block::bordered().title("schedule"), area);
}
