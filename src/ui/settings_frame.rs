//! Draws the settings screen: the section list on the left, the current section's fields on
//! the right with the staged edits under them, and a hint row of the focused pane's keys.

use std::iter::once;

use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::settings::{SettingsRow, provenance};
use crate::settings_screen::{Pane, SettingsScreen};
use crate::settings_stage::{StagedEdit, argv, command_string};
use crate::theme::Theme;

/// Columns between two cells of a field row.
const GAP: usize = 2;
/// Width of the `> ` selection marker that starts a row.
const MARKER: usize = 2;
/// Width of `machine-local`, the longer of the two provenance strings.
const PROVENANCE_WIDTH: usize = 13;
const PAT_ONLY: &str = "Pat-only";
const CURSOR: char = '▏';
const KEY_CAP: usize = 26;
const VALUE_CAP: usize = 24;
const SOURCE_CAP: usize = 28;

/// Width of the section column for a frame `total` columns wide.
fn left_width(total: u16) -> u16 {
    (total / 4).clamp(12, 22).min(total)
}

/// Cuts `text` to `width` characters, ending in an ellipsis when it was cut.
fn clip(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        text.to_string()
    } else if width == 0 {
        String::new()
    } else {
        text.chars().take(width - 1).chain(once('…')).collect()
    }
}

/// Like `clip`, but keeps the end of `text`, so an edit cursor stays visible.
fn clip_tail(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        text.to_string()
    } else if width == 0 {
        String::new()
    } else {
        once('…')
            .chain(text.chars().skip(count - (width - 1)))
            .collect()
    }
}

fn pad(text: &str, width: usize) -> String {
    format!("{text:<width$}")
}

/// Splits `text` into chunks of at most `width` characters. Nothing is trimmed or dropped.
fn wrap_chars(text: &str, width: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    chars
        .chunks(width.max(1))
        .map(|chunk| chunk.iter().collect())
        .collect()
}

