//! The pure form state every coxtop form shares. A kind's constructor sets `build`, so the `App`
//! never matches on a form kind. Nothing here draws, runs a command or reads the clock.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

// 'a' is activate on a machine target, so the add key is uppercase.
pub const KEY_ADD_MACHINE: char = 'A';
// 'n' is bound on no target and no global key.
pub const KEY_NEW: char = 'n';
// 'e' is bound on no target and no global key.
pub const KEY_EDIT: char = 'e';
// 'x' is deny on an inbox item only, and remove acts on machines and initiatives.
pub const KEY_REMOVE: char = 'x';

/// Argv lists run in order, each starting with `cox`. They are never quoted.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub confirm_title: String,
    pub steps: Vec<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FieldKind {
    Text,
    /// Typed as free text. Left and Right cycle these options into the value.
    Choice(Vec<String>),
    /// Never typed into. Enter asks for the editor, and `text` holds what it returned.
    Body,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub label: String,
    pub kind: FieldKind,
    pub value: String,
    pub initial: String,
    pub text: String,
    pub path: Option<PathBuf>,
}

impl Field {
    fn new(label: &str, kind: FieldKind, initial: &str, text: &str) -> Self {
        Self {
            label: label.to_string(),
            kind,
            value: initial.to_string(),
            initial: initial.to_string(),
            text: text.to_string(),
            path: None,
        }
    }

    pub fn text(label: &str, initial: &str) -> Self {
        Self::new(label, FieldKind::Text, initial, "")
    }

    pub fn choice(label: &str, options: &[&str], initial: &str) -> Self {
        let options = options.iter().map(|o| o.to_string()).collect();
        Self::new(label, FieldKind::Choice(options), initial, "")
    }

    /// A body field's `initial` is the text it started with.
    pub fn body(label: &str, initial: &str) -> Self {
        Self::new(label, FieldKind::Body, initial, initial)
    }

