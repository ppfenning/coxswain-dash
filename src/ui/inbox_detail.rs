//! The inbox detail view: one `InboxEntry` in full, in a bordered block. The reason is wrapped
//! by words and an over-long token is broken, so every character of it is drawn.

use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::feed::InboxEntry;
use crate::theme::Theme;

const LABEL_WIDTH: usize = 9;
const NONE: &str = "\u{2014}";

/// Wraps `text` to `width` columns by words. A token longer than `width` is cut into `width`
/// chunks. Existing newlines start a new line. No character is dropped but spaces at a wrap.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    text.split('\n')
        .flat_map(|line| wrap_line(line, width))
        .collect()
}

fn wrap_line(line: &str, width: usize) -> Vec<String> {
    let wrapped = line
        .split_whitespace()
        .fold(Vec::<String>::new(), |lines, word| {
            let chars: Vec<char> = word.chars().collect();
            let chunks = chars.chunks(width).map(|c| c.iter().collect::<String>());
            let (open, closed) = match lines.split_last() {
                Some((last, closed)) => (last.as_str(), closed),
                None => ("", &lines[..]),
            };
            let joined = format!("{open} {word}");
            if chars.len() <= width && !open.is_empty() && joined.chars().count() <= width {
                closed.iter().cloned().chain([joined]).collect()
            } else {
                lines.iter().cloned().chain(chunks).collect()
            }
        });
    if wrapped.is_empty() {
        vec![String::new()]
    } else {
        wrapped
    }
}

/// `label` then `value` wrapped under it; a missing value is a dim dash.
fn field<'a>(label: &str, value: Option<&str>, width: usize, theme: &Theme) -> Vec<Line<'a>> {
    let label = format!("{label:<LABEL_WIDTH$}");
    let indent = " ".repeat(LABEL_WIDTH);
    match value {
        None => vec![Line::from(vec![
            Span::styled(label, Style::default().fg(theme.accent)),
            Span::styled(NONE, Style::default().fg(theme.dim)),
        ])],
        Some(text) => wrap_text(text, width.saturating_sub(LABEL_WIDTH))
            .into_iter()
            .enumerate()
            .map(|(i, part)| {
                let lead = if i == 0 {
                    label.clone()
                } else {
                    indent.clone()
                };
                Line::from(vec![
                    Span::styled(lead, Style::default().fg(theme.accent)),
                    Span::raw(part),
                ])
            })
            .collect(),
    }
}

/// The key line: a stale entry leads with the one-step dismiss.
fn key_line(stale: bool) -> &'static str {
    if stale {
        "d dismiss  a acknowledge  c copy  p preview  o open target  Esc back"
    } else {
        "a acknowledge  d dismiss  c copy  p preview  o open target  Esc back"
    }
}

/// The key line broken between keys, never inside one, into rows of at most `width` columns.
fn key_rows(line: &str, width: usize) -> Vec<String> {
    line.split("  ")
        .fold(Vec::new(), |rows, key| match rows.split_last() {
            Some((last, rest)) if last.chars().count() + 2 + key.chars().count() <= width => {
                [rest.to_vec(), vec![format!("{last}  {key}")]].concat()
            }
            _ => [rows, vec![key.to_string()]].concat(),
        })
}

/// The line above the full text of a stale entry, and a blank line after it. Empty when fresh.
fn stale_lines<'a>(stale: bool, theme: &Theme) -> Vec<Line<'a>> {
    if stale {
        vec![
            Line::styled(
                "stale: the target has moved on",
                Style::default().fg(theme.dim),
            ),
            Line::from(""),
        ]
    } else {
        Vec::new()
    }
}

