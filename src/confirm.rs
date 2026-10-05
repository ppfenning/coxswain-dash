//! Pure state of the confirm dialog and its key answer. It takes plain strings, so it has no
//! dependency on `src/actions.rs`. The dialog defaults to no.

use crossterm::event::{KeyCode, KeyEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmAnswer {
    Yes,
    No,
    Pending,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmState {
    pub title: String,
    pub command: String,
    pub argv: Vec<String>,
}

impl ConfirmState {
    pub fn new(title: String, argv: Vec<String>, command: String) -> Self {
        Self {
            title,
            command,
            argv,
        }
    }

    pub fn handle_key(&self, key: KeyEvent) -> ConfirmAnswer {
        match key.code {
            KeyCode::Char('y' | 'Y') => ConfirmAnswer::Yes,
            KeyCode::Char('n' | 'N') | KeyCode::Esc => ConfirmAnswer::No,
            // Enter stays pending so a held Enter from the previous screen never confirms.
            _ => ConfirmAnswer::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn state() -> ConfirmState {
        ConfirmState::new(
            "Land".to_string(),
            vec!["cox".to_string(), "land".to_string(), "t-1".to_string()],
            "cox land t-1".to_string(),
        )
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn y_gives_yes() {
        assert_eq!(
            state().handle_key(key(KeyCode::Char('y'))),
            ConfirmAnswer::Yes
        );
    }

    #[test]
    fn uppercase_y_gives_yes() {
        assert_eq!(
            state().handle_key(key(KeyCode::Char('Y'))),
            ConfirmAnswer::Yes
        );
    }

    #[test]
    fn n_gives_no() {
        assert_eq!(
            state().handle_key(key(KeyCode::Char('n'))),
            ConfirmAnswer::No
        );
    }

    #[test]
    fn esc_gives_no() {
        assert_eq!(state().handle_key(key(KeyCode::Esc)), ConfirmAnswer::No);
    }

    #[test]
    fn enter_gives_pending() {
        assert_eq!(
            state().handle_key(key(KeyCode::Enter)),
            ConfirmAnswer::Pending
        );
    }

    #[test]
    fn an_unrelated_key_gives_pending() {
        assert_eq!(
            state().handle_key(key(KeyCode::Char('x'))),
            ConfirmAnswer::Pending
        );
    }

    #[test]
    fn the_stored_argv_is_returned_unchanged() {
        assert_eq!(
            state().argv,
            vec!["cox".to_string(), "land".to_string(), "t-1".to_string()]
        );
    }
}
