//! Dispatches rendering to the current page's module. No layout logic lives here; each page
//! module owns its own `render`.

pub mod initiative_drill;
pub mod machine_drill;
mod regatta;
pub mod run_drill;
mod slipstream;

use chrono::{DateTime, FixedOffset};
use ratatui::Frame;

use crate::app::{App, AppPage};
use crate::theme::Theme;

pub use regatta::frame_rects as regatta_frame_rects;

pub fn render(f: &mut Frame, app: &App, theme: &Theme) {
    match app.page() {
        AppPage::Regatta => regatta::render(f, app, theme),
        AppPage::Slipstream => slipstream::render(f, app, theme),
    }
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
}
