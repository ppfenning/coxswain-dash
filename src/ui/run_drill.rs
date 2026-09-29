//! Stub for the run-drill board (`cox dash --detail run <id>`). Draws a single bordered
//! block naming the run; the later build task replaces this with the real layout. Nothing
//! calls `render` yet, so unused-code warnings are silenced the way `src/detail.rs` silences
//! them until its own reader lands.
#![allow(dead_code)]

use ratatui::{
    Frame,
    style::Style,
    widgets::{Block, Borders, Paragraph},
};

use crate::detail::RunDetail;
use crate::theme::Theme;

/// Draws a single bordered block titled `Run <run>` over the whole frame. `offset` is
/// threaded through for the later layout that formats timestamps; this stub does not use it.
pub fn render(f: &mut Frame, detail: &RunDetail, theme: &Theme, offset: chrono::FixedOffset) {
    let _ = offset;
    let area = f.area();
    let block = Block::default()
        .title(format!("Run {}", detail.run))
        .borders(Borders::ALL);
    let paragraph = Paragraph::new("")
        .block(block)
        .style(Style::default().fg(theme.fg).bg(theme.bg));
    f.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ThemeId;
    use crate::detail::{DetailSnapshot, parse_detail};
    use ratatui::{Terminal, backend::TestBackend};

    const FIXTURE: &str = include_str!("../../tests/fixtures/dash_detail_run_v1.json");

    #[test]
    fn renders_the_run_title_from_the_fixture() {
        let snapshot = parse_detail(FIXTURE).expect("fixture should parse");
        let DetailSnapshot::Run(detail) = snapshot else {
            panic!("expected DetailSnapshot::Run");
        };
        let theme = crate::theme::resolve(ThemeId::Regatta);
        let offset = chrono::FixedOffset::east_opt(0).unwrap();
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render(f, &detail, &theme, offset))
            .expect("draw should not fail");
        assert!(terminal.backend().to_string().contains("Run dash-feed-1"));
    }
}
