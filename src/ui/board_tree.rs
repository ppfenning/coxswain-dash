//! The board's tree panel: the epic to task tree, drawn from `App::board_rows`.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};

use super::status::{self, Status};
use crate::app::{App, Focus};
use crate::board_rows::{Counts, Node, Row, StatusKind, status_kind};
use crate::theme::Theme;

const INDENT: usize = 2;
const CLOSED: char = '▸';
const OPEN: char = '▾';

/// A run of row text and its foreground.
type Seg = (String, Color);

/// The status look a kind wears. Dead is the failure red for dropped too, where `status::from_word`
/// would draw dropped dim.
fn kind_status(kind: StatusKind) -> Status {
    match kind {
        StatusKind::Building => Status::Running,
        StatusKind::Approved => Status::Approved,
        StatusKind::Done => Status::Up,
        StatusKind::Dead => Status::Failed,
        StatusKind::Waiting => Status::Ready,
    }
}

fn word_style(word: &str, theme: &Theme) -> (char, Color) {
    status::style(theme, kind_status(status_kind(word)))
}

/// The non-zero counts as `(word, n)`, building and quarantined first.
fn count_parts(c: &Counts) -> Vec<(&'static str, u32)> {
    [
        ("building", c.building),
        ("quarantined", c.quarantined),
        ("approved", c.approved),
        ("ready", c.ready),
        ("blocked", c.blocked),
        ("merged", c.merged),
        ("landed", c.landed),
        ("dropped", c.dropped),
    ]
    .into_iter()
    .filter(|(_, n)| *n > 0)
    .collect()
}