/// Greedy word wrap to `width`. A word longer than `width` is split. Blank text gives no lines.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    text.split_whitespace()
        .flat_map(|word| wrap_chars(word, width))
        .fold(Vec::new(), |lines: Vec<String>, piece| {
            match lines.split_last() {
                Some((last, rest)) if last.chars().count() + 1 + piece.chars().count() <= width => {
                    rest.iter()
                        .cloned()
                        .chain(once(format!("{last} {piece}")))
                        .collect()
                }
                _ => lines.into_iter().chain(once(piece)).collect(),
            }
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Col {
    Key,
    Value,
    Source,
    Provenance,
    Mark,
}

/// Column widths shared by every row of a section, so the cells line up.
struct Widths {
    key: usize,
    value: usize,
    source: usize,
}

fn longest(lens: impl Iterator<Item = usize>, cap: usize) -> usize {
    lens.max().unwrap_or(0).min(cap)
}

fn widths(rows: &[SettingsRow]) -> Widths {
    Widths {
        key: longest(rows.iter().map(|r| r.key.chars().count()), KEY_CAP),
        value: longest(rows.iter().map(|r| r.value.chars().count()), VALUE_CAP),
        source: longest(rows.iter().map(|r| r.source.chars().count()), SOURCE_CAP),
    }
}

/// The cells of one field row inside `width` columns. A narrow width drops the source, then
/// the provenance, then the `Pat-only` mark. The value is shortened only after all three.
fn field_cells(row: &SettingsRow, w: &Widths, width: usize) -> Vec<(Col, String)> {
    let key_w = w.key.min(width);
    let base = key_w + GAP + w.value;
    let cost = |source: bool, prov: bool, mark: bool| {
        usize::from(source) * (GAP + w.source)
            + usize::from(prov) * (GAP + PROVENANCE_WIDTH)
            + usize::from(mark) * (GAP + PAT_ONLY.len())
    };
    let (show_source, show_prov, show_mark) = [
        (true, true, row.pat_only),
        (false, true, row.pat_only),
        (false, false, row.pat_only),
        (false, false, false),
    ]
    .into_iter()
    .find(|&(s, p, m)| base + cost(s, p, m) <= width)
    .unwrap_or((false, false, false));
    let value_w = if base <= width {
        w.value
    } else {
        width.saturating_sub(key_w + GAP).max(1)
    };
    let cells: Vec<(Col, String, usize)> = [
        (Col::Key, clip(&row.key, key_w), key_w),
        (Col::Value, clip(&row.value, value_w), value_w),
    ]
    .into_iter()
    .chain(show_source.then(|| (Col::Source, clip(&row.source, w.source), w.source)))
    .chain(show_prov.then(|| {
        (
            Col::Provenance,
            provenance(row).to_string(),
            PROVENANCE_WIDTH,
        )
    }))
    .chain(show_mark.then(|| (Col::Mark, PAT_ONLY.to_string(), PAT_ONLY.len())))
    .collect();
    let last = cells.len().saturating_sub(1);
    cells
        .into_iter()
        .enumerate()
        .map(|(i, (col, text, cell_w))| {
            if i == last {
                (col, text)
            } else {
                (col, pad(&text, cell_w + GAP))
            }
        })
        .collect()
}

/// The keys of the focused pane. While a field is being edited the edit keys replace them.
fn hint(pane: Pane, editing: bool) -> &'static str {
    match (editing, pane) {
        (true, _) => "type to edit   Backspace delete   Enter stage   Esc cancel",
        (false, Pane::Sections) => "Up/Down section   Tab next pane   r reload   Esc close",
        (false, Pane::Fields) => {
            "Up/Down field   Enter edit   Tab next pane   r reload   Esc close"
        }
        (false, Pane::Staged) => {
            "Up/Down entry   a apply   x unstage   Tab next pane   r reload   Esc close"
        }
    }
}

fn col_style(theme: &Theme, col: Col) -> Style {
    let fg = match col {
        Col::Key => theme.fg,
        Col::Value => theme.accent,
        Col::Source | Col::Provenance => theme.dim,
        Col::Mark => theme.status_waiting,
    };
    Style::default().fg(fg)
}

fn selected_style(theme: &Theme, selected: bool) -> Style {
    if selected {
        Style::default().bg(theme.selected_row)
    } else {
        Style::default()
    }
}

fn marker_span(theme: &Theme, selected: bool, focused: bool) -> Span<'static> {
    let color = if selected && focused {
        theme.border_focus
    } else {
        theme.dim
    };
    Span::styled(
        if selected { "> " } else { "  " },
        Style::default().fg(color),
    )
}

/// One field row. A refusal goes on the row when it fits, else on its own wrapped lines
/// directly under it. It is never cut.
fn field_lines(
    theme: &Theme,
    screen: &SettingsScreen,
    index: usize,
    row: &SettingsRow,
    w: &Widths,
    width: usize,
) -> Vec<Line<'static>> {
    let selected = index == screen.field;
    let focused = screen.pane == Pane::Fields;
    let inner = width.saturating_sub(MARKER);
    let cells = match screen.editing().filter(|_| selected) {
        Some(buffer) => {
            let key_w = w.key.min(inner);
            vec![
                (Col::Key, pad(&clip(&row.key, key_w), key_w + GAP)),
                (
                    Col::Value,
                    clip_tail(
                        &format!("{buffer}{CURSOR}"),
                        inner.saturating_sub(key_w + GAP).max(1),
                    ),
                ),
            ]
        }
        None => field_cells(row, w, inner),
    };
    let used = MARKER + cells.iter().map(|(_, t)| t.chars().count()).sum::<usize>();
    let refusal = screen.staged().refusal_for(&row.scope, &row.key);
    let beside = refusal.filter(|r| used + GAP + r.chars().count() <= width);
    let warn = Style::default().fg(theme.quarantined);
    let spans: Vec<Span<'static>> = once(marker_span(theme, selected, focused))
        .chain(
            cells
                .into_iter()
                .map(|(col, text)| Span::styled(text, col_style(theme, col))),
        )
        .chain(
            beside
                .into_iter()
                .flat_map(|r| [Span::raw("  "), Span::styled(r.to_string(), warn)]),
        )
        .collect();
    let line = Line::from(spans).style(selected_style(theme, selected));
    let below = refusal
        .filter(|_| beside.is_none())
        .into_iter()
        .flat_map(|r| wrap(r, inner))
        .map(move |text| Line::from(vec![Span::raw("  "), Span::styled(text, warn)]));
    once(line).chain(below).collect()
}

