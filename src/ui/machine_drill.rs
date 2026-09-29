//! Stub for the machine-drill board (`cox dash --detail machine <id>`). Draws a single
//! bordered block naming the machine; the later build task replaces this with the real
//! layout. Nothing calls `render` yet, so unused-code warnings are silenced the way
//! `src/detail.rs` silences them until its own reader lands.
#![allow(dead_code)]

use ratatui::{
    Frame,
    style::Style,
    widgets::{Block, Borders, Paragraph},
};

use crate::detail::MachineDetail;
use crate::theme::Theme;

/// Draws a single bordered block titled `Machine <machine>` over the whole frame. `offset` is
/// threaded through for the later layout that formats timestamps; this stub does not use it.
pub fn render(f: &mut Frame, detail: &MachineDetail, theme: &Theme, offset: chrono::FixedOffset) {
    let _ = offset;
    let area = f.area();
    let block = Block::default()
        .title(format!("Machine {}", detail.machine))
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

    const FIXTURE: &str = include_str!("../../tests/fixtures/dash_detail_machine_v1.json");

    #[test]
    fn renders_the_machine_title_from_the_fixture() {
        let snapshot = parse_detail(FIXTURE).expect("fixture should parse");
        let DetailSnapshot::Machine(detail) = snapshot else {
            panic!("expected DetailSnapshot::Machine");
        };
        let theme = crate::theme::resolve(ThemeId::Regatta);
        let offset = chrono::FixedOffset::east_opt(0).unwrap();
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render(f, &detail, &theme, offset))
            .expect("draw should not fail");
        assert!(terminal.backend().to_string().contains("Machine omarchy"));
    }
}
