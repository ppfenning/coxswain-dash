//! Dispatches rendering to the current page's module. No layout logic lives here; each page
//! module owns its own `render`.

pub mod initiative_drill;
pub mod machine_drill;
mod regatta;
pub mod run_drill;
mod slipstream;

use chrono::{DateTime, FixedOffset};
use ratatui::{
    Frame,
    style::Style,
    widgets::{Block, BorderType, Borders, Paragraph},
};

use crate::app::{App, AppPage, DetailKind};
use crate::detail::DetailSnapshot;
use crate::theme::Theme;

pub use regatta::frame_rects as regatta_frame_rects;

pub fn render(f: &mut Frame, app: &App, theme: &Theme) {
    match app.detail() {
        Some((_, _, Some(DetailSnapshot::Run(detail)))) => {
            run_drill::render(f, detail, theme, app.utc_offset())
        }
        Some((_, _, Some(DetailSnapshot::Initiative(detail)))) => {
            initiative_drill::render(f, detail, theme, app.utc_offset())
        }
        Some((_, _, Some(DetailSnapshot::Machine(detail)))) => {
            machine_drill::render(f, detail, theme, app.utc_offset())
        }
        Some((kind, id, None)) => render_loading(f, *kind, id, theme),
        None => match app.page() {
            AppPage::Regatta => regatta::render(f, app, theme),
            AppPage::Slipstream => slipstream::render(f, app, theme),
        },
    }
}

/// The lowercase noun a loading paragraph names a [`DetailKind`] with. `src/main.rs`'s
/// `kind_arg` picks the same three strings for the `cox dash --detail` argument.
fn kind_label(kind: DetailKind) -> &'static str {
    match kind {
        DetailKind::Run => "run",
        DetailKind::Initiative => "initiative",
        DetailKind::Machine => "machine",
    }
}

/// Drawn in place of a drill board while its detail snapshot is still in flight.
fn render_loading(f: &mut Frame, kind: DetailKind, id: &str, theme: &Theme) {
    let area = f.area();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded);
    let paragraph = Paragraph::new(format!("loading {} {}", kind_label(kind), id))
        .block(block)
        .style(Style::default().fg(theme.fg).bg(theme.bg));
    f.render_widget(paragraph, area);
}

/// Formats an RFC 3339 timestamp as `%H:%M` in `offset`. A string that does not parse as
/// RFC 3339 comes back unchanged. Pure: the offset is passed in, never read from the clock.
pub fn local_time(utc: &str, offset: FixedOffset) -> String {
    match DateTime::parse_from_rfc3339(utc) {
        Ok(at) => at.with_timezone(&offset).format("%H:%M").to_string(),
        Err(_) => utc.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_time_returns_unparseable_input_unchanged() {
        let utc = FixedOffset::east_opt(0).unwrap();
        assert_eq!(local_time("not a time", utc), "not a time");
    }

    #[test]
    fn local_time_formats_in_the_given_offset() {
        let et = FixedOffset::west_opt(4 * 3600).unwrap();
        assert_eq!(local_time("2026-09-29T18:00:00Z", et), "14:00");
    }

    use crate::app::ThemeId;
    use crate::detail;
    use ratatui::{Terminal, backend::TestBackend};

    const FEED_FIXTURE: &str = include_str!("../../tests/fixtures/dash_feed_v1.json");
    const RUN_DETAIL_FIXTURE: &str = include_str!("../../tests/fixtures/dash_detail_run_v1.json");

    /// An app on the Regatta page with the fixture's one run selected and its detail opened
    /// (so `app.detail()` is `Some((DetailKind::Run, "dash-feed-1", _))`).
    fn app_with_open_run_detail() -> App {
        let snapshot =
            crate::feed::parse_snapshot(FEED_FIXTURE.trim()).expect("fixture should parse");
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.apply_snapshot(snapshot);
        app.open_detail();
        app
    }

    #[test]
    fn renders_a_loading_paragraph_when_the_detail_snapshot_has_not_arrived_yet() {
        let app = app_with_open_run_detail();
        let theme = crate::theme::resolve(ThemeId::Regatta);
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render(f, &app, &theme))
            .expect("draw should not fail");
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn renders_the_run_drill_once_its_detail_snapshot_has_arrived() {
        let mut app = app_with_open_run_detail();
        let snapshot =
            detail::parse_detail(RUN_DETAIL_FIXTURE.trim()).expect("fixture should parse");
        app.apply_detail_snapshot(snapshot);
        let theme = crate::theme::resolve(ThemeId::Regatta);
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render(f, &app, &theme))
            .expect("draw should not fail");
        insta::assert_snapshot!(terminal.backend().to_string());
    }
}
