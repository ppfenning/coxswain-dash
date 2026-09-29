//! The application's own state: the latest feed snapshot, the current page and theme, and the
//! regatta page's frame visibility and layout preset. Every method here mutates only `self` and
//! does no I/O; loading and saving that state to disk lives in [`crate::config`].

// `apply_snapshot` and the `snapshot` accessor aren't exercised yet: the feed-reading loop that
// calls them lands with the UI wiring, so clippy would otherwise flag them as dead code.
#![allow(dead_code)]

use crate::feed::FeedSnapshot;

/// How many layout presets the regatta page cycles through.
const REGATTA_LAYOUT_PRESET_COUNT: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppPage {
    Regatta,
    Slipstream,
}

/// The two themes `t` toggles between. `themes/slipstream.toml` is a separate asset the
/// Slipstream page loads on its own; it is not a third variant here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeId {
    Regatta,
    HarborLight,
}

#[derive(Debug)]
pub struct App {
    snapshot: Option<FeedSnapshot>,
    page: AppPage,
    theme: ThemeId,
    regatta_frames_visible: [bool; 6],
    regatta_layout_preset: usize,
}

impl Default for App {
    fn default() -> Self {
        App {
            snapshot: None,
            page: AppPage::Regatta,
            theme: ThemeId::Regatta,
            regatta_frames_visible: [true; 6],
            regatta_layout_preset: 0,
        }
    }
}

impl App {
    pub fn new(page: AppPage, theme: ThemeId) -> Self {
        App {
            page,
            theme,
            ..App::default()
        }
    }

    pub fn snapshot(&self) -> Option<&FeedSnapshot> {
        self.snapshot.as_ref()
    }

    pub fn page(&self) -> AppPage {
        self.page
    }

    pub fn theme(&self) -> ThemeId {
        self.theme
    }

    pub fn regatta_frames_visible(&self) -> [bool; 6] {
        self.regatta_frames_visible
    }

    pub fn regatta_layout_preset(&self) -> usize {
        self.regatta_layout_preset
    }

    /// Replaces the held snapshot with a freshly parsed one. No I/O: the caller already read
    /// and parsed the feed line.
    pub fn apply_snapshot(&mut self, snap: FeedSnapshot) {
        self.snapshot = Some(snap);
    }

    pub fn next_page(&mut self) {
        self.page = match self.page {
            AppPage::Regatta => AppPage::Slipstream,
            AppPage::Slipstream => AppPage::Regatta,
        };
    }

    pub fn prev_page(&mut self) {
        // Only two variants exist, so "previous" and "next" wrap the same way.
        self.next_page();
    }

    pub fn toggle_theme(&mut self) {
        self.theme = match self.theme {
            ThemeId::Regatta => ThemeId::HarborLight,
            ThemeId::HarborLight => ThemeId::Regatta,
        };
    }

    /// `n` is 1..=6; flips `regatta_frames_visible[n - 1]`. Out-of-range `n` is a no-op.
    pub fn toggle_regatta_frame(&mut self, n: usize) {
        if (1..=6).contains(&n) {
            let slot = n - 1;
            self.regatta_frames_visible[slot] = !self.regatta_frames_visible[slot];
        }
    }

    pub fn cycle_regatta_layout_preset(&mut self) {
        self.regatta_layout_preset = (self.regatta_layout_preset + 1) % REGATTA_LAYOUT_PRESET_COUNT;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_page_wraps_from_regatta_to_slipstream() {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.next_page();
        assert_eq!(app.page(), AppPage::Slipstream);
    }

    #[test]
    fn next_page_wraps_from_slipstream_to_regatta() {
        let mut app = App::new(AppPage::Slipstream, ThemeId::Regatta);
        app.next_page();
        assert_eq!(app.page(), AppPage::Regatta);
    }

    #[test]
    fn prev_page_wraps_from_regatta_to_slipstream() {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.prev_page();
        assert_eq!(app.page(), AppPage::Slipstream);
    }

    #[test]
    fn prev_page_wraps_from_slipstream_to_regatta() {
        let mut app = App::new(AppPage::Slipstream, ThemeId::Regatta);
        app.prev_page();
        assert_eq!(app.page(), AppPage::Regatta);
    }

    #[test]
    fn toggle_theme_wraps_from_regatta_to_harbor_light() {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.toggle_theme();
        assert_eq!(app.theme(), ThemeId::HarborLight);
    }

    #[test]
    fn toggle_theme_wraps_from_harbor_light_to_regatta() {
        let mut app = App::new(AppPage::Regatta, ThemeId::HarborLight);
        app.toggle_theme();
        assert_eq!(app.theme(), ThemeId::Regatta);
    }

    #[test]
    fn toggle_regatta_frame_flips_slot_three() {
        let mut app = App::default();
        app.toggle_regatta_frame(3);
        assert_eq!(
            app.regatta_frames_visible(),
            [true, true, false, true, true, true]
        );
    }

    #[test]
    fn toggle_regatta_frame_flips_slot_six() {
        let mut app = App::default();
        app.toggle_regatta_frame(6);
        assert_eq!(
            app.regatta_frames_visible(),
            [true, true, true, true, true, false]
        );
    }

    #[test]
    fn toggle_regatta_frame_ignores_zero() {
        let mut app = App::default();
        app.toggle_regatta_frame(0);
        assert_eq!(app.regatta_frames_visible(), [true; 6]);
    }

    #[test]
    fn toggle_regatta_frame_ignores_seven() {
        let mut app = App::default();
        app.toggle_regatta_frame(7);
        assert_eq!(app.regatta_frames_visible(), [true; 6]);
    }

    #[test]
    fn cycle_regatta_layout_preset_wraps_over_three_presets() {
        let mut app = App::default();
        assert_eq!(app.regatta_layout_preset(), 0);
        app.cycle_regatta_layout_preset();
        assert_eq!(app.regatta_layout_preset(), 1);
        app.cycle_regatta_layout_preset();
        assert_eq!(app.regatta_layout_preset(), 2);
        app.cycle_regatta_layout_preset();
        assert_eq!(app.regatta_layout_preset(), 0);
    }
}
