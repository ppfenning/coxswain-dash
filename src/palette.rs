//! The colon palette's pure state: an input line with a cursor, the output of a finished
//! command, and its scroll offset. No I/O; a later task draws it and spawns the command.

use crossterm::event::KeyCode;

/// The output buffer keeps only this many trailing lines.
pub const MAX_OUTPUT_LINES: usize = 500;

/// Lines moved by PageUp and PageDown; the frame's real height is not known here.
const PAGE_STEP: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteEvent {
    None,
    Close,
    Submit(Vec<String>),
}

/// `output` being `Some` is frame mode; `cursor` counts chars, not bytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PaletteState {
    input: String,
    cursor: usize,
    output: Option<Vec<String>>,
    scroll: usize,
    status: Option<String>,
}

#[derive(Default)]
struct Split {
    done: Vec<String>,
    cur: Option<String>,
    quote: Option<char>,
    escaped: bool,
}

fn push_char(cur: Option<String>, c: char) -> Option<String> {
    let mut s = cur.unwrap_or_default();
    s.push(c);
    Some(s)
}

fn step(s: Split, c: char) -> Split {
    match (s.escaped, s.quote, c) {
        (true, _, _) => Split {
            cur: push_char(s.cur, c),
            escaped: false,
            ..s
        },
        (false, Some('\''), '\\') => Split {
            cur: push_char(s.cur, c),
            ..s
        },
        (false, _, '\\') => Split {
            cur: Some(s.cur.unwrap_or_default()),
            escaped: true,
            ..s
        },
        (false, Some(q), _) if q == c => Split {
            cur: Some(s.cur.unwrap_or_default()),
            quote: None,
            ..s
        },
        (false, Some(_), _) => Split {
            cur: push_char(s.cur, c),
            ..s
        },
        (false, None, '\'' | '"') => Split {
            cur: Some(s.cur.unwrap_or_default()),
            quote: Some(c),
            ..s
        },
        (false, None, _) if c.is_whitespace() => Split {
            done: s.done.into_iter().chain(s.cur).collect(),
            cur: None,
            ..s
        },
        (false, None, _) => Split {
            cur: push_char(s.cur, c),
            ..s
        },
    }
}

/// Splits on whitespace; quotes group, a backslash escapes outside single quotes.
pub fn split_args(line: &str) -> Result<Vec<String>, String> {
    let end = line.chars().fold(Split::default(), step);
    match (end.escaped, end.quote) {
        (true, _) => Err("trailing backslash".to_string()),
        (false, Some(q)) => Err(format!("unbalanced quote {q}")),
        (false, None) => Ok(end.done.into_iter().chain(end.cur).collect()),
    }
}

fn byte_at(s: &str, char_idx: usize) -> usize {
    s.char_indices().nth(char_idx).map_or(s.len(), |(b, _)| b)
}

fn insert_at(s: &str, char_idx: usize, c: char) -> String {
    let at = byte_at(s, char_idx);
    format!("{}{}{}", &s[..at], c, &s[at..])
}

fn remove_at(s: &str, char_idx: usize) -> String {
    s.chars()
        .enumerate()
        .filter(|(i, _)| *i != char_idx)
        .map(|(_, c)| c)
        .collect()
}

fn with_cox(args: Vec<String>) -> Vec<String> {
    if args.first().map(String::as_str) == Some("cox") {
        args
    } else {
        std::iter::once("cox".to_string()).chain(args).collect()
    }
}

impl PaletteState {
    pub fn open() -> Self {
        Self::default()
    }

