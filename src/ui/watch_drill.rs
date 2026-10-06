//! Rendering for the watch board: a bordered block titled `Watch` with one row per followed
//! pull request or run. Nothing calls `render` yet, so unused-code warnings are silenced
//! until the wiring phase.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::feed::WatchItem;
use crate::theme::Theme;

/// Width of the kind chip column: `run` is the longest chip.
const CHIP_W: usize = 3;
/// Width of the state column. Longer state words are not cut, they push the time right.
const STATE_W: usize = 8;
/// Cells every row spends on the separators between its five columns.
const GAPS: usize = 4;

/// `status_running` for running or open, `status_done` for merged or done, `status_failed`
/// for failed or closed, `status_waiting` for any other word.
fn state_color(theme: &Theme, state: &str) -> Color {
    match state {
        "running" | "open" => theme.status_running,
        "merged" | "done" => theme.status_done,
        "failed" | "closed" => theme.status_failed,
        _ => theme.status_waiting,
    }
}

/// Cuts `text` to `width` chars, ending in `…` when anything was dropped, and pads with
/// spaces up to `width`.
fn fit(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if len <= width {
        format!("{text:<width$}")
    } else if width == 0 {
        String::new()
    } else {
        let head: String = text.chars().take(width - 1).collect();
        format!("{head}…")
    }
}

/// One row: kind chip (`pr` or `run`, as the feed sends it), id, title, state word, time.
/// The title takes whatever width the other columns leave, so the state and time columns
/// line up down the board.
fn row_line(
    item: &WatchItem,
    theme: &Theme,
    offset: chrono::FixedOffset,
    width: usize,
    id_w: usize,
) -> Line<'static> {
    let time = super::local_time(&item.at, offset);
    let title_w = width.saturating_sub(CHIP_W + id_w + STATE_W + GAPS + time.chars().count());
    Line::from(vec![
        Span::styled(
            format!("{:<CHIP_W$}", item.kind),
            Style::default().fg(theme.accent),
        ),
        Span::raw(" "),
        Span::raw(format!("{:<id_w$}", item.id)),
        Span::raw(" "),
        Span::raw(fit(&item.title, title_w)),
        Span::raw(" "),
        Span::styled(
            format!("{:<STATE_W$}", item.state),
            Style::default().fg(state_color(theme, &item.state)),
        ),
        Span::raw(" "),
        Span::styled(time, Style::default().fg(theme.dim)),
    ])
}

/// Draws the watch board. Rows past the frame height are cut; there is no scrolling.
/// `offset` only converts each item's timestamp to local time.
pub fn render(
    f: &mut Frame,
    area: Rect,
    items: &[WatchItem],
    theme: &Theme,
    offset: chrono::FixedOffset,
) {
    let base = Style::default().fg(theme.fg).bg(theme.bg);
    let block = Block::default()
        .title("Watch")
        .title_bottom(Line::styled(
            "Esc back",
            Style::default().fg(theme.dim).bg(theme.bg),
        ))
        .borders(Borders::ALL)
        .style(base);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let lines: Vec<Line> = if items.is_empty() {
        vec![Line::styled(
            "nothing watched",
            Style::default().fg(theme.dim),
        )]
    } else {
        let id_w = items
            .iter()
            .map(|i| i.id.chars().count())
            .max()
            .unwrap_or(0);
        items
            .iter()
            .take(usize::from(inner.height))
            .map(|item| row_line(item, theme, offset, usize::from(inner.width), id_w))
            .collect()
    };
    f.render_widget(Paragraph::new(lines).style(base), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ThemeId;
    use ratatui::{Terminal, backend::TestBackend};

    fn item(kind: &str, id: &str, title: &str, state: &str, at: &str) -> WatchItem {
        WatchItem {
            kind: kind.to_string(),
            id: id.to_string(),
            title: title.to_string(),
            state: state.to_string(),
            at: at.to_string(),
        }
    }

    fn items() -> Vec<WatchItem> {
        vec![
            item(
                "pr",
                "#41",
                "Add the watch board",
                "open",
                "2026-10-05T14:05:00Z",
            ),
            item(
                "pr",
                "#38",
                "Fix the lane meter",
                "merged",
                "2026-10-05T13:10:00Z",
            ),
            item(
                "run",
                "r-7",
                "Nightly build",
                "failed",
                "2026-10-05T12:30:00Z",
            ),
            item("run", "r-9", "Docs refresh", "done", "2026-10-05T11:45:00Z"),
            item(
                "pr",
                "#44",
                "Rework the settings screen so that its staged changes, refusals and diffs read in one pass on a narrow terminal",
                "closed",
                "2026-10-05T10:00:00Z",
            ),
            item(
                "run",
                "r-12",
                "Queued rebase",
                "queued",
                "2026-10-05T09:15:00Z",
            ),
        ]
    }

    fn draw(id: ThemeId, items: &[WatchItem]) -> (Terminal<TestBackend>, Theme) {
        let theme = crate::theme::resolve_for(id, Some("truecolor"));
        let offset = chrono::FixedOffset::east_opt(0).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).expect("terminal");
        terminal
            .draw(|f| render(f, f.area(), items, &theme, offset))
            .expect("draw should not fail");
        (terminal, theme)
    }

    /// Foreground of the first cell of `needle` on the row that holds it.
    fn fg_of(terminal: &Terminal<TestBackend>, needle: &str) -> Color {
        let buffer = terminal.backend().buffer();
        let area = buffer.area;
        for y in 0..area.height {
            let row: String = (0..area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect();
            if let Some(at) = row.find(needle) {
                let x = row[..at].chars().count() as u16;
                return buffer[(x, y)].fg;
            }
        }
        panic!("{needle} should be drawn somewhere in the buffer");
    }

    #[test]
    fn the_board_in_the_regatta_theme() {
        let (terminal, theme) = draw(ThemeId::Regatta, &items());
        insta::with_settings!({snapshot_path => "snapshots/watch_drill"}, {
            insta::assert_snapshot!(terminal.backend().to_string());
        });
        assert_eq!(fg_of(&terminal, "failed"), theme.status_failed);
    }

    #[test]
    fn the_board_in_the_harbor_light_theme() {
        let (terminal, theme) = draw(ThemeId::HarborLight, &items());
        insta::with_settings!({snapshot_path => "snapshots/watch_drill"}, {
            insta::assert_snapshot!(terminal.backend().to_string());
        });
        assert_eq!(fg_of(&terminal, "failed"), theme.status_failed);
    }

    #[test]
    fn an_empty_list_draws_one_dim_line() {
        let (terminal, theme) = draw(ThemeId::Regatta, &[]);
        insta::with_settings!({snapshot_path => "snapshots/watch_drill"}, {
            insta::assert_snapshot!(terminal.backend().to_string());
        });
        assert_eq!(fg_of(&terminal, "nothing watched"), theme.dim);
    }
}
