//! Rendering for the Regatta page: seven rounded, titled frames (chair, spend, machines,
//! lanes-over-24h, runs, queue, and an always-shown inbox) laid out by `app.regatta_layout_preset()`
//! and reflowed around whichever of frames 1-6 `app.regatta_frames_visible()` hides.
//! [`frame_rects`] is the pure layout core; [`render`] calls it so the rects it draws into and
//! the rects a mouse click is tested against never disagree.

use chrono::DateTime;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Gauge, Paragraph, Sparkline},
};

use crate::app::App;
use crate::feed::{FeedSnapshot, Run};
use crate::theme::Theme;

/// Titles for numbered frames 1-6, in slot order.
const FRAME_NAMES: [&str; 6] = [
    "chair",
    "spend",
    "machines",
    "lanes over 24h",
    "runs",
    "queue",
];

pub fn render(f: &mut Frame, app: &App, theme: &Theme) {
    let area = f.area();
    let Some(snapshot) = app.snapshot() else {
        render_waiting(f, area, theme);
        return;
    };
    let (rects, inbox_area) = layout_rects(area, app);
    for (idx, rect) in rects.into_iter().enumerate() {
        if let Some(rect) = rect {
            render_frame(f, rect, idx, snapshot, app, theme);
        }
    }
    render_inbox(f, inbox_area, snapshot, theme);
}

/// The rects frames 1-6 render into (`None` for a hidden frame), exactly as `render` lays
/// them out. A mouse handler can use this without touching the render path itself.
pub fn frame_rects(area: Rect, app: &App) -> [Option<Rect>; 6] {
    layout_rects(area, app).0
}

fn visible_frame_indices(app: &App) -> Vec<usize> {
    app.regatta_frames_visible()
        .iter()
        .enumerate()
        .filter_map(|(i, visible)| visible.then_some(i))
        .collect()
}

fn layout_rects(area: Rect, app: &App) -> ([Option<Rect>; 6], Rect) {
    let visible = visible_frame_indices(app);
    let mut rects: [Option<Rect>; 6] = [None; 6];
    let [grid_area, inbox_area] = {
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(3), Constraint::Length(5)])
            .split(area);
        [split[0], split[1]]
    };
    let placed = match app.regatta_layout_preset() {
        0 => grid_layout(grid_area, &visible),
        1 => stacked_layout(grid_area, &visible),
        _ => sidebar_layout(grid_area, &visible),
    };
    for (slot, rect) in visible.into_iter().zip(placed) {
        rects[slot] = Some(rect);
    }
    (rects, inbox_area)
}

/// Preset 0: a 2-column grid, filled row by row; an odd frame out spans the full row.
fn grid_layout(area: Rect, visible: &[usize]) -> Vec<Rect> {
    let n = visible.len();
    if n == 0 {
        return Vec::new();
    }
    let rows = n.div_ceil(2);
    let row_rects = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Ratio(1, rows as u32); rows])
        .split(area);
    let mut out = Vec::with_capacity(n);
    for (row_idx, row_rect) in row_rects.iter().enumerate() {
        let remaining = n - row_idx * 2;
        if remaining >= 2 {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(*row_rect);
            out.push(cols[0]);
            out.push(cols[1]);
        } else {
            out.push(*row_rect);
        }
    }
    out
}

/// Preset 1: one stacked column, one full-width row per visible frame.
fn stacked_layout(area: Rect, visible: &[usize]) -> Vec<Rect> {
    let n = visible.len();
    if n == 0 {
        return Vec::new();
    }
    Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Ratio(1, n as u32); n])
        .split(area)
        .to_vec()
}

/// Preset 2: a wide main area (the first visible frame) plus the rest stacked in a right
/// sidebar.
fn sidebar_layout(area: Rect, visible: &[usize]) -> Vec<Rect> {
    let n = visible.len();
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![area];
    }
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
        .split(area);
    let sidebar_n = n - 1;
    let sidebar_rects = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Ratio(1, sidebar_n as u32); sidebar_n])
        .split(cols[1]);
    let mut out = Vec::with_capacity(n);
    out.push(cols[0]);
    out.extend(sidebar_rects.iter().copied());
    out
}

fn base_style(theme: &Theme) -> Style {
    Style::default().fg(theme.fg).bg(theme.bg)
}

fn numbered_block(n: usize, name: &str, theme: &Theme) -> Block<'static> {
    Block::default()
        .title(Span::styled(
            format!("{n} {name}"),
            Style::default().fg(theme.accent),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(base_style(theme))
}

fn render_waiting(f: &mut Frame, area: Rect, theme: &Theme) {
    let block = Block::default()
        .title("Regatta")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded);
    let paragraph = Paragraph::new("waiting for cox dash --feed")
        .block(block)
        .style(base_style(theme));
    f.render_widget(paragraph, area);
}