/// The diff line's color: added lines done, removed lines warning, the rest dim.
fn diff_style(theme: &Theme, text: &str) -> Style {
    let fg = if text.starts_with('+') {
        theme.status_done
    } else if text.starts_with('-') {
        theme.quarantined
    } else {
        theme.dim
    };
    Style::default().fg(fg)
}

/// One staged edit: its `$` command, its dry-run diff, and its refusal. The edit stays listed
/// whatever the dry-run said.
fn entry_lines(
    theme: &Theme,
    edit: &StagedEdit,
    selected: bool,
    focused: bool,
    width: usize,
) -> Vec<Line<'static>> {
    let inner = width.saturating_sub(MARKER);
    let command = format!("$ {}", command_string(&argv(edit, true)));
    let command_lines = wrap_chars(&command, inner)
        .into_iter()
        .enumerate()
        .map(|(i, text)| {
            let lead = if i == 0 {
                marker_span(theme, selected, focused)
            } else {
                Span::raw("  ")
            };
            let style = if i == 0 {
                selected_style(theme, selected && focused)
            } else {
                Style::default()
            };
            Line::from(vec![
                lead,
                Span::styled(text, Style::default().fg(theme.fg)),
            ])
            .style(style)
        });
    let indent = MARKER * 2;
    let diff_lines = edit
        .diff
        .as_deref()
        .into_iter()
        .flat_map(str::lines)
        .filter(|l| !l.trim().is_empty())
        .map(move |l| {
            Line::from(vec![
                Span::raw("    "),
                Span::styled(clip(l, width.saturating_sub(indent)), diff_style(theme, l)),
            ])
        });
    let warn = Style::default().fg(theme.quarantined);
    let refusal_lines = edit
        .refusal
        .as_deref()
        .into_iter()
        .flat_map(move |r| wrap(r, width.saturating_sub(indent)))
        .map(move |text| Line::from(vec![Span::raw("    "), Span::styled(text, warn)]));
    command_lines
        .chain(diff_lines)
        .chain(refusal_lines)
        .collect()
}

fn staged_lines(theme: &Theme, screen: &SettingsScreen, width: usize) -> Vec<Line<'static>> {
    let focused = screen.pane == Pane::Staged;
    let heading_color = if focused {
        theme.border_focus
    } else {
        theme.accent
    };
    let edits = screen.staged().edits();
    let body: Vec<Line<'static>> = if edits.is_empty() {
        vec![Line::styled(
            "  nothing staged",
            Style::default().fg(theme.dim),
        )]
    } else {
        edits
            .iter()
            .enumerate()
            .flat_map(|(i, edit)| entry_lines(theme, edit, i == screen.staged_sel, focused, width))
            .collect()
    };
    [
        Line::raw(""),
        Line::styled(
            "staged",
            Style::default()
                .fg(heading_color)
                .add_modifier(Modifier::BOLD),
        ),
    ]
    .into_iter()
    .chain(body)
    .collect()
}

fn right_lines(theme: &Theme, screen: &SettingsScreen, width: usize) -> Vec<Line<'static>> {
    let rows = screen.current_rows();
    let w = widths(rows);
    let heading = screen.current_section().map_or("", |s| s.label.as_str());
    let heading_color = if screen.pane == Pane::Fields {
        theme.border_focus
    } else {
        theme.accent
    };
    once(Line::styled(
        clip(heading, width),
        Style::default()
            .fg(heading_color)
            .add_modifier(Modifier::BOLD),
    ))
    .chain(
        rows.iter()
            .enumerate()
            .flat_map(|(i, row)| field_lines(theme, screen, i, row, &w, width)),
    )
    .chain(staged_lines(theme, screen, width))
    .collect()
}

fn section_lines(
    theme: &Theme,
    screen: &SettingsScreen,
    labels: &[String],
    width: usize,
) -> Vec<Line<'static>> {
    let focused = screen.pane == Pane::Sections;
    labels
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let selected = i == screen.section;
            let lead = if selected { "> " } else { "  " };
            let text = pad(&clip(&format!("{lead}{label}"), width), width);
            let style = if selected {
                Style::default()
                    .fg(if focused {
                        theme.border_focus
                    } else {
                        theme.fg
                    })
                    .bg(theme.selected_row)
                    .add_modifier(if focused {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    })
            } else {
                Style::default().fg(theme.fg)
            };
            Line::styled(text, style)
        })
        .collect()
}

