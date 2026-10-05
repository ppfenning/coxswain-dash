//! The application's own state: the latest feed snapshot, the current page and theme, the
//! regatta page's frame visibility and layout preset, the focused list and per-page selection,
//! the open detail request, and the last feed and detail error lines. Every method here mutates
//! only `self` and does no I/O; loading and saving that state to disk lives in
//! [`crate::config`].

// `apply_snapshot`, `apply_feed_error`, `apply_detail_error` and their accessors aren't exercised
// yet: the feed-reading loop that calls them lands with the UI wiring, so clippy would otherwise
// flag them as dead code.
#![allow(dead_code)]

use chrono::{DateTime, FixedOffset};

use crate::chair_panel::ChairPanel;
use crate::decision_card::DecisionCard;
use crate::detail::DetailSnapshot;
use crate::feed::{Chair, FeedSnapshot};
use crate::pty::{PtySession, RealPty};

/// The pty size the panel opens at. The rendering task sizes it from the layout; until then a
/// resize event from `main` is the only thing that changes it.
const PANEL_ROWS: u16 = 24;
const PANEL_COLS: u16 = 80;

/// How many layout presets the regatta page cycles through.
const REGATTA_LAYOUT_PRESET_COUNT: usize = 4;

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

/// Which of the Regatta page's five row lists Up/Down/Enter act on. The Slipstream page has
/// only a runs list, so its focus never leaves `Focus::Runs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Runs,
    Queue,
    Machines,
    History,
    Inbox,
}

/// Which kind of entity an open detail request names, matching the id field the detail
/// snapshot for that kind carries (`run`, `initiative`, or `machine`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailKind {
    Run,
    Initiative,
    Machine,
}

/// The id a [`DetailSnapshot`] identifies itself with, whatever kind it is.
fn detail_snapshot_id(snap: &DetailSnapshot) -> &str {
    match snap {
        DetailSnapshot::Run(r) => &r.run,
        DetailSnapshot::Initiative(i) => &i.initiative,
        DetailSnapshot::Machine(m) => &m.machine,
    }
}

/// What the app knows about the feed: whether a snapshot ever arrived, the last error line,
/// and whether the feed failed before its first snapshot. `failed` is sticky.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct FeedState {
    has_snapshot: bool,
    error: Option<String>,
    failed: bool,
}

/// One thing the feed reader reports.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FeedEvent {
    Snapshot,
    Error(String),
}

/// The pure transition. A snapshot clears the error and keeps `failed`; an error records its
/// text and sets `failed` only when no snapshot has ever arrived.
fn next_feed_state(old: FeedState, event: FeedEvent) -> FeedState {
    match event {
        FeedEvent::Snapshot => FeedState {
            has_snapshot: true,
            error: None,
            failed: old.failed,
        },
        FeedEvent::Error(line) => FeedState {
            has_snapshot: old.has_snapshot,
            error: Some(line),
            failed: old.failed || !old.has_snapshot,
        },
    }
}

#[derive(Debug)]
pub struct App {
    snapshot: Option<FeedSnapshot>,
    feed: FeedState,
    detail_error: Option<String>,
    page: AppPage,
    theme: ThemeId,
    regatta_frames_visible: [bool; 6],
    /// The history frame's visibility, kept apart from the six numbered frames so their
    /// fixed-size array and its consumers in `ui` stay as they are.
    history_visible: bool,
    /// The run cost frame's visibility, apart from the six numbered frames for the same reason.
    run_cost_visible: bool,
    regatta_layout_preset: usize,
    lanes_history: Vec<(String, u32)>,
    utc_offset: FixedOffset,
    focus: Focus,
    /// Selection index per page (`AppPage::Regatta` then `AppPage::Slipstream`) and per focus
    /// (`Focus::Runs`, `Focus::Queue`, `Focus::Machines`, `Focus::History`, `Focus::Inbox` in
    /// that order).
    selected: [[usize; 5]; 2],
    detail: Option<(DetailKind, String, Option<DetailSnapshot>)>,
    chair_panel: ChairPanel<Box<dyn PtySession>>,
    decision_card: DecisionCard,
    card_visible: bool,
    /// How many times a new decision asked for the bell. `main` rings once per increment.
    bells: u32,
}

/// The chair's session id, or None when the feed carries an empty one.
fn session_id(chair: &Chair) -> Option<&str> {
    (!chair.session.is_empty()).then_some(chair.session.as_str())
}

