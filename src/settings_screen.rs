//! The pure, key-driven state of the settings screen. It runs nothing: the caller acts on each `ScreenEvent`.

use crossterm::event::{KeyCode, KeyEvent};

use crate::settings::{SettingsRow, SettingsSection, SettingsSnapshot};
use crate::settings_stage::Staged;

/// Which list the arrow keys move in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Sections,
    Fields,
    Staged,
}

impl Pane {
    fn next(self) -> Pane {
        match self {
            Pane::Sections => Pane::Fields,
            Pane::Fields => Pane::Staged,
            Pane::Staged => Pane::Sections,
        }
    }

    fn prev(self) -> Pane {
        match self {
            Pane::Sections => Pane::Staged,
            Pane::Fields => Pane::Sections,
            Pane::Staged => Pane::Fields,
        }
    }
}

/// What a key asks the caller to do. `Stage` only updates the staged list; the caller runs the dry-run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenEvent {
    None,
    Close,
    Reload,
    Stage {
        scope: String,
        key: String,
        original: String,
        value: String,
    },
    Apply(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SettingsScreen {
    pub snapshot: Option<SettingsSnapshot>,
    pub staged: Staged,
    pub section: usize,
    pub field: usize,
    pub staged_sel: usize,
    pub pane: Pane,
    pub editing: Option<String>,
}

/// The index after one step down or up, held inside a list of `len` items.
fn step(index: usize, len: usize, down: bool) -> usize {
    if down {
        (index + 1).min(len.saturating_sub(1))
    } else {
        index.saturating_sub(1)
    }
}

fn clamp(index: usize, len: usize) -> usize {
    index.min(len.saturating_sub(1))
}

impl SettingsScreen {
    /// A screen with no snapshot yet; the frame shows a loading line until `load`.
    pub fn open() -> SettingsScreen {
        SettingsScreen {
            snapshot: None,
            staged: Staged::default(),
            section: 0,
            field: 0,
            staged_sel: 0,
            pane: Pane::Sections,
            editing: None,
        }
    }

    /// Replaces the snapshot, pulls every selection back into range and drops an open edit.
    pub fn load(&mut self, snapshot: SettingsSnapshot) {
        self.section = clamp(self.section, snapshot.sections.len());
        let rows = snapshot
            .sections
            .get(self.section)
            .map_or(0, |s| s.rows.len());
        self.field = clamp(self.field, rows);
        self.staged_sel = clamp(self.staged_sel, self.staged.edits().len());
        self.editing = None;
        self.snapshot = Some(snapshot);
    }

    pub fn current_section(&self) -> Option<&SettingsSection> {
        self.snapshot
            .as_ref()
            .and_then(|s| s.sections.get(self.section))
    }

    pub fn current_rows(&self) -> &[SettingsRow] {
        self.current_section().map_or(&[], |s| s.rows.as_slice())
    }

    pub fn selected_row(&self) -> Option<&SettingsRow> {
        self.current_rows().get(self.field)
    }

    pub fn staged(&self) -> &Staged {
        &self.staged
    }

    /// The caller records dry-run and apply results here.
    pub fn staged_mut(&mut self) -> &mut Staged {
        &mut self.staged
    }

    pub fn editing(&self) -> Option<&str> {
        self.editing.as_deref()
    }

    pub fn pane(&self) -> Pane {
        self.pane
    }

    /// The last dry-run refusal for the selected row's staged edit, if it has one.
    pub fn refusal_for_selected(&self) -> Option<&str> {
        self.selected_row()
            .and_then(|r| self.staged.refusal_for(&r.scope, &r.key))
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> ScreenEvent {
        match self.editing {
            Some(_) => self.edit_key(key.code),
            None => self.nav_key(key.code),
        }
    }

    fn edit_key(&mut self, code: KeyCode) -> ScreenEvent {
        match code {
            KeyCode::Char(c) => {
                if let Some(buffer) = self.editing.as_mut() {
                    buffer.push(c);
                }
                ScreenEvent::None
            }
            KeyCode::Backspace => {
                if let Some(buffer) = self.editing.as_mut() {
                    buffer.pop();
                }
                ScreenEvent::None
            }
            KeyCode::Enter => self.commit_edit(),
            KeyCode::Esc => {
                self.editing = None;
                ScreenEvent::None
            }
            // The buffer has no cursor: edits happen at its end, so Left and Right move nothing.
            _ => ScreenEvent::None,
        }
    }

    fn commit_edit(&mut self) -> ScreenEvent {
        let value = self.editing.take();
        let row = self.selected_row().cloned();
        match (value, row) {
            (Some(value), Some(row)) => {
                self.staged.stage(&row.scope, &row.key, &row.value, &value);
                ScreenEvent::Stage {
                    scope: row.scope,
                    key: row.key,
                    original: row.value,
                    value,
                }
            }
            _ => ScreenEvent::None,
        }
    }

    fn nav_key(&mut self, code: KeyCode) -> ScreenEvent {
        match code {
            KeyCode::Tab => self.pane = self.pane.next(),
            KeyCode::BackTab => self.pane = self.pane.prev(),
            KeyCode::Up => self.move_selection(false),
            KeyCode::Down => self.move_selection(true),
            KeyCode::Enter if self.pane == Pane::Fields => {
                self.editing = self.selected_row().map(|r| r.value.clone());
            }
            KeyCode::Char('x') if self.pane == Pane::Staged => {
                self.staged.unstage(self.staged_sel);
                self.staged_sel = clamp(self.staged_sel, self.staged.edits().len());
            }
            KeyCode::Char('a')
                if self.pane == Pane::Staged && self.staged_sel < self.staged.edits().len() =>
            {
                return ScreenEvent::Apply(self.staged_sel);
            }
            KeyCode::Char('r') => return ScreenEvent::Reload,
            KeyCode::Esc => return ScreenEvent::Close,
            _ => {}
        }
        ScreenEvent::None
    }

    fn move_selection(&mut self, down: bool) {
        match self.pane {
            Pane::Sections => {
                let len = self.snapshot.as_ref().map_or(0, |s| s.sections.len());
                let next = step(self.section, len, down);
                if next != self.section {
                    self.section = next;
                    self.field = 0;
                }
            }
            Pane::Fields => self.field = step(self.field, self.current_rows().len(), down),
            Pane::Staged => {
                self.staged_sel = step(self.staged_sel, self.staged.edits().len(), down)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn row(scope: &str, key: &str, value: &str) -> SettingsRow {
        SettingsRow {
            section: "s".into(),
            scope: scope.into(),
            key: key.into(),
            value: value.into(),
            source: "f".into(),
            tracked: true,
            pat_only: false,
        }
    }

    fn section(id: &str, rows: Vec<SettingsRow>) -> SettingsSection {
        SettingsSection {
            id: id.into(),
            label: id.into(),
            rows,
        }
    }

    fn snapshot() -> SettingsSnapshot {
        SettingsSnapshot {
            sections: vec![
                section(
                    "lanes",
                    vec![row("repo", "lanes.max", "4"), row("repo", "lanes.min", "1")],
                ),
                section("spend", vec![row("machine", "spend.cap", "20")]),
                section("crew", vec![row("repo", "crew.seats", "2")]),
            ],
        }
    }

    fn loaded() -> SettingsScreen {
        let mut screen = SettingsScreen::open();
        screen.load(snapshot());
        screen
    }

    #[test]
    fn open_has_no_snapshot_and_no_section() {
        let screen = SettingsScreen::open();
        assert!(screen.snapshot.is_none());
        assert!(screen.current_section().is_none());
        assert!(screen.current_rows().is_empty());
    }

    #[test]
    fn tab_cycles_three_panes() {
        let mut screen = loaded();
        assert_eq!(screen.pane(), Pane::Sections);
        screen.handle_key(key(KeyCode::Tab));
        assert_eq!(screen.pane(), Pane::Fields);
        screen.handle_key(key(KeyCode::Tab));
        assert_eq!(screen.pane(), Pane::Staged);
        screen.handle_key(key(KeyCode::Tab));
        assert_eq!(screen.pane(), Pane::Sections);
        screen.handle_key(key(KeyCode::BackTab));
        assert_eq!(screen.pane(), Pane::Staged);
    }

    #[test]
    fn down_stops_at_the_last_field() {
        let mut screen = loaded();
        screen.handle_key(key(KeyCode::Tab));
        screen.handle_key(key(KeyCode::Down));
        screen.handle_key(key(KeyCode::Down));
        assert_eq!(screen.field, 1);
        assert_eq!(
            screen.selected_row().map(|r| r.key.as_str()),
            Some("lanes.min")
        );
    }

    #[test]
    fn enter_typing_enter_returns_stage_with_the_exact_values() {
        let mut screen = loaded();
        screen.handle_key(key(KeyCode::Tab));
        assert_eq!(screen.handle_key(key(KeyCode::Enter)), ScreenEvent::None);
        assert_eq!(screen.editing(), Some("4"));
        screen.handle_key(key(KeyCode::Backspace));
        screen.handle_key(key(KeyCode::Char('8')));
        let event = screen.handle_key(key(KeyCode::Enter));
        assert_eq!(
            event,
            ScreenEvent::Stage {
                scope: "repo".into(),
                key: "lanes.max".into(),
                original: "4".into(),
                value: "8".into(),
            }
        );
        assert_eq!(screen.editing(), None);
        assert_eq!(screen.staged().edits().len(), 1);
        assert_eq!(screen.staged().edits()[0].value, "8");
    }

    #[test]
    fn esc_during_an_edit_returns_none_and_stages_nothing() {
        let mut screen = loaded();
        screen.handle_key(key(KeyCode::Tab));
        screen.handle_key(key(KeyCode::Enter));
        screen.handle_key(key(KeyCode::Char('9')));
        assert_eq!(screen.handle_key(key(KeyCode::Esc)), ScreenEvent::None);
        assert_eq!(screen.editing(), None);
        assert!(screen.staged().edits().is_empty());
    }

    #[test]
    fn a_in_the_staged_pane_returns_apply_with_the_index() {
        let mut screen = loaded();
        screen.staged_mut().stage("repo", "lanes.max", "4", "8");
        screen
            .staged_mut()
            .stage("machine", "spend.cap", "20", "30");
        screen.handle_key(key(KeyCode::BackTab));
        screen.handle_key(key(KeyCode::Down));
        assert_eq!(
            screen.handle_key(key(KeyCode::Char('a'))),
            ScreenEvent::Apply(1)
        );
    }

    #[test]
    fn a_with_nothing_staged_returns_none() {
        let mut screen = loaded();
        screen.handle_key(key(KeyCode::BackTab));
        assert_eq!(
            screen.handle_key(key(KeyCode::Char('a'))),
            ScreenEvent::None
        );
    }

    #[test]
    fn x_unstages_the_selected_edit() {
        let mut screen = loaded();
        screen.staged_mut().stage("repo", "lanes.max", "4", "8");
        screen
            .staged_mut()
            .stage("machine", "spend.cap", "20", "30");
        screen.handle_key(key(KeyCode::BackTab));
        screen.handle_key(key(KeyCode::Down));
        screen.handle_key(key(KeyCode::Char('x')));
        assert_eq!(screen.staged().edits().len(), 1);
        assert_eq!(screen.staged().edits()[0].key, "lanes.max");
        assert_eq!(screen.staged_sel, 0);
    }

    #[test]
    fn esc_outside_an_edit_closes_and_r_reloads() {
        let mut screen = loaded();
        assert_eq!(
            screen.handle_key(key(KeyCode::Char('r'))),
            ScreenEvent::Reload
        );
        assert_eq!(screen.handle_key(key(KeyCode::Esc)), ScreenEvent::Close);
    }

    #[test]
    fn load_keeps_selection_in_range_when_sections_shrink() {
        let mut screen = loaded();
        screen.section = 2;
        screen.field = 3;
        screen.load(SettingsSnapshot {
            sections: vec![section("lanes", vec![row("repo", "lanes.max", "4")])],
        });
        assert_eq!(screen.section, 0);
        assert_eq!(screen.field, 0);
    }

    #[test]
    fn the_selected_rows_refusal_is_the_last_dry_run_line() {
        let mut screen = loaded();
        screen.staged_mut().stage("repo", "lanes.max", "4", "99");
        screen.staged_mut().record_dry_run(
            "repo",
            "lanes.max",
            Some(2),
            "checking\nlanes.max must be at most 16\n",
        );
        assert_eq!(
            screen.refusal_for_selected(),
            Some("lanes.max must be at most 16")
        );
    }
}
