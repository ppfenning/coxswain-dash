//! Pure keyboard dispatch: turns one crossterm `KeyCode` into the single `App` mutation it
//! maps to. No I/O and no rendering; `src/main.rs`'s event loop is the only caller.

use crossterm::event::{KeyCode, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::app::App;

pub fn handle_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Tab => app.next_page(),
        KeyCode::BackTab => app.prev_page(),
        KeyCode::Char('t') => app.toggle_theme(),
        KeyCode::Char('p') => app.cycle_regatta_layout_preset(),
        KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
            app.toggle_regatta_frame(c.to_digit(10).unwrap() as usize)
        }
        KeyCode::Right => app.cycle_focus_next(),
        KeyCode::Left => app.cycle_focus_prev(),
        KeyCode::Down | KeyCode::Char('j') => app.select_next(),
        KeyCode::Up | KeyCode::Char('k') => app.select_prev(),
        KeyCode::Enter => app.open_detail(),
        KeyCode::Esc => app.close_detail(),
        _ => {}
    }
}

fn contains(rect: Rect, x: u16, y: u16) -> bool {
    x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height
}

/// A left-button press inside `rects[i]` toggles regatta frame `i + 1`; a press outside every
/// rect is a no-op. `rects` is `regatta::frame_rects`'s output, so this never disagrees with
/// what is actually drawn.
pub fn handle_mouse(app: &mut App, event: MouseEvent, rects: &[Option<Rect>; 6]) {
    if event.kind != MouseEventKind::Down(MouseButton::Left) {
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
    use crossterm::event::KeyModifiers;

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
    fn char_p_advances_the_regatta_layout_preset() {
        let mut app = App::default();
        handle_key(&mut app, KeyCode::Char('p'));
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
    fn down_and_char_j_both_select_the_next_row() {
        let mut app = App::default();
        app.apply_snapshot(snapshot_with_two_runs());
        handle_key(&mut app, KeyCode::Down);
        assert_eq!(app.selected(), 1);
        handle_key(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected(), 0);
    }

    #[test]
    fn up_and_char_k_both_select_the_previous_row() {
        let mut app = App::default();
        app.apply_snapshot(snapshot_with_two_runs());
        handle_key(&mut app, KeyCode::Up);
        assert_eq!(app.selected(), 1);
        handle_key(&mut app, KeyCode::Char('k'));
        assert_eq!(app.selected(), 0);
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
        assert_eq!(app.focus(), Focus::Machines);
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
}
