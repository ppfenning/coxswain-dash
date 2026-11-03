//! The inbox detail view: one inbox entry in full. The reason is wrapped, never clipped, so a long
//! command path stays readable. Key handling lives in `input.rs`; this module only draws.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::feed::InboxEntry;
use crate::theme::Theme;

/// Width of the label column in front of each header value.
const LABEL_WIDTH: usize = 8;

fn base_style(theme: &Theme) -> Style {
    Style::default().fg(theme.fg).bg(theme.bg)
}

fn dim_style(theme: &Theme) -> Style {
    Style::default().fg(theme.dim).bg(theme.bg)
}

/// Draws `entry` in `area`: a header of kind, target, raised-at and run, the full reason, and a
/// key line on the last row.
pub fn render(f: &mut Frame, area: Rect, entry: &InboxEntry, theme: &Theme) {
    let block = Block::default()
        .title("Inbox entry")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border).bg(theme.bg))
        .style(base_style(theme));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);
    let body = body_lines(entry, usize::from(rows[0].width), theme);
    f.render_widget(Paragraph::new(body).style(base_style(theme)), rows[0]);
    f.render_widget(
        Paragraph::new(key_line(theme)).style(base_style(theme)),
        rows[1],
    );
}

/// Every line of the body, already wrapped to `width`, so none needs the widget to wrap or clip.
fn body_lines(entry: &InboxEntry, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let header = [
        field("Kind", Some(&entry.kind), width, theme),
        field("Target", Some(&entry.target), width, theme),
        field("Raised", entry.at.as_deref(), width, theme),
        field("Run", entry.run.as_deref(), width, theme),
    ]
    .concat();
    let reason = wrap_text(&entry.reason, width)
        .into_iter()
        .map(|text| Line::styled(text, base_style(theme)));
    header
        .into_iter()
        .chain([Line::raw(""), Line::styled("Reason", dim_style(theme))])
        .chain(reason)
        .collect()
}

/// A labelled value, wrapped in the column after the label. `None` is a dim dash.
fn field(label: &str, value: Option<&str>, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let value_width = width.saturating_sub(LABEL_WIDTH);
    match value {
        None => vec![Line::from(vec![
            Span::styled(format!("{label:<LABEL_WIDTH$}"), dim_style(theme)),
            Span::styled("-", dim_style(theme)),
        ])],
        Some(text) => wrap_text(text, value_width)
            .into_iter()
            .enumerate()
            .map(|(i, part)| {
                let head = if i == 0 { label } else { "" };
                Line::from(vec![
                    Span::styled(format!("{head:<LABEL_WIDTH$}"), dim_style(theme)),
                    Span::styled(part, base_style(theme)),
                ])
            })
            .collect(),
    }
}

fn key_line(theme: &Theme) -> Line<'static> {
    let key = Style::default().fg(theme.accent).bg(theme.bg);
    Line::from(vec![
        Span::styled("a", key),
        Span::styled(" acknowledge  ", dim_style(theme)),
        Span::styled("d", key),
        Span::styled(" dismiss  ", dim_style(theme)),
        Span::styled("Esc", key),
        Span::styled(" back", dim_style(theme)),
    ])
}

/// Wraps `text` to `width` columns by words, one input line at a time. A word longer than the
/// width is broken at the width, so every non-space character survives. A width of 0 counts as 1.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let wrapped: Vec<String> = text
        .lines()
        .flat_map(|line| wrap_line(line, width))
        .collect();
    if wrapped.is_empty() {
        vec![String::new()]
    } else {
        wrapped
    }
}

fn wrap_line(line: &str, width: usize) -> Vec<String> {
    let lines = line
        .split_whitespace()
        .flat_map(|word| break_word(word, width))
        .fold(Vec::<String>::new(), |lines, piece| {
            match lines.split_last() {
                Some((last, rest)) if last.chars().count() + 1 + piece.chars().count() <= width => {
                    rest.iter()
                        .cloned()
                        .chain([format!("{last} {piece}")])
                        .collect()
                }
                _ => lines.into_iter().chain([piece]).collect(),
            }
        });
    if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    }
}