impl Default for App {
    fn default() -> Self {
        App {
            snapshot: None,
            feed: FeedState::default(),
            detail_error: None,
            page: AppPage::Regatta,
            theme: ThemeId::Regatta,
            regatta_frames_visible: [true; 6],
            history_visible: true,
            run_cost_visible: true,
            regatta_layout_preset: 0,
            lanes_history: Vec::new(),
            utc_offset: FixedOffset::east_opt(0).expect("zero is a valid UTC offset"),
            focus: Focus::Runs,
            selected: [[0; 5]; 2],
            detail: None,
            chair_panel: ChairPanel::new(Box::new(RealPty::new())),
            decision_card: DecisionCard::new(),
            card_visible: false,
            bells: 0,
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

    /// Swaps the panel's pty, so a test can record what the panel spawns.
    #[cfg(test)]
    pub fn with_pty(self, pty: Box<dyn PtySession>) -> Self {
        App {
            chair_panel: ChairPanel::new(pty),
            ..self
        }
    }

    pub fn snapshot(&self) -> Option<&FeedSnapshot> {
        self.snapshot.as_ref()
    }

    pub fn chair_panel(&self) -> &ChairPanel<Box<dyn PtySession>> {
        &self.chair_panel
    }

    pub fn chair_panel_mut(&mut self) -> &mut ChairPanel<Box<dyn PtySession>> {
        &mut self.chair_panel
    }

    pub fn decision_card_mut(&mut self) -> &mut DecisionCard {
        &mut self.decision_card
    }

    pub fn card_visible(&self) -> bool {
        self.card_visible
    }

    /// Hides the decision card without answering. The decision stays open in the feed.
    pub fn hide_card(&mut self) {
        self.card_visible = false;
    }

    /// How many bells new decisions have asked for so far.
    pub fn bells(&self) -> u32 {
        self.bells
    }

    /// Closes an open panel, killing only the local attach process. A closed panel attaches to
    /// the chair's session, if the feed names one, and takes focus.
    pub fn toggle_chair_panel(&mut self) {
        if self.chair_panel.is_open() {
            self.chair_panel.close();
        } else {
            let id = self.snapshot.as_ref().and_then(|s| session_id(&s.chair));
            self.chair_panel.open(id, PANEL_ROWS, PANEL_COLS);
            self.chair_panel.set_focus(true);
        }
    }

    /// The last feed error line, or `None` once a good snapshot has cleared it.
    pub fn feed_error(&self) -> Option<&str> {
        self.feed.error.as_deref()
    }

    /// Whether any snapshot ever arrived. False with `feed_failed` false means waiting.
    pub fn has_snapshot(&self) -> bool {
        self.feed.has_snapshot
    }

    /// Whether the feed errored before its first snapshot. Never reverts, so the renderer
    /// never shows waiting again once this is true.
    pub fn feed_failed(&self) -> bool {
        self.feed.failed
    }

    /// The last detail error line, or `None` once the next detail body has cleared it.
    pub fn detail_error(&self) -> Option<&str> {
        self.detail_error.as_deref()
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

    pub fn regatta_history_visible(&self) -> bool {
        self.history_visible
    }

    pub fn regatta_run_cost_visible(&self) -> bool {
        self.run_cost_visible
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
        // The first snapshot only records its decisions as seen. A later new one opens the
        // panel on the chair's session, shows the card and asks for one bell.
        let first = !self.feed.has_snapshot;
        let has_new = self.decision_card.note_decisions(&snap.decisions);
        if has_new && !first {
            self.chair_panel
                .open(session_id(&snap.chair), PANEL_ROWS, PANEL_COLS);
            self.card_visible = true;
            self.bells += 1;
        } else if self.decision_card.decision().is_none() {
            self.card_visible = false;
        }
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
        self.feed = next_feed_state(self.feed.clone(), FeedEvent::Snapshot);
    }

    /// Records the feed's one-line failure. A held snapshot is kept.
    pub fn apply_feed_error(&mut self, line: String) {
        self.feed = next_feed_state(self.feed.clone(), FeedEvent::Error(line));
    }

    /// Records the detail child's one-line failure.
    pub fn apply_detail_error(&mut self, line: String) {
        self.detail_error = Some(line);
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

    /// Flips the history frame's visibility; the history frame's number key is 7.
    pub fn toggle_history_frame(&mut self) {
        self.history_visible = !self.history_visible;
    }

    /// Flips the run cost frame's visibility; the run cost frame's number key is 8.
    pub fn toggle_run_cost_frame(&mut self) {
        self.run_cost_visible = !self.run_cost_visible;
    }

    pub fn cycle_regatta_layout_preset(&mut self) {
        self.regatta_layout_preset = (self.regatta_layout_preset + 1) % REGATTA_LAYOUT_PRESET_COUNT;
    }

    /// The focused list for the current page. The stored field is the Regatta page's focus;
    /// Slipstream has only a runs list, so it reads as `Focus::Runs` there whatever Regatta
    /// last focused. Every selection and detail method reads focus through here.
    pub fn focus(&self) -> Focus {
        match self.page {
            AppPage::Regatta => self.focus,
            AppPage::Slipstream => Focus::Runs,
        }
    }

    fn page_index(&self) -> usize {
        match self.page {
            AppPage::Regatta => 0,
            AppPage::Slipstream => 1,
        }
    }

    fn focus_index(&self) -> usize {
        match self.focus() {
            Focus::Runs => 0,
            Focus::Queue => 1,
            Focus::Machines => 2,
            Focus::History => 3,
            Focus::Inbox => 4,
        }
    }

    /// The current page's, current focus's selection index. The history and inbox rows are
    /// clamped to the held list, so a feed refresh that shortens it never leaves the selection
    /// past the end.
    pub fn selected(&self) -> usize {
        let idx = self.selected[self.page_index()][self.focus_index()];
        match self.focus() {
            Focus::History | Focus::Inbox => self.clamp_to_list(idx),
            _ => idx,
        }
    }

    /// `idx` limited to the focused list's last row; 0 when the list is empty.
    fn clamp_to_list(&self, idx: usize) -> usize {
        idx.min(self.focused_list_len().saturating_sub(1))
    }

    pub fn detail(&self) -> Option<&(DetailKind, String, Option<DetailSnapshot>)> {
        self.detail.as_ref()
    }

    /// Right cycles focus Runs -> Queue -> Machines -> History -> Inbox -> Runs on the Regatta
    /// page; a no-op on the Slipstream page, which only ever has a runs list to focus.
    /// Inbox follows History, not Machines, because History already sits after Machines.
    pub fn cycle_focus_next(&mut self) {
        if self.page == AppPage::Regatta {
            self.focus = match self.focus {
                Focus::Runs => Focus::Queue,
                Focus::Queue => Focus::Machines,
                Focus::Machines => Focus::History,
                Focus::History => Focus::Inbox,
                Focus::Inbox => Focus::Runs,
            };
        }
    }

    /// Left cycles focus the other way around the same five stops; a no-op on Slipstream.
    pub fn cycle_focus_prev(&mut self) {
        if self.page == AppPage::Regatta {
            self.focus = match self.focus {
                Focus::Runs => Focus::Inbox,
                Focus::Queue => Focus::Runs,
                Focus::Machines => Focus::Queue,
                Focus::History => Focus::Machines,
                Focus::Inbox => Focus::History,
            };
        }
    }

    /// The length of the focused list in the held snapshot, 0 when there is no snapshot yet.
    fn focused_list_len(&self) -> usize {
        let Some(snap) = &self.snapshot else {
            return 0;
        };
        match self.focus() {
            Focus::Runs => snap.runs.len(),
            Focus::Queue => snap.queue.len(),
            Focus::Machines => snap.machines.len(),
            Focus::History => snap.history.len(),
            Focus::Inbox => snap.inbox.len(),
        }
    }

    /// Moves the focused list's selection index forward one row, wrapping past the end. The
    /// history list stops at its last row instead of wrapping. A no-op, staying at 0, when the
    /// focused list is empty or there is no snapshot yet.
    pub fn select_next(&mut self) {
        let len = self.focused_list_len();
        let (p, f) = (self.page_index(), self.focus_index());
        let current = self.selected();
        self.selected[p][f] = match (len, self.focus()) {
            (0, _) => 0,
            (_, Focus::History) => (current + 1).min(len - 1),
            _ => (current + 1) % len,
        };
    }

    /// Moves the focused list's selection index back one row, wrapping before the start. The
    /// history list stops at its first row instead of wrapping.
    pub fn select_prev(&mut self) {
        let len = self.focused_list_len();
        let (p, f) = (self.page_index(), self.focus_index());
        let current = self.selected();
        self.selected[p][f] = match (len, self.focus()) {
            (0, _) => 0,
            (_, Focus::History) => current.saturating_sub(1),
            _ => (current + len - 1) % len,
        };
    }

    /// Sets the focused list's selection index directly, clamped to the list's last row.
    pub fn select_at(&mut self, idx: usize) {
        let len = self.focused_list_len();
        let (p, f) = (self.page_index(), self.focus_index());
        self.selected[p][f] = if len == 0 { 0 } else { idx.min(len - 1) };
    }

    /// The kind and id the focused list's selected row names, or `None` when there is no
    /// snapshot yet or the selection index is out of range.
    pub fn selected_entity(&self) -> Option<(DetailKind, String)> {
        let snap = self.snapshot.as_ref()?;
        let idx = self.selected();
        match self.focus() {
            Focus::Runs => snap.runs.get(idx).map(|r| (DetailKind::Run, r.run.clone())),
            Focus::Queue => snap
                .queue
                .get(idx)
                .map(|q| (DetailKind::Initiative, q.initiative.clone())),
            Focus::Machines => snap
                .machines
                .get(idx)
                .map(|m| (DetailKind::Machine, m.name.clone())),
            Focus::History => snap
                .history
                .get(idx)
                .map(|h| (DetailKind::Run, h.run.clone())),
            // Inbox rows have no drill-down.
            Focus::Inbox => None,
        }
    }

    /// The `target` of the focused inbox row (the feed carries no separate id), or `None` when
    /// the focus is not the inbox or the inbox is empty.
    pub fn selected_inbox_id(&self) -> Option<String> {
        match self.focus() {
            Focus::Inbox => self
                .snapshot
                .as_ref()?
                .inbox
                .get(self.selected())
                .map(|e| e.target.clone()),
            _ => None,
        }
    }

    /// Opens the detail for the currently selected row; a no-op when nothing is selected. On
    /// the history list this is the run drill-down for the ended run's id.
    pub fn open_detail(&mut self) {
        if let Some((kind, id)) = self.selected_entity() {
            self.detail = Some((kind, id, None));
            self.detail_error = None;
        }
    }

    /// Closes the open detail, if any; a no-op when none is open.
    pub fn close_detail(&mut self) {
        self.detail = None;
        self.detail_error = None;
    }

    /// Fills in the open detail's snapshot slot, but only when the newly arrived snapshot's id
    /// still matches the open detail's id. A stale response for a detail the user has since
    /// navigated away from is dropped rather than overwriting the current one.
    pub fn apply_detail_snapshot(&mut self, snap: DetailSnapshot) {
        if let Some((_, id, slot)) = &mut self.detail {
            if detail_snapshot_id(&snap) == id.as_str() {
                *slot = Some(snap);
                self.detail_error = None;
            }
        }
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

    /// A snapshot with three runs (`r0`..`r2`), one queue entry (`i0`), and two machines
    /// (`m0`, `m1`), for the selection and focus tests below.
    fn make_rich_snapshot() -> FeedSnapshot {
        let json = r#"{"schema":1,"at":"2026-09-29T00:00:00Z","chair":{"holder":"h","host":"h","epoch":1,"liveness":"live","beat_age_s":0},"spend":{"five_hour_fraction":0.0,"five_hour_source":"meter","weekly_fraction":0.0,"weekly_source":"meter","hard_stop_fraction":0.0,"five_hour_resets_at":"2026-09-29T00:00:00Z","weekly_resets_at":"2026-09-29T00:00:00Z"},"machines":[{"name":"m0","state":"active","lanes_in_use":0,"capacity":3,"login_ok":true,"login_checked_at":"2026-09-29T00:00:00Z","beat_age_s":0,"checkouts":{}},{"name":"m1","state":"active","lanes_in_use":0,"capacity":3,"login_ok":true,"login_checked_at":"2026-09-29T00:00:00Z","beat_age_s":0,"checkouts":{}}],"runs":[{"run":"r0","machine":"m0","phase":"p","node":"n","attempt":1,"turns":1,"cost":0.0,"verdict":"ok","status":"running"},{"run":"r1","machine":"m0","phase":"p","node":"n","attempt":1,"turns":1,"cost":0.0,"verdict":"ok","status":"running"},{"run":"r2","machine":"m0","phase":"p","node":"n","attempt":1,"turns":1,"cost":0.0,"verdict":"ok","status":"running"}],"queue":[{"initiative":"i0","priority":1,"phases_landed":0,"phases_total":1,"current_phase":"p"}],"inbox":[],"watch":[]}"#;
        crate::feed::parse_snapshot(json).expect("literal snapshot should parse")
    }

    /// A snapshot whose chair session is `s1` and whose open decisions have the given ids.
    fn snapshot_with_decisions(ids: &[&str]) -> FeedSnapshot {
        let decisions = ids
            .iter()
            .map(|id| {
                format!(
                    r#"{{"id":"{id}","question":"q","options":["a","b"],"context":"c","asked_at":"2026-09-29T00:00:00Z"}}"#
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let json = format!(
            r#"{{"schema":1,"at":"2026-09-29T00:00:00Z","chair":{{"holder":"h","host":"h","epoch":1,"liveness":"live","beat_age_s":0,"session":"s1"}},"spend":{{"five_hour_fraction":0.0,"five_hour_source":"meter","weekly_fraction":0.0,"weekly_source":"meter","hard_stop_fraction":0.0,"five_hour_resets_at":"2026-09-29T00:00:00Z","weekly_resets_at":"2026-09-29T00:00:00Z"}},"machines":[],"runs":[],"queue":[],"inbox":[],"watch":[],"decisions":[{decisions}]}}"#
        );
        crate::feed::parse_snapshot(&json).expect("literal snapshot should parse")
    }

    #[test]
    fn a_decision_in_the_first_snapshot_opens_nothing() {
        let fake = crate::pty::SharedFakePty::default();
        let mut app = App::default().with_pty(Box::new(fake.clone()));
        app.apply_snapshot(snapshot_with_decisions(&["d1"]));
        assert!(!app.chair_panel().is_open());
        assert!(!app.card_visible());
        assert_eq!(app.bells(), 0);
        assert!(fake.0.borrow().calls().is_empty());
    }

    #[test]
    fn a_new_decision_in_a_later_snapshot_opens_the_panel_and_rings_once() {
        let fake = crate::pty::SharedFakePty::default();
        let mut app = App::default().with_pty(Box::new(fake.clone()));
        app.apply_snapshot(snapshot_with_decisions(&["d1"]));
        app.apply_snapshot(snapshot_with_decisions(&["d1", "d2"]));
        app.apply_snapshot(snapshot_with_decisions(&["d1", "d2"]));
        assert!(app.chair_panel().is_open());
        assert!(app.card_visible());
        assert_eq!(app.bells(), 1);
        let argv = ["claude", "attach", "s1"].map(String::from).to_vec();
        assert_eq!(
            fake.0.borrow().calls(),
            [crate::pty::PtyCall::Spawn {
                argv,
                rows: PANEL_ROWS,
                cols: PANEL_COLS
            }]
        );
    }

    #[test]
    fn the_card_hides_once_the_feed_drops_every_decision() {
        let mut app = App::default().with_pty(Box::new(crate::pty::SharedFakePty::default()));
        app.apply_snapshot(snapshot_with_decisions(&[]));
        app.apply_snapshot(snapshot_with_decisions(&["d1"]));
        assert!(app.card_visible());
        app.apply_snapshot(snapshot_with_decisions(&[]));
        assert!(!app.card_visible());
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
    fn cycle_regatta_layout_preset_wraps_over_four_presets() {
        let mut app = App::default();
        assert_eq!(app.regatta_layout_preset(), 0);
        app.cycle_regatta_layout_preset();
        assert_eq!(app.regatta_layout_preset(), 1);
        app.cycle_regatta_layout_preset();
        assert_eq!(app.regatta_layout_preset(), 2);
        app.cycle_regatta_layout_preset();
        assert_eq!(app.regatta_layout_preset(), 3);
        app.cycle_regatta_layout_preset();
        assert_eq!(app.regatta_layout_preset(), 0);
    }

    #[test]
    fn select_next_wraps_around_the_runs_list() {
        let mut app = App::default();
        app.apply_snapshot(make_rich_snapshot());
        assert_eq!(app.selected(), 0);
        app.select_next();
        assert_eq!(app.selected(), 1);
        app.select_next();
        assert_eq!(app.selected(), 2);
        app.select_next();
        assert_eq!(app.selected(), 0);
    }

    #[test]
    fn select_prev_wraps_around_the_runs_list() {
        let mut app = App::default();
        app.apply_snapshot(make_rich_snapshot());
        app.select_prev();
        assert_eq!(app.selected(), 2);
    }

    #[test]
    fn select_at_clamps_an_out_of_range_index() {
        let mut app = App::default();
        app.apply_snapshot(make_rich_snapshot());
        app.select_at(100);
        assert_eq!(app.selected(), 2);
    }

    #[test]
    fn selected_entity_returns_the_run_at_the_selected_index_on_the_regatta_page() {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.apply_snapshot(make_rich_snapshot());
        app.select_at(1);
        assert_eq!(
            app.selected_entity(),
            Some((DetailKind::Run, "r1".to_string()))
        );
    }

    #[test]
    fn selected_entity_returns_the_machine_at_the_selected_index() {
        let mut app = App::default();
        app.apply_snapshot(make_rich_snapshot());
        app.cycle_focus_next();
        app.cycle_focus_next();
        assert_eq!(app.focus(), Focus::Machines);
        app.select_at(1);
        assert_eq!(
            app.selected_entity(),
            Some((DetailKind::Machine, "m1".to_string()))
        );
    }

    #[test]
    fn open_detail_then_close_detail_round_trips_app_detail() {
        let mut app = App::default();
        app.apply_snapshot(make_rich_snapshot());
        assert!(app.detail().is_none());
        app.open_detail();
        let (kind, id, slot) = app
            .detail()
            .expect("detail should be open after open_detail");
        assert_eq!(*kind, DetailKind::Run);
        assert_eq!(id, "r0");
        assert!(slot.is_none());
        app.close_detail();
        assert!(app.detail().is_none());
    }

    #[test]
    fn apply_detail_snapshot_updates_the_slot_when_the_id_matches() {
        let mut app = App::default();
        app.apply_snapshot(make_rich_snapshot());
        app.open_detail();
        let detail_json = r#"{"kind":"run","schema":1,"at":"2026-09-29T00:00:00Z","run":"r0","machine":"m0","initiative":"i0","phase":"p","steps":[],"stopped_reason":null,"files":[],"last_tool_calls":[],"log_tail":[]}"#;
        let snap = crate::detail::parse_detail(detail_json).expect("literal detail should parse");
        app.apply_detail_snapshot(snap);
        let (_, _, slot) = app.detail().expect("detail should still be open");
        assert!(slot.is_some());
    }

    #[test]
    fn apply_detail_snapshot_is_ignored_when_the_id_does_not_match() {
        let mut app = App::default();
        app.apply_snapshot(make_rich_snapshot());
        app.open_detail();
        let detail_json = r#"{"kind":"run","schema":1,"at":"2026-09-29T00:00:00Z","run":"not-r0","machine":"m0","initiative":"i0","phase":"p","steps":[],"stopped_reason":null,"files":[],"last_tool_calls":[],"log_tail":[]}"#;
        let snap = crate::detail::parse_detail(detail_json).expect("literal detail should parse");
        app.apply_detail_snapshot(snap);
        let (_, _, slot) = app.detail().expect("detail should still be open");
        assert!(slot.is_none());
    }

    #[test]
    fn next_feed_state_keeps_the_snapshot_flag_on_an_error_after_a_snapshot() {
        let old = FeedState {
            has_snapshot: true,
            error: None,
            failed: false,
        };
        let new = next_feed_state(old, FeedEvent::Error("boom".to_string()));
        assert_eq!(
            new,
            FeedState {
                has_snapshot: true,
                error: Some("boom".to_string()),
                failed: false,
            }
        );
    }

    #[test]
    fn next_feed_state_marks_an_error_before_any_snapshot_as_failed() {
        let new = next_feed_state(FeedState::default(), FeedEvent::Error("boom".to_string()));
        assert_eq!(
            new,
            FeedState {
                has_snapshot: false,
                error: Some("boom".to_string()),
                failed: true,
            }
        );
    }

    #[test]
    fn next_feed_state_snapshot_clears_the_error_and_keeps_failed() {
        let old = FeedState {
            has_snapshot: false,
            error: Some("boom".to_string()),
            failed: true,
        };
        let new = next_feed_state(old, FeedEvent::Snapshot);
        assert_eq!(
            new,
            FeedState {
                has_snapshot: true,
                error: None,
                failed: true,
            }
        );
    }

    #[test]
    fn an_error_after_a_good_snapshot_keeps_that_snapshot() {
        let mut app = App::default();
        app.apply_snapshot(make_snapshot("2026-09-29T00:00:00Z", 1));
        app.apply_feed_error("boom".to_string());
        assert_eq!(app.feed_error(), Some("boom"));
        assert!(app.has_snapshot());
        assert!(!app.feed_failed());
        assert_eq!(
            app.snapshot().map(|s| s.at.as_str()),
            Some("2026-09-29T00:00:00Z")
        );
    }

    #[test]
    fn an_error_before_any_snapshot_marks_the_feed_failed_for_good() {
        let mut app = App::default();
        assert!(!app.has_snapshot() && !app.feed_failed());
        app.apply_feed_error("boom".to_string());
        assert_eq!(app.feed_error(), Some("boom"));
        assert!(!app.has_snapshot());
        assert!(app.feed_failed());
        app.apply_feed_error("again".to_string());
        assert!(app.feed_failed());
        app.apply_snapshot(make_snapshot("2026-09-29T00:00:00Z", 1));
        assert!(app.feed_failed());
        assert!(app.has_snapshot());
    }

    #[test]
    fn a_good_snapshot_clears_the_feed_error() {
        let mut app = App::default();
        app.apply_feed_error("boom".to_string());
        app.apply_snapshot(make_snapshot("2026-09-29T00:00:00Z", 1));
        assert_eq!(app.feed_error(), None);
    }

    #[test]
    fn a_detail_error_clears_when_the_matching_detail_body_arrives() {
        let mut app = App::default();
        app.apply_snapshot(make_rich_snapshot());
        app.open_detail();
        app.apply_detail_error("boom".to_string());
        assert_eq!(app.detail_error(), Some("boom"));
        let detail_json = r#"{"kind":"run","schema":1,"at":"2026-09-29T00:00:00Z","run":"r0","machine":"m0","initiative":"i0","phase":"p","steps":[],"stopped_reason":null,"files":[],"last_tool_calls":[],"log_tail":[]}"#;
        let snap = crate::detail::parse_detail(detail_json).expect("literal detail should parse");
        app.apply_detail_snapshot(snap);
        assert_eq!(app.detail_error(), None);
    }

    #[test]
    fn a_stale_detail_body_leaves_the_detail_error_in_place() {
        let mut app = App::default();
        app.apply_snapshot(make_rich_snapshot());
        app.open_detail();
        app.apply_detail_error("boom".to_string());
        let detail_json = r#"{"kind":"run","schema":1,"at":"2026-09-29T00:00:00Z","run":"not-r0","machine":"m0","initiative":"i0","phase":"p","steps":[],"stopped_reason":null,"files":[],"last_tool_calls":[],"log_tail":[]}"#;
        let snap = crate::detail::parse_detail(detail_json).expect("literal detail should parse");
        app.apply_detail_snapshot(snap);
        assert_eq!(app.detail_error(), Some("boom"));
    }

    #[test]
    fn closing_or_reopening_the_detail_drops_its_error() {
        let mut app = App::default();
        app.apply_snapshot(make_rich_snapshot());
        app.open_detail();
        app.apply_detail_error("boom".to_string());
        app.close_detail();
        assert_eq!(app.detail_error(), None);
        app.apply_detail_error("boom".to_string());
        app.open_detail();
        assert_eq!(app.detail_error(), None);
    }

    #[test]
    fn cycle_focus_next_cycles_runs_queue_machines_and_back_on_regatta() {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        assert_eq!(app.focus(), Focus::Runs);
        app.cycle_focus_next();
        assert_eq!(app.focus(), Focus::Queue);
        app.cycle_focus_next();
        assert_eq!(app.focus(), Focus::Machines);
        app.cycle_focus_next();
        assert_eq!(app.focus(), Focus::History);
        app.cycle_focus_next();
        assert_eq!(app.focus(), Focus::Inbox);
        app.cycle_focus_next();
        assert_eq!(app.focus(), Focus::Runs);
        app.cycle_focus_prev();
        assert_eq!(app.focus(), Focus::Inbox);
        app.cycle_focus_prev();
        assert_eq!(app.focus(), Focus::History);
    }

    /// A snapshot whose inbox has `targets.len()` rows, one per target.
    fn snapshot_with_inbox(targets: &[&str]) -> FeedSnapshot {
        let rows = targets
            .iter()
            .map(|t| format!(r#"{{"kind":"needs_chair","target":"{t}","reason":"r"}}"#))
            .collect::<Vec<_>>()
            .join(",");
        let json = format!(
            r#"{{"schema":1,"at":"2026-09-29T00:00:00Z","chair":{{"holder":"h","host":"h","epoch":1,"liveness":"live","beat_age_s":0}},"spend":{{"five_hour_fraction":0.0,"five_hour_source":"meter","weekly_fraction":0.0,"weekly_source":"meter","hard_stop_fraction":0.0,"five_hour_resets_at":"2026-09-29T00:00:00Z","weekly_resets_at":"2026-09-29T00:00:00Z"}},"machines":[],"runs":[],"queue":[],"inbox":[{rows}],"watch":[]}}"#
        );
        crate::feed::parse_snapshot(&json).expect("literal snapshot should parse")
    }

    fn app_on_inbox(snap: Option<FeedSnapshot>) -> App {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        if let Some(snap) = snap {
            app.apply_snapshot(snap);
        }
        for _ in 0..4 {
            app.cycle_focus_next();
        }
        app
    }

    #[test]
    fn right_from_machines_reaches_history_then_inbox_then_runs() {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.cycle_focus_prev();
        app.cycle_focus_prev();
        assert_eq!(app.focus(), Focus::History);
        app.cycle_focus_prev();
        assert_eq!(app.focus(), Focus::Machines);
        app.cycle_focus_next();
        assert_eq!(app.focus(), Focus::History);
        app.cycle_focus_next();
        assert_eq!(app.focus(), Focus::Inbox);
        app.cycle_focus_next();
        assert_eq!(app.focus(), Focus::Runs);
    }

    #[test]
    fn inbox_selection_wraps_in_a_three_row_inbox() {
        let mut app = app_on_inbox(Some(snapshot_with_inbox(&["a", "b", "c"])));
        assert_eq!(app.focus(), Focus::Inbox);
        app.select_prev();
        assert_eq!(app.selected(), 2);
        app.select_next();
        assert_eq!(app.selected(), 0);
        app.select_next();
        app.select_next();
        assert_eq!(app.selected(), 2);
        app.select_at(9);
        assert_eq!(app.selected(), 2);
    }

    #[test]
    fn inbox_selection_is_zero_with_no_snapshot() {
        let mut app = app_on_inbox(None);
        app.select_next();
        assert_eq!(app.selected(), 0);
        app.select_prev();
        assert_eq!(app.selected(), 0);
        app.select_at(2);
        assert_eq!(app.selected(), 0);
        assert_eq!(app.selected_inbox_id(), None);
    }

    #[test]
    fn selected_inbox_id_names_the_focused_row_and_is_none_elsewhere() {
        let mut app = app_on_inbox(Some(snapshot_with_inbox(&["a", "b", "c"])));
        app.select_next();
        assert_eq!(app.selected_inbox_id(), Some("b".to_string()));
        app.cycle_focus_next();
        assert_eq!(app.focus(), Focus::Runs);
        assert_eq!(app.selected_inbox_id(), None);
    }

    #[test]
    fn selected_inbox_id_is_none_on_an_empty_inbox() {
        let app = app_on_inbox(Some(snapshot_with_inbox(&[])));
        assert_eq!(app.selected_inbox_id(), None);
    }

    #[test]
    fn selected_entity_is_none_on_the_inbox() {
        let app = app_on_inbox(Some(snapshot_with_inbox(&["a", "b", "c"])));
        assert_eq!(app.selected_entity(), None);
    }

    #[test]
    fn inbox_selection_reads_clamped_after_the_inbox_shrinks() {
        let mut app = app_on_inbox(Some(snapshot_with_inbox(&["a", "b", "c"])));
        app.select_at(2);
        assert_eq!(app.selected_inbox_id(), Some("c".to_string()));
        app.apply_snapshot(snapshot_with_inbox(&["d"]));
        assert_eq!(app.selected(), 0);
        assert_eq!(app.selected_inbox_id(), Some("d".to_string()));
    }

    #[test]
    fn the_slipstream_page_never_reaches_inbox() {
        let mut app = App::new(AppPage::Slipstream, ThemeId::Regatta);
        app.apply_snapshot(snapshot_with_inbox(&["a"]));
        for _ in 0..6 {
            app.cycle_focus_next();
            assert_eq!(app.focus(), Focus::Runs);
            app.cycle_focus_prev();
            assert_eq!(app.focus(), Focus::Runs);
        }
        assert_eq!(app.selected_inbox_id(), None);
    }

    /// A snapshot with two ended runs (`h0`, `h1`) in its history and nothing else of note.
    fn make_history_snapshot() -> FeedSnapshot {
        let json = r#"{"schema":1,"at":"2026-09-29T00:00:00Z","chair":{"holder":"h","host":"h","epoch":1,"liveness":"live","beat_age_s":0},"spend":{"five_hour_fraction":0.0,"five_hour_source":"meter","weekly_fraction":0.0,"weekly_source":"meter","hard_stop_fraction":0.0,"five_hour_resets_at":"2026-09-29T00:00:00Z","weekly_resets_at":"2026-09-29T00:00:00Z"},"machines":[],"runs":[],"queue":[],"inbox":[],"watch":[],"history":[{"run":"h0","machine":"m0","initiative":"i0","ended_at":"2026-09-29T00:00:00Z","outcome":"landed","cost_usd":1.0},{"run":"h1","machine":"m0","initiative":"i0","ended_at":"2026-09-29T00:00:00Z","outcome":"stopped","cost_usd":2.0}]}"#;
        crate::feed::parse_snapshot(json).expect("literal snapshot should parse")
    }

    fn app_on_history(snap: FeedSnapshot) -> App {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.apply_snapshot(snap);
        for _ in 0..3 {
            app.cycle_focus_next();
        }
        app
    }

    #[test]
    fn toggle_run_cost_frame_flips_only_the_run_cost_flag() {
        let mut app = App::default();
        assert!(app.regatta_run_cost_visible());
        app.toggle_run_cost_frame();
        assert!(!app.regatta_run_cost_visible());
        assert!(app.regatta_history_visible());
        assert_eq!(app.regatta_frames_visible(), [true; 6]);
        app.toggle_run_cost_frame();
        assert!(app.regatta_run_cost_visible());
    }

    #[test]
    fn toggle_history_frame_flips_only_the_history_flag() {
        let mut app = App::default();
        assert!(app.regatta_history_visible());
        app.toggle_history_frame();
        assert!(!app.regatta_history_visible());
        assert_eq!(app.regatta_frames_visible(), [true; 6]);
        app.toggle_history_frame();
        assert!(app.regatta_history_visible());
    }

    #[test]
    fn history_selection_clamps_at_both_ends() {
        let mut app = app_on_history(make_history_snapshot());
        assert_eq!(app.focus(), Focus::History);
        app.select_prev();
        assert_eq!(app.selected(), 0);
        app.select_next();
        app.select_next();
        assert_eq!(app.selected(), 1);
        app.select_at(9);
        assert_eq!(app.selected(), 1);
    }

    #[test]
    fn history_selection_reads_clamped_after_the_history_shrinks() {
        let mut app = app_on_history(make_history_snapshot());
        app.select_next();
        assert_eq!(app.selected(), 1);
        app.apply_snapshot(make_snapshot("2026-09-29T00:01:00Z", 0));
        assert_eq!(app.selected(), 0);
    }

    #[test]
    fn open_detail_on_a_history_row_opens_that_rows_run() {
        let mut app = app_on_history(make_history_snapshot());
        app.select_next();
        app.open_detail();
        assert_eq!(
            app.detail()
                .map(|(kind, id, slot)| (*kind, id.as_str(), slot.is_some())),
            Some((DetailKind::Run, "h1", false))
        );
    }

    #[test]
    fn open_detail_on_empty_history_changes_nothing() {
        let mut app = app_on_history(make_snapshot("2026-09-29T00:00:00Z", 0));
        app.open_detail();
        assert!(app.detail().is_none());
        assert_eq!(app.selected(), 0);
    }

    #[test]
    fn cycle_focus_next_is_a_no_op_on_slipstream() {
        let mut app = App::new(AppPage::Slipstream, ThemeId::Regatta);
        app.cycle_focus_next();
        assert_eq!(app.focus(), Focus::Runs);
    }

    #[test]
    fn switching_to_slipstream_after_moving_regatta_focus_reads_runs() {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.apply_snapshot(make_rich_snapshot());
        app.cycle_focus_next();
        app.cycle_focus_next();
        app.next_page();
        assert_eq!(app.focus(), Focus::Runs);
        assert_eq!(
            app.selected_entity(),
            Some((DetailKind::Run, "r0".to_string()))
        );
        app.open_detail();
        let (kind, _, _) = app.detail().expect("detail should be open");
        assert_eq!(*kind, DetailKind::Run);
    }

    #[test]
    fn regatta_focus_survives_a_round_trip_through_slipstream() {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.cycle_focus_next();
        app.next_page();
        app.next_page();
        assert_eq!(app.focus(), Focus::Queue);
    }
}
