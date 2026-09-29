//! The two themes `t` toggles between: literal, hard-coded color sets, no I/O. `App::theme`
//! carries only which of the two is selected; `resolve` is the pure lookup from that id to the
//! colors a page renders with. `themes/slipstream.toml` (see `crate::app::ThemeId`'s doc
//! comment) is a separate asset the Slipstream page loads on its own; it is not built here.

use ratatui::style::Color;

use crate::app::ThemeId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub fg: Color,
    pub bg: Color,
    pub accent: Color,
    pub dim: Color,
    pub meter_low: Color,
    pub meter_mid: Color,
    pub meter_high: Color,
    pub status_running: Color,
    pub status_done: Color,
    pub status_failed: Color,
    pub status_waiting: Color,
    pub machine_accents: [Color; 4],
}

const REGATTA: Theme = Theme {
    fg: Color::White,
    bg: Color::Black,
    accent: Color::Cyan,
    dim: Color::DarkGray,
    meter_low: Color::Green,
    meter_mid: Color::Yellow,
    meter_high: Color::Red,
    status_running: Color::Cyan,
    status_done: Color::Green,
    status_failed: Color::Red,
    status_waiting: Color::DarkGray,
    machine_accents: [Color::Cyan, Color::Magenta, Color::Yellow, Color::Blue],
};

const HARBOR_LIGHT: Theme = Theme {
    fg: Color::Black,
    bg: Color::White,
    accent: Color::Blue,
    dim: Color::Gray,
    meter_low: Color::Green,
    meter_mid: Color::Yellow,
    meter_high: Color::Red,
    status_running: Color::Blue,
    status_done: Color::Green,
    status_failed: Color::Red,
    status_waiting: Color::Gray,
    machine_accents: [Color::Blue, Color::Magenta, Color::Yellow, Color::Cyan],
};

/// Looks up the literal `Theme` for a `ThemeId`. Pure: no file I/O, no clock.
pub fn resolve(id: ThemeId) -> Theme {
    match id {
        ThemeId::Regatta => REGATTA,
        ThemeId::HarborLight => HARBOR_LIGHT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regatta_and_harbor_light_resolve_to_different_themes() {
        assert_ne!(resolve(ThemeId::Regatta), resolve(ThemeId::HarborLight));
    }
}
