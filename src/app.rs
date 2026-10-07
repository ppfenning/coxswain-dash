//! The application's own state: the latest feed snapshot, the current page and theme, the
//! regatta page's frame visibility and layout preset, the focused list and per-page selection,
//! the open detail request, and the last feed and detail error lines. Every method here mutates
//! only `self` and does no I/O; loading and saving that state to disk lives in
//! [`crate::config`].

// `apply_snapshot`, `apply_feed_error`, `apply_detail_error` and their accessors aren't exercised
// yet: the feed-reading loop that calls them lands with the UI wiring, so clippy would otherwise
// flag them as dead code.
#![allow(dead_code)]

use std::path::PathBuf;

use chrono::{DateTime, FixedOffset};
use crossterm::event::KeyEvent;

use crate::actions::{Target, action_for, move_prefill};
use crate::chair_panel::{ChairPanel, CloseOutcome};
use crate::confirm::{ConfirmAnswer, ConfirmState};
use crate::decision_card::DecisionCard;
use crate::detail::{DetailSnapshot, InitiativeDetail};
use crate::exec::{ExecResult, StatusLevel, frame_lines, status_summary};
use crate::feed::{Chair, FeedSnapshot, QueueEntry, queue_repos};
use crate::form::{Form, FormAnswer, shell_join};
use crate::form_initiative::{self, Prefill};
use crate::form_machine;
use crate::palette::{PaletteEvent, PaletteState};
use crate::pty::{PtySession, RealPty};
use crate::settings;
use crate::settings_screen::{ScreenEvent, SettingsScreen};
use crate::settings_stage::{StagedEdit, argv as set_argv, command_string};

/// The pty size the panel opens at. The rendering task sizes it from the layout; until then a
/// resize event from `main` is the only thing that changes it.
pub(crate) const PANEL_ROWS: u16 = 24;
pub(crate) const PANEL_COLS: u16 = 80;

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
/// snapshot for that kind carries (`run`, `initiative`, or `machine`). The last three name no
/// entity: their id is the kind's own word, `watch` reads the feed and holds no snapshot, and
/// `spend` and `health` take no id on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailKind {
    Run,
    Initiative,
    Machine,
    Watch,
    Spend,
    Health,
}

/// The open overlay: a confirm for a bound action, or the colon palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modal {
    Confirm(ConfirmState),
    Palette(PaletteState),
}

/// Where a pending command came from, so its result goes to the status line, the palette or the
/// settings screen. The settings origins name the field their `cox settings set` was for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    Action,
    Palette,
    SettingsLoad,
    SettingsDryRun {
        scope: String,
        key: String,
    },
    SettingsApply {
        scope: String,
        key: String,
    },
    /// A step of the open form's plan, counted from zero.
    Form {
        step: usize,
    },
}

/// A command the app wants run. The loop in `main` takes it, runs it and reports back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub argv: Vec<String>,
    pub origin: Origin,
}

