//! Stub for the watch board. Nothing calls `render` yet, so unused-code warnings are
//! silenced until the wiring phase; the board body is built in its own task.
#![allow(dead_code)]

use ratatui::{
    Frame,
    style::Style,
    widgets::{Block, Borders},
};

use crate::feed::WatchItem;
use crate::theme::Theme;

pub fn render(f: &mut Frame, items: &[WatchItem], theme: &Theme, offset: chrono::FixedOffset) {
    let _ = (items, offset);
    let block = Block::default()
        .title("Watch")
        .borders(Borders::ALL)
        .style(Style::default().fg(theme.fg).bg(theme.bg));
    f.render_widget(block, f.area());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ThemeId;
    use crate::feed::FeedSnapshot;
    use ratatui::{Terminal, backend::TestBackend};

    const FIXTURE: &str = include_str!("../../tests/fixtures/dash_feed_watch_v1.json");

    #[test]
    fn renders_the_watch_title() {
        let snap: FeedSnapshot = serde_json::from_str(FIXTURE).expect("fixture should parse");
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let offset = chrono::FixedOffset::east_opt(0).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).expect("terminal");
        terminal
            .draw(|f| render(f, &snap.watch, &theme, offset))
            .expect("draw should not fail");
        assert!(terminal.backend().to_string().contains("Watch"));
    }
}