    pub fn changed(&self) -> bool {
        match self.kind {
            FieldKind::Body => self.text != self.initial,
            _ => self.value != self.initial,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormAnswer {
    Editing,
    Submit,
    Cancel,
    OpenEditor(usize),
}

#[derive(Debug, Clone)]
pub struct Form {
    pub title: String,
    pub fields: Vec<Field>,
    pub focus: usize,
    pub error: Option<String>,
    pub build: fn(&Form) -> Result<Plan, String>,
}

/// The option after (step 1) or before (step -1) `current`, wrapping. A value that is not an
/// option starts from the first option going forward and the last going back.
fn cycle(options: &[String], current: &str, step: isize) -> Option<String> {
    let len = options.len() as isize;
    let next = match options.iter().position(|o| o == current) {
        Some(at) => (at as isize + step).rem_euclid(len),
        None if step > 0 => 0,
        None => len - 1,
    };
    options.get(next as usize).cloned()
}

impl Form {
    pub fn new(title: &str, fields: Vec<Field>, build: fn(&Form) -> Result<Plan, String>) -> Self {
        Self {
            title: title.to_string(),
            fields,
            focus: 0,
            error: None,
            build,
        }
    }

    /// The trimmed value of the field labelled `label`, or empty when there is none.
    pub fn value_of(&self, label: &str) -> &str {
        self.fields
            .iter()
            .find(|f| f.label == label)
            .map_or("", |f| f.value.trim())
    }

    fn move_focus(&mut self, step: isize) {
        let len = self.fields.len() as isize;
        if len > 0 {
            self.focus = (self.focus as isize + step).rem_euclid(len) as usize;
        }
    }

    fn edit(&mut self, change: impl FnOnce(&mut Field)) {
        if let Some(field) = self.fields.get_mut(self.focus)
            && field.kind != FieldKind::Body
        {
            change(field);
            self.error = None;
        }
    }

    fn cycle_focused(&mut self, step: isize) {
        let next = match self.fields.get(self.focus) {
            Some(Field {
                kind: FieldKind::Choice(options),
                value,
                ..
            }) => cycle(options, value, step),
            _ => None,
        };
        if let Some(next) = next {
            self.edit(|f| f.value = next);
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> FormAnswer {
        let on_body = matches!(
            self.fields.get(self.focus).map(|f| &f.kind),
            Some(FieldKind::Body)
        );
        let on_last = self.focus + 1 >= self.fields.len();
        match key.code {
            KeyCode::Esc => return FormAnswer::Cancel,
            KeyCode::Tab => self.move_focus(1),
            KeyCode::BackTab => self.move_focus(-1),
            KeyCode::Enter if on_body => return FormAnswer::OpenEditor(self.focus),
            KeyCode::Enter if on_last => return FormAnswer::Submit,
            KeyCode::Enter => self.move_focus(1),
            KeyCode::Backspace => self.edit(|f| {
                f.value.pop();
            }),
            KeyCode::Left => self.cycle_focused(-1),
            KeyCode::Right => self.cycle_focused(1),
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.edit(|f| f.value.push(c));
            }
            _ => {}
        }
        FormAnswer::Editing
    }

    /// Stores what the editor returned. A non-body or out-of-range index is ignored.
    pub fn set_body(&mut self, idx: usize, path: Option<PathBuf>, text: String) {
        if let Some(field) = self.fields.get_mut(idx)
            && field.kind == FieldKind::Body
        {
            field.path = path;
            field.text = text;
            self.error = None;
        }
    }

    pub fn submit(&mut self) -> Option<Plan> {
        match (self.build)(self) {
            Ok(plan) => {
                self.error = None;
                Some(plan)
            }
            Err(message) => {
                self.error = Some(message);
                None
            }
        }
    }
}

fn is_plain(c: char) -> bool {
    c.is_alphanumeric() || "-_./:@=,+%^".contains(c)
}

/// Display only. A token with whitespace, a quote or a shell metacharacter is single-quoted,
/// and an embedded single quote is closed, escaped and reopened.
pub fn shell_join(argv: &[String]) -> String {
    argv.iter()
        .map(|token| {
            if !token.is_empty() && token.chars().all(is_plain) {
                token.clone()
            } else {
                format!("'{}'", token.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn no_plan(_: &Form) -> Result<Plan, String> {
        Err("nope".into())
    }

    fn form_of(fields: Vec<Field>) -> Form {
        Form::new("T", fields, no_plan)
    }

    #[test]
    fn typing_appends_to_a_text_field() {
        let mut form = form_of(vec![Field::text("a", "x")]);
        form.handle_key(press(KeyCode::Char('y')));
        assert_eq!(form.fields[0].value, "xy");
        assert!(form.fields[0].changed());
    }

    #[test]
    fn backspace_deletes_the_last_character() {
        let mut form = form_of(vec![Field::text("a", "xy")]);
        form.handle_key(press(KeyCode::Backspace));
        assert_eq!(form.fields[0].value, "x");
    }

    #[test]
    fn tab_wraps_forward_and_backtab_wraps_back() {
        let mut form = form_of(vec![Field::text("a", ""), Field::text("b", "")]);
        form.handle_key(press(KeyCode::Tab));
        assert_eq!(form.focus, 1);
        form.handle_key(press(KeyCode::Tab));
        assert_eq!(form.focus, 0);
        form.handle_key(press(KeyCode::BackTab));
        assert_eq!(form.focus, 1);
    }

    #[test]
    fn enter_moves_on_and_submits_on_the_last_field() {
        let mut form = form_of(vec![Field::text("a", ""), Field::text("b", "")]);
        assert_eq!(form.handle_key(press(KeyCode::Enter)), FormAnswer::Editing);
        assert_eq!(form.focus, 1);
        assert_eq!(form.handle_key(press(KeyCode::Enter)), FormAnswer::Submit);
    }

    #[test]
    fn enter_on_a_body_field_opens_the_editor_and_typing_is_ignored() {
        let mut form = form_of(vec![Field::text("a", ""), Field::body("b", "old")]);
        form.handle_key(press(KeyCode::Tab));
        form.handle_key(press(KeyCode::Char('z')));
        assert_eq!(form.fields[1].value, "old");
        assert_eq!(
            form.handle_key(press(KeyCode::Enter)),
            FormAnswer::OpenEditor(1)
        );
        form.set_body(1, Some(PathBuf::from("/tmp/b.md")), "new".into());
        assert_eq!(form.fields[1].text, "new");
        assert!(form.fields[1].changed());
    }

    #[test]
    fn esc_cancels() {
        let mut form = form_of(vec![Field::text("a", "")]);
        assert_eq!(form.handle_key(press(KeyCode::Esc)), FormAnswer::Cancel);
    }

    #[test]
    fn choice_cycles_with_left_and_right_and_wraps() {
        let mut form = form_of(vec![Field::choice("c", &["a", "b", "c"], "a")]);
        form.handle_key(press(KeyCode::Right));
        assert_eq!(form.fields[0].value, "b");
        form.handle_key(press(KeyCode::Left));
        form.handle_key(press(KeyCode::Left));
        assert_eq!(form.fields[0].value, "c");
        form.handle_key(press(KeyCode::Right));
        assert_eq!(form.fields[0].value, "a");
    }

    #[test]
    fn submit_stores_the_error_and_returns_none() {
        let mut form = form_of(vec![Field::text("a", "")]);
        assert_eq!(form.submit(), None);
        assert_eq!(form.error.as_deref(), Some("nope"));
    }

    #[test]
    fn shell_join_quotes_a_space_and_a_quote_and_leaves_plain_tokens() {
        let argv: Vec<String> = ["cox", "x", "a b", "it's", "pat@h"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(shell_join(&argv), "cox x 'a b' 'it'\\''s' pat@h");
        assert_eq!(argv[2], "a b");
    }
}
