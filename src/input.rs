//! Pure keyboard dispatch: turns one crossterm `KeyCode` into the single `App` mutation it
//! maps to. No I/O and no rendering; `src/main.rs`'s event loop is the only caller.

use crossterm::event::KeyCode;

use crate::app::App;

pub fn handle_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Tab => app.next_page(),
        KeyCode::BackTab => app.prev_page(),
        KeyCode::Char('t') => app.toggle_theme(),
        KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
            app.toggle_regatta_frame(c.to_digit(10).unwrap() as usize)
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppPage, ThemeId};

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
