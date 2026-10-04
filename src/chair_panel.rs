//! The chair panel: one pty terminal, drawn from a vt100 screen, with focus and width state.
//! `open` takes a session id, not a `Chair`, so this module does not depend on the feed.
//! Closing kills only the local `claude attach` process. The remote session keeps running.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Paragraph, Wrap};

use crate::pty::PtySession;
use crate::theme::Theme;

/// How wide the panel is when the caller lays it out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidthMode {
    Narrow,
    Wide,
    Full,
}

impl WidthMode {
    pub fn next(self) -> Self {
        match self {
            Self::Narrow => Self::Wide,
            Self::Wide => Self::Full,
            Self::Full => Self::Narrow,
        }
    }

    /// Share of the screen width, in percent.
    pub fn percent(self) -> u16 {
        match self {
            Self::Narrow => 35,
            Self::Wide => 60,
            Self::Full => 100,
        }
    }
}

/// What `handle_key` did with a key, so the caller can route on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOutcome {
    /// The panel is not focused. The caller handles the key.
    Unfocused,
    /// `Ctrl-]` cleared focus. The caller takes focus back and does not handle the key.
    ReleasedFocus,
    /// The panel kept the key. It wrote the key's bytes, or the key has none in a terminal.
    Consumed,
}

/// Owns exactly one `PtySession` and the state of the terminal drawn from it.
pub struct ChairPanel<P: PtySession> {
    pty: P,
    open: bool,
    focused: bool,
    width: WidthMode,
    screen: Option<vt100::Parser>,
    error: Option<String>,
}

impl<P: PtySession> ChairPanel<P> {
    pub fn new(pty: P) -> Self {
        Self {
            pty,
            open: false,
            focused: false,
            width: WidthMode::Narrow,
            screen: None,
            error: None,
        }
    }

