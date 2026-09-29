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
}

const REGATTA: Theme = Theme {
    fg: Color::White,
    bg: Color::Black,
    accent: Color::Cyan,
};

const HARBOR_LIGHT: Theme = Theme {
    fg: Color::Black,
    bg: Color::White,
    accent: Color::Blue,
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
