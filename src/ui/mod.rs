//! Dispatches rendering to the current page's module. No layout logic lives here; each page
//! module owns its own `render`.

mod regatta;
mod slipstream;

use ratatui::Frame;

use crate::app::{App, AppPage};
use crate::theme::Theme;

pub fn render(f: &mut Frame, app: &App, theme: &Theme) {
    match app.page() {
        AppPage::Regatta => regatta::render(f, app, theme),
        AppPage::Slipstream => slipstream::render(f, app, theme),
    }
}