fn break_word(word: &str, width: usize) -> Vec<String> {
    word.chars()
        .collect::<Vec<char>>()
        .chunks(width)
        .map(|chunk| chunk.iter().collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ThemeId;
    use ratatui::{Terminal, backend::TestBackend};

    const LAND_REASON: &str = "approved but not landed on lane-1: cox land --run run-20261103-1 \
        --task inbox-detail-view --repo /home/operator/repos/demo --apply";

    fn entry(reason: &str, at: Option<&str>, run: Option<&str>) -> InboxEntry {
        InboxEntry {
            kind: "approved-not-landed".to_string(),
            target: "demo/inbox-detail-view".to_string(),
            reason: reason.to_string(),
            at: at.map(str::to_string),
            run: run.map(str::to_string),
            initiative: Some("demo".to_string()),
            stale: None,
        }
    }

    fn drawn(entry: &InboxEntry, id: ThemeId, width: u16, height: u16) -> Terminal<TestBackend> {
        let theme = crate::theme::resolve_for(id, Some("truecolor"));
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|f| render(f, f.area(), entry, &theme))
            .expect("draw should not fail");
        terminal
    }

    fn row_text(terminal: &Terminal<TestBackend>, y: u16) -> String {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol().to_string())
            .collect()
    }

    #[test]
    fn wrap_text_keeps_every_character_of_a_long_token() {
        let text = "run /home/operator/repos/demo/worktrees/a-very-long-unbroken-path --apply";
        let lines = wrap_text(text, 12);
        assert!(lines.iter().all(|l| l.chars().count() <= 12), "{lines:?}");
        let squeeze = |s: &str| s.split_whitespace().collect::<String>();
        assert_eq!(squeeze(&lines.join(" ")), squeeze(text));
    }

    #[test]
    fn a_narrow_render_keeps_the_whole_land_command() {
        let command = "cox land --run run-20261103-1 --task inbox-detail-view \
            --repo /home/operator/repos/demo/worktrees/inbox-detail-view/detail-c --apply";
        let reason = format!("approved but not landed on lane-1: {command}");
        let terminal = drawn(&entry(&reason, None, None), ThemeId::Regatta, 30, 24);
        let inside: String = (0..24)
            .flat_map(|y| {
                let row: Vec<char> = row_text(&terminal, y).chars().collect();
                row[1..row.len() - 1].to_vec()
            })
            .filter(|c| !c.is_whitespace())
            .collect();
        let wanted: String = command.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(inside.contains(&wanted), "buffer was {inside:?}");
        assert!(!inside.contains('…'));
    }

    #[test]
    fn a_missing_at_and_run_show_dim_dashes() {
        let terminal = drawn(&entry("reason", None, None), ThemeId::Regatta, 60, 14);
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let buffer = terminal.backend().buffer();
        for label in ["Raised", "Run"] {
            let y = (0..14)
                .find(|y| row_text(&terminal, *y).contains(label))
                .expect("label row");
            let x = 1 + LABEL_WIDTH as u16;
            assert_eq!(buffer[(x, y)].symbol(), "-", "{label}");
            assert_eq!(buffer[(x, y)].fg, theme.dim, "{label}");
        }
    }

    #[test]
    fn the_key_line_names_acknowledge_dismiss_and_back() {
        let terminal = drawn(&entry("reason", None, None), ThemeId::Regatta, 60, 14);
        let last = row_text(&terminal, 12);
        assert!(
            last.contains("a acknowledge  d dismiss  Esc back"),
            "{last:?}"
        );
    }

    #[test]
    fn renders_inbox_detail_snapshot_in_the_regatta_theme() {
        let e = entry(
            LAND_REASON,
            Some("2026-11-03T09:30:00+00:00"),
            Some("run-20261103-1"),
        );
        insta::assert_snapshot!(drawn(&e, ThemeId::Regatta, 60, 16).backend().to_string());
    }

    #[test]
    fn renders_inbox_detail_snapshot_in_the_harbor_light_theme() {
        let e = entry(
            LAND_REASON,
            Some("2026-11-03T09:30:00+00:00"),
            Some("run-20261103-1"),
        );
        insta::assert_snapshot!(
            drawn(&e, ThemeId::HarborLight, 60, 16)
                .backend()
                .to_string()
        );
    }
}
