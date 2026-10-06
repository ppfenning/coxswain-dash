//! Pure keyboard dispatch: turns one crossterm `KeyCode` into the single `App` mutation it
//! maps to. No I/O and no rendering; `src/main.rs`'s event loop is the only caller.

use std::process::{Command, Stdio};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::actions::Target;
use crate::app::{App, Focus, PANEL_COLS, PANEL_ROWS};
use crate::decision_card::KeyOutcome;
use crate::form::{KEY_ADD_MACHINE, KEY_EDIT, KEY_NEW, KEY_REMOVE};

/// Runs the card's answer argv without waiting on it. A failed spawn has nowhere to report to
/// and the decision stays open in the feed, so the error is dropped here at the edge.
fn run_answer(argv: &[String]) {
    let Some((program, args)) = argv.split_first() else {
        return;
    };
    let spawned = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Ok(mut child) = spawned {
        // Reaps the child off the UI thread so it does not linger as a zombie.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

/// Routes one key press: a focused panel takes every key, then a showing decision card takes its
/// keys, and anything left goes to `handle_key`.
pub fn handle_event(app: &mut App, key: KeyEvent) {
    handle_event_with(app, key, &mut run_answer);
}

/// `handle_event` with the answer runner passed in, so a test runs no process.
pub fn handle_event_with(app: &mut App, key: KeyEvent, run: &mut dyn FnMut(&[String])) {
    // The close prompt spends every key, so nothing reaches the pty or a handler behind it.
    if app.chair_panel().confirming_close() {
        match key.code {
            KeyCode::Char('y' | 'Y') => app.answer_chair_close(true),
            KeyCode::Char('n' | 'N') | KeyCode::Esc => app.answer_chair_close(false),
            _ => {}
        }
        return;
    }
    if app.chair_panel().focused() {
        // `Ctrl-]` makes the panel clear its own focus; either way the key is spent.
        app.chair_panel_mut().handle_key(key);
        return;
    }
    // A shown panel with no session keeps focus until `Ctrl-]` or a frame-focus key moves it.
    if app.chair_panel_focused() {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char(']') {
            app.set_chair_panel_focus(false);
            return;
        }
        if matches!(key.code, KeyCode::Left | KeyCode::Right) {
            app.set_chair_panel_focus(false);
        }
    }
    // An open modal spends every key, so the confirm's `y` never reaches the decision card.
    if app.modal().is_some() {
        app.modal_key(key);
        return;
    }
    // An open form spends every key, so a showing card does not see them.
    if app.card_visible() && app.form().is_none() && app.settings().is_none() {
        match app.decision_card_mut().on_key(key.code) {
            KeyOutcome::Selected(_) => return,
            KeyOutcome::Answer(argv) => {
                run(&argv);
                app.hide_card();
                return;
            }
            KeyOutcome::Cancelled => {
                app.hide_card();
                return;
            }
            KeyOutcome::FocusSession if app.chair_panel().is_open() => {
                app.chair_panel_mut().set_focus(true);
                return;
            }
            // An option number past the last option must not toggle a frame behind the card.
            KeyOutcome::Ignored if matches!(key.code, KeyCode::Char('1'..='9')) => return,
            KeyOutcome::FocusSession | KeyOutcome::Ignored => {}
        }
    }
    handle_key(app, key.code);
}

pub fn handle_key(app: &mut App, key: KeyCode) {
    if app.modal().is_some() {
        app.modal_key(KeyEvent::new(key, KeyModifiers::NONE));
        return;
    }
    // An open settings screen spends every key, so its Tab moves a pane and not the page.
    if app.settings().is_some() {
        app.settings_key(KeyEvent::new(key, KeyModifiers::NONE));
        return;
    }
    // An open form spends every key, so a typed `k` or `t` never reaches a handler behind it.
    if app.form().is_some() {
        app.form_key(KeyEvent::new(key, KeyModifiers::NONE));
        return;
    }
    if key == KeyCode::Char(':') {
        app.open_palette();
        return;
    }
    // `p` is run pause on a run target; the layout toggle is `v`, so the two never share a key.
    if let KeyCode::Char(c) = key {
        if app.begin_action(c) {
            return;
        }
    }
    match key {
        KeyCode::Char('`') => app.toggle_chair_panel(),
        KeyCode::Char('\\') => app.hide_chair_panel(),
        KeyCode::Char('~') => app.chair_panel_mut().cycle_width(),
        KeyCode::Tab => app.next_page(),
        KeyCode::BackTab => app.prev_page(),
        KeyCode::Char('t') => app.toggle_theme(),
        // `s` was unbound in the landed keymap, so it opens the settings screen.
        KeyCode::Char('s') => app.open_settings(),
        KeyCode::Char('v') => app.cycle_regatta_layout_preset(),
        // No action binding uses `w`, `$` or `h`; a test over `actions::bindings` holds that.
        KeyCode::Char('w') => app.open_watch(),
        KeyCode::Char('$') => app.open_spend(),
        KeyCode::Char('h') => app.open_health(),
        KeyCode::Char('7') => app.toggle_history_frame(),
        KeyCode::Char('8') => app.toggle_run_cost_frame(),
        KeyCode::Char('9') => app.toggle_inbox_frame(),
        KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
            app.toggle_regatta_frame(c.to_digit(10).unwrap() as usize)
        }
        KeyCode::Right => app.cycle_focus_next(),
        KeyCode::Left => app.cycle_focus_prev(),
        // Arrows only: `k` is the kill action, so j/k no longer move the selection.
        KeyCode::Down => app.select_next(),
        KeyCode::Up => app.select_prev(),
        KeyCode::Enter if app.chair_panel_awaits_start() => {
            app.chair_panel_mut().start_session(PANEL_ROWS, PANEL_COLS);
            app.chair_panel_mut().set_focus(true);
            // The live pty holds focus from here; a failed start drops it back to the frames.
            app.set_chair_panel_focus(false);
        }
        KeyCode::Enter => app.open_detail(),
        KeyCode::Esc => app.close_detail(),
        KeyCode::Char(c) if c == KEY_ADD_MACHINE && app.focus() == Focus::Machines => {
            app.open_add_machine()
        }
        KeyCode::Char(c) if c == KEY_NEW && app.focus() == Focus::Queue => {
            app.open_new_initiative()
        }
        KeyCode::Char(c) if c == KEY_EDIT => on_initiative(app, App::begin_edit),
        KeyCode::Char(c) if c == KEY_REMOVE => on_initiative(app, App::open_remove_initiative),
        _ => {}
    }
}

/// Applies `act` to the initiative `app.target()` names; any other target does nothing.
fn on_initiative(app: &mut App, act: fn(&mut App, &str)) {
    if let Some(Target::Initiative(id)) = app.target() {
        act(app, &id)
    }
}

fn contains(rect: Rect, x: u16, y: u16) -> bool {
    x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
}

/// The zero-based row inside `rect`'s bordered block that `(x, y)` falls on, given the list
/// drawn there has `len` rows. `None` for a point on the border, past the last row, or outside
/// `rect` entirely. Mirrors `ui::regatta::row_at`'s per-rect math; duplicated here because
/// `ui::regatta` is a private module this crate's mouse handler cannot reach, so this stays the
/// mouse handler's own small, independently-tested copy.
fn row_in_rect(rect: Rect, len: usize, x: u16, y: u16) -> Option<usize> {
    if x < rect.x + 1
        || x + 1 >= rect.x + rect.width
        || y < rect.y + 1
        || y + 1 >= rect.y + rect.height
    {
        return None;
    }
    let row = (y - rect.y - 1) as usize;
    (row < len).then_some(row)
}

/// Which selectable list and row `(x, y)` lands on among `rects`' machines (index 2), runs
/// (index 4) or queue (index 5) frames, or `None` when there is no snapshot yet or the point
/// misses every one of them.
fn row_hit(app: &App, rects: &[Option<Rect>; 6], x: u16, y: u16) -> Option<(Focus, usize)> {
    let snapshot = app.snapshot()?;
    let candidates = [
        (2, Focus::Machines, snapshot.machines.len()),
        (4, Focus::Runs, snapshot.runs.len()),
        (5, Focus::Queue, snapshot.queue.len()),
    ];
    for (idx, focus, len) in candidates {
        if let Some(rect) = rects[idx] {
            if let Some(row) = row_in_rect(rect, len, x, y) {
                return Some((focus, row));
            }
        }
    }
    None
}

/// The list a click on `rects[idx]` focuses: machines (2), runs (4) or queue (5).
fn list_focus(idx: usize) -> Option<Focus> {
    match idx {
        2 => Some(Focus::Machines),
        4 => Some(Focus::Runs),
        5 => Some(Focus::Queue),
        _ => None,
    }
}

/// While the chair panel has focus, a press on a frame releases it and focuses that frame's
/// list, when the frame has one. `rects` never overlaps the panel, since `regatta::frame_rects`
/// lays the frames out in the page the panel leaves, so a press in the panel, or in no frame,
/// keeps focus where it is.
fn press_with_chair_focus(app: &mut App, rects: &[Option<Rect>; 6], x: u16, y: u16) {
    if let Some(idx) = rects
        .iter()
        .position(|rect| rect.is_some_and(|rect| contains(rect, x, y)))
    {
        app.release_chair_focus_to(list_focus(idx));
    } else {
        // The press is in the panel or between frames: focus stays on the panel.
    }
}

/// A left-button press on a row of the runs, queue or machines frame focuses that list, selects
/// the row and opens its detail. A press elsewhere inside `rects[i]` toggles regatta frame
/// `i + 1`, today's behavior; a press outside every rect is a no-op. `rects` is
/// `regatta::frame_rects`'s output, so this never disagrees with what is actually drawn.
/// While the chair panel has focus, a press inside it changes nothing and a press on any other
/// frame releases focus and focuses that frame, without hiding it or opening a detail. A frame
/// with no list (chair, spend, lanes) takes the release and leaves the list focus unchanged.
pub fn handle_mouse(app: &mut App, event: MouseEvent, rects: &[Option<Rect>; 6]) {
    if event.kind != MouseEventKind::Down(MouseButton::Left) {
        return;
    }
    if app.chair_focused() {
        return press_with_chair_focus(app, rects, event.column, event.row);
    }
    if let Some((focus, row)) = row_hit(app, rects, event.column, event.row) {
        for _ in 0..3 {
            if app.focus() == focus {
                break;
            }
            app.cycle_focus_next();
        }
        app.select_at(row);
        app.open_detail();
        return;
    }
    for (i, rect) in rects.iter().enumerate() {
        if let Some(rect) = rect {
            if contains(*rect, event.column, event.row) {
                app.toggle_regatta_frame(i + 1);
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppPage, Focus, ThemeId};

    /// A snapshot with two runs (`r0`, `r1`), for the selection-key tests below.
    fn snapshot_with_two_runs() -> crate::feed::FeedSnapshot {
        let json = r#"{"schema":1,"at":"2026-09-29T00:00:00Z","chair":{"holder":"h","host":"h","epoch":1,"liveness":"live","beat_age_s":0},"spend":{"five_hour_fraction":0.0,"five_hour_source":"meter","weekly_fraction":0.0,"weekly_source":"meter","hard_stop_fraction":0.0,"five_hour_resets_at":"2026-09-29T00:00:00Z","weekly_resets_at":"2026-09-29T00:00:00Z"},"machines":[{"name":"m0","state":"active","lanes_in_use":0,"capacity":3,"login_ok":true,"login_checked_at":"2026-09-29T00:00:00Z","beat_age_s":0,"checkouts":{}}],"runs":[{"run":"r0","machine":"m0","phase":"p","node":"n","attempt":1,"turns":1,"cost":0.0,"verdict":"ok","status":"running"},{"run":"r1","machine":"m0","phase":"p","node":"n","attempt":1,"turns":1,"cost":0.0,"verdict":"ok","status":"running"}],"queue":[],"inbox":[],"watch":[]}"#;
        crate::feed::parse_snapshot(json).expect("literal snapshot should parse")
    }

    fn left_click_at(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn sample_rects() -> [Option<Rect>; 6] {
        [
            Some(Rect::new(0, 0, 10, 5)),
            Some(Rect::new(10, 0, 10, 5)),
            Some(Rect::new(0, 5, 10, 5)),
            Some(Rect::new(10, 5, 10, 5)),
            Some(Rect::new(0, 10, 10, 5)),
            Some(Rect::new(10, 10, 10, 5)),
        ]
    }

    #[test]
    fn a_click_inside_rects_one_hides_frame_two() {
        let mut app = App::default();
        let rects = sample_rects();
        handle_mouse(&mut app, left_click_at(12, 2), &rects);
        assert_eq!(
            app.regatta_frames_visible(),
            [true, false, true, true, true, true]
        );
    }

    #[test]
    fn a_click_outside_every_rect_changes_nothing() {
        let mut app = App::default();
        let rects = sample_rects();
        handle_mouse(&mut app, left_click_at(100, 100), &rects);
        assert_eq!(app.regatta_frames_visible(), [true; 6]);
    }

    #[test]
    fn a_click_on_the_runs_frames_second_row_selects_it_and_opens_its_detail() {
        let mut app = App::default();
        app.apply_snapshot(snapshot_with_two_runs());
        let area = Rect::new(0, 0, 120, 40);
        let rects = crate::ui::regatta_frame_rects(area, &app);
        let runs_rect = rects[4].expect("runs frame is visible");
        handle_mouse(
            &mut app,
            left_click_at(runs_rect.x + 1, runs_rect.y + 2),
            &rects,
        );
        assert_eq!(app.focus(), Focus::Runs);
        assert_eq!(app.selected(), 1);
        let (kind, id, _) = app.detail().expect("detail should be open");
        assert_eq!(*kind, crate::app::DetailKind::Run);
        assert_eq!(id, "r1");
    }

    #[test]
    fn a_click_outside_every_row_changes_nothing() {
        let mut app = App::default();
        app.apply_snapshot(snapshot_with_two_runs());
        let area = Rect::new(0, 0, 120, 40);
        let rects = crate::ui::regatta_frame_rects(area, &app);
        let before_focus = app.focus();
        let before_selected = app.selected();
        let before_visible = app.regatta_frames_visible();
        // (119, 39) sits in the always-visible inbox strip below the six numbered frames, so it
        // is outside every one of `rects`.
        handle_mouse(&mut app, left_click_at(119, 39), &rects);
        assert_eq!(app.focus(), before_focus);
        assert_eq!(app.selected(), before_selected);
        assert!(app.detail().is_none());
        assert_eq!(app.regatta_frames_visible(), before_visible);
    }

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// A snapshot naming chair session `s1` with open decisions `d1` (options a, b) and, when
    /// `second` is set, `d2`.
    fn snapshot_with_decisions(second: bool) -> crate::feed::FeedSnapshot {
        let extra = if second {
            r#",{"id":"d2","question":"q2","options":["x","y"],"context":"c","asked_at":"2026-09-29T00:00:00Z"}"#
        } else {
            ""
        };
        let json = format!(
            r#"{{"schema":1,"at":"2026-09-29T00:00:00Z","chair":{{"holder":"h","host":"h","epoch":1,"liveness":"live","beat_age_s":0,"session":"s1"}},"spend":{{"five_hour_fraction":0.0,"five_hour_source":"meter","weekly_fraction":0.0,"weekly_source":"meter","hard_stop_fraction":0.0,"five_hour_resets_at":"2026-09-29T00:00:00Z","weekly_resets_at":"2026-09-29T00:00:00Z"}},"machines":[],"runs":[],"queue":[],"inbox":[],"watch":[],"decisions":[{{"id":"d1","question":"q","options":["a","b"],"context":"c","asked_at":"2026-09-29T00:00:00Z"}}{extra}]}}"#
        );
        crate::feed::parse_snapshot(&json).expect("literal snapshot should parse")
    }

    /// An app on a shared fake pty that has seen `d1` and then `d2`, so the panel is open
    /// unfocused and the card shows `d1`.
    fn app_with_card() -> (App, crate::pty::SharedFakePty) {
        let fake = crate::pty::SharedFakePty::default();
        let mut app = App::default().with_pty(Box::new(fake.clone()));
        app.apply_snapshot(snapshot_with_decisions(false));
        app.apply_snapshot(snapshot_with_decisions(true));
        (app, fake)
    }

    #[test]
    fn backtick_opens_and_backslash_closes_the_panel_on_either_page() {
        for page in [AppPage::Regatta, AppPage::Slipstream] {
            let fake = crate::pty::SharedFakePty::default();
            let mut app = App::new(page, ThemeId::Regatta).with_pty(Box::new(fake.clone()));
            app.apply_snapshot(snapshot_with_decisions(false));
            handle_event_with(&mut app, press(KeyCode::Char('`')), &mut |_| {});
            assert!(app.chair_panel().is_open());
            handle_event_with(&mut app, press(KeyCode::Esc), &mut |_| {});
            assert!(app.chair_panel().is_open(), "a focused panel keeps Esc");
            let ctrl_close = KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL);
            handle_event_with(&mut app, ctrl_close, &mut |_| {});
            handle_event_with(&mut app, press(KeyCode::Char('\\')), &mut |_| {});
            assert!(!app.chair_panel().is_open());
            assert!(fake.0.borrow().calls().contains(&crate::pty::PtyCall::Kill));
        }
    }

    /// An app on a shared fake pty with runs and machines rows and no published chair session.
    fn app_with_rows() -> (App, crate::pty::SharedFakePty) {
        let fake = crate::pty::SharedFakePty::default();
        let mut app = App::default().with_pty(Box::new(fake.clone()));
        app.apply_snapshot(snapshot_with_two_runs());
        (app, fake)
    }

    fn spawn_cox_session() -> crate::pty::PtyCall {
        crate::pty::PtyCall::Spawn {
            argv: ["cox", "session"].map(String::from).to_vec(),
            rows: PANEL_ROWS,
            cols: PANEL_COLS,
        }
    }

    fn kills(fake: &crate::pty::SharedFakePty) -> usize {
        let calls = fake.0.borrow();
        calls
            .calls()
            .iter()
            .filter(|c| **c == crate::pty::PtyCall::Kill)
            .count()
    }

    /// A started session with focus handed back, so the next key reaches `handle_key`.
    fn app_with_started_session() -> (App, crate::pty::SharedFakePty) {
        let (mut app, fake) = app_with_rows();
        handle_event_with(&mut app, press(KeyCode::Char('`')), &mut |_| {});
        handle_event_with(&mut app, press(KeyCode::Enter), &mut |_| {});
        let release = KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL);
        handle_event_with(&mut app, release, &mut |_| {});
        (app, fake)
    }

    #[test]
    fn enter_on_the_shown_panel_with_no_session_spawns_cox_session_and_takes_focus() {
        let (mut app, fake) = app_with_rows();
        handle_event_with(&mut app, press(KeyCode::Char('`')), &mut |_| {});
        assert!(fake.0.borrow().calls().is_empty());
        handle_event_with(&mut app, press(KeyCode::Enter), &mut |_| {});
        assert_eq!(fake.0.borrow().calls(), [spawn_cox_session()]);
        assert!(app.chair_panel().started());
        assert!(app.chair_panel().focused());
    }

    #[test]
    fn enter_on_a_focused_frame_beside_a_shown_panel_opens_the_drill_down_and_spawns_nothing() {
        let release = KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL);
        for give_back in [release, press(KeyCode::Right)] {
            let (mut app, fake) = app_with_rows();
            handle_event_with(&mut app, press(KeyCode::Char('`')), &mut |_| {});
            assert!(app.chair_panel_focused());
            handle_event_with(&mut app, give_back, &mut |_| {});
            if give_back.code == KeyCode::Right {
                handle_event_with(&mut app, press(KeyCode::Left), &mut |_| {});
            }
            handle_event_with(&mut app, press(KeyCode::Down), &mut |_| {});
            handle_event_with(&mut app, press(KeyCode::Enter), &mut |_| {});
            assert!(app.chair_panel_shown());
            assert!(!app.chair_panel_focused());
            let (kind, id, _) = app.detail().expect("detail should be open");
            assert_eq!(*kind, crate::app::DetailKind::Run);
            assert_eq!(id, "r1");
            assert!(fake.0.borrow().calls().is_empty());
        }
    }

    /// `snapshot_with_two_runs` with chair session `s1` published and no decisions.
    fn snapshot_with_two_runs_and_a_session() -> crate::feed::FeedSnapshot {
        let snap = snapshot_with_two_runs();
        crate::feed::FeedSnapshot {
            chair: crate::feed::Chair {
                session: "s1".to_string(),
                ..snap.chair
            },
            ..snap
        }
    }

    #[test]
    fn enter_on_the_focused_panel_with_a_published_session_does_not_spawn() {
        let (mut app, fake) = app_with_rows();
        handle_event_with(&mut app, press(KeyCode::Char('`')), &mut |_| {});
        app.apply_snapshot(snapshot_with_two_runs_and_a_session());
        assert!(app.chair_panel_focused());
        assert!(!app.chair_panel().is_open());
        handle_event_with(&mut app, press(KeyCode::Enter), &mut |_| {});
        assert!(fake.0.borrow().calls().is_empty());
        handle_event_with(&mut app, press(KeyCode::Char('`')), &mut |_| {});
        let attach = crate::pty::PtyCall::Spawn {
            argv: ["claude", "attach", "s1"].map(String::from).to_vec(),
            rows: PANEL_ROWS,
            cols: PANEL_COLS,
        };
        assert_eq!(fake.0.borrow().calls(), [attach]);
    }

    #[test]
    fn backslash_on_a_started_session_asks_and_y_then_kills_once() {
        let (mut app, fake) = app_with_started_session();
        handle_event_with(&mut app, press(KeyCode::Char('\\')), &mut |_| {});
        assert_eq!(kills(&fake), 0);
        assert!(app.chair_panel().confirming_close());
        assert!(app.chair_panel().is_open());
        handle_event_with(&mut app, press(KeyCode::Char('y')), &mut |_| {});
        assert_eq!(kills(&fake), 1);
        assert!(!app.chair_panel().is_open());
        assert!(!app.chair_panel_shown());
    }

    #[test]
    fn n_and_esc_keep_the_started_session_and_other_keys_are_swallowed() {
        let (mut app, fake) = app_with_started_session();
        handle_event_with(&mut app, press(KeyCode::Char('\\')), &mut |_| {});
        handle_event_with(&mut app, press(KeyCode::Char('x')), &mut |_| {});
        handle_event_with(&mut app, press(KeyCode::Tab), &mut |_| {});
        assert!(app.chair_panel().confirming_close());
        assert_eq!(app.page(), AppPage::Regatta);
        assert!(
            !fake
                .0
                .borrow()
                .calls()
                .iter()
                .any(|c| matches!(c, crate::pty::PtyCall::Write(_)))
        );
        handle_event_with(&mut app, press(KeyCode::Char('n')), &mut |_| {});
        assert!(!app.chair_panel().confirming_close());
        handle_event_with(&mut app, press(KeyCode::Char('\\')), &mut |_| {});
        handle_event_with(&mut app, press(KeyCode::Esc), &mut |_| {});
        assert!(!app.chair_panel().confirming_close());
        assert!(app.chair_panel().is_open());
        assert!(app.chair_panel_shown());
        assert_eq!(kills(&fake), 0);
    }

    #[test]
    fn an_attach_process_closes_on_backslash_without_asking() {
        let fake = crate::pty::SharedFakePty::default();
        let mut app = App::default().with_pty(Box::new(fake.clone()));
        app.apply_snapshot(snapshot_with_decisions(false));
        handle_event_with(&mut app, press(KeyCode::Char('`')), &mut |_| {});
        let release = KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL);
        handle_event_with(&mut app, release, &mut |_| {});
        handle_event_with(&mut app, press(KeyCode::Char('\\')), &mut |_| {});
        assert!(!app.chair_panel().confirming_close());
        assert_eq!(kills(&fake), 1);
    }

    #[test]
    fn backtick_focuses_an_open_unfocused_pane_without_closing_it() {
        let (mut app, fake) = app_with_started_session();
        assert!(!app.chair_focused());
        handle_event_with(&mut app, press(KeyCode::Char('`')), &mut |_| {});
        assert!(app.chair_panel().is_open());
        assert!(app.chair_panel_shown());
        assert!(app.chair_focused());
        assert!(!app.chair_panel().confirming_close());
        assert_eq!(kills(&fake), 0);
    }

    #[test]
    fn backtick_opens_a_hidden_pane() {
        let (mut app, fake) = app_with_rows();
        assert!(!app.chair_panel_shown());
        handle_event_with(&mut app, press(KeyCode::Char('`')), &mut |_| {});
        assert!(app.chair_panel_shown());
        assert!(app.chair_focused());
        assert!(fake.0.borrow().calls().is_empty());
    }

    #[test]
    fn backslash_hides_an_unfocused_pane_through_the_confirm_path() {
        let (mut app, fake) = app_with_started_session();
        handle_event_with(&mut app, press(KeyCode::Char('\\')), &mut |_| {});
        assert!(app.chair_panel().confirming_close());
        assert!(app.chair_panel_shown());
        assert_eq!(kills(&fake), 0);
        handle_event_with(&mut app, press(KeyCode::Char('y')), &mut |_| {});
        assert!(!app.chair_panel_shown());
        assert_eq!(kills(&fake), 1);
    }

    #[test]
    fn tilde_cycles_the_panel_width_through_all_three_modes() {
        use crate::chair_panel::WidthMode;
        let mut app = App::default();
        let widths: Vec<WidthMode> = (0..3)
            .map(|_| {
                handle_key(&mut app, KeyCode::Char('~'));
                app.chair_panel().width()
            })
            .collect();
        assert_eq!(
            widths,
            [WidthMode::Wide, WidthMode::Full, WidthMode::Narrow]
        );
    }

    #[test]
    fn ctrl_right_bracket_returns_focus_and_other_keys_go_to_the_pty() {
        let (mut app, fake) = app_with_card();
        app.hide_card();
        app.chair_panel_mut().set_focus(true);
        handle_event_with(&mut app, press(KeyCode::Char('q')), &mut |_| {});
        assert!(app.chair_panel().focused());
        assert!(
            fake.0
                .borrow()
                .calls()
                .contains(&crate::pty::PtyCall::Write(b"q".to_vec()))
        );
        let release = KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL);
        handle_event_with(&mut app, release, &mut |_| {});
        assert!(!app.chair_panel().focused());
        assert!(app.chair_panel().is_open());
    }

    #[test]
    fn y_runs_the_built_argv_through_the_seam_and_hides_the_card() {
        let (mut app, _fake) = app_with_card();
        let mut ran: Vec<Vec<String>> = Vec::new();
        let mut record = |argv: &[String]| ran.push(argv.to_vec());
        handle_event_with(&mut app, press(KeyCode::Char('y')), &mut record);
        handle_event_with(&mut app, press(KeyCode::Char('2')), &mut record);
        handle_event_with(&mut app, press(KeyCode::Char('y')), &mut record);
        assert_eq!(
            ran,
            [["cox", "chair", "answer", "d1", "b"].map(String::from)]
        );
        assert!(!app.card_visible());
    }

    #[test]
    fn esc_closes_the_card_without_answering_and_tab_focuses_the_panel() {
        let (mut app, _fake) = app_with_card();
        let mut ran = 0;
        let mut count = |_: &[String]| ran += 1;
        handle_event_with(&mut app, press(KeyCode::Tab), &mut count);
        assert!(app.chair_panel().focused());
        assert_eq!(app.page(), AppPage::Regatta);
        let release = KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL);
        handle_event_with(&mut app, release, &mut count);
        handle_event_with(&mut app, press(KeyCode::Esc), &mut count);
        assert!(!app.card_visible());
        assert_eq!(ran, 0);
    }

    #[test]
    fn a_digit_past_the_last_option_does_not_toggle_a_frame_behind_the_card() {
        let (mut app, _fake) = app_with_card();
        let before = app.regatta_frames_visible();
        handle_event_with(&mut app, press(KeyCode::Char('5')), &mut |_| {});
        assert_eq!(app.regatta_frames_visible(), before);
    }

    #[test]
    fn char_v_advances_the_regatta_layout_preset() {
        let mut app = App::default();
        handle_key(&mut app, KeyCode::Char('v'));
        assert_eq!(app.regatta_layout_preset(), 1);
    }

    #[test]
    fn tab_advances_to_the_next_page() {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        handle_key(&mut app, KeyCode::Tab);
        assert_eq!(app.page(), AppPage::Slipstream);
    }

    #[test]
    fn back_tab_goes_to_the_previous_page() {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        handle_key(&mut app, KeyCode::BackTab);
        assert_eq!(app.page(), AppPage::Slipstream);
    }

    #[test]
    fn char_t_toggles_the_theme() {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        handle_key(&mut app, KeyCode::Char('t'));
        assert_eq!(app.theme(), ThemeId::HarborLight);
    }

    #[test]
    fn an_ascii_digit_toggles_the_matching_regatta_frame() {
        let mut app = App::default();
        handle_key(&mut app, KeyCode::Char('3'));
        assert_eq!(
            app.regatta_frames_visible(),
            [true, true, false, true, true, true]
        );
    }

    #[test]
    fn esc_is_a_no_op_when_no_detail_is_open() {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        handle_key(&mut app, KeyCode::Esc);
        assert_eq!(app.page(), AppPage::Regatta);
        assert_eq!(app.theme(), ThemeId::Regatta);
        assert_eq!(app.regatta_frames_visible(), [true; 6]);
        assert!(app.detail().is_none());
    }

    #[test]
    fn down_selects_the_next_row_and_j_does_not() {
        let mut app = App::default();
        app.apply_snapshot(snapshot_with_two_runs());
        handle_key(&mut app, KeyCode::Down);
        assert_eq!(app.selected(), 1);
        handle_key(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected(), 1);
    }

    #[test]
    fn up_selects_the_previous_row() {
        let mut app = App::default();
        app.apply_snapshot(snapshot_with_two_runs());
        handle_key(&mut app, KeyCode::Up);
        assert_eq!(app.selected(), 1);
        handle_key(&mut app, KeyCode::Up);
        assert_eq!(app.selected(), 0);
    }

    fn app_on_runs() -> App {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.apply_snapshot(snapshot_with_two_runs());
        app
    }

    fn confirm_command(app: &App) -> Option<&str> {
        match app.modal() {
            Some(crate::app::Modal::Confirm(confirm)) => Some(confirm.command.as_str()),
            _ => None,
        }
    }

    fn stop_argv() -> Vec<String> {
        ["cox", "runs", "stop", "r0"].map(String::from).to_vec()
    }

    const SETTINGS_ROWS: &str = r#"{"sections":[{"id":"budgets","rows":[{"section":"budgets","scope":"budgets","key":"max_usd","value":"20","file":"f.toml","tracked":true,"pat_only":false}]}]}"#;

    fn words(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|p| p.to_string()).collect()
    }

    /// An app with the settings screen open, its rows loaded and no command pending.
    fn app_with_settings() -> App {
        let mut app = App::default();
        handle_key(&mut app, KeyCode::Char('s'));
        app.take_pending();
        app.apply_exec_result(
            crate::exec::ExecResult {
                argv: words(&["cox", "settings", "get", "--json"]),
                code: Some(0),
                output: SETTINGS_ROWS.to_string(),
            },
            crate::app::Origin::SettingsLoad,
        );
        app
    }

    #[test]
    fn s_opens_the_settings_screen_and_leaves_the_get_pending() {
        let mut app = App::default();
        handle_key(&mut app, KeyCode::Char('s'));
        assert!(app.settings().is_some());
        let pending = app.take_pending().expect("a pending command");
        assert_eq!(pending.argv, words(&["cox", "settings", "get", "--json"]));
    }

    #[test]
    fn tab_with_the_screen_open_moves_its_pane_and_not_the_page() {
        let mut app = app_with_settings();
        let page = app.page();
        let pane = app.settings().map(|s| s.pane());
        handle_key(&mut app, KeyCode::Tab);
        assert_eq!(app.page(), page);
        assert_ne!(app.settings().map(|s| s.pane()), pane);
    }

    #[test]
    fn y_with_a_confirm_over_the_screen_goes_to_the_modal_and_queues_the_apply() {
        let mut app = app_with_settings();
        for code in [
            KeyCode::Tab,
            KeyCode::Enter,
            KeyCode::Backspace,
            KeyCode::Backspace,
            KeyCode::Char('4'),
            KeyCode::Char('0'),
            KeyCode::Enter,
            KeyCode::Tab,
            KeyCode::Char('a'),
        ] {
            handle_key(&mut app, code);
        }
        app.take_pending();
        assert!(matches!(app.modal(), Some(crate::app::Modal::Confirm(_))));
        handle_key(&mut app, KeyCode::Char('y'));
        assert!(app.modal().is_none());
        let pending = app.take_pending().expect("a pending command");
        assert_eq!(
            pending.argv,
            words(&["cox", "settings", "set", "budgets", "max_usd", "40"])
        );
    }

    #[test]
    fn esc_closes_the_settings_screen() {
        let mut app = app_with_settings();
        handle_key(&mut app, KeyCode::Esc);
        assert!(app.settings().is_none());
    }

    #[test]
    fn t_with_the_screen_closed_still_toggles_the_theme() {
        let mut app = App::default();
        let theme = app.theme();
        handle_key(&mut app, KeyCode::Char('t'));
        assert_ne!(app.theme(), theme);
        assert!(app.settings().is_none());
    }

    #[test]
    fn colon_opens_the_palette() {
        let mut app = App::default();
        handle_key(&mut app, KeyCode::Char(':'));
        assert!(matches!(app.modal(), Some(crate::app::Modal::Palette(_))));
    }

    #[test]
    fn k_on_a_run_opens_the_stop_confirm() {
        let mut app = app_on_runs();
        handle_key(&mut app, KeyCode::Char('k'));
        assert_eq!(confirm_command(&app), Some("cox runs stop r0"));
        assert_eq!(app.selected(), 0);
    }

    #[test]
    fn k_with_a_modal_open_goes_to_the_modal() {
        let mut app = app_on_runs();
        app.open_palette();
        handle_key(&mut app, KeyCode::Char('k'));
        assert!(matches!(app.modal(), Some(crate::app::Modal::Palette(_))));
        assert_eq!(app.selected(), 0);
        let mut app = app_on_runs();
        handle_key(&mut app, KeyCode::Char('k'));
        handle_key(&mut app, KeyCode::Char('k'));
        assert_eq!(confirm_command(&app), Some("cox runs stop r0"));
        assert!(app.take_pending().is_none());
    }

    #[test]
    fn p_on_a_run_opens_pause_and_v_cycles_the_layout() {
        let mut app = app_on_runs();
        handle_key(&mut app, KeyCode::Char('p'));
        assert_eq!(confirm_command(&app), Some("cox runs pause r0"));
        assert_eq!(app.regatta_layout_preset(), 0);

        let mut app = app_on_runs();
        handle_key(&mut app, KeyCode::Right);
        handle_key(&mut app, KeyCode::Right);
        assert_eq!(app.focus(), Focus::Machines);
        handle_key(&mut app, KeyCode::Char('p'));
        assert!(app.modal().is_none());
        assert_eq!(app.regatta_layout_preset(), 0);
        handle_key(&mut app, KeyCode::Char('v'));
        assert!(app.modal().is_none());
        assert_eq!(app.regatta_layout_preset(), 1);
    }

    #[test]
    fn y_confirms_and_queues_the_stop_argv() {
        let mut app = app_on_runs();
        handle_key(&mut app, KeyCode::Char('k'));
        handle_key(&mut app, KeyCode::Char('y'));
        assert!(app.modal().is_none());
        let pending = app.take_pending().expect("a pending command");
        assert_eq!(pending.argv, stop_argv());
    }

    #[test]
    fn y_with_a_decision_card_showing_goes_to_the_confirm_not_the_card() {
        let (mut app, _fake) = app_with_card();
        assert!(app.card_visible());
        app.open_palette();
        let mut ran = 0;
        handle_event_with(&mut app, press(KeyCode::Char('y')), &mut |_| ran += 1);
        assert_eq!(ran, 0);
        assert!(app.card_visible());
    }

    #[test]
    fn n_leaves_nothing_pending() {
        let mut app = app_on_runs();
        handle_key(&mut app, KeyCode::Char('k'));
        handle_key(&mut app, KeyCode::Char('n'));
        assert!(app.modal().is_none());
        assert!(app.take_pending().is_none());
    }

    #[test]
    fn esc_closes_the_palette() {
        let mut app = App::default();
        handle_key(&mut app, KeyCode::Char(':'));
        handle_key(&mut app, KeyCode::Esc);
        assert!(app.modal().is_none());
    }

    #[test]
    fn right_cycles_focus_to_the_next_frame() {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        handle_key(&mut app, KeyCode::Right);
        assert_eq!(app.focus(), Focus::Queue);
    }

    #[test]
    fn left_cycles_focus_to_the_previous_frame() {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        handle_key(&mut app, KeyCode::Left);
        assert_eq!(app.focus(), Focus::Inbox);
    }

    /// A snapshot with two ended runs (`h0`, `h1`) in its history, or none when `rows` is false.
    fn snapshot_with_history(rows: bool) -> crate::feed::FeedSnapshot {
        let history = if rows {
            r#","history":[{"run":"h0","machine":"m0","initiative":"i0","ended_at":"2026-09-29T00:00:00Z","outcome":"landed","cost_usd":1.0},{"run":"h1","machine":"m0","initiative":"i0","ended_at":"2026-09-29T00:00:00Z","outcome":"stopped","cost_usd":2.0}]"#
        } else {
            ""
        };
        let json = format!(
            r#"{{"schema":1,"at":"2026-09-29T00:00:00Z","chair":{{"holder":"h","host":"h","epoch":1,"liveness":"live","beat_age_s":0}},"spend":{{"five_hour_fraction":0.0,"five_hour_source":"meter","weekly_fraction":0.0,"weekly_source":"meter","hard_stop_fraction":0.0,"five_hour_resets_at":"2026-09-29T00:00:00Z","weekly_resets_at":"2026-09-29T00:00:00Z"}},"machines":[],"runs":[],"queue":[],"inbox":[],"watch":[]{history}}}"#
        );
        crate::feed::parse_snapshot(&json).expect("literal snapshot should parse")
    }

    fn app_focused_on_history(rows: bool) -> App {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.apply_snapshot(snapshot_with_history(rows));
        handle_key(&mut app, KeyCode::Left);
        handle_key(&mut app, KeyCode::Left);
        assert_eq!(app.focus(), Focus::History);
        app
    }

    #[test]
    fn char_eight_toggles_the_run_cost_frame_and_nothing_else() {
        let mut app = App::default();
        handle_key(&mut app, KeyCode::Char('8'));
        assert!(!app.regatta_run_cost_visible());
        assert!(app.regatta_history_visible());
        assert_eq!(app.regatta_frames_visible(), [true; 6]);
        handle_key(&mut app, KeyCode::Char('8'));
        assert!(app.regatta_run_cost_visible());
    }

    #[test]
    fn char_nine_toggles_the_inbox_frame_and_nothing_else() {
        let mut app = App::default();
        handle_key(&mut app, KeyCode::Char('9'));
        assert!(!app.regatta_inbox_visible());
        assert!(app.regatta_history_visible());
        assert!(app.regatta_run_cost_visible());
        assert_eq!(app.regatta_frames_visible(), [true; 6]);
        handle_key(&mut app, KeyCode::Char('9'));
        assert!(app.regatta_inbox_visible());
    }

    #[test]
    fn a_hidden_inbox_is_not_focusable_through_the_arrow_keys() {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        handle_key(&mut app, KeyCode::Char('9'));
        handle_key(&mut app, KeyCode::Left);
        assert_eq!(app.focus(), Focus::History);
    }

    #[test]
    fn char_seven_toggles_the_history_frame_and_no_numbered_frame() {
        let mut app = App::default();
        handle_key(&mut app, KeyCode::Char('7'));
        assert!(!app.regatta_history_visible());
        assert_eq!(app.regatta_frames_visible(), [true; 6]);
        handle_key(&mut app, KeyCode::Char('7'));
        assert!(app.regatta_history_visible());
    }

    #[test]
    fn up_and_down_clamp_the_history_selection_at_both_ends() {
        let mut app = app_focused_on_history(true);
        handle_key(&mut app, KeyCode::Up);
        assert_eq!(app.selected(), 0);
        handle_key(&mut app, KeyCode::Down);
        handle_key(&mut app, KeyCode::Down);
        assert_eq!(app.selected(), 1);
    }

    #[test]
    fn enter_on_a_history_row_opens_that_rows_run() {
        let mut app = app_focused_on_history(true);
        handle_key(&mut app, KeyCode::Down);
        handle_key(&mut app, KeyCode::Enter);
        let (kind, id, _) = app.detail().expect("detail should be open");
        assert_eq!(*kind, crate::app::DetailKind::Run);
        assert_eq!(id, "h1");
    }

    #[test]
    fn enter_on_empty_history_changes_nothing() {
        let mut app = app_focused_on_history(false);
        handle_key(&mut app, KeyCode::Enter);
        assert!(app.detail().is_none());
        assert_eq!(app.selected(), 0);
        assert_eq!(app.focus(), Focus::History);
    }

    #[test]
    fn enter_opens_the_detail_for_the_selected_row() {
        let mut app = App::default();
        app.apply_snapshot(snapshot_with_two_runs());
        handle_key(&mut app, KeyCode::Enter);
        assert!(app.detail().is_some());
    }

    #[test]
    fn esc_closes_an_open_detail() {
        let mut app = App::default();
        app.apply_snapshot(snapshot_with_two_runs());
        handle_key(&mut app, KeyCode::Enter);
        assert!(app.detail().is_some());
        handle_key(&mut app, KeyCode::Esc);
        assert!(app.detail().is_none());
    }

    #[test]
    fn right_then_tab_then_enter_opens_a_run_detail_on_slipstream() {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.apply_snapshot(snapshot_with_two_runs());
        handle_key(&mut app, KeyCode::Right);
        handle_key(&mut app, KeyCode::Right);
        handle_key(&mut app, KeyCode::Tab);
        assert_eq!(app.focus(), Focus::Runs);
        handle_key(&mut app, KeyCode::Enter);
        let (kind, id, _) = app.detail().expect("detail should be open");
        assert_eq!(*kind, crate::app::DetailKind::Run);
        assert_eq!(id, "r0");
    }

    fn app_on_machines() -> App {
        let mut app = app_on_runs();
        handle_key(&mut app, KeyCode::Right);
        handle_key(&mut app, KeyCode::Right);
        assert_eq!(app.focus(), Focus::Machines);
        app
    }

    fn type_keys(app: &mut App, text: &str) {
        text.chars().for_each(|c| handle_key(app, KeyCode::Char(c)));
    }

    #[test]
    fn the_add_machine_key_on_the_machines_frame_opens_the_form() {
        let mut app = app_on_machines();
        handle_key(&mut app, KeyCode::Char(KEY_ADD_MACHINE));
        assert_eq!(app.form().map(|f| f.title.as_str()), Some("Add machine"));
    }

    #[test]
    fn the_add_machine_key_on_another_frame_opens_nothing() {
        let mut app = app_on_runs();
        assert_eq!(app.focus(), Focus::Runs);
        handle_key(&mut app, KeyCode::Char(KEY_ADD_MACHINE));
        assert!(app.form().is_none());
        assert!(app.modal().is_none());
    }

    #[test]
    fn a_printable_key_with_a_form_open_lands_in_the_focused_field() {
        let mut app = app_on_machines();
        app.open_add_machine();
        type_keys(&mut app, "kt");
        assert_eq!(app.form().map(|f| f.value_of("name")), Some("kt"));
        assert!(app.modal().is_none());
        assert_eq!(app.theme(), ThemeId::Regatta);
    }

    #[test]
    fn y_with_a_confirm_over_a_form_reaches_the_confirm_not_the_form() {
        let mut app = app_on_machines();
        app.open_add_machine();
        type_keys(&mut app, "edge-1");
        handle_key(&mut app, KeyCode::Enter);
        type_keys(&mut app, "pat@edge-1");
        [KeyCode::Enter, KeyCode::Enter, KeyCode::Enter]
            .into_iter()
            .for_each(|code| handle_key(&mut app, code));
        assert_eq!(
            confirm_command(&app),
            Some("cox host add edge-1 --ssh pat@edge-1 --capacity 1")
        );
        handle_key(&mut app, KeyCode::Char('y'));
        let argv = app.take_pending().map(|p| p.argv);
        assert_eq!(
            argv.as_ref().and_then(|a| a.get(1..3)),
            Some(&["host".to_string(), "add".to_string()][..])
        );
        assert_eq!(app.form().map(|f| f.value_of("capabilities")), Some(""));
    }

    #[test]
    fn esc_with_a_form_open_and_no_modal_closes_the_form() {
        let mut app = app_on_machines();
        app.open_add_machine();
        handle_key(&mut app, KeyCode::Esc);
        assert!(app.form().is_none());
    }

    fn view_kind(app: &App) -> Option<crate::app::DetailKind> {
        app.detail().map(|d| d.0)
    }

    #[test]
    fn w_dollar_and_h_open_their_view_and_esc_closes_it_on_both_pages() {
        use crate::app::DetailKind::{Health, Spend, Watch};
        for page in [AppPage::Regatta, AppPage::Slipstream] {
            for (key, kind) in [('w', Watch), ('$', Spend), ('h', Health)] {
                let mut app = App::new(page, ThemeId::Regatta);
                handle_key(&mut app, KeyCode::Char(key));
                assert_eq!(view_kind(&app), Some(kind), "{key} on {page:?}");
                handle_key(&mut app, KeyCode::Esc);
                assert!(app.detail().is_none(), "Esc after {key} on {page:?}");
            }
        }
    }

    #[test]
    fn a_second_view_key_while_a_view_is_open_is_ignored() {
        let mut app = App::new(AppPage::Slipstream, ThemeId::Regatta);
        handle_key(&mut app, KeyCode::Char('$'));
        handle_key(&mut app, KeyCode::Char('h'));
        handle_key(&mut app, KeyCode::Char('w'));
        assert_eq!(view_kind(&app), Some(crate::app::DetailKind::Spend));
    }

    #[test]
    fn a_view_key_with_a_run_detail_open_leaves_the_run_open() {
        let mut app = app_on_runs();
        handle_key(&mut app, KeyCode::Enter);
        handle_key(&mut app, KeyCode::Char('h'));
        assert_eq!(view_kind(&app), Some(crate::app::DetailKind::Run));
    }

    #[test]
    fn a_form_keeps_w_dollar_and_h_for_its_field() {
        let mut app = app_on_machines();
        app.open_add_machine();
        type_keys(&mut app, "w$h");
        assert!(app.detail().is_none());
        assert_eq!(app.form().map(|f| f.value_of("name")), Some("w$h"));
    }

    #[test]
    fn a_modal_keeps_w_dollar_and_h() {
        let mut app = app_on_runs();
        handle_key(&mut app, KeyCode::Char(':'));
        type_keys(&mut app, "w$h");
        assert!(app.detail().is_none());
        assert!(app.modal().is_some());
    }

    #[test]
    fn the_settings_screen_keeps_w_dollar_and_h() {
        let mut app = app_on_runs();
        handle_key(&mut app, KeyCode::Char('s'));
        assert!(app.settings().is_some());
        type_keys(&mut app, "w$h");
        assert!(app.detail().is_none());
    }

    #[test]
    fn no_action_binding_uses_w_dollar_or_h() {
        use crate::actions::{Target, bindings};
        let targets = [
            Target::Run("r".into()),
            Target::Initiative("i".into()),
            Target::Machine("m".into()),
            Target::InboxItem("x".into()),
        ];
        for target in &targets {
            for binding in bindings(target) {
                assert!(
                    !matches!(binding.key, 'w' | '$' | 'h'),
                    "{:?} binds {}",
                    target,
                    binding.key
                );
            }
        }
    }

    #[test]
    fn a_on_a_selected_machine_is_still_activate_and_opens_the_activate_confirm() {
        let mut app = app_on_machines();
        handle_key(&mut app, KeyCode::Char('a'));
        assert_eq!(confirm_command(&app), Some("cox host activate m0"));
        assert!(app.form().is_none());
    }

    /// Two queue rows (`q0`, `q1`), one machine and one inbox item.
    fn snapshot_with_queue() -> crate::feed::FeedSnapshot {
        let json = r#"{"schema":1,"at":"2026-09-29T00:00:00Z","chair":{"holder":"h","host":"h","epoch":1,"liveness":"live","beat_age_s":0},"spend":{"five_hour_fraction":0.0,"five_hour_source":"meter","weekly_fraction":0.0,"weekly_source":"meter","hard_stop_fraction":0.0,"five_hour_resets_at":"2026-09-29T00:00:00Z","weekly_resets_at":"2026-09-29T00:00:00Z"},"machines":[{"name":"m0","state":"active","lanes_in_use":0,"capacity":3,"login_ok":true,"login_checked_at":"2026-09-29T00:00:00Z","beat_age_s":0,"checkouts":{}}],"runs":[],"queue":[{"initiative":"q0","priority":1,"phases_landed":0,"phases_total":1,"current_phase":"p1"},{"initiative":"q1","priority":2,"phases_landed":0,"phases_total":1,"current_phase":"p1"}],"inbox":[{"kind":"needs_chair","target":"t-1","reason":"r"}],"watch":[]}"#;
        crate::feed::parse_snapshot(json).expect("literal snapshot should parse")
    }

    fn app_with_focus(rights: usize) -> App {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.apply_snapshot(snapshot_with_queue());
        (0..rights).for_each(|_| handle_key(&mut app, KeyCode::Right));
        app
    }

    fn app_on_queue() -> App {
        let app = app_with_focus(1);
        assert_eq!(app.focus(), Focus::Queue);
        app
    }

    fn nothing_opened(app: &mut App) -> bool {
        app.form().is_none() && app.modal().is_none() && app.take_detail_request().is_none()
    }

    #[test]
    fn the_new_key_on_the_queue_frame_opens_the_new_form() {
        let mut app = app_on_queue();
        handle_key(&mut app, KeyCode::Char(KEY_NEW));
        assert_eq!(app.form().map(|f| f.title.as_str()), Some("New initiative"));
    }

    #[test]
    fn the_edit_key_on_a_selected_queue_row_asks_for_that_initiatives_detail() {
        let mut app = app_on_queue();
        app.select_next();
        handle_key(&mut app, KeyCode::Char(KEY_EDIT));
        assert_eq!(app.take_detail_request(), Some("q1".to_string()));
    }

    #[test]
    fn the_edit_key_with_the_drill_down_open_targets_the_drill_down_not_the_selection() {
        let mut app = app_on_queue();
        app.open_detail();
        app.select_next();
        handle_key(&mut app, KeyCode::Char(KEY_EDIT));
        assert_eq!(app.take_detail_request(), Some("q0".to_string()));
    }

    #[test]
    fn the_remove_key_on_a_queue_row_opens_the_remove_form() {
        let mut app = app_on_queue();
        handle_key(&mut app, KeyCode::Char(KEY_REMOVE));
        assert_eq!(
            app.form().map(|f| f.title.as_str()),
            Some("Remove initiative q0")
        );
    }

    #[test]
    fn the_remove_key_on_an_inbox_item_still_opens_the_deny_confirm() {
        let mut app = app_with_focus(4);
        assert_eq!(app.focus(), Focus::Inbox);
        handle_key(&mut app, KeyCode::Char(KEY_REMOVE));
        assert_eq!(confirm_command(&app), Some("cox inbox deny t-1"));
        assert!(app.form().is_none());
    }

    #[test]
    fn the_new_edit_and_remove_keys_on_the_machines_list_do_nothing_new() {
        let mut app = app_with_focus(2);
        assert_eq!(app.focus(), Focus::Machines);
        [KEY_NEW, KEY_EDIT, KEY_REMOVE]
            .into_iter()
            .for_each(|c| handle_key(&mut app, KeyCode::Char(c)));
        assert!(nothing_opened(&mut app));
    }
}