/// Draws the settings screen into `area`. Before a snapshot loads it is one loading line.
pub fn render(f: &mut Frame, area: Rect, theme: &Theme, screen: &SettingsScreen) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let Some(snapshot) = screen.snapshot.as_ref() else {
        f.render_widget(
            Paragraph::new(Line::styled(
                "loading settings…",
                Style::default().fg(theme.dim),
            )),
            area,
        );
        return;
    };
    let body_height = area.height - 1;
    let left_w = left_width(area.width);
    let left = Rect::new(area.x, area.y, left_w, body_height);
    let right_x = area.x + left_w + 1;
    let right = Rect::new(
        right_x.min(area.x + area.width),
        area.y,
        area.width.saturating_sub(left_w + 1),
        body_height,
    );
    let hint_rect = Rect::new(area.x, area.y + body_height, area.width, 1);

    let labels: Vec<String> = snapshot.sections.iter().map(|s| s.label.clone()).collect();
    f.render_widget(
        Paragraph::new(section_lines(theme, screen, &labels, usize::from(left_w))),
        left,
    );
    f.render_widget(
        Paragraph::new(right_lines(theme, screen, usize::from(right.width))),
        right,
    );
    f.render_widget(
        Paragraph::new(Line::styled(
            clip(
                hint(screen.pane, screen.editing().is_some()),
                usize::from(area.width),
            ),
            Style::default().fg(theme.dim),
        )),
        hint_rect,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ThemeId;
    use crate::settings::{SettingsSection, SettingsSnapshot, label};
    use crate::settings_stage::Staged;
    use ratatui::{Terminal, backend::TestBackend};

    const LONG_REFUSAL: &str =
        "lanes.max 6 is above the machine ceiling of 5 set in ~/.cox/local.toml";

    fn row(
        section: &str,
        scope: &str,
        key: &str,
        value: &str,
        source: &str,
        tracked: bool,
        pat_only: bool,
    ) -> SettingsRow {
        SettingsRow {
            section: section.into(),
            scope: scope.into(),
            key: key.into(),
            value: value.into(),
            source: source.into(),
            tracked,
            pat_only,
        }
    }

    fn section(id: &str, rows: Vec<SettingsRow>) -> SettingsSection {
        SettingsSection {
            id: id.into(),
            label: label(id),
            rows,
        }
    }

    fn snapshot() -> SettingsSnapshot {
        SettingsSnapshot {
            sections: vec![
                section(
                    "lanes",
                    vec![
                        row(
                            "lanes",
                            "repo",
                            "lanes.max",
                            "4",
                            ".cox/settings.toml",
                            true,
                            false,
                        ),
                        row(
                            "lanes",
                            "machine",
                            "lanes.idle",
                            "2",
                            "~/.cox/local.toml",
                            false,
                            false,
                        ),
                    ],
                ),
                section(
                    "models",
                    vec![
                        row(
                            "models",
                            "repo",
                            "models.tier.fast",
                            "haiku",
                            ".cox/models.toml",
                            true,
                            true,
                        ),
                        row(
                            "models",
                            "machine",
                            "models.default",
                            "sonnet",
                            "~/.cox/local.toml",
                            false,
                            false,
                        ),
                    ],
                ),
                section(
                    "crew",
                    vec![row(
                        "crew",
                        "repo",
                        "crew.seats",
                        "2",
                        ".cox/settings.toml",
                        true,
                        false,
                    )],
                ),
            ],
        }
    }

    fn screen(section: usize, pane: Pane, staged: Staged) -> SettingsScreen {
        SettingsScreen {
            snapshot: Some(snapshot()),
            staged,
            section,
            pane,
            ..SettingsScreen::open()
        }
    }

    fn staged_lanes(code: Option<i32>, output: &str) -> Staged {
        let mut staged = Staged::default();
        staged.stage("repo", "lanes.max", "4", "6");
        staged.record_dry_run("repo", "lanes.max", code, output);
        staged
    }

    fn draw(screen: &SettingsScreen, width: u16, height: u16) -> Terminal<TestBackend> {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|f| render(f, f.area(), &theme, screen))
            .expect("draw should not fail");
        terminal
    }

    /// Every buffer row as plain text.
    fn rows(terminal: &Terminal<TestBackend>) -> Vec<String> {
        let buffer = terminal.backend().buffer();
        let width = usize::from(buffer.area.width);
        buffer
            .content()
            .chunks(width)
            .map(|cells| cells.iter().map(|c| c.symbol()).collect())
            .collect()
    }

    /// The right column of a row, trimmed.
    fn right_column(line: &str, total: u16) -> String {
        line.chars()
            .skip(usize::from(left_width(total)) + 1)
            .collect::<String>()
            .trim()
            .to_string()
    }

    #[test]
    fn wrap_breaks_on_words_and_splits_a_long_word() {
        assert_eq!(
            wrap("alpha beta gamma delta", 11),
            ["alpha beta", "gamma delta"]
        );
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
    }

    #[test]
    fn narrow_widths_drop_source_then_provenance_but_keep_the_value() {
        let rows = snapshot().sections[0].rows.clone();
        let w = widths(&rows);
        let cols = |width| {
            field_cells(&rows[0], &w, width)
                .into_iter()
                .map(|(c, _)| c)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            cols(100),
            [Col::Key, Col::Value, Col::Source, Col::Provenance]
        );
        assert_eq!(cols(30), [Col::Key, Col::Value, Col::Provenance]);
        assert_eq!(cols(14), [Col::Key, Col::Value]);
    }

    #[test]
    fn hints_differ_by_pane_and_while_editing() {
        assert!(hint(Pane::Staged, false).contains("a apply"));
        assert!(hint(Pane::Fields, false).contains("Enter edit"));
        assert!(hint(Pane::Fields, true).contains("Enter stage"));
        assert_ne!(hint(Pane::Sections, false), hint(Pane::Fields, false));
    }

    #[test]
    fn an_edited_field_shows_its_buffer_and_a_cursor() {
        let s = SettingsScreen {
            editing: Some("12".into()),
            ..screen(0, Pane::Fields, Staged::default())
        };
        let text = rows(&draw(&s, 100, 14)).join("\n");
        assert!(text.contains("12▏"), "{text}");
    }

    #[test]
    fn renders_sections_and_fields_with_pat_only_and_machine_local_rows() {
        let terminal = draw(&screen(1, Pane::Fields, Staged::default()), 100, 14);
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn renders_a_staged_change_with_its_diff_and_command() {
        let staged = staged_lanes(
            Some(0),
            "--- .cox/settings.toml\n+++ .cox/settings.toml\n-lanes.max = 4\n+lanes.max = 6\n",
        );
        let terminal = draw(&screen(0, Pane::Staged, staged), 100, 14);
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn renders_a_refusal_beside_the_field_with_the_edit_still_staged() {
        let staged = staged_lanes(Some(2), "error: refused\nabove the ceiling of 5\n");
        let terminal = draw(&screen(0, Pane::Fields, staged), 100, 14);
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn renders_the_loading_state() {
        let terminal = draw(&SettingsScreen::open(), 100, 4);
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn a_refusal_that_does_not_fit_beside_its_row_is_drawn_whole_directly_below_it() {
        for width in [60u16, 100] {
            let staged = staged_lanes(Some(2), &format!("error: refused\n{LONG_REFUSAL}\n"));
            let terminal = draw(&screen(0, Pane::Fields, staged), width, 16);
            let lines = rows(&terminal);
            let at = lines
                .iter()
                .position(|l| right_column(l, width).starts_with("> lanes.max"))
                .expect("the lanes.max row is drawn");
            let below = lines[at + 1..]
                .iter()
                .take(4)
                .map(|l| right_column(l, width))
                .collect::<Vec<_>>()
                .join(" ");
            assert!(
                below.starts_with(LONG_REFUSAL),
                "width {width}: refusal not under its row: {below:?}"
            );
            let all = lines.join("\n");
            assert!(
                all.contains("$ cox settings set"),
                "width {width}: entry dropped"
            );
        }
    }
}