    pub fn pty(&self) -> &P {
        &self.pty
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn focused(&self) -> bool {
        self.focused
    }

    pub fn width(&self) -> WidthMode {
        self.width
    }

    /// The message of the last failed pty call. A later successful call, `close`, or `open`
    /// clears it.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Plain text of the vt100 screen; empty when there is no screen.
    pub fn screen_text(&self) -> String {
        self.screen
            .as_ref()
            .map(|parser| parser.screen().contents())
            .unwrap_or_default()
    }

    /// Attaches to the session. A no-op while already open. With no id, spawns nothing.
    pub fn open(&mut self, session_id: Option<&str>, rows: u16, cols: u16) {
        if self.open {
            return;
        }
        let Some(id) = session_id else {
            self.error = None;
            return;
        };
        let argv = ["claude", "attach", id].map(String::from);
        match self.pty.spawn(&argv, rows, cols) {
            Ok(()) => {
                self.open = true;
                self.error = None;
                self.screen = Some(vt100::Parser::new(rows, cols, 0));
            }
            Err(e) => {
                self.open = false;
                self.focused = false;
                self.screen = None;
                self.error = Some(e.0);
            }
        }
    }

    /// Kills the local attach process once and clears the panel. A no-op while closed.
    pub fn close(&mut self) {
        if !self.open {
            return;
        }
        // The panel is closed either way; a failed kill has nothing left to retry.
        let _ = self.pty.kill();
        self.open = false;
        self.focused = false;
        self.screen = None;
        self.error = None;
    }

    /// Focus needs a live terminal to receive the keys.
    pub fn set_focus(&mut self, focused: bool) {
        self.focused = focused && self.open;
    }

    pub fn cycle_width(&mut self) {
        self.width = self.width.next();
    }

    /// While focused, forwards every key to the pty except `Ctrl-]`, which hands focus back.
    pub fn handle_key(&mut self, key: KeyEvent) -> KeyOutcome {
        if !self.focused {
            return KeyOutcome::Unfocused;
        }
        if is_release_focus(key) {
            self.focused = false;
            return KeyOutcome::ReleasedFocus;
        }
        if let Some(bytes) = key_bytes(key) {
            self.error = self.pty.write(&bytes).err().map(|e| e.0);
        }
        KeyOutcome::Consumed
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        if !self.open {
            return;
        }
        self.error = self.pty.resize(rows, cols).err().map(|e| e.0);
        if let Some(parser) = self.screen.as_mut() {
            parser.set_size(rows, cols);
        }
    }

    /// Feeds whatever the pty has printed since the last poll into the screen.
    pub fn poll(&mut self) {
        if let Some(parser) = self.screen.as_mut() {
            parser.process(&self.pty.read_output());
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, theme: &Theme) {
        let border = if self.focused {
            theme.accent
        } else {
            theme.dim
        };
        let block = Block::bordered()
            .title(" chair ")
            .style(Style::new().fg(theme.fg).bg(theme.bg))
            .border_style(Style::new().fg(border).bg(theme.bg));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        match (&self.screen, &self.error) {
            (Some(parser), _) => draw_screen(frame.buffer_mut(), inner, parser.screen(), theme),
            (None, Some(message)) => frame.render_widget(
                Paragraph::new(message.as_str())
                    .style(Style::new().fg(theme.fg).bg(theme.bg))
                    .wrap(Wrap { trim: true }),
                inner,
            ),
            (None, None) => frame.render_widget(
                Paragraph::new("no session published")
                    .style(Style::new().fg(theme.dim).bg(theme.bg)),
                inner,
            ),
        }
    }
}

impl<P: PtySession> Drop for ChairPanel<P> {
    fn drop(&mut self) {
        self.close();
    }
}

/// `Ctrl-]`. Terminals send byte 0x1d, which crossterm reports as Ctrl-5, so both are accepted.
fn is_release_focus(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char(']') | KeyCode::Char('5'))
}

/// The bytes an xterm sends for `key`. None only for keys a terminal sends nothing for:
/// lock keys, PrintScreen, Pause, Menu, media keys and bare modifiers.
fn key_bytes(key: KeyEvent) -> Option<Vec<u8>> {
    let mods = key.modifiers;
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    let backspace = if ctrl { 0x08 } else { 0x7f };
    match key.code {
        KeyCode::Char(c) => Some(alt_prefixed(char_bytes(c, ctrl), mods)),
        KeyCode::Enter => Some(alt_prefixed(vec![b'\r'], mods)),
        KeyCode::Tab => Some(alt_prefixed(vec![b'\t'], mods)),
        KeyCode::BackTab => Some(b"\x1b[Z".to_vec()),
        KeyCode::Backspace => Some(alt_prefixed(vec![backspace], mods)),
        KeyCode::Esc => Some(alt_prefixed(vec![0x1b], mods)),
        KeyCode::Null => Some(vec![0]),
        code => named_key(code, modifier_param(mods)).map(String::into_bytes),
    }
}

/// Alt sends ESC before the key's own bytes.
fn alt_prefixed(bytes: Vec<u8>, mods: KeyModifiers) -> Vec<u8> {
    if mods.contains(KeyModifiers::ALT) {
        [vec![0x1b], bytes].concat()
    } else {
        bytes
    }
}

/// Ctrl maps a character to its C0 control byte where one exists; otherwise the character goes
/// out unchanged.
fn char_bytes(c: char, ctrl: bool) -> Vec<u8> {
    match (ctrl, control_byte(c)) {
        (true, Some(byte)) => vec![byte],
        _ => c.to_string().into_bytes(),
    }
}

/// The xterm control byte for Ctrl plus `c`. Crossterm reports 0x1c to 0x1f as Ctrl-4 to Ctrl-7
/// and 0x00 as Ctrl-Space, so the digit forms map back to the same bytes.
fn control_byte(c: char) -> Option<u8> {
    match c {
        'a'..='z' => Some(c as u8 - b'a' + 1),
        '@'..='_' => Some(c as u8 & 0x1f),
        ' ' | '2' => Some(0x00),
        '3' => Some(0x1b),
        '4' => Some(0x1c),
        '5' => Some(0x1d),
        '6' => Some(0x1e),
        '7' | '/' => Some(0x1f),
        '8' | '?' => Some(0x7f),
        _ => None,
    }
}

/// The xterm modifier parameter: 1, plus 1 for Shift, 2 for Alt, 4 for Ctrl.
fn modifier_param(mods: KeyModifiers) -> u8 {
    1 + u8::from(mods.contains(KeyModifiers::SHIFT))
        + 2 * u8::from(mods.contains(KeyModifiers::ALT))
        + 4 * u8::from(mods.contains(KeyModifiers::CONTROL))
}

/// The xterm escape sequence for a cursor, editing or function key, carrying `param` when a
/// modifier is held. F13 to F24 are xterm's shifted F1 to F12.
fn named_key(code: KeyCode, param: u8) -> Option<String> {
    let csi = |c: char| match param {
        1 => format!("\x1b[{c}"),
        _ => format!("\x1b[1;{param}{c}"),
    };
    let ss3 = |c: char| match param {
        1 => format!("\x1bO{c}"),
        _ => format!("\x1b[1;{param}{c}"),
    };
    let tilde = |n: u8| match param {
        1 => format!("\x1b[{n}~"),
        _ => format!("\x1b[{n};{param}~"),
    };
    match code {
        KeyCode::Up => Some(csi('A')),
        KeyCode::Down => Some(csi('B')),
        KeyCode::Right => Some(csi('C')),
        KeyCode::Left => Some(csi('D')),
        KeyCode::KeypadBegin => Some(csi('E')),
        KeyCode::End => Some(csi('F')),
        KeyCode::Home => Some(csi('H')),
        KeyCode::Insert => Some(tilde(2)),
        KeyCode::Delete => Some(tilde(3)),
        KeyCode::PageUp => Some(tilde(5)),
        KeyCode::PageDown => Some(tilde(6)),
        KeyCode::F(n @ 1..=4) => Some(ss3(['P', 'Q', 'R', 'S'][usize::from(n - 1)])),
        KeyCode::F(n @ 5..=12) => Some(tilde([15, 17, 18, 19, 20, 21, 23, 24][usize::from(n - 5)])),
        KeyCode::F(n @ 13..=24) => named_key(KeyCode::F(n - 12), param + 1),
        _ => None,
    }
}

/// A vt100 color on the theme: the terminal default becomes the theme's own color.
fn map_color(color: vt100::Color, default: Color) -> Color {
    match color {
        vt100::Color::Default => default,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

fn draw_screen(buf: &mut Buffer, area: Rect, screen: &vt100::Screen, theme: &Theme) {
    buf.set_style(area, Style::new().fg(theme.fg).bg(theme.bg));
    let (rows, cols) = screen.size();
    for row in 0..area.height.min(rows) {
        for col in 0..area.width.min(cols) {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };
            if cell.is_wide_continuation() {
                continue;
            }
            let text = cell.contents();
            let symbol = if text.is_empty() { " " } else { text.as_str() };
            let weight = if cell.bold() {
                Modifier::BOLD
            } else {
                Modifier::empty()
            };
            let style = Style::new()
                .fg(map_color(cell.fgcolor(), theme.fg))
                .bg(map_color(cell.bgcolor(), theme.bg))
                .add_modifier(weight);
            if let Some(target) = buf.cell_mut((area.x + col, area.y + row)) {
                target.set_symbol(symbol).set_style(style);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::app::ThemeId;
    use crate::pty::{FakePty, PtyCall, PtyError};
    use crate::theme::resolve;

    /// A pty whose write always fails, and whose spawn fails when `spawn_fails` is set.
    struct FailPty {
        spawn_fails: bool,
    }

    impl PtySession for FailPty {
        fn spawn(&mut self, _: &[String], _: u16, _: u16) -> Result<(), PtyError> {
            if self.spawn_fails {
                Err(PtyError("no such binary".to_string()))
            } else {
                Ok(())
            }
        }
        fn write(&mut self, _: &[u8]) -> Result<(), PtyError> {
            Err(PtyError("broken pipe".to_string()))
        }
        fn resize(&mut self, _: u16, _: u16) -> Result<(), PtyError> {
            Ok(())
        }
        fn read_output(&mut self) -> Vec<u8> {
            Vec::new()
        }
        fn kill(&mut self) -> Result<(), PtyError> {
            Ok(())
        }
    }

    /// A `FakePty` that also appends each call to a log that outlives the panel.
    struct LogPty {
        inner: FakePty,
        log: Rc<RefCell<Vec<PtyCall>>>,
    }

    impl LogPty {
        fn new(log: &Rc<RefCell<Vec<PtyCall>>>) -> Self {
            Self {
                inner: FakePty::default(),
                log: Rc::clone(log),
            }
        }
    }

    impl PtySession for LogPty {
        fn spawn(&mut self, argv: &[String], rows: u16, cols: u16) -> Result<(), PtyError> {
            self.log.borrow_mut().push(PtyCall::Spawn {
                argv: argv.to_vec(),
                rows,
                cols,
            });
            self.inner.spawn(argv, rows, cols)
        }
        fn write(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
            self.log.borrow_mut().push(PtyCall::Write(bytes.to_vec()));
            self.inner.write(bytes)
        }
        fn resize(&mut self, rows: u16, cols: u16) -> Result<(), PtyError> {
            self.log.borrow_mut().push(PtyCall::Resize { rows, cols });
            self.inner.resize(rows, cols)
        }
        fn read_output(&mut self) -> Vec<u8> {
            self.log.borrow_mut().push(PtyCall::ReadOutput);
            self.inner.read_output()
        }
        fn kill(&mut self) -> Result<(), PtyError> {
            self.log.borrow_mut().push(PtyCall::Kill);
            self.inner.kill()
        }
    }

    fn spawn_call(id: &str, rows: u16, cols: u16) -> PtyCall {
        PtyCall::Spawn {
            argv: ["claude", "attach", id].map(String::from).to_vec(),
            rows,
            cols,
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn drawn<P: PtySession>(panel: &ChairPanel<P>, theme: ThemeId) -> String {
        let mut terminal = Terminal::new(TestBackend::new(30, 6)).unwrap();
        terminal
            .draw(|frame| panel.render(frame, frame.area(), &resolve(theme)))
            .unwrap();
        terminal.backend().to_string()
    }

    #[test]
    fn width_mode_cycles_35_60_full_and_back() {
        let seen = [WidthMode::Narrow, WidthMode::Wide, WidthMode::Full].map(WidthMode::percent);
        assert_eq!(seen, [35, 60, 100]);
        assert_eq!(WidthMode::Full.next(), WidthMode::Narrow);
    }

    #[test]
    fn open_spawns_claude_attach_once_and_sets_open() {
        let mut panel = ChairPanel::new(FakePty::default());
        panel.open(Some("abc"), 24, 80);
        assert!(panel.is_open());
        assert_eq!(panel.pty().calls(), [spawn_call("abc", 24, 80)]);
    }

    #[test]
    fn open_while_open_is_a_no_op() {
        let mut panel = ChairPanel::new(FakePty::default());
        panel.open(Some("abc"), 24, 80);
        panel.open(Some("other"), 10, 10);
        assert!(panel.is_open());
        assert_eq!(panel.pty().calls(), [spawn_call("abc", 24, 80)]);
    }

    #[test]
    fn open_without_a_session_spawns_nothing_and_says_so() {
        let mut panel = ChairPanel::new(FakePty::default());
        panel.open(None, 24, 80);
        assert!(!panel.is_open());
        assert!(panel.pty().calls().is_empty());
        assert!(drawn(&panel, ThemeId::Regatta).contains("no session published"));
    }

    #[test]
    fn failed_spawn_leaves_the_panel_closed_with_the_error_shown() {
        let mut panel = ChairPanel::new(FailPty { spawn_fails: true });
        panel.open(Some("abc"), 24, 80);
        assert_eq!(
            (panel.is_open(), panel.focused(), panel.screen.is_none()),
            (false, false, true)
        );
        assert_eq!(panel.error(), Some("no such binary"));
        assert!(drawn(&panel, ThemeId::Regatta).contains("no such binary"));
    }

    #[test]
    fn open_without_a_session_after_a_failed_spawn_drops_the_stale_error() {
        let mut panel = ChairPanel::new(FailPty { spawn_fails: true });
        panel.open(Some("abc"), 24, 80);
        panel.open(None, 24, 80);
        assert_eq!(panel.error(), None);
        assert!(drawn(&panel, ThemeId::Regatta).contains("no session published"));
    }

    #[test]
    fn close_after_a_failed_write_drops_the_stale_error() {
        let mut panel = ChairPanel::new(FailPty { spawn_fails: false });
        panel.open(Some("abc"), 24, 80);
        panel.set_focus(true);
        panel.handle_key(key(KeyCode::Char('x')));
        assert_eq!(panel.error(), Some("broken pipe"));
        panel.close();
        assert_eq!(panel.error(), None);
        assert!(drawn(&panel, ThemeId::Regatta).contains("no session published"));
    }

    #[test]
    fn close_kills_once_and_clears_state() {
        let mut panel = ChairPanel::new(FakePty::with_output(b"hi"));
        panel.open(Some("abc"), 24, 80);
        panel.set_focus(true);
        panel.close();
        panel.close();
        let kills = panel.pty().calls().iter().filter(|c| **c == PtyCall::Kill);
        assert_eq!(kills.count(), 1);
        assert_eq!(
            (panel.is_open(), panel.focused(), panel.screen.is_none()),
            (false, false, true)
        );
    }

    #[test]
    fn reopen_after_close_spawns_fresh_with_an_empty_screen() {
        let mut panel = ChairPanel::new(FakePty::with_output(b"hi"));
        panel.open(Some("abc"), 24, 80);
        panel.poll();
        assert_eq!(panel.screen_text(), "hi");
        panel.close();
        panel.open(Some("abc"), 24, 80);
        let lifecycle: Vec<_> = panel
            .pty()
            .calls()
            .iter()
            .filter(|c| **c != PtyCall::ReadOutput)
            .cloned()
            .collect();
        assert_eq!(
            lifecycle,
            [
                spawn_call("abc", 24, 80),
                PtyCall::Kill,
                spawn_call("abc", 24, 80)
            ]
        );
        assert_eq!(panel.screen_text(), "");
    }

    #[test]
    fn dropping_an_open_panel_kills_and_a_closed_one_does_not() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut open_panel = ChairPanel::new(LogPty::new(&log));
        open_panel.open(Some("abc"), 24, 80);
        drop(open_panel);
        assert_eq!(*log.borrow(), [spawn_call("abc", 24, 80), PtyCall::Kill]);

        let quiet = Rc::new(RefCell::new(Vec::new()));
        drop(ChairPanel::new(LogPty::new(&quiet)));
        assert!(quiet.borrow().is_empty());
    }

    #[test]
    fn keys_reach_the_pty_only_while_focused_and_ctrl_right_bracket_releases_focus() {
        let mut panel = ChairPanel::new(FakePty::default());
        panel.open(Some("abc"), 24, 80);
        let unfocused = panel.handle_key(key(KeyCode::Char('a')));
        panel.set_focus(true);
        let released = panel.handle_key(ctrl(']'));
        let after_release = panel.focused();
        panel.set_focus(true);
        let written = panel.handle_key(key(KeyCode::Char('b')));
        assert_eq!(
            (unfocused, released, after_release, written),
            (
                KeyOutcome::Unfocused,
                KeyOutcome::ReleasedFocus,
                false,
                KeyOutcome::Consumed
            )
        );
        assert_eq!(
            panel.pty().calls(),
            [spawn_call("abc", 24, 80), PtyCall::Write(b"b".to_vec())]
        );
    }

    #[test]
    fn key_bytes_encodes_text_controls_named_and_function_keys() {
        let alt_x = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT);
        let ctrl_left = KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL);
        let encoded = [
            key(KeyCode::Char('é')),
            ctrl('c'),
            ctrl('\\'),
            ctrl(' '),
            alt_x,
            key(KeyCode::BackTab),
            key(KeyCode::Home),
            key(KeyCode::Delete),
            key(KeyCode::PageDown),
            ctrl_left,
            key(KeyCode::F(1)),
            key(KeyCode::F(12)),
            key(KeyCode::F(13)),
            key(KeyCode::CapsLock),
        ]
        .map(key_bytes);
        let expected: [Option<&[u8]>; 14] = [
            Some("é".as_bytes()),
            Some(b"\x03"),
            Some(b"\x1c"),
            Some(b"\x00"),
            Some(b"\x1bx"),
            Some(b"\x1b[Z"),
            Some(b"\x1b[H"),
            Some(b"\x1b[3~"),
            Some(b"\x1b[6~"),
            Some(b"\x1b[1;5D"),
            Some(b"\x1bOP"),
            Some(b"\x1b[24~"),
            Some(b"\x1b[1;2P"),
            None,
        ];
        assert_eq!(encoded, expected.map(|bytes| bytes.map(<[u8]>::to_vec)));
    }

    #[test]
    fn render_draws_with_the_theme_colors() {
        for id in [ThemeId::Regatta, ThemeId::HarborLight] {
            let theme = resolve(id);
            let closed = ChairPanel::new(FakePty::default());
            let mut open = ChairPanel::new(FakePty::with_output(b"h\x1b[31mr"));
            open.open(Some("abc"), 4, 28);
            open.poll();
            open.set_focus(true);
            let style_at = |panel: &ChairPanel<FakePty>, x: u16, y: u16| {
                let mut terminal = Terminal::new(TestBackend::new(30, 6)).unwrap();
                terminal
                    .draw(|frame| panel.render(frame, frame.area(), &theme))
                    .unwrap();
                let cell = &terminal.backend().buffer()[(x, y)];
                (cell.fg, cell.bg)
            };
            assert_eq!(
                [
                    style_at(&closed, 0, 0),
                    style_at(&closed, 1, 1),
                    style_at(&open, 0, 0),
                    style_at(&open, 1, 1),
                    style_at(&open, 2, 1),
                ],
                [
                    (theme.dim, theme.bg),
                    (theme.dim, theme.bg),
                    (theme.accent, theme.bg),
                    (theme.fg, theme.bg),
                    (Color::Indexed(1), theme.bg),
                ]
            );
        }
    }

    #[test]
    fn resize_forwards_to_the_pty() {
        let mut panel = ChairPanel::new(FakePty::default());
        panel.open(Some("abc"), 24, 80);
        panel.resize(10, 40);
        assert_eq!(
            panel.pty().calls().last(),
            Some(&PtyCall::Resize { rows: 10, cols: 40 })
        );
    }

    fn open_panel() -> ChairPanel<FakePty> {
        let mut panel = ChairPanel::new(FakePty::with_output(b"hello\r\n\x1b[1mworld"));
        panel.open(Some("abc"), 4, 28);
        panel.poll();
        panel
    }

    #[test]
    fn renders_closed_snapshot_in_the_regatta_theme() {
        let panel = ChairPanel::new(FakePty::default());
        insta::assert_snapshot!(drawn(&panel, ThemeId::Regatta));
    }

    #[test]
    fn renders_closed_snapshot_in_the_harbor_light_theme() {
        let panel = ChairPanel::new(FakePty::default());
        insta::assert_snapshot!(drawn(&panel, ThemeId::HarborLight));
    }

    #[test]
    fn renders_open_snapshot_in_the_regatta_theme() {
        insta::assert_snapshot!(drawn(&open_panel(), ThemeId::Regatta));
    }

    #[test]
    fn renders_open_snapshot_in_the_harbor_light_theme() {
        insta::assert_snapshot!(drawn(&open_panel(), ThemeId::HarborLight));
    }
}