/// The id a [`DetailSnapshot`] identifies itself with, whatever kind it is.
fn detail_snapshot_id(snap: &DetailSnapshot) -> &str {
    match snap {
        DetailSnapshot::Run(r) => &r.run,
        DetailSnapshot::Initiative(i) => &i.initiative,
        DetailSnapshot::Machine(m) => &m.machine,
        DetailSnapshot::Spend(_) => "spend",
        DetailSnapshot::Health(_) => "health",
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
    /// The inbox frame's visibility; its number key is 9. A hidden inbox is not a focus stop.
    inbox_visible: bool,
    /// Whether the queue is shown grouped by project. Session-only: never written to config.
    queue_grouped: bool,
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
    /// Set by the backtick toggle. A shown panel with no open session is where Enter starts one.
    chair_panel_shown: bool,
    /// Focus on a shown panel with no session open. `ChairPanel` focus needs a live pty, so the
    /// focus that lets Enter start a session lives here. Ctrl-], Left, Right and hiding clear it.
    chair_panel_focused: bool,
    decision_card: DecisionCard,
    card_visible: bool,
    /// How many times a new decision asked for the bell. `main` rings once per increment.
    bells: u32,
    modal: Option<Modal>,
    /// Set by a confirmed action or a submitted palette line; emptied by `take_pending`.
    pending: Option<Pending>,
    /// The origin a Yes on the open confirm queues. Set with the confirm, cleared on No.
    confirm_origin: Option<Origin>,
    status: Option<(StatusLevel, String)>,
    settings: Option<SettingsScreen>,
    /// The open form. It stays here underneath its confirm and until the plan ends.
    form: Option<Form>,
    /// The argv of each step of the submitted plan, kept so later edits cannot change it.
    form_steps: Vec<Vec<String>>,
    /// The body field a form asked an editor for. Emptied by `take_editor_request`.
    editor_request: Option<usize>,
    /// The initiative whose detail the edge must fetch before its edit form can open.
    detail_request: Option<String>,
}

/// The edit form for an initiative, prefilled from its parsed detail.
fn edit_form_of(detail: &InitiativeDetail) -> Form {
    form_initiative::edit_form(
        &detail.initiative,
        Prefill {
            repo: detail.repo.clone(),
            title: detail.title.clone(),
            body: detail.body.clone(),
        },
    )
}

/// The `cox settings get --json` command that fills the settings screen.
fn load_pending() -> Pending {
    Pending {
        argv: ["cox", "settings", "get", "--json"]
            .map(str::to_string)
            .to_vec(),
        origin: Origin::SettingsLoad,
    }
}

/// True for a palette line that is exactly `settings` or `cox settings`; the verb has no bare form.
fn is_settings_argv(argv: &[String]) -> bool {
    matches!(argv, [cox, settings] if cox == "cox" && settings == "settings")
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
            inbox_visible: true,
            queue_grouped: false,
            regatta_layout_preset: 0,
            lanes_history: Vec::new(),
            utc_offset: FixedOffset::east_opt(0).expect("zero is a valid UTC offset"),
            focus: Focus::Runs,
            selected: [[0; 5]; 2],
            detail: None,
            chair_panel: ChairPanel::new(Box::new(RealPty::new())),
            chair_panel_shown: false,
            chair_panel_focused: false,
            decision_card: DecisionCard::new(),
            card_visible: false,
            bells: 0,
            modal: None,
            pending: None,
            confirm_origin: None,
            status: None,
            settings: None,
            form: None,
            form_steps: Vec::new(),
            editor_request: None,
            detail_request: None,
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

    pub fn decision_card(&self) -> &DecisionCard {
        &self.decision_card
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

    /// Whether the backtick toggle has the panel showing, open or not.
    pub fn chair_panel_shown(&self) -> bool {
        self.chair_panel_shown
    }

    /// Whether a shown panel with no session open holds focus.
    pub fn chair_panel_focused(&self) -> bool {
        self.chair_panel_focused
    }

    /// Hands focus to or from a shown panel with no session. Only a shown, closed panel takes it.
    /// Polls the panel's pty. When its session ended on its own, the shown panel keeps focus so
    /// Enter starts a new session and the backtick hides it.
    pub fn poll_chair_panel(&mut self) {
        let was_open = self.chair_panel.is_open();
        self.chair_panel.poll();
        if was_open && !self.chair_panel.is_open() {
            self.set_chair_panel_focus(true);
        }
    }

    /// Whether the chair panel holds focus, with a session or without one.
    pub fn chair_focused(&self) -> bool {
        self.chair_panel.focused() || self.chair_panel_focused
    }

    /// Clears both panel focus flags and moves Regatta focus to `focus` when a frame was hit.
    pub fn release_chair_focus_to(&mut self, focus: Option<Focus>) {
        self.chair_panel.set_focus(false);
        self.chair_panel_focused = false;
        if let (Some(focus), AppPage::Regatta) = (focus, self.page) {
            self.focus = focus;
        }
    }

    pub fn set_chair_panel_focus(&mut self, focused: bool) {
        self.chair_panel_focused = focused && self.chair_panel_shown && !self.chair_panel.is_open();
    }

    /// True when Enter should start a session: the panel is shown and focused with nothing
    /// running, no error, and the feed publishes no chair session to attach to.
    pub fn chair_panel_awaits_start(&self) -> bool {
        self.chair_panel_focused
            && self.chair_panel_shown
            && !self.chair_panel.is_open()
            && self.chair_panel.error().is_none()
            && self.published_session().is_none()
    }

    fn published_session(&self) -> Option<&str> {
        self.snapshot
            .as_ref()
            .and_then(|s| session_id(&s.chair))
            .filter(|id| self.chair_panel.attachable(id))
    }

    /// The backtick path. It never closes a live session: an open pane, or a shown pane with no
    /// session that lost focus, only takes focus back. A focused sessionless pane still hides
    /// when the feed names no session, and attaches when it does. A hidden panel attaches to the
    /// chair's session, if the feed names one, and takes focus either way. The name is kept
    /// because a test under `ui/` calls it.
    pub fn toggle_chair_panel(&mut self) {
        if self.chair_panel.is_open() {
            self.chair_panel.set_focus(true);
        } else if self.chair_panel_shown && !self.chair_panel_focused {
            self.set_chair_panel_focus(true);
        } else if self.chair_panel_shown && self.published_session().is_none() {
            self.chair_panel_shown = false;
            self.chair_panel_focused = false;
        } else {
            self.chair_panel_shown = true;
            let id = self.published_session().map(str::to_string);
            self.chair_panel.open(id.as_deref(), PANEL_ROWS, PANEL_COLS);
            self.chair_panel.set_focus(true);
            self.set_chair_panel_focus(true);
        }
    }

    /// The backslash path: hides a shown, unfocused pane. A live session goes through
    /// `request_close`, and a started session waits there for `answer_chair_close`. A focused
    /// or hidden pane is left alone.
    pub fn hide_chair_panel(&mut self) {
        if self.chair_focused() {
            return;
        }
        if self.chair_panel.is_open() {
            if self.chair_panel.request_close() == CloseOutcome::Closed {
                self.chair_panel_shown = false;
                self.chair_panel_focused = false;
            }
        } else if self.chair_panel_shown {
            self.chair_panel_shown = false;
            self.chair_panel_focused = false;
        } else {
            // Hidden already: nothing to hide.
        }
    }

    /// Answers the close prompt. Yes closes the session and hides the panel.
    pub fn answer_chair_close(&mut self, close: bool) {
        self.chair_panel.answer_close(close);
        if close {
            self.chair_panel_shown = false;
            self.chair_panel_focused = false;
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

    pub fn regatta_inbox_visible(&self) -> bool {
        self.inbox_visible
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

    /// Flips the inbox frame's visibility; the inbox frame's number key is 9. Hiding it while
    /// it holds focus moves focus to the runs list, so a hidden inbox never keeps focus.
    pub fn toggle_inbox_frame(&mut self) {
        self.inbox_visible = !self.inbox_visible;
        if !self.inbox_visible && self.focus == Focus::Inbox {
            self.focus = Focus::Runs;
        }
    }

    /// Flips whether the queue is shown grouped by project; the key is `g`.
    pub fn toggle_queue_grouped(&mut self) {
        self.queue_grouped = !self.queue_grouped;
    }

    pub fn queue_grouped(&self) -> bool {
        self.queue_grouped
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
                Focus::History if self.inbox_visible => Focus::Inbox,
                Focus::History => Focus::Runs,
                Focus::Inbox => Focus::Runs,
            };
        }
    }

    /// Left cycles focus the other way around the same five stops; a no-op on Slipstream.
    pub fn cycle_focus_prev(&mut self) {
        if self.page == AppPage::Regatta {
            self.focus = match self.focus {
                Focus::Runs if self.inbox_visible => Focus::Inbox,
                Focus::Runs => Focus::History,
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

    /// Opens the watch board. A no-op while a detail is open, so Esc comes first. It reads the
    /// feed at draw time, so it opens before any feed snapshot has arrived.
    pub fn open_watch(&mut self) {
        self.open_view(DetailKind::Watch, "watch");
    }

    /// Opens the spend board; a no-op while a detail is open.
    pub fn open_spend(&mut self) {
        self.open_view(DetailKind::Spend, "spend");
    }

    /// Opens the health board; a no-op while a detail is open.
    pub fn open_health(&mut self) {
        self.open_view(DetailKind::Health, "health");
    }

    fn open_view(&mut self, kind: DetailKind, id: &str) {
        if self.detail.is_none() {
            self.detail = Some((kind, id.to_string(), None));
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

    pub fn modal(&self) -> Option<&Modal> {
        self.modal.as_ref()
    }

    pub fn status(&self) -> Option<&(StatusLevel, String)> {
        self.status.as_ref()
    }

    /// What an action key applies to: the open detail's entity, else the focused list's row.
    pub fn target(&self) -> Option<Target> {
        // The watch, spend and health views name no entity, so no action key applies to them.
        let of_entity = |kind: DetailKind, id: String| match kind {
            DetailKind::Run => Some(Target::Run(id)),
            DetailKind::Initiative => Some(Target::Initiative(id)),
            DetailKind::Machine => Some(Target::Machine(id)),
            DetailKind::Watch | DetailKind::Spend | DetailKind::Health => None,
        };
        match &self.detail {
            Some((kind, id, _)) => of_entity(*kind, id.clone()),
            None => self
                .selected_inbox_id()
                .map(Target::InboxItem)
                .or_else(|| self.selected_entity().and_then(|(k, id)| of_entity(k, id))),
        }
    }

    /// Opens the confirm for the action `key` means on the target, or the move palette for `m`
    /// on a run. False, with nothing opened, when the key means nothing here.
    pub fn begin_action(&mut self, key: char) -> bool {
        let modal = self.modal_for(key);
        let opened = modal.is_some();
        if opened {
            self.modal = modal;
            self.confirm_origin = Some(Origin::Action);
        }
        opened
    }

    /// Opens a confirm showing `display`; a Yes queues `argv` with `origin`.
    fn open_confirm(&mut self, title: String, argv: Vec<String>, display: String, origin: Origin) {
        self.modal = Some(Modal::Confirm(ConfirmState::new(title, argv, display)));
        self.confirm_origin = Some(origin);
    }

    fn modal_for(&self, key: char) -> Option<Modal> {
        let target = self.target()?;
        if let (Target::Run(run), 'm') = (&target, key) {
            return Some(Modal::Palette(PaletteState::prefilled(&move_prefill(run))));
        }
        let action = action_for(key, &target, self.snapshot.as_ref()?)?;
        Some(Modal::Confirm(ConfirmState::new(
            action.title(),
            action.argv(),
            action.display(),
        )))
    }

    pub fn open_palette(&mut self) {
        self.modal = Some(Modal::Palette(PaletteState::open()));
    }

    pub fn form(&self) -> Option<&Form> {
        self.form.as_ref()
    }

    pub fn open_add_machine(&mut self) {
        self.form = Some(form_machine::form());
    }

    pub fn open_new_initiative(&mut self) {
        let repos = self.snapshot.as_ref().map(queue_repos).unwrap_or_default();
        self.form = Some(form_initiative::new_form(repos));
    }

    /// Opens the edit form from the open initiative detail when it is for `id` and loaded.
    /// Otherwise asks the edge for that detail; it answers with `open_edit_initiative`.
    pub fn begin_edit(&mut self, id: &str) {
        let loaded = match &self.detail {
            Some((DetailKind::Initiative, open, Some(DetailSnapshot::Initiative(detail))))
                if open == id =>
            {
                Some(edit_form_of(detail))
            }
            _ => None,
        };
        match loaded {
            Some(form) => self.form = Some(form),
            None => self.detail_request = Some(id.to_string()),
        }
    }

    pub fn open_edit_initiative(&mut self, detail: &InitiativeDetail) {
        self.form = Some(edit_form_of(detail));
    }

    pub fn open_remove_initiative(&mut self, id: &str) {
        self.form = Some(form_initiative::remove_form(id));
    }

    /// The initiative id an edit has to fetch the detail of, once.
    pub fn take_detail_request(&mut self) -> Option<String> {
        self.detail_request.take()
    }

    /// The body field index a form asked an editor for, once.
    pub fn take_editor_request(&mut self) -> Option<usize> {
        self.editor_request.take()
    }

    /// Takes what the editor returned for body field `idx`. An editor that failed to run
    /// leaves the body as it was and sets a Failed status with its message.
    pub fn editor_done(&mut self, idx: usize, path: Option<PathBuf>, text: Result<String, String>) {
        match text {
            Ok(text) => {
                if let Some(form) = self.form.as_mut() {
                    form.set_body(idx, path, text);
                }
            }
            Err(message) => self.status = Some((StatusLevel::Failed, message)),
        }
    }

    /// Sends a key to the open form. A confirm over the form takes keys through `modal_key`.
    pub fn form_key(&mut self, key: KeyEvent) {
        if self.modal.is_some() {
            return;
        }
        let answer = self.form.as_mut().map(|form| form.handle_key(key));
        match answer {
            Some(FormAnswer::Cancel) => self.form = None,
            Some(FormAnswer::OpenEditor(idx)) => self.editor_request = Some(idx),
            Some(FormAnswer::Submit) => self.submit_form(),
            Some(FormAnswer::Editing) | None => {}
        }
    }

    /// Opens a confirm on the plan's first step, or leaves the form open showing its error.
    fn submit_form(&mut self) {
        let plan = self.form.as_mut().and_then(Form::submit);
        if let Some(plan) = plan
            && let Some(first) = plan.steps.first().cloned()
        {
            let display = shell_join(&first);
            self.form_steps = plan.steps;
            self.open_confirm(plan.confirm_title, first, display, Origin::Form { step: 0 });
        }
    }

    /// Runs the form plan forward. A further step is queued on success. The last step, or a
    /// failure after the first, closes the form and shows the result in the palette frame.
    /// A failure on the first step leaves the form open for another try.
    fn form_result(&mut self, result: &ExecResult, step: usize) {
        let next = if result.code == Some(0) {
            self.form_steps.get(step + 1).cloned()
        } else {
            None
        };
        match next {
            Some(argv) => self.queue(Pending {
                argv,
                origin: Origin::Form { step: step + 1 },
            }),
            None if step == 0 && result.code != Some(0) => {
                self.status = Some(status_summary(result));
            }
            None => {
                self.form = None;
                self.form_steps = Vec::new();
                self.status = Some(status_summary(result));
                let mut palette = PaletteState::open();
                palette.set_output(frame_lines(result));
                self.modal = Some(Modal::Palette(palette));
            }
        }
    }

    /// Routes a key to the open modal. A command is queued only while none is pending, so one
    /// confirm runs one command.
    pub fn modal_key(&mut self, key: KeyEvent) {
        let free = self.pending.is_none();
        let queued = match &mut self.modal {
            Some(Modal::Confirm(confirm)) => match confirm.handle_key(key) {
                ConfirmAnswer::Yes => {
                    let argv = confirm.argv.clone();
                    self.modal = None;
                    Some(Pending {
                        argv,
                        origin: self.confirm_origin.take().unwrap_or(Origin::Action),
                    })
                }
                ConfirmAnswer::No => {
                    self.modal = None;
                    self.confirm_origin = None;
                    None
                }
                ConfirmAnswer::Pending => None,
            },
            Some(Modal::Palette(palette)) => match palette.handle_key(key.code) {
                // The palette reaches the settings screen; bare `cox settings` runs nothing.
                PaletteEvent::Submit(argv) if is_settings_argv(&argv) => free.then(|| {
                    self.modal = None;
                    self.settings = Some(SettingsScreen::open());
                    load_pending()
                }),
                PaletteEvent::Submit(argv) => Some(Pending {
                    argv,
                    origin: Origin::Palette,
                }),
                PaletteEvent::Close => {
                    self.modal = None;
                    None
                }
                PaletteEvent::None => None,
            },
            None => None,
        };
        if free {
            self.pending = queued;
        }
    }

    pub fn take_pending(&mut self) -> Option<Pending> {
        self.pending.take()
    }

    /// Records a finished command: the status line for an action or palette command, the output
    /// lines on the open palette for a palette command, and the settings screen for its own.
    pub fn apply_exec_result(&mut self, result: ExecResult, origin: Origin) {
        match origin {
            Origin::Action => self.status = Some(status_summary(&result)),
            Origin::Palette => {
                self.status = Some(status_summary(&result));
                if let Some(Modal::Palette(palette)) = &mut self.modal {
                    palette.set_output(frame_lines(&result));
                }
            }
            Origin::Form { step } => self.form_result(&result, step),
            Origin::SettingsLoad => self.settings_loaded(&result),
            Origin::SettingsDryRun { scope, key } => {
                if let Some(screen) = self.settings.as_mut() {
                    screen
                        .staged_mut()
                        .record_dry_run(&scope, &key, result.code, &result.output);
                }
            }
            Origin::SettingsApply { scope, key } => {
                self.status = Some(status_summary(&result));
                if let Some(screen) = self.settings.as_mut() {
                    // A nonzero exit records its last line as the refusal and keeps the edit staged.
                    match result.code {
                        Some(0) => screen.staged_mut().record_applied(&scope, &key),
                        code => {
                            screen
                                .staged_mut()
                                .record_dry_run(&scope, &key, code, &result.output)
                        }
                    }
                }
                if result.code == Some(0) {
                    self.queue(load_pending());
                }
            }
        }
    }

    /// Loads the rows of a `cox settings get --json` result, or sets a Failed status.
    fn settings_loaded(&mut self, result: &ExecResult) {
        let parsed = match result.code {
            Some(0) => settings::parse(&result.output)
                .map_err(|e| (StatusLevel::Failed, format!("settings: {e}"))),
            _ => Err(status_summary(result)),
        };
        match (parsed, self.settings.as_mut()) {
            (Ok(snapshot), Some(screen)) => screen.load(snapshot),
            (Ok(_), None) => {}
            (Err(status), _) => self.status = Some(status),
        }
    }

    /// Queues a command only while none is pending, so one event runs one command.
    fn queue(&mut self, pending: Pending) {
        if self.pending.is_none() {
            self.pending = Some(pending);
        }
    }

    /// The staged edit at `index` or for the field, copied out so the caller may borrow `self`.
    fn staged_edit(&self, find: impl Fn(usize, &StagedEdit) -> bool) -> Option<StagedEdit> {
        self.settings
            .as_ref()?
            .staged()
            .edits()
            .iter()
            .enumerate()
            .find(|(i, e)| find(*i, e))
            .map(|(_, e)| e.clone())
    }

    // The three settings methods are not routed from `main` or `input` yet.
    #[allow(dead_code)]
    pub fn settings(&self) -> Option<&SettingsScreen> {
        self.settings.as_ref()
    }

    /// Opens the settings screen and asks for its rows.
    #[allow(dead_code)]
    pub fn open_settings(&mut self) {
        self.settings = Some(SettingsScreen::open());
        self.queue(load_pending());
    }

    /// Routes a key to the open settings screen and acts on the event it returns.
    #[allow(dead_code)]
    pub fn settings_key(&mut self, key: KeyEvent) {
        let Some(screen) = self.settings.as_mut() else {
            return;
        };
        match screen.handle_key(key) {
            ScreenEvent::None => {}
            ScreenEvent::Close => self.settings = None,
            ScreenEvent::Reload => self.queue(load_pending()),
            ScreenEvent::Stage {
                scope, key: field, ..
            } => {
                if let Some(edit) = self.staged_edit(|_, e| e.scope == scope && e.key == field) {
                    self.queue(Pending {
                        argv: set_argv(&edit, true),
                        origin: Origin::SettingsDryRun { scope, key: field },
                    });
                }
            }
            ScreenEvent::Apply(index) => {
                if let Some(edit) = self.staged_edit(|i, _| i == index) {
                    let argv = set_argv(&edit, false);
                    self.open_confirm(
                        format!("Apply {} {}", edit.scope, edit.key),
                        argv.clone(),
                        command_string(&argv),
                        Origin::SettingsApply {
                            scope: edit.scope,
                            key: edit.key,
                        },
                    );
                }
            }
        }
    }
}

/// The label of the group holding rows with no project.
const OTHER_GROUP: &str = "other";

/// Groups queue rows by project: named projects in first-seen order, then "other" for rows with
/// no project, which is left out when no row lacks one. Rows keep their relative order.
pub fn group_queue(rows: &[QueueEntry]) -> Vec<(&str, Vec<&QueueEntry>)> {
    let projects = rows.iter().filter_map(|row| row.project.as_deref()).fold(
        Vec::new(),
        |seen: Vec<&str>, project| {
            if seen.contains(&project) {
                seen
            } else {
                seen.into_iter().chain([project]).collect()
            }
        },
    );
    let named = projects.into_iter().map(|project| {
        let members = rows
            .iter()
            .filter(|row| row.project.as_deref() == Some(project))
            .collect();
        (project, members)
    });
    let unassigned: Vec<&QueueEntry> = rows.iter().filter(|row| row.project.is_none()).collect();
    named
        .chain((!unassigned.is_empty()).then_some((OTHER_GROUP, unassigned)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(initiative: &str, project: Option<&str>) -> QueueEntry {
        QueueEntry {
            initiative: initiative.to_string(),
            priority: 1,
            phases_landed: 0,
            phases_total: 1,
            current_phase: "p".to_string(),
            repo: String::new(),
            project: project.map(str::to_string),
        }
    }

    /// Each group as its label and the initiative names it holds.
    fn shape<'a>(groups: &[(&'a str, Vec<&QueueEntry>)]) -> Vec<(&'a str, Vec<String>)> {
        groups
            .iter()
            .map(|(label, rows)| (*label, rows.iter().map(|r| r.initiative.clone()).collect()))
            .collect()
    }

    #[test]
    fn group_queue_orders_two_interleaved_projects_then_other() {
        let rows = [
            entry("a1", Some("alpha")),
            entry("n1", None),
            entry("b1", Some("beta")),
            entry("a2", Some("alpha")),
        ];
        assert_eq!(
            shape(&group_queue(&rows)),
            vec![
                ("alpha", vec!["a1".to_string(), "a2".to_string()]),
                ("beta", vec!["b1".to_string()]),
                ("other", vec!["n1".to_string()]),
            ]
        );
    }

    #[test]
    fn group_queue_puts_all_null_rows_in_one_other_group() {
        let rows = [entry("n1", None), entry("n2", None)];
        assert_eq!(
            shape(&group_queue(&rows)),
            vec![("other", vec!["n1".to_string(), "n2".to_string()])]
        );
    }

    #[test]
    fn group_queue_of_an_empty_queue_is_empty() {
        assert!(group_queue(&[]).is_empty());
    }

    #[test]
    fn group_queue_keeps_input_order_within_a_group() {
        let rows = [
            entry("z", Some("alpha")),
            entry("m", Some("alpha")),
            entry("a", Some("alpha")),
        ];
        assert_eq!(
            shape(&group_queue(&rows)),
            vec![(
                "alpha",
                vec!["z".to_string(), "m".to_string(), "a".to_string()]
            )]
        );
    }

    #[test]
    fn queue_grouping_starts_off_and_toggle_flips_it() {
        let mut app = App::default();
        assert!(!app.queue_grouped());
        app.toggle_queue_grouped();
        assert!(app.queue_grouped());
    }

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
    fn a_failed_attach_closes_the_panel_frees_the_keys_and_is_not_retried() {
        let fake = crate::pty::SharedFakePty::default();
        let mut app = App::default().with_pty(Box::new(fake.clone()));
        app.apply_snapshot(snapshot_with_decisions(&["d1"]));
        app.toggle_chair_panel();
        assert!(app.chair_panel().is_open());
        fake.0.borrow_mut().set_exited();
        app.poll_chair_panel();
        assert!(!app.chair_panel().is_open());
        assert!(!app.chair_panel().focused());
        assert!(app.chair_panel().ended().is_some());
        // The dead session is no longer offered, so Enter would start a new one.
        assert!(app.chair_panel_awaits_start());
        let spawns = fake
            .0
            .borrow()
            .calls()
            .iter()
            .filter(|c| matches!(c, crate::pty::PtyCall::Spawn { .. }))
            .count();
        app.toggle_chair_panel();
        let after = fake
            .0
            .borrow()
            .calls()
            .iter()
            .filter(|c| matches!(c, crate::pty::PtyCall::Spawn { .. }))
            .count();
        assert_eq!(spawns, after, "no second attach to the dead session");
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

    const SPEND_FIXTURE: &str = include_str!("../tests/fixtures/dash_detail_spend_v1.json");
    const HEALTH_FIXTURE: &str = include_str!("../tests/fixtures/dash_detail_health_v1.json");

    #[test]
    fn each_view_opens_with_its_kind_and_word_and_no_feed() {
        for (open, kind, word) in [
            (App::open_watch as fn(&mut App), DetailKind::Watch, "watch"),
            (App::open_spend, DetailKind::Spend, "spend"),
            (App::open_health, DetailKind::Health, "health"),
        ] {
            let mut app = App::default();
            assert!(app.snapshot().is_none());
            open(&mut app);
            let (open_kind, id, slot) = app.detail().expect("the view should open");
            assert_eq!((*open_kind, id.as_str()), (kind, word));
            assert!(slot.is_none());
            assert!(app.target().is_none());
        }
    }

    #[test]
    fn a_second_view_while_one_is_open_is_ignored() {
        let mut app = App::default();
        app.open_spend();
        app.open_health();
        app.open_watch();
        assert_eq!(app.detail().map(|d| d.0), Some(DetailKind::Spend));
        app.close_detail();
        app.open_health();
        assert_eq!(app.detail().map(|d| d.0), Some(DetailKind::Health));
    }

    #[test]
    fn an_open_spend_takes_a_spend_snapshot_and_ignores_a_health_one() {
        let mut app = App::default();
        app.open_spend();
        let health = crate::detail::parse_detail(HEALTH_FIXTURE.trim()).expect("health parses");
        app.apply_detail_snapshot(health);
        assert!(app.detail().is_some_and(|d| d.2.is_none()));
        let spend = crate::detail::parse_detail(SPEND_FIXTURE.trim()).expect("spend parses");
        app.apply_detail_snapshot(spend);
        assert!(matches!(
            app.detail().and_then(|d| d.2.as_ref()),
            Some(DetailSnapshot::Spend(_))
        ));
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

    #[test]
    fn toggle_inbox_frame_flips_only_the_inbox_flag() {
        let mut app = App::default();
        assert!(app.regatta_inbox_visible());
        app.toggle_inbox_frame();
        assert!(!app.regatta_inbox_visible());
        assert!(app.regatta_history_visible());
        assert!(app.regatta_run_cost_visible());
        assert_eq!(app.regatta_frames_visible(), [true; 6]);
        app.toggle_inbox_frame();
        assert!(app.regatta_inbox_visible());
    }

    #[test]
    fn a_hidden_inbox_is_skipped_by_focus_cycling_in_both_directions() {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.toggle_inbox_frame();
        for _ in 0..8 {
            app.cycle_focus_next();
            assert_ne!(app.focus(), Focus::Inbox);
        }
        assert_eq!(app.focus(), Focus::Runs);
        app.cycle_focus_prev();
        assert_eq!(app.focus(), Focus::History);
        app.cycle_focus_next();
        assert_eq!(app.focus(), Focus::Runs);
    }

    #[test]
    fn hiding_the_focused_inbox_returns_focus_to_runs() {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.cycle_focus_prev();
        assert_eq!(app.focus(), Focus::Inbox);
        app.toggle_inbox_frame();
        assert_eq!(app.focus(), Focus::Runs);
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

    fn press(code: crossterm::event::KeyCode) -> KeyEvent {
        KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
    }

    fn chars(app: &mut App, text: &str) {
        text.chars()
            .for_each(|c| app.modal_key(press(crossterm::event::KeyCode::Char(c))));
    }

    fn app_with_runs() -> App {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.apply_snapshot(make_rich_snapshot());
        app
    }

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    fn result(code: Option<i32>, output: &str) -> ExecResult {
        ExecResult {
            argv: argv(&["cox", "runs", "stop", "r0"]),
            code,
            output: output.to_string(),
        }
    }

    #[test]
    fn a_new_app_has_no_modal_and_no_status() {
        let app = App::default();
        assert_eq!(app.modal(), None);
        assert_eq!(app.status(), None);
    }

    #[test]
    fn begin_action_k_on_the_selected_run_confirms_the_exact_stop_command() {
        let mut app = app_with_runs();
        assert!(app.begin_action('k'));
        let Some(Modal::Confirm(confirm)) = app.modal() else {
            panic!("expected a confirm, got {:?}", app.modal());
        };
        assert_eq!(confirm.command, "cox runs stop r0");
    }

    #[test]
    fn begin_action_with_a_detail_open_targets_the_detail_run() {
        let mut app = app_with_runs();
        app.select_next();
        app.open_detail();
        app.select_prev();
        assert_eq!(app.target(), Some(Target::Run("r1".to_string())));
        assert!(app.begin_action('k'));
        let Some(Modal::Confirm(confirm)) = app.modal() else {
            panic!("expected a confirm, got {:?}", app.modal());
        };
        assert_eq!(confirm.command, "cox runs stop r1");
    }

    #[test]
    fn target_follows_the_focused_list() {
        let mut app = app_with_runs();
        app.cycle_focus_next();
        assert_eq!(app.target(), Some(Target::Initiative("i0".to_string())));
        app.cycle_focus_next();
        assert_eq!(app.target(), Some(Target::Machine("m0".to_string())));
    }

    #[test]
    fn an_unbound_key_returns_false_and_opens_nothing() {
        let mut app = app_with_runs();
        assert!(!app.begin_action('z'));
        assert_eq!(app.modal(), None);
    }

    #[test]
    fn y_sets_pending_to_the_argv_and_take_pending_empties_it() {
        let mut app = app_with_runs();
        app.begin_action('k');
        app.modal_key(press(crossterm::event::KeyCode::Char('y')));
        assert_eq!(app.modal(), None);
        let expected = Pending {
            argv: argv(&["cox", "runs", "stop", "r0"]),
            origin: Origin::Action,
        };
        assert_eq!(app.take_pending(), Some(expected));
        assert_eq!(app.take_pending(), None);
    }

    #[test]
    fn n_and_esc_close_the_confirm_and_set_nothing() {
        for code in [
            crossterm::event::KeyCode::Char('n'),
            crossterm::event::KeyCode::Esc,
        ] {
            let mut app = app_with_runs();
            app.begin_action('k');
            app.modal_key(press(code));
            assert_eq!(app.modal(), None);
            assert_eq!(app.take_pending(), None);
        }
    }

    #[test]
    fn a_second_yes_while_a_command_is_pending_is_ignored() {
        let mut app = app_with_runs();
        app.begin_action('k');
        app.modal_key(press(crossterm::event::KeyCode::Char('y')));
        app.begin_action('p');
        app.modal_key(press(crossterm::event::KeyCode::Char('y')));
        assert_eq!(
            app.take_pending().map(|p| p.argv),
            Some(argv(&["cox", "runs", "stop", "r0"]))
        );
        assert_eq!(app.take_pending(), None);
    }

    #[test]
    fn m_on_a_run_opens_the_palette_prefilled_with_the_move() {
        let mut app = app_with_runs();
        assert!(app.begin_action('m'));
        let Some(Modal::Palette(palette)) = app.modal() else {
            panic!("expected a palette, got {:?}", app.modal());
        };
        assert_eq!(palette.input(), move_prefill("r0"));
    }

    #[test]
    fn palette_submit_sets_pending_with_origin_palette_and_stays_open() {
        let mut app = App::default();
        app.open_palette();
        chars(&mut app, "route list");
        app.modal_key(press(crossterm::event::KeyCode::Enter));
        assert!(matches!(app.modal(), Some(Modal::Palette(_))));
        assert_eq!(
            app.take_pending(),
            Some(Pending {
                argv: argv(&["cox", "route", "list"]),
                origin: Origin::Palette,
            })
        );
    }

    #[test]
    fn palette_esc_closes_it() {
        let mut app = App::default();
        app.open_palette();
        app.modal_key(press(crossterm::event::KeyCode::Esc));
        assert_eq!(app.modal(), None);
    }

    #[test]
    fn apply_exec_result_sets_an_ok_and_a_failed_status() {
        let mut app = App::default();
        app.apply_exec_result(result(Some(0), "stopped r0\n"), Origin::Action);
        assert_eq!(
            app.status(),
            Some(&(StatusLevel::Ok, "stopped r0".to_string()))
        );
        app.apply_exec_result(result(Some(2), "no such run\n"), Origin::Action);
        assert_eq!(
            app.status(),
            Some(&(
                StatusLevel::Failed,
                "failed (exit 2): no such run".to_string()
            ))
        );
    }

    #[test]
    fn a_palette_result_fills_the_open_palette_output_and_the_status() {
        let mut app = App::default();
        app.open_palette();
        app.apply_exec_result(result(Some(0), "stopped r0\n"), Origin::Palette);
        let Some(Modal::Palette(palette)) = app.modal() else {
            panic!("expected a palette, got {:?}", app.modal());
        };
        assert_eq!(
            palette.output(),
            Some(&argv(&["$ cox runs stop r0", "stopped r0", "exit 0"])[..])
        );
        assert_eq!(app.status().map(|s| s.0), Some(StatusLevel::Ok));
    }

    const ROWS: &str = r#"{"sections":[{"id":"budgets","rows":[{"section":"budgets","scope":"budgets","key":"max_usd","value":"20","file":"f.toml","tracked":true,"pat_only":false}]}]}"#;

    fn settings_result(argv_words: &[&str], code: Option<i32>, output: &str) -> ExecResult {
        ExecResult {
            argv: argv(argv_words),
            code,
            output: output.to_string(),
        }
    }

    fn settings_press(app: &mut App, code: crossterm::event::KeyCode) {
        app.settings_key(press(code));
    }

    /// An app whose screen holds the loaded rows and has `budgets max_usd` staged at 40.
    fn app_with_staged_edit() -> App {
        use crossterm::event::KeyCode;
        let mut app = App::default();
        app.open_settings();
        app.take_pending();
        app.apply_exec_result(
            settings_result(&["cox", "settings", "get", "--json"], Some(0), ROWS),
            Origin::SettingsLoad,
        );
        settings_press(&mut app, KeyCode::Tab);
        settings_press(&mut app, KeyCode::Enter);
        settings_press(&mut app, KeyCode::Backspace);
        settings_press(&mut app, KeyCode::Backspace);
        settings_press(&mut app, KeyCode::Char('4'));
        settings_press(&mut app, KeyCode::Char('0'));
        settings_press(&mut app, KeyCode::Enter);
        app
    }

    fn dry_run_origin() -> Origin {
        Origin::SettingsDryRun {
            scope: "budgets".into(),
            key: "max_usd".into(),
        }
    }

    fn apply_origin() -> Origin {
        Origin::SettingsApply {
            scope: "budgets".into(),
            key: "max_usd".into(),
        }
    }

    #[test]
    fn open_settings_sets_the_load_pending() {
        let mut app = App::default();
        app.open_settings();
        assert!(app.settings().is_some());
        assert_eq!(
            app.take_pending(),
            Some(Pending {
                argv: argv(&["cox", "settings", "get", "--json"]),
                origin: Origin::SettingsLoad,
            })
        );
    }

    #[test]
    fn a_load_result_fills_the_rows_and_a_failed_one_sets_a_failed_status() {
        let mut app = App::default();
        app.open_settings();
        app.apply_exec_result(settings_result(&[], Some(0), ROWS), Origin::SettingsLoad);
        let rows = app.settings().map(|s| s.current_rows().len());
        assert_eq!(rows, Some(1));
        app.apply_exec_result(
            settings_result(&[], Some(1), "boom\n"),
            Origin::SettingsLoad,
        );
        assert_eq!(app.status().map(|s| s.0), Some(StatusLevel::Failed));
    }

    #[test]
    fn stage_sets_the_dry_run_pending() {
        let mut app = app_with_staged_edit();
        assert_eq!(
            app.take_pending(),
            Some(Pending {
                argv: argv(&[
                    "cox",
                    "settings",
                    "set",
                    "budgets",
                    "max_usd",
                    "40",
                    "--dry-run"
                ]),
                origin: dry_run_origin(),
            })
        );
    }

    #[test]
    fn a_refused_dry_run_stores_the_refusal_and_keeps_the_edit_staged() {
        let mut app = app_with_staged_edit();
        app.apply_exec_result(
            settings_result(&[], Some(1), "max_usd must be below 30\n"),
            dry_run_origin(),
        );
        let staged = app
            .settings()
            .map(|s| s.staged().clone())
            .unwrap_or_default();
        assert_eq!(staged.edits().len(), 1);
        assert_eq!(
            staged.refusal_for("budgets", "max_usd"),
            Some("max_usd must be below 30")
        );
    }

    #[test]
    fn apply_opens_a_confirm_showing_the_exact_set_command() {
        use crossterm::event::KeyCode;
        let mut app = app_with_staged_edit();
        app.take_pending();
        settings_press(&mut app, KeyCode::Tab);
        settings_press(&mut app, KeyCode::Char('a'));
        let Some(Modal::Confirm(confirm)) = app.modal() else {
            panic!("expected a confirm, got {:?}", app.modal());
        };
        assert_eq!(confirm.command, "cox settings set budgets max_usd 40");
    }

    #[test]
    fn y_on_the_apply_confirm_sets_the_apply_pending_and_n_sets_nothing() {
        use crossterm::event::KeyCode;
        let mut app = app_with_staged_edit();
        app.take_pending();
        settings_press(&mut app, KeyCode::Tab);
        settings_press(&mut app, KeyCode::Char('a'));
        app.modal_key(press(KeyCode::Char('y')));
        assert_eq!(
            app.take_pending(),
            Some(Pending {
                argv: argv(&["cox", "settings", "set", "budgets", "max_usd", "40"]),
                origin: apply_origin(),
            })
        );
        settings_press(&mut app, KeyCode::Char('a'));
        app.modal_key(press(KeyCode::Char('n')));
        assert_eq!(app.take_pending(), None);
        assert_eq!(app.modal(), None);
    }

    #[test]
    fn an_applied_result_removes_the_edit_and_queues_a_reload() {
        let mut app = app_with_staged_edit();
        app.take_pending();
        app.apply_exec_result(settings_result(&[], Some(0), "set\n"), apply_origin());
        let edits = app.settings().map(|s| s.staged().edits().len());
        assert_eq!(edits, Some(0));
        assert_eq!(app.status().map(|s| s.0), Some(StatusLevel::Ok));
        assert_eq!(
            app.take_pending().map(|p| (p.argv, p.origin)),
            Some((
                argv(&["cox", "settings", "get", "--json"]),
                Origin::SettingsLoad
            ))
        );
    }

    #[test]
    fn a_refused_apply_keeps_the_edit_staged_with_its_refusal() {
        let mut app = app_with_staged_edit();
        app.take_pending();
        app.apply_exec_result(settings_result(&[], Some(1), "locked\n"), apply_origin());
        let staged = app
            .settings()
            .map(|s| s.staged().clone())
            .unwrap_or_default();
        assert_eq!(staged.refusal_for("budgets", "max_usd"), Some("locked"));
        assert_eq!(app.take_pending(), None);
    }

    #[test]
    fn palette_settings_opens_the_screen_and_runs_no_command() {
        for line in ["settings", "cox settings"] {
            let mut app = App::default();
            app.open_palette();
            chars(&mut app, line);
            app.modal_key(press(crossterm::event::KeyCode::Enter));
            assert!(app.settings().is_some());
            assert_eq!(app.modal(), None);
            assert_eq!(
                app.take_pending().map(|p| p.origin),
                Some(Origin::SettingsLoad)
            );
        }
    }

    fn form_keys(app: &mut App, keys: &[crossterm::event::KeyCode]) {
        keys.iter().for_each(|code| app.form_key(press(*code)));
    }

    fn type_into_form(app: &mut App, text: &str) {
        text.chars()
            .for_each(|c| app.form_key(press(crossterm::event::KeyCode::Char(c))));
    }

    /// Fills the add machine form and presses Enter on its last field.
    fn submitted_machine_form() -> App {
        use crossterm::event::KeyCode::{Backspace, Enter};
        let mut app = App::default();
        app.open_add_machine();
        type_into_form(&mut app, "edge-1");
        form_keys(&mut app, &[Enter]);
        type_into_form(&mut app, "pat@edge-1");
        form_keys(&mut app, &[Enter, Backspace]);
        type_into_form(&mut app, "4");
        form_keys(&mut app, &[Enter, Enter]);
        app
    }

    fn form_result_of(words: &[&str], code: Option<i32>, output: &str) -> ExecResult {
        ExecResult {
            argv: argv(words),
            code,
            output: output.to_string(),
        }
    }

    const ADD: [&str; 8] = [
        "cox",
        "host",
        "add",
        "edge-1",
        "--ssh",
        "pat@edge-1",
        "--capacity",
        "4",
    ];

    #[test]
    fn submitting_a_filled_machine_form_confirms_the_exact_add_command() {
        let app = submitted_machine_form();
        let Some(Modal::Confirm(confirm)) = app.modal() else {
            panic!("expected a confirm, got {:?}", app.modal());
        };
        assert_eq!(
            confirm.command,
            "cox host add edge-1 --ssh pat@edge-1 --capacity 4"
        );
        assert!(app.form().is_some());
    }

    #[test]
    fn no_on_the_confirm_returns_to_the_form_with_its_values() {
        let mut app = submitted_machine_form();
        app.modal_key(press(crossterm::event::KeyCode::Char('n')));
        assert_eq!(app.modal(), None);
        assert_eq!(app.take_pending(), None);
        let form = app.form().expect("the form stays open");
        assert_eq!(form.value_of("name"), "edge-1");
        assert_eq!(form.value_of("ssh"), "pat@edge-1");
        assert_eq!(form.value_of("capacity"), "4");
    }

    #[test]
    fn yes_on_the_confirm_sets_pending_to_the_add_argv_with_step_zero() {
        let mut app = submitted_machine_form();
        app.modal_key(press(crossterm::event::KeyCode::Char('y')));
        assert_eq!(
            app.take_pending(),
            Some(Pending {
                argv: argv(&ADD),
                origin: Origin::Form { step: 0 },
            })
        );
        assert!(app.form().is_some());
    }

    #[test]
    fn a_refused_add_keeps_the_form_open_and_shows_the_refusal() {
        let mut app = submitted_machine_form();
        app.modal_key(press(crossterm::event::KeyCode::Char('y')));
        app.take_pending();
        let refused = form_result_of(&ADD, Some(2), "host edge-1 already exists");
        app.apply_exec_result(refused, Origin::Form { step: 0 });
        assert!(app.form().is_some());
        assert_eq!(app.modal(), None);
        assert_eq!(
            app.status(),
            Some(&(
                StatusLevel::Failed,
                "failed (exit 2): host edge-1 already exists".to_string()
            ))
        );
    }

    #[test]
    fn a_successful_add_sets_pending_to_the_doctor_argv_with_step_one() {
        let mut app = submitted_machine_form();
        app.modal_key(press(crossterm::event::KeyCode::Char('y')));
        app.take_pending();
        app.apply_exec_result(form_result_of(&ADD, Some(0), ""), Origin::Form { step: 0 });
        assert_eq!(
            app.take_pending(),
            Some(Pending {
                argv: argv(&["cox", "host", "doctor", "edge-1"]),
                origin: Origin::Form { step: 1 },
            })
        );
        assert!(app.form().is_some());
    }

    #[test]
    fn the_doctor_result_closes_the_form_and_fills_the_palette_frame() {
        let mut app = submitted_machine_form();
        app.modal_key(press(crossterm::event::KeyCode::Char('y')));
        app.take_pending();
        app.apply_exec_result(form_result_of(&ADD, Some(0), ""), Origin::Form { step: 0 });
        app.take_pending();
        let doctor = ["cox", "host", "doctor", "edge-1"];
        app.apply_exec_result(
            form_result_of(&doctor, Some(0), "ssh ok"),
            Origin::Form { step: 1 },
        );
        assert!(app.form().is_none());
        let Some(Modal::Palette(palette)) = app.modal() else {
            panic!("expected a palette, got {:?}", app.modal());
        };
        let output = palette.output().expect("the palette is in frame mode");
        assert_eq!(
            output.first().map(String::as_str),
            Some("$ cox host doctor edge-1")
        );
        assert_eq!(app.status(), Some(&(StatusLevel::Ok, "ssh ok".to_string())));
    }

    #[test]
    fn a_failed_doctor_still_closes_the_form_and_shows_its_lines() {
        let mut app = submitted_machine_form();
        app.modal_key(press(crossterm::event::KeyCode::Char('y')));
        app.take_pending();
        app.apply_exec_result(form_result_of(&ADD, Some(0), ""), Origin::Form { step: 0 });
        app.take_pending();
        let doctor = ["cox", "host", "doctor", "edge-1"];
        app.apply_exec_result(
            form_result_of(&doctor, Some(1), "ssh refused"),
            Origin::Form { step: 1 },
        );
        assert!(app.form().is_none());
        assert!(matches!(app.modal(), Some(Modal::Palette(_))));
        assert_eq!(
            app.status().map(|(level, _)| *level),
            Some(StatusLevel::Failed)
        );
    }

    #[test]
    fn an_invalid_form_shows_its_error_and_opens_no_confirm() {
        use crossterm::event::KeyCode::Enter;
        let mut app = App::default();
        app.open_add_machine();
        form_keys(&mut app, &[Enter, Enter, Enter, Enter]);
        assert_eq!(app.modal(), None);
        let form = app.form().expect("the form stays open");
        assert_eq!(
            form.error.as_deref(),
            Some("name must be non-empty with no whitespace")
        );
    }

    #[test]
    fn esc_closes_the_form() {
        let mut app = App::default();
        app.open_add_machine();
        form_keys(&mut app, &[crossterm::event::KeyCode::Esc]);
        assert!(app.form().is_none());
    }

    #[test]
    fn form_keys_are_ignored_while_a_confirm_is_open() {
        let mut app = submitted_machine_form();
        type_into_form(&mut app, "x");
        assert_eq!(app.form().map(|f| f.value_of("capabilities")), Some(""));
    }

    #[test]
    fn a_second_yes_on_a_form_confirm_while_pending_is_ignored() {
        let mut app = submitted_machine_form();
        app.modal_key(press(crossterm::event::KeyCode::Char('y')));
        form_keys(&mut app, &[crossterm::event::KeyCode::Enter]);
        app.modal_key(press(crossterm::event::KeyCode::Char('y')));
        assert_eq!(app.take_pending().map(|p| p.argv), Some(argv(&ADD)));
        assert_eq!(app.take_pending(), None);
    }

    #[test]
    fn take_editor_request_is_empty_without_a_body_field() {
        let mut app = App::default();
        app.open_add_machine();
        assert_eq!(app.take_editor_request(), None);
    }

    fn initiative_detail() -> InitiativeDetail {
        let json = r#"{"schema":1,"kind":"initiative","at":"2026-09-29T00:00:00Z","initiative":"i0","title":"Old title","repo":"coxtop","body":"old body","phases":[],"history":[]}"#;
        match crate::detail::parse_detail(json).expect("literal detail should parse") {
            DetailSnapshot::Initiative(detail) => detail,
            other => panic!("expected an initiative detail, got {other:?}"),
        }
    }

    fn field_values(app: &App) -> Vec<(String, String)> {
        app.form()
            .map(|f| {
                f.fields
                    .iter()
                    .map(|field| (field.label.clone(), field.value.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn begin_edit_with_a_loaded_matching_detail_opens_the_prefilled_form() {
        let mut app = app_with_runs();
        app.cycle_focus_next();
        app.open_detail();
        app.apply_detail_snapshot(DetailSnapshot::Initiative(initiative_detail()));
        app.begin_edit("i0");
        let form = app.form().expect("the edit form opens");
        assert_eq!(form.title, "Edit initiative i0");
        assert_eq!(form.value_of("title"), "Old title");
        assert_eq!(app.take_detail_request(), None);
    }

    #[test]
    fn begin_edit_without_a_detail_requests_it_and_opens_no_form() {
        let mut app = app_with_runs();
        app.begin_edit("i0");
        assert!(app.form().is_none());
        assert_eq!(app.take_detail_request(), Some("i0".to_string()));
        assert_eq!(app.take_detail_request(), None);
    }

    #[test]
    fn begin_edit_for_another_initiative_requests_its_detail() {
        let mut app = app_with_runs();
        app.cycle_focus_next();
        app.open_detail();
        app.apply_detail_snapshot(DetailSnapshot::Initiative(initiative_detail()));
        app.begin_edit("i9");
        assert!(app.form().is_none());
        assert_eq!(app.take_detail_request(), Some("i9".to_string()));
    }

    #[test]
    fn open_edit_initiative_prefills_repo_title_and_body() {
        let mut app = App::default();
        app.open_edit_initiative(&initiative_detail());
        let form = app.form().expect("the edit form opens");
        assert_eq!(form.value_of("repo"), "coxtop");
        assert_eq!(form.value_of("title"), "Old title");
        let body = form.fields.iter().find(|f| f.label == "body");
        assert_eq!(body.map(|f| f.text.as_str()), Some("old body"));
    }

    #[test]
    fn open_new_initiative_offers_the_queues_distinct_repos() {
        let json = r#"{"schema":1,"at":"2026-09-29T00:00:00Z","chair":{"holder":"h","host":"h","epoch":1,"liveness":"live","beat_age_s":0},"spend":{"five_hour_fraction":0.0,"five_hour_source":"meter","weekly_fraction":0.0,"weekly_source":"meter","hard_stop_fraction":0.0,"five_hour_resets_at":"2026-09-29T00:00:00Z","weekly_resets_at":"2026-09-29T00:00:00Z"},"machines":[],"runs":[],"queue":[{"initiative":"a","priority":1,"phases_landed":0,"phases_total":1,"current_phase":"p","repo":"coxtop"},{"initiative":"b","priority":1,"phases_landed":0,"phases_total":1,"current_phase":"p","repo":"pat-skills"},{"initiative":"c","priority":1,"phases_landed":0,"phases_total":1,"current_phase":"p","repo":"coxtop"}],"inbox":[],"watch":[]}"#;
        let mut app = App::default();
        app.apply_snapshot(crate::feed::parse_snapshot(json).expect("literal snapshot"));
        app.open_new_initiative();
        let kind = app.form().map(|f| f.fields[0].kind.clone());
        assert_eq!(
            kind,
            Some(crate::form::FieldKind::Choice(argv(&[
                "coxtop",
                "pat-skills"
            ])))
        );
    }

    #[test]
    fn editor_done_replaces_the_body_and_marks_the_field_changed() {
        let mut app = App::default();
        app.open_edit_initiative(&initiative_detail());
        app.editor_done(
            2,
            Some(PathBuf::from("/tmp/body.md")),
            Ok("new body".to_string()),
        );
        let body = app.form().and_then(|f| f.fields.get(2)).cloned();
        let body = body.expect("the body field exists");
        assert_eq!(body.text, "new body");
        assert_eq!(body.path, Some(PathBuf::from("/tmp/body.md")));
        assert!(body.changed());
    }

    #[test]
    fn a_failed_editor_leaves_the_body_and_sets_a_failed_status() {
        let mut app = App::default();
        app.open_edit_initiative(&initiative_detail());
        app.editor_done(2, None, Err("editor: not found".to_string()));
        let body = app.form().and_then(|f| f.fields.get(2)).cloned();
        assert_eq!(body.map(|f| f.text), Some("old body".to_string()));
        assert_eq!(
            app.status(),
            Some(&(StatusLevel::Failed, "editor: not found".to_string()))
        );
    }

    #[test]
    fn a_remove_form_with_a_reason_confirms_the_exact_remove_command() {
        let mut app = App::default();
        app.open_remove_initiative("ID");
        assert_eq!(
            field_values(&app),
            vec![("reason".to_string(), String::new())]
        );
        type_into_form(&mut app, "dup");
        form_keys(&mut app, &[crossterm::event::KeyCode::Enter]);
        let Some(Modal::Confirm(confirm)) = app.modal() else {
            panic!("expected a confirm, got {:?}", app.modal());
        };
        assert_eq!(confirm.command, "cox route remove ID --reason dup");
    }

    #[test]
    fn an_edit_that_exits_2_keeps_the_form_open_with_the_refusal() {
        let mut app = App::default();
        app.open_edit_initiative(&initiative_detail());
        form_keys(&mut app, &[crossterm::event::KeyCode::Enter]);
        type_into_form(&mut app, "!");
        app.submit_form();
        assert!(matches!(app.modal(), Some(Modal::Confirm(_))));
        app.modal_key(press(crossterm::event::KeyCode::Char('y')));
        let pending = app.take_pending().expect("a Yes queues the edit");
        assert_eq!(pending.origin, Origin::Form { step: 0 });
        let refused = form_result_of(&["cox", "route", "edit", "i0"], Some(2), "no such repo");
        app.apply_exec_result(refused, Origin::Form { step: 0 });
        assert!(app.form().is_some());
        assert_eq!(app.modal(), None);
        assert_eq!(
            app.status(),
            Some(&(
                StatusLevel::Failed,
                "failed (exit 2): no such repo".to_string()
            ))
        );
    }
}