    pub fn prefilled(text: &str) -> Self {
        Self {
            input: text.to_string(),
            cursor: text.chars().count(),
            ..Self::default()
        }
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn output(&self) -> Option<&[String]> {
        self.output.as_deref()
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// The last parse error, shown in the status area until the next edit.
    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    pub fn in_frame_mode(&self) -> bool {
        self.output.is_some()
    }

    /// Replaces the output with its last 500 lines, resets scroll and enters frame mode.
    pub fn set_output(&mut self, lines: Vec<String>) {
        let skip = lines.len().saturating_sub(MAX_OUTPUT_LINES);
        self.output = Some(lines.into_iter().skip(skip).collect());
        self.scroll = 0;
    }

    pub fn handle_key(&mut self, key: KeyCode) -> PaletteEvent {
        if self.in_frame_mode() {
            self.frame_key(key)
        } else {
            self.input_key(key)
        }
    }

    fn input_key(&mut self, key: KeyCode) -> PaletteEvent {
        match key {
            KeyCode::Esc => PaletteEvent::Close,
            KeyCode::Enter => self.submit(),
            other => {
                self.edit(other);
                PaletteEvent::None
            }
        }
    }

    fn submit(&mut self) -> PaletteEvent {
        if self.input.trim().is_empty() {
            return PaletteEvent::None;
        }
        match split_args(&self.input) {
            Ok(args) => {
                self.status = None;
                PaletteEvent::Submit(with_cox(args))
            }
            Err(msg) => {
                self.status = Some(msg);
                PaletteEvent::None
            }
        }
    }

    fn edit(&mut self, key: KeyCode) {
        let len = self.input.chars().count();
        match key {
            KeyCode::Char(c) => {
                self.input = insert_at(&self.input, self.cursor, c);
                self.cursor += 1;
            }
            KeyCode::Backspace if self.cursor > 0 => {
                self.input = remove_at(&self.input, self.cursor - 1);
                self.cursor -= 1;
            }
            KeyCode::Delete if self.cursor < len => {
                self.input = remove_at(&self.input, self.cursor);
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(len),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = len,
            _ => return,
        }
        self.status = None;
    }

    fn frame_key(&mut self, key: KeyCode) -> PaletteEvent {
        let last = self.output.as_ref().map_or(0, Vec::len).saturating_sub(1);
        match key {
            KeyCode::Esc | KeyCode::Char('q') => return PaletteEvent::Close,
            KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::Down => self.scroll = (self.scroll + 1).min(last),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(PAGE_STEP),
            KeyCode::PageDown => self.scroll = (self.scroll + PAGE_STEP).min(last),
            _ => {}
        }
        PaletteEvent::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn numbered(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("line {i}")).collect()
    }

    #[test]
    fn split_args_splits_a_plain_line_on_whitespace() {
        assert_eq!(split_args("a b  c"), Ok(strs(&["a", "b", "c"])));
    }

    #[test]
    fn split_args_keeps_a_quoted_argument_whole() {
        assert_eq!(
            split_args("say \"hello world\" 'it is'"),
            Ok(strs(&["say", "hello world", "it is"]))
        );
    }

    #[test]
    fn split_args_honours_an_escaped_space() {
        assert_eq!(split_args("a\\ b c"), Ok(strs(&["a b", "c"])));
    }

    #[test]
    fn split_args_reports_an_unbalanced_quote_as_a_value() {
        assert_eq!(
            split_args("say \"hello"),
            Err("unbalanced quote \"".to_string())
        );
    }

    #[test]
    fn split_args_reports_a_trailing_backslash() {
        assert_eq!(split_args("a\\"), Err("trailing backslash".to_string()));
    }

    #[test]
    fn open_is_empty_and_prefilled_puts_the_cursor_at_the_end() {
        let open = PaletteState::open();
        assert_eq!((open.input(), open.cursor()), ("", 0));
        let pre = PaletteState::prefilled("run é");
        assert_eq!((pre.input(), pre.cursor()), ("run é", 5));
        assert!(!pre.in_frame_mode());
    }

    #[test]
    fn enter_submits_with_cox_prepended() {
        let mut p = PaletteState::prefilled("run list");
        assert_eq!(
            p.handle_key(KeyCode::Enter),
            PaletteEvent::Submit(strs(&["cox", "run", "list"]))
        );
    }

    #[test]
    fn a_line_starting_with_cox_is_not_doubled() {
        let mut p = PaletteState::prefilled("cox status");
        assert_eq!(
            p.handle_key(KeyCode::Enter),
            PaletteEvent::Submit(strs(&["cox", "status"]))
        );
    }

    #[test]
    fn enter_on_a_blank_line_is_none() {
        assert_eq!(
            PaletteState::open().handle_key(KeyCode::Enter),
            PaletteEvent::None
        );
        assert_eq!(
            PaletteState::prefilled("   ").handle_key(KeyCode::Enter),
            PaletteEvent::None
        );
    }

    #[test]
    fn enter_on_an_unbalanced_quote_is_none_and_sets_the_status() {
        let mut p = PaletteState::prefilled("say \"hi");
        assert_eq!(p.handle_key(KeyCode::Enter), PaletteEvent::None);
        assert_eq!(p.status(), Some("unbalanced quote \""));
        p.handle_key(KeyCode::Char('"'));
        assert_eq!(p.status(), None);
    }

    #[test]
    fn esc_closes_in_input_mode_and_in_frame_mode() {
        assert_eq!(
            PaletteState::open().handle_key(KeyCode::Esc),
            PaletteEvent::Close
        );
        let mut p = PaletteState::open();
        p.set_output(numbered(3));
        assert_eq!(p.handle_key(KeyCode::Esc), PaletteEvent::Close);
    }

    #[test]
    fn q_closes_in_frame_mode_and_types_in_input_mode() {
        let mut framed = PaletteState::open();
        framed.set_output(numbered(3));
        assert_eq!(framed.handle_key(KeyCode::Char('q')), PaletteEvent::Close);
        let mut typing = PaletteState::open();
        assert_eq!(typing.handle_key(KeyCode::Char('q')), PaletteEvent::None);
        assert_eq!(typing.input(), "q");
    }

    #[test]
    fn editing_keys_insert_delete_and_move_the_cursor() {
        let mut p = PaletteState::prefilled("ab");
        p.handle_key(KeyCode::Left);
        p.handle_key(KeyCode::Char('é'));
        assert_eq!((p.input(), p.cursor()), ("aéb", 2));
        p.handle_key(KeyCode::Backspace);
        assert_eq!((p.input(), p.cursor()), ("ab", 1));
        p.handle_key(KeyCode::Delete);
        assert_eq!((p.input(), p.cursor()), ("a", 1));
        p.handle_key(KeyCode::Home);
        assert_eq!(p.cursor(), 0);
        p.handle_key(KeyCode::Backspace);
        p.handle_key(KeyCode::Left);
        assert_eq!((p.input(), p.cursor()), ("a", 0));
        p.handle_key(KeyCode::End);
        p.handle_key(KeyCode::Right);
        p.handle_key(KeyCode::Delete);
        assert_eq!((p.input(), p.cursor()), ("a", 1));
    }

    #[test]
    fn scroll_stays_within_bounds() {
        let mut p = PaletteState::open();
        p.set_output(numbered(15));
        p.handle_key(KeyCode::Up);
        assert_eq!(p.scroll(), 0);
        p.handle_key(KeyCode::Down);
        assert_eq!(p.scroll(), 1);
        p.handle_key(KeyCode::PageDown);
        assert_eq!(p.scroll(), 11);
        p.handle_key(KeyCode::PageDown);
        assert_eq!(p.scroll(), 14);
        p.handle_key(KeyCode::Down);
        assert_eq!(p.scroll(), 14);
        p.handle_key(KeyCode::PageUp);
        assert_eq!(p.scroll(), 4);
        p.handle_key(KeyCode::PageUp);
        assert_eq!(p.scroll(), 0);
    }

    #[test]
    fn scroll_on_empty_output_stays_at_zero() {
        let mut p = PaletteState::open();
        p.set_output(Vec::new());
        p.handle_key(KeyCode::Down);
        p.handle_key(KeyCode::PageDown);
        assert_eq!(p.scroll(), 0);
    }

    #[test]
    fn frame_mode_keys_never_edit_the_output() {
        let mut p = PaletteState::open();
        p.set_output(numbered(3));
        for key in [
            KeyCode::Char('x'),
            KeyCode::Backspace,
            KeyCode::Delete,
            KeyCode::Enter,
            KeyCode::Home,
        ] {
            assert_eq!(p.handle_key(key), PaletteEvent::None);
        }
        assert_eq!(p.output(), Some(numbered(3).as_slice()));
        assert_eq!(p.input(), "");
    }

    #[test]
    fn set_output_keeps_the_last_500_lines_and_resets_scroll() {
        let mut p = PaletteState::open();
        p.set_output(numbered(20));
        p.handle_key(KeyCode::PageDown);
        p.set_output(numbered(700));
        let out = p.output().unwrap();
        assert_eq!(out.len(), 500);
        assert_eq!(out[0], "line 200");
        assert_eq!(out[499], "line 699");
        assert_eq!(p.scroll(), 0);
        assert!(p.in_frame_mode());
    }
}