fn render_frame(
    f: &mut Frame,
    rect: Rect,
    idx: usize,
    snapshot: &FeedSnapshot,
    app: &App,
    theme: &Theme,
) {
    match idx {
        0 => render_chair(f, rect, snapshot, app, theme),
        1 => render_spend(f, rect, snapshot, app, theme),
        2 => render_machines(f, rect, snapshot, theme),
        3 => render_lanes(f, rect, app, theme),
        4 => render_runs(f, rect, snapshot, theme),
        5 => render_queue(f, rect, snapshot, theme),
        _ => unreachable!("regatta frame index out of range: {idx}"),
    }
}

fn render_chair(f: &mut Frame, rect: Rect, snapshot: &FeedSnapshot, app: &App, theme: &Theme) {
    let chair = &snapshot.chair;
    let at = super::local_time(&snapshot.at, app.utc_offset());
    let lines = vec![
        Line::from(format!("as of {at}")),
        Line::from(format!("holder: {}", chair.holder)),
        Line::from(format!("host: {}", chair.host)),
        Line::from(format!("liveness: {}", chair.liveness)),
        Line::from(format!("beat age: {}s", chair.beat_age_s)),
    ];
    let paragraph = Paragraph::new(lines)
        .block(numbered_block(1, FRAME_NAMES[0], theme))
        .style(base_style(theme));
    f.render_widget(paragraph, rect);
}

fn meter_color(theme: &Theme, fraction: f64) -> Color {
    if fraction < 0.6 {
        theme.meter_low
    } else if fraction < 0.85 {
        theme.meter_mid
    } else {
        theme.meter_high
    }
}

fn render_spend(f: &mut Frame, rect: Rect, snapshot: &FeedSnapshot, app: &App, theme: &Theme) {
    let spend = &snapshot.spend;
    let block = numbered_block(2, FRAME_NAMES[1], theme);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(inner);
    let five_hour_reset = super::local_time(&spend.five_hour_resets_at, app.utc_offset());
    let five_hour_pct = spend.five_hour_fraction * 100.0;
    let five_hour = Gauge::default()
        .label(format!("5h {five_hour_pct:.0}% (resets {five_hour_reset})"))
        .ratio(spend.five_hour_fraction.clamp(0.0, 1.0))
        .gauge_style(Style::default().fg(meter_color(theme, spend.five_hour_fraction)));
    f.render_widget(five_hour, rows[0]);
    let weekly_reset = super::local_time(&spend.weekly_resets_at, app.utc_offset());
    let weekly_pct = spend.weekly_fraction * 100.0;
    let weekly = Gauge::default()
        .label(format!("wk {weekly_pct:.0}% (resets {weekly_reset})"))
        .ratio(spend.weekly_fraction.clamp(0.0, 1.0))
        .gauge_style(Style::default().fg(meter_color(theme, spend.weekly_fraction)));
    f.render_widget(weekly, rows[1]);
    let hard_stop_pct = spend.hard_stop_fraction * 100.0;
    let hard_stop =
        Paragraph::new(format!("hard stop marked at {hard_stop_pct:.0}%")).style(base_style(theme));
    f.render_widget(hard_stop, rows[2]);
}

fn render_machines(f: &mut Frame, rect: Rect, snapshot: &FeedSnapshot, theme: &Theme) {
    let lines: Vec<Line> = snapshot
        .machines
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let accent = theme.machine_accents[i % theme.machine_accents.len()];
            let login = if m.login_ok { "login ok" } else { "login down" };
            let name = &m.name;
            let lanes_in_use = m.lanes_in_use;
            let capacity = m.capacity;
            Line::styled(
                format!("{name} {lanes_in_use}/{capacity} {login}"),
                Style::default().fg(accent),
            )
        })
        .collect();
    let paragraph = Paragraph::new(lines)
        .block(numbered_block(3, FRAME_NAMES[2], theme))
        .style(base_style(theme));
    f.render_widget(paragraph, rect);
}

/// Buckets `history` into 24 hourly cells by hours before the newest sample: a sample `h`
/// whole hours older than the newest lands in cell `23 - h`, so cell 23 is the last hour.
/// Samples older than 24h, or a timestamp that fails to parse, are dropped. Max per cell.
fn lane_cells(history: &[(String, u32)]) -> [u32; 24] {
    let mut cells = [0u32; 24];
    let Some(newest) = history
        .iter()
        .filter_map(|(at, _)| DateTime::parse_from_rfc3339(at).ok())
        .max()
    else {
        return cells;
    };
    for (at, lanes) in history {
        let Ok(at) = DateTime::parse_from_rfc3339(at) else {
            continue;
        };
        let hours_before = (newest - at).num_minutes() as f64 / 60.0;
        if hours_before < 0.0 {
            continue;
        }
        let h = hours_before.floor() as i64;
        if let Ok(h) = usize::try_from(h) {
            if h < 24 {
                let cell = 23 - h;
                cells[cell] = cells[cell].max(*lanes);
            }
        }
    }
    cells
}

fn render_lanes(f: &mut Frame, rect: Rect, app: &App, theme: &Theme) {
    let cells = lane_cells(app.lanes_history());
    let data: Vec<u64> = cells.iter().map(|&n| u64::from(n)).collect();
    let sparkline = Sparkline::default()
        .block(numbered_block(4, FRAME_NAMES[3], theme))
        .data(&data)
        .style(Style::default().fg(theme.accent).bg(theme.bg));
    f.render_widget(sparkline, rect);
}