#[cfg(test)]
fn counts_text(c: &Counts) -> String {
    count_parts(c)
        .iter()
        .map(|(word, n)| format!("{n} {word}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whole seconds as `45s`, `2m` or `1h05m`.
fn elapsed_text(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        _ => format!("{}h{:02}m", secs / 3600, secs % 3600 / 60),
    }
}

fn cost_text(dollars: f64) -> String {
    format!("${dollars:.2}")
}

/// The first row of the window that keeps `selected` in view, with it on the last line once the
/// list is longer than `height`. The panel keeps no scroll state, so this is a pure function of
/// the selection.
fn scroll_offset(selected: usize, height: usize) -> usize {
    (selected + 1).saturating_sub(height)
}

fn marker(open: bool) -> char {
    if open { OPEN } else { CLOSED }
}

fn indent(depth: usize, theme: &Theme) -> Seg {
    (" ".repeat(depth * INDENT), theme.fg)
}

fn parent_segs(node: &Node, today: Option<u32>, theme: &Theme) -> Vec<Seg> {
    let (glyph, color) = word_style(&node.status, theme);
    let counts = count_parts(&node.counts)
        .into_iter()
        .enumerate()
        .map(|(i, (word, n))| {
            let gap = if i == 0 { "  " } else { " " };
            (format!("{gap}{n} {word}"), word_style(word, theme).1)
        });
    let landed = today.map(|n| (format!("  landed today {n}"), theme.dim));
    [
        indent(node.depth, theme),
        (format!("{} ", marker(!node.collapsed)), theme.dim),
        (format!("{glyph} "), color),
        (node.title.clone(), theme.fg),
    ]
    .into_iter()
    .chain(counts)
    .chain(landed)
    .collect()
}

fn task_segs(
    id: &str,
    depth: usize,
    status: &str,
    host: &str,
    elapsed_s: Option<u64>,
    theme: &Theme,
) -> Vec<Seg> {
    let (glyph, color) = word_style(status, theme);
    let live = if status == "building" {
        let host = (!host.is_empty()).then(|| host.to_string());
        let elapsed = elapsed_s.map(elapsed_text);
        let text = [host, elapsed].into_iter().flatten().collect::<Vec<_>>();
        (!text.is_empty()).then(|| (format!("  {}", text.join(" ")), theme.dim))
    } else {
        None
    };
    [
        indent(depth, theme),
        ("  ".to_string(), theme.dim),
        (format!("{glyph} "), color),
        (id.to_string(), color),
    ]
    .into_iter()
    .chain(live)
    .collect()
}

fn landed_segs(depth: usize, today: u32, earlier: u32, opened: bool, theme: &Theme) -> Vec<Seg> {
    let (glyph, color) = word_style("landed", theme);
    vec![
        indent(depth, theme),
        (format!("{} ", marker(opened)), theme.dim),
        (format!("{glyph} "), color),
        (
            format!("landed today {today} and earlier {earlier}"),
            theme.dim,
        ),
    ]
}

/// The row's left text and its cost, if it has one. A landed row has none.
fn row_segs(row: &Row, theme: &Theme) -> (Vec<Seg>, Option<f64>) {
    match row {
        Row::Epic(n) | Row::Phase(n) => (parent_segs(n, None, theme), Some(n.cost_usd)),
        Row::Initiative { node, today } => {
            (parent_segs(node, Some(*today), theme), Some(node.cost_usd))
        }
        Row::Task {
            id,
            depth,
            status,
            host,
            cost_usd,
            elapsed_s,
        } => (
            task_segs(id, *depth, status, host, *elapsed_s, theme),
            Some(*cost_usd),
        ),
        Row::Landed {
            depth,
            today,
            earlier,
            opened,
            ..
        } => (landed_segs(*depth, *today, *earlier, *opened, theme), None),
    }
}

/// Cuts the segments to at most `budget` chars. Every glyph used here is one column wide.
fn clip(segs: Vec<Seg>, budget: usize) -> Vec<Seg> {
    segs.into_iter()
        .scan(budget, |left, (text, color)| {
            let kept: String = text.chars().take(*left).collect();
            *left -= kept.chars().count();
            Some((kept, color))
        })
        .filter(|(text, _)| !text.is_empty())
        .collect()
}

/// One row, `width` columns wide, with its cost right-aligned and the selected row on the
/// `selected_row` background.
fn row_line(row: &Row, selected: bool, width: u16, theme: &Theme) -> Line<'static> {
    let (segs, cost) = row_segs(row, theme);
    let cost = cost.map(cost_text).unwrap_or_default();
    let width = usize::from(width);
    let cost_cols = cost.chars().count();
    let reserve = if cost_cols == 0 { 0 } else { cost_cols + 1 };
    let left = clip(segs, width.saturating_sub(reserve));
    let used: usize = left.iter().map(|(t, _)| t.chars().count()).sum();
    let pad = " ".repeat(width.saturating_sub(used + cost_cols));
    let bg = if selected {
        Style::default().bg(theme.selected_row)
    } else {
        Style::default()
    };
    left.into_iter()
        .chain([(pad, theme.fg), (cost, theme.fg)])
        .map(|(text, color)| Span::styled(text, bg.fg(color)))
        .collect()
}

pub fn render(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let border = if app.focus() == Focus::BoardTree {
        theme.border_focus
    } else {
        theme.border
    };
    let block = Block::bordered()
        .title("tree")
        .border_style(Style::default().fg(border));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = app.board_rows();
    let selected = app
        .selected_in(Focus::BoardTree)
        .min(rows.len().saturating_sub(1));
    let height = usize::from(inner.height);
    let lines: Vec<Line> = if rows.is_empty() {
        vec![Line::styled("no epics yet", Style::default().fg(theme.dim))]
    } else {
        rows.iter()
            .enumerate()
            .skip(scroll_offset(selected, height))
            .take(height)
            .map(|(i, row)| row_line(row, i == selected, inner.width, theme))
            .collect()
    };
    f.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppPage, ThemeId};
    use crate::board_rows::flatten;
    use crate::feed::{BoardTask, FeedSnapshot, parse_snapshot};
    use crate::theme;
    use ratatui::{Terminal, backend::TestBackend};
    use serde_json::json;
    use std::collections::HashSet;

    const BOARD_FIXTURE: &str = include_str!("../../tests/fixtures/dash_feed_board_v1.json");
    const WIDTH: u16 = 90;
    const HEIGHT: u16 = 17;

    fn regatta() -> Theme {
        theme::resolve_for(ThemeId::Regatta, Some("truecolor"))
    }

    fn fixture() -> FeedSnapshot {
        parse_snapshot(BOARD_FIXTURE).expect("board fixture parses")
    }

    /// The fixture plus a merged task in p1-data. Today's history row landed board-view-parse and
    /// yesterday's landed board-view-old.
    fn with_landed() -> FeedSnapshot {
        let mut snap = fixture();
        snap.board.epics[0].initiatives[0].phases[0]
            .tasks
            .push(BoardTask {
                id: "board-view-old".into(),
                status: "merged".into(),
                ..BoardTask::default()
            });
        let row = |run: &str, ended_at: &str, task: &str| {
            serde_json::from_value(json!({
                "run": run, "machine": "host-b", "initiative": "board-view",
                "ended_at": ended_at, "outcome": "landed", "cost_usd": 0.5,
                "landed": [{"task": task, "pr": 7}]
            }))
            .expect("history row parses")
        };
        snap.history = vec![
            row("board-0", "2026-11-09T08:00:00Z", "board-view-parse"),
            row("board-9", "2026-11-08T23:59:00Z", "board-view-old"),
        ];
        snap
    }

    fn app_with(snap: FeedSnapshot) -> App {
        let mut app = App::new(AppPage::Board, ThemeId::Regatta);
        app.apply_snapshot(snap);
        app
    }

    /// Rows of the fixture: 0 epic, 1 initiative, 2 phase p1-data, 3 building task, 4 landed,
    /// 5 phase p2-render, 6 quarantined task, 7 ready task, 8 stray epic, 9 initiative,
    /// 10 phase, 11 building task, 12 blocked task.
    fn draw(app: &App, w: u16, h: u16) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("terminal");
        terminal
            .draw(|f| render(f, f.area(), app, &regatta()))
            .expect("draw should not fail");
        terminal
    }

    fn rows_of(terminal: &Terminal<TestBackend>) -> Vec<String> {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect()
            })
            .collect()
    }

    /// The foreground of the cell where `needle` starts on the first row that holds it.
    fn fg_of(terminal: &Terminal<TestBackend>, needle: &str) -> Color {
        let (y, x) = rows_of(terminal)
            .iter()
            .enumerate()
            .find_map(|(y, row)| {
                row.find(needle)
                    .map(|byte| (y, row[..byte].chars().count()))
            })
            .expect("needle is drawn");
        terminal.backend().buffer()[(x as u16, y as u16)].fg
    }

    fn text_of(row: &Row) -> String {
        let (segs, cost) = row_segs(row, &regatta());
        let left: String = segs.into_iter().map(|(t, _)| t).collect();
        format!("{left}|{}", cost.map(cost_text).unwrap_or_default())
    }

    fn rows_for(snap: &FeedSnapshot) -> Vec<Row> {
        flatten(snap, &HashSet::new(), &HashSet::new())
    }

    #[test]
    fn a_building_task_is_the_running_colour_and_a_quarantined_one_the_failed_colour() {
        let theme = regatta();
        let terminal = draw(&app_with(fixture()), WIDTH, HEIGHT);
        let running = status::color(&theme, Status::Running);
        let failed = status::color(&theme, Status::Failed);
        assert_eq!(fg_of(&terminal, "board-view-load"), running);
        assert_eq!(fg_of(&terminal, "▒"), running);
        assert_eq!(fg_of(&terminal, "board-view-frame"), failed);
        assert_eq!(fg_of(&terminal, "✕"), failed);
    }

    #[test]
    fn each_kind_wears_its_colour_and_dropped_is_red_not_dim() {
        let theme = regatta();
        let color = |word: &str| word_style(word, &theme).1;
        assert_eq!(color("building"), status::color(&theme, Status::Running));
        assert_eq!(color("approved"), theme.approved);
        assert_eq!(color("landed"), status::color(&theme, Status::Up));
        assert_eq!(color("quarantined"), status::color(&theme, Status::Failed));
        assert_eq!(color("dropped"), status::color(&theme, Status::Failed));
        assert_ne!(color("dropped"), theme.dim);
        assert_eq!(color("ready"), status::color(&theme, Status::Ready));
    }

    #[test]
    fn the_selected_row_is_highlighted() {
        let theme = regatta();
        let mut app = app_with(fixture());
        app.select_at(3);
        let terminal = draw(&app, WIDTH, HEIGHT);
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(2, 4)].bg, theme.selected_row);
        assert_ne!(buffer[(2, 3)].bg, theme.selected_row);
    }

    #[test]
    fn counts_omit_zero_and_name_the_rest() {
        let counts = Counts {
            building: 2,
            quarantined: 1,
            ..Counts::default()
        };
        assert_eq!(counts_text(&counts), "2 building 1 quarantined");
        assert_eq!(counts_text(&Counts::default()), "");
    }

    #[test]
    fn elapsed_reads_in_seconds_minutes_then_hours() {
        assert_eq!(elapsed_text(45), "45s");
        assert_eq!(elapsed_text(120), "2m");
        assert_eq!(elapsed_text(3900), "1h05m");
    }

    #[test]
    fn the_window_scrolls_only_as_far_as_keeps_the_selection_in_view() {
        assert_eq!(scroll_offset(0, 5), 0);
        assert_eq!(scroll_offset(4, 5), 0);
        assert_eq!(scroll_offset(5, 5), 1);
        assert_eq!(scroll_offset(9, 0), 10);
    }

    #[test]
    fn a_building_task_shows_host_and_elapsed_and_a_cost() {
        let rows = rows_for(&fixture());
        assert_eq!(
            text_of(&rows[3]),
            "        ▒ board-view-load  host-b 2m|$0.25"
        );
    }

    #[test]
    fn parents_show_counts_and_the_initiative_shows_landed_today() {
        let rows = rows_for(&with_landed());
        assert_eq!(
            text_of(&rows[1]),
            "  ▾ ░ bv  1 building 1 quarantined 1 ready 1 landed  landed today 1|$1.75"
        );
        assert_eq!(text_of(&rows[4]), "      ▸ █ landed today 1 and earlier 1|");
    }

    #[test]
    fn a_row_too_long_for_the_pane_is_cut_before_its_cost() {
        let line = row_line(&rows_for(&fixture())[1], false, 30, &regatta());
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text.chars().count(), 30);
        assert!(text.ends_with("$1.75"), "{text:?}");
    }

    #[test]
    fn a_collapsed_and_an_expanded_initiative() {
        let mut app = app_with(fixture());
        app.select_at(1);
        app.toggle_board_node();
        insta::assert_snapshot!(draw(&app, WIDTH, HEIGHT).backend().to_string());
    }

    #[test]
    fn cost_rolls_up_on_parent_rows() {
        let app = app_with(fixture());
        insta::assert_snapshot!(draw(&app, WIDTH, HEIGHT).backend().to_string());
    }

    #[test]
    fn landed_work_folds_into_a_count_row_then_expands() {
        let mut app = app_with(with_landed());
        let folded = draw(&app, WIDTH, HEIGHT).backend().to_string();
        app.select_at(4);
        app.toggle_board_node();
        let opened = draw(&app, WIDTH, HEIGHT).backend().to_string();
        insta::assert_snapshot!(format!("{folded}\n--- opened ---\n{opened}"));
    }
}