/// Draws `entry` in `area`: a stale line when `stale`, kind, target, the whole reason, when it
/// was raised and its run, then the keys that act on it.
pub fn render(f: &mut Frame, area: Rect, entry: &InboxEntry, stale: bool, theme: &Theme) {
    let base = Style::default().fg(theme.fg).bg(theme.bg);
    let block = Block::default()
        .title("Inbox")
        .borders(Borders::ALL)
        .style(base);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let width = usize::from(inner.width);
    let reason = wrap_text(&entry.reason, width)
        .into_iter()
        .map(Line::from)
        .collect::<Vec<_>>();
    let lines: Vec<Line> = [
        stale_lines(stale, theme),
        field("kind", Some(&entry.kind), width, theme),
        field("target", Some(&entry.target), width, theme),
        vec![Line::from("")],
        vec![Line::styled("reason", Style::default().fg(theme.accent))],
        reason,
        vec![Line::from("")],
        field("raised", entry.at.as_deref(), width, theme),
        field("run", entry.run.as_deref(), width, theme),
        vec![Line::from("")],
        key_rows(key_line(stale), width)
            .into_iter()
            .map(|part| Line::styled(part, Style::default().fg(theme.dim)))
            .collect(),
    ]
    .concat();
    f.render_widget(Paragraph::new(lines).style(base), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ThemeId;
    use ratatui::{Terminal, backend::TestBackend};

    fn entry(at: Option<&str>, run: Option<&str>) -> InboxEntry {
        InboxEntry {
            kind: "approved".to_string(),
            target: "demo-task".to_string(),
            reason: "approved but not landed: cox land --run run-0042 --task demo-task \
                     --repo /home/user/repos/demo/services/gateway/internal/adapters/handlers \
                     --apply"
                .to_string(),
            at: at.map(str::to_string),
            run: run.map(str::to_string),
            initiative: None,
            stale: None,
        }
    }

    fn draw(
        entry: &InboxEntry,
        stale: bool,
        theme: &Theme,
        w: u16,
        h: u16,
    ) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("terminal");
        terminal
            .draw(|f| render(f, f.area(), entry, stale, theme))
            .expect("draw should not fail");
        terminal
    }

    fn rows(terminal: &Terminal<TestBackend>) -> Vec<String> {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn wrap_text_breaks_a_long_token_and_keeps_every_character() {
        let lines = wrap_text("ab cdefghijkl mn", 5);
        assert_eq!(lines, ["ab", "cdefg", "hijkl", "mn"]);
    }

    #[test]
    fn a_narrow_render_keeps_the_whole_land_command() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let e = entry(Some("2026-11-03T04:12:00Z"), Some("run-0042"));
        let drawn: String = rows(&draw(&e, false, &theme, 40, 24)).concat();
        let bare: String = drawn
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '\u{2502}')
            .collect();
        let want: String = e.reason.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(bare.contains(&want), "buffer was {bare:?}");
        assert!(!bare.contains('\u{2026}'));
    }

    #[test]
    fn an_entry_with_no_at_or_run_shows_two_dim_dashes() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let terminal = draw(&entry(None, None), false, &theme, 60, 20);
        let buffer = terminal.backend().buffer();
        let dashes: Vec<_> = (0..buffer.area.height)
            .flat_map(|y| (0..buffer.area.width).map(move |x| (x, y)))
            .filter(|pos| buffer[*pos].symbol() == NONE)
            .collect();
        assert_eq!(dashes.len(), 2);
        assert!(dashes.iter().all(|pos| buffer[*pos].fg == theme.dim));
    }

    #[test]
    fn a_full_entry_shows_when_it_was_raised_and_its_run() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let e = entry(Some("2026-11-03T04:12:00Z"), Some("run-0042"));
        let text = rows(&draw(&e, false, &theme, 80, 20)).join("\n");
        assert!(text.contains("2026-11-03T04:12:00Z"));
        assert!(text.contains("run-0042"));
        assert!(
            text.contains("a acknowledge  d dismiss  c copy  p preview  o open target  Esc back")
        );
    }

    #[test]
    fn key_rows_break_between_keys_and_never_inside_one() {
        let line = key_line(false);
        assert_eq!(key_rows(line, 70), vec![line.to_string()]);
        assert_eq!(
            key_rows(line, 48),
            vec![
                "a acknowledge  d dismiss  c copy  p preview",
                "o open target  Esc back"
            ]
        );
    }

    #[test]
    fn key_line_leads_with_dismiss_only_when_stale() {
        assert_eq!(
            key_line(true),
            "d dismiss  a acknowledge  c copy  p preview  o open target  Esc back"
        );
        assert_eq!(
            key_line(false),
            "a acknowledge  d dismiss  c copy  p preview  o open target  Esc back"
        );
    }

    #[test]
    fn a_fresh_entry_has_no_stale_line() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let e = entry(Some("2026-11-03T04:12:00Z"), Some("run-0042"));
        let text = rows(&draw(&e, false, &theme, 50, 18)).join("\n");
        assert!(!text.contains("stale"), "buffer was {text:?}");
        assert!(!text.contains("moved on"));
        assert!(text.contains("a acknowledge  d dismiss  c copy  p preview"));
    }

    #[test]
    fn a_stale_entry_shows_a_dim_stale_line_above_the_text_and_leads_with_dismiss() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let e = entry(Some("2026-11-03T04:12:00Z"), Some("run-0042"));
        let terminal = draw(&e, true, &theme, 50, 18);
        let lines = rows(&terminal);
        let stale_row = lines
            .iter()
            .position(|r| r.contains("the target has moved on"))
            .expect("stale line is drawn");
        let kind_row = lines.iter().position(|r| r.contains("kind")).expect("kind");
        assert!(stale_row < kind_row);
        assert_eq!(
            terminal.backend().buffer()[(1, stale_row as u16)].fg,
            theme.dim
        );
        assert!(
            lines
                .iter()
                .any(|r| r.contains("\u{2502}d dismiss  a acknowledge"))
        );
    }

    #[test]
    fn renders_inbox_detail_snapshot_in_the_regatta_theme() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let e = entry(Some("2026-11-03T04:12:00Z"), Some("run-0042"));
        insta::assert_snapshot!(draw(&e, false, &theme, 50, 18).backend().to_string());
    }

    #[test]
    fn renders_inbox_detail_snapshot_in_the_harbor_light_theme() {
        let theme = crate::theme::resolve_for(ThemeId::HarborLight, Some("truecolor"));
        let e = entry(Some("2026-11-03T04:12:00Z"), Some("run-0042"));
        insta::assert_snapshot!(draw(&e, false, &theme, 50, 18).backend().to_string());
    }

    #[test]
    fn renders_a_stale_inbox_detail_in_the_regatta_theme() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let e = entry(Some("2026-11-03T04:12:00Z"), Some("run-0042"));
        insta::assert_snapshot!(draw(&e, true, &theme, 50, 18).backend().to_string());
    }

    #[test]
    fn renders_a_stale_inbox_detail_in_the_harbor_light_theme() {
        let theme = crate::theme::resolve_for(ThemeId::HarborLight, Some("truecolor"));
        let e = entry(Some("2026-11-03T04:12:00Z"), Some("run-0042"));
        insta::assert_snapshot!(draw(&e, true, &theme, 50, 18).backend().to_string());
    }
}
