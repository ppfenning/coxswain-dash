//! The application's own state: the latest feed snapshot, the current page and theme, and the
//! regatta page's frame visibility and layout preset. Every method here mutates only `self` and
//! does no I/O; loading and saving that state to disk lives in [`crate::config`].

// `apply_snapshot` and the `snapshot` accessor aren't exercised yet: the feed-reading loop that
// calls them lands with the UI wiring, so clippy would otherwise flag them as dead code.
#![allow(dead_code)]

use chrono::{DateTime, FixedOffset};

use crate::feed::FeedSnapshot;

/// How many layout presets the regatta page cycles through.
const REGATTA_LAYOUT_PRESET_COUNT: usize = 3;

/// How long a lanes-in-use sample is kept once a newer one has arrived.
const LANES_HISTORY_MAX_AGE_HOURS: i64 = 24;

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
    lanes_history: Vec<(String, u32)>,
    utc_offset: FixedOffset,
}

impl Default for App {
    fn default() -> Self {
        App {
            snapshot: None,
            page: AppPage::Regatta,
            theme: ThemeId::Regatta,
            regatta_frames_visible: [true; 6],
            regatta_layout_preset: 0,
            lanes_history: Vec::new(),
            utc_offset: FixedOffset::east_opt(0).expect("zero is a valid UTC offset"),
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

    pub fn utc_offset(&self) -> FixedOffset {
        self.utc_offset
    }

    /// Builder: sets the offset used to render every timestamp. Only `src/main.rs` should ever
    /// pass anything but UTC in.
    pub fn with_utc_offset(self, offset: FixedOffset) -> Self {
        App {
            utc_offset: offset,
            ..self
        }
    }

    /// The kept lanes-in-use samples, oldest first, each within 24h of the newest.
    pub fn lanes_history(&self) -> &[(String, u32)] {
        &self.lanes_history
    }

    /// Replaces the held snapshot with a freshly parsed one, and appends its total
    /// lanes-in-use to the history, dropping samples more than 24h older than the newest.
    /// No I/O: the caller already read and parsed the feed line.
    pub fn apply_snapshot(&mut self, snap: FeedSnapshot) {
        let lanes_in_use: u32 = snap.machines.iter().map(|m| m.lanes_in_use).sum();
        self.lanes_history.push((snap.at.clone(), lanes_in_use));
        if let Some(newest) = self
            .lanes_history
            .iter()
            .filter_map(|(at, _)| DateTime::parse_from_rfc3339(at).ok())
            .max()
        {
            let cutoff = chrono::Duration::hours(LANES_HISTORY_MAX_AGE_HOURS);
            self.lanes_history.retain(|(at, _)| {
                DateTime::parse_from_rfc3339(at)
                    .map(|at| newest - at <= cutoff)
                    .unwrap_or(true)
            });
        }
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

    /// Builds a literal `FeedSnapshot` with the given timestamp and total lanes-in-use,
    /// via `feed::parse_snapshot` so the test never constructs the wire structs by hand.
    fn make_snapshot(at: &str, lanes_in_use: u32) -> FeedSnapshot {
        let json = format!(
            r#"{{"schema":1,"at":"{at}","chair":{{"holder":"h","host":"h","epoch":1,"liveness":"live","beat_age_s":0}},"spend":{{"five_hour_fraction":0.0,"five_hour_source":"meter","weekly_fraction":0.0,"weekly_source":"meter","hard_stop_fraction":0.0,"five_hour_resets_at":"{at}","weekly_resets_at":"{at}"}},"machines":[{{"name":"m","state":"active","lanes_in_use":{lanes_in_use},"capacity":3,"login_ok":true,"login_checked_at":"{at}","beat_age_s":0,"checkouts":{{}}}}],"runs":[],"queue":[],"inbox":[],"watch":[]}}"#
        );
        crate::feed::parse_snapshot(&json).expect("literal snapshot should parse")
    }

    #[test]
    fn default_utc_offset_is_utc() {
        assert_eq!(
            App::default().utc_offset(),
            FixedOffset::east_opt(0).unwrap()
        );
    }

    #[test]
    fn apply_snapshot_drops_samples_more_than_24h_older_than_the_newest() {
        let mut app = App::default();
        app.apply_snapshot(make_snapshot("2026-09-28T00:00:00Z", 1));
        app.apply_snapshot(make_snapshot("2026-09-28T12:00:00Z", 2));
        app.apply_snapshot(make_snapshot("2026-09-29T01:00:00Z", 3));
        assert_eq!(
            app.lanes_history(),
            &[
                ("2026-09-28T12:00:00Z".to_string(), 2),
                ("2026-09-29T01:00:00Z".to_string(), 3),
            ]
        );
    }

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
