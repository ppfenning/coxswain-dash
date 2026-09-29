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
    use crate::app::{AppPage, ThemeId};
    use crossterm::event::KeyModifiers;

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
    fn esc_is_a_no_op() {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        handle_key(&mut app, KeyCode::Esc);
        assert_eq!(app.page(), AppPage::Regatta);
        assert_eq!(app.theme(), ThemeId::Regatta);
        assert_eq!(app.regatta_frames_visible(), [true; 6]);
    }
}
