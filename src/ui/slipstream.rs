//! Stub rendering for the Slipstream page: a single bordered paragraph naming the page. The
//! p4-pages task replaces this body with the full board; `render`'s signature stays.

use ratatui::{
    Frame,
    style::Style,
    widgets::{Block, Borders, Paragraph},
};

use crate::app::App;
use crate::theme::Theme;

pub fn render(f: &mut Frame, _app: &App, theme: &Theme) {
    let block = Block::default().title("Slipstream").borders(Borders::ALL);
    let paragraph = Paragraph::new("Slipstream")
        .block(block)
        .style(Style::default().fg(theme.fg).bg(theme.bg));
    let area = f.area();
    f.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppPage, ThemeId};
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn renders_a_paragraph_naming_slipstream() {
        let app = App::new(AppPage::Slipstream, ThemeId::Regatta);
        let theme = crate::theme::resolve(ThemeId::Regatta);
        let backend = TestBackend::new(20, 5);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render(f, &app, &theme))
            .expect("draw should not fail");
        let content = format!("{:?}", terminal.backend().buffer());
        assert!(content.contains("Slipstream"));
    }
}