fn run_color(theme: &Theme, run: &Run) -> Color {
    if run.status == "quarantined" || run.status == "failed" {
        theme.status_failed
    } else if run.verdict == "approve" {
        theme.status_done
    } else if run.status == "running" {
        theme.status_running
    } else {
        theme.status_waiting
    }
}

fn render_runs(f: &mut Frame, rect: Rect, snapshot: &FeedSnapshot, theme: &Theme) {
    let lines: Vec<Line> = snapshot
        .runs
        .iter()
        .map(|r| {
            let color = run_color(theme, r);
            let run = &r.run;
            let machine = &r.machine;
            let phase = &r.phase;
            let node = &r.node;
            let cost = r.cost;
            Line::styled(
                format!("{run} {machine} {phase} {node} {cost:.2}"),
                Style::default().fg(color),
            )
        })
        .collect();
    let paragraph = Paragraph::new(lines)
        .block(numbered_block(5, FRAME_NAMES[4], theme))
        .style(base_style(theme));
    f.render_widget(paragraph, rect);
}

fn render_queue(f: &mut Frame, rect: Rect, snapshot: &FeedSnapshot, theme: &Theme) {
    let lines: Vec<Line> = snapshot
        .queue
        .iter()
        .map(|q| {
            let initiative = &q.initiative;
            let phases_landed = q.phases_landed;
            let phases_total = q.phases_total;
            let current_phase = &q.current_phase;
            Line::from(format!(
                "{initiative} {phases_landed}/{phases_total} {current_phase}"
            ))
        })
        .collect();
    let paragraph = Paragraph::new(lines)
        .block(numbered_block(6, FRAME_NAMES[5], theme))
        .style(base_style(theme));
    f.render_widget(paragraph, rect);
}

fn render_inbox(f: &mut Frame, rect: Rect, snapshot: &FeedSnapshot, theme: &Theme) {
    let block = Block::default()
        .title(Span::styled("inbox", Style::default().fg(theme.accent)))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(base_style(theme));
    let lines: Vec<Line> = snapshot
        .inbox
        .iter()
        .map(|i| {
            let kind = &i.kind;
            let target = &i.target;
            let reason = &i.reason;
            Line::from(format!("{kind} {target} {reason}"))
        })
        .collect();
    let paragraph = Paragraph::new(lines).block(block).style(base_style(theme));
    f.render_widget(paragraph, rect);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppPage, ThemeId};
    use ratatui::{Terminal, backend::TestBackend};

    const FIXTURE: &str = include_str!("../../tests/fixtures/dash_feed_v1.json");

    fn app_with_fixture() -> App {
        let snapshot = crate::feed::parse_snapshot(FIXTURE.trim()).expect("fixture should parse");
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.apply_snapshot(snapshot);
        app
    }

    #[test]
    fn lane_cells_buckets_by_hours_before_the_newest_sample() {
        let history = vec![
            ("2026-09-29T18:00:00Z".to_string(), 4),
            ("2026-09-29T12:30:00Z".to_string(), 2),
        ];
        let cells = lane_cells(&history);
        assert_eq!(cells[23], 4);
        assert_eq!(cells[18], 2);
    }

    #[test]
    fn renders_regatta_snapshot_in_the_regatta_theme() {
        let app = app_with_fixture();
        let theme = crate::theme::resolve(ThemeId::Regatta);
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render(f, &app, &theme))
            .expect("draw should not fail");
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn renders_regatta_snapshot_in_the_harbor_light_theme() {
        let app = app_with_fixture();
        let theme = crate::theme::resolve(ThemeId::HarborLight);
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render(f, &app, &theme))
            .expect("draw should not fail");
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    fn title_cell_fg(theme_id: ThemeId) -> Color {
        let app = app_with_fixture();
        let theme = crate::theme::resolve(theme_id);
        let area = Rect::new(0, 0, 120, 40);
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render(f, &app, &theme))
            .expect("draw should not fail");
        let rect = frame_rects(area, &app)[1].expect("frame 2 is visible");
        terminal.backend().buffer()[(rect.x + 1, rect.y)].fg
    }

    #[test]
    fn frame_two_title_uses_the_regatta_accent_color() {
        assert_eq!(
            title_cell_fg(ThemeId::Regatta),
            crate::theme::resolve(ThemeId::Regatta).accent
        );
    }

    #[test]
    fn frame_two_title_uses_the_harbor_light_accent_color() {
        assert_eq!(
            title_cell_fg(ThemeId::HarborLight),
            crate::theme::resolve(ThemeId::HarborLight).accent
        );
    }

    #[test]
    fn hiding_frame_two_makes_its_rect_none_and_leaves_the_rest_some() {
        let mut app = App::default();
        app.toggle_regatta_frame(2);
        let area = Rect::new(0, 0, 120, 40);
        let rects = frame_rects(area, &app);
        assert_eq!(rects[1], None);
        for (i, rect) in rects.iter().enumerate() {
            if i != 1 {
                assert!(rect.is_some());
            }
        }
    }
}
