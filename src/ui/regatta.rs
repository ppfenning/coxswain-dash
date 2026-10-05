//! Rendering for the Regatta page: seven rounded, titled frames (chair, spend, machines,
//! lanes-over-24h, runs, queue, and an always-shown inbox) laid out by `app.regatta_layout_preset()`
//! and reflowed around whichever of frames 1-6 `app.regatta_frames_visible()` hides. Two more
//! frames, history (7) and run cost (8), are full-width bands under them with their own flags.
//! [`frame_rects`] is the pure layout core; [`render`] calls it so the rects it draws into and
//! the rects a mouse click is tested against never disagree.

use chrono::{DateTime, FixedOffset, Timelike};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols::Marker,
    text::{Line, Span},
    widgets::{Axis, Block, BorderType, Borders, Chart, Dataset, GraphType, Paragraph},
};

use crate::app::{App, Focus};
use crate::feed::{
    Chair, FeedSnapshot, HistoryRow, HistoryToday, InboxEntry, Machine, Outcome, QueueEntry, Run,
};
use crate::theme::Theme;

use super::chair_card::{Freshness, TICK_INTERVAL_S, beat_freshness, short_age};
use super::local_time;
use super::run_cost::{chart_bounds, dollar_labels, drawable};

/// Cells in a spend meter, and the least one shrinks to in a narrow frame.
const METER_WIDTH: usize = 26;
const METER_MIN: usize = 4;
/// Width of the spend row labels, `5 hour` being the longest.
const SPEND_LABEL_WIDTH: usize = 6;

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
    if let Some(rect) = history_rect(area, app) {
        render_history(
            f,
            rect,
            snapshot,
            frame_selection(app, Focus::History),
            app.utc_offset(),
            theme,
        );
    }
    if let Some(rect) = run_cost_rect(area, app) {
        render_run_cost(f, rect, snapshot, app.utc_offset(), theme);
    }
    render_key_bar(f, split_key_bar(area).1, theme);
}

/// The rects frames 1-6 render into (`None` for a hidden frame), exactly as `render` lays
/// them out. A mouse handler can use this without touching the render path itself.
pub fn frame_rects(area: Rect, app: &App) -> [Option<Rect>; 6] {
    layout_rects(area, app).0
}

/// The list and row `(x, y)` falls on, using the same layout `render` draws with: the runs
/// (frame 5), queue (frame 6) or machines (frame 3) rect, whichever one's list actually has a
/// row there. `None` when there is no snapshot yet, the point misses all three rects, lands on
/// a border, or overshoots the matching list's last row.
// `ui::regatta` is a private module, so `src/input.rs`'s mouse handler cannot call this
// directly today (it carries its own small copy of the same math instead); clippy would
// otherwise flag this pub fn and its `row_in_rect` helper as dead code outside the tests below.
#[allow(dead_code)]
pub fn row_at(area: Rect, app: &App, x: u16, y: u16) -> Option<(Focus, usize)> {
    let snapshot = app.snapshot()?;
    let (rects, _) = layout_rects(area, app);
    let candidates = [
        (2, Focus::Machines, snapshot.machines.len()),
        (4, Focus::Runs, snapshot.runs.len()),
        (5, Focus::Queue, snapshot.queue.len()),
    ];
    for (idx, focus, len) in candidates {
        if let Some(rect) = rects[idx] {
            if let Some(row) = row_in_rect(rect, len, x, y) {
                return Some((focus, row));
            }
        }
    }
    None
}

/// The zero-based row inside `rect`'s bordered block that `(x, y)` falls on, given the list
/// drawn there has `len` rows. `None` for a point on the border, past the last row, or outside
/// `rect` entirely.
#[allow(dead_code)]
fn row_in_rect(rect: Rect, len: usize, x: u16, y: u16) -> Option<usize> {
    if x < rect.x + 1
        || x + 1 >= rect.x + rect.width
        || y < rect.y + 1
        || y + 1 >= rect.y + rect.height
    {
        return None;
    }
    let row = (y - rect.y - 1) as usize;
    (row < len).then_some(row)
}

fn visible_frame_indices(app: &App) -> Vec<usize> {
    app.regatta_frames_visible()
        .iter()
        .enumerate()
        .filter_map(|(i, visible)| visible.then_some(i))
        .collect()
}

/// Canvas layout row heights: chair|spend, machines|lanes, and the inbox's floor.
const CANVAS_TOP_ROWS: u16 = 4;
const CANVAS_MIDDLE_ROWS: u16 = 6;
const CANVAS_INBOX_MIN: u16 = 5;
/// Width of the machines frame; lanes take the rest of its row.
const CANVAS_MACHINES_WIDTH: u16 = 48;

/// Two side-by-side frames in `area`, the left one `left_width` wide. A hidden frame gives its
/// share to the other; both hidden places neither.
fn split_pair(
    area: Rect,
    left_width: u16,
    show_left: bool,
    show_right: bool,
) -> (Option<Rect>, Option<Rect>) {
    let left_width = left_width.min(area.width);
    match (show_left, show_right) {
        (true, true) => (
            Some(Rect {
                width: left_width,
                ..area
            }),
            Some(Rect {
                x: area.x + left_width,
                width: area.width - left_width,
                ..area
            }),
        ),
        (true, false) => (Some(area), None),
        (false, true) => (None, Some(area)),
        (false, false) => (None, None),
    }
}

/// Preset 0, the canvas: chair|spend, machines|lanes, runs, then queue|inbox, top to bottom.
/// Returns the rects of frames 1-6 (`None` when hidden) and the inbox rect. The inbox is never
/// shorter than `CANVAS_INBOX_MIN` rows (unless `area` itself is), so the rows above it give up
/// height first, top to bottom. `runs_len` is the runs list length; frame 5 is as tall as its
/// rows, its header row and its borders need, at most half of the height left after the first
/// two rows.
fn canvas_layout(area: Rect, visible: [bool; 6], runs_len: usize) -> ([Option<Rect>; 6], Rect) {
    let rows_of = |shown: bool, rows: u16| if shown { rows } else { 0 };
    let budget = area.height.saturating_sub(CANVAS_INBOX_MIN);
    let top_h = rows_of(visible[0] || visible[1], CANVAS_TOP_ROWS).min(budget);
    let middle_h = rows_of(visible[2] || visible[3], CANVAS_MIDDLE_ROWS).min(budget - top_h);
    let left = area.height - top_h - middle_h;
    let runs_need = u16::try_from(runs_len.saturating_add(3)).unwrap_or(u16::MAX);
    let runs_h = rows_of(visible[4], runs_need)
        .min(left / 2)
        .min(budget - top_h - middle_h);
    let bottom_h = left - runs_h;
    let row = |y: u16, height: u16| Rect { y, height, ..area };
    let top = row(area.y, top_h);
    let middle = row(area.y + top_h, middle_h);
    let runs = row(area.y + top_h + middle_h, runs_h);
    let bottom = row(area.y + top_h + middle_h + runs_h, bottom_h);
    let (chair, spend) = split_pair(top, top.width / 2, visible[0], visible[1]);
    let (machines, lanes) = split_pair(middle, CANVAS_MACHINES_WIDTH, visible[2], visible[3]);
    let (queue, inbox) = split_pair(bottom, bottom.width * 58 / 100, visible[5], true);
    (
        [
            chair,
            spend,
            machines,
            lanes,
            visible[4].then_some(runs),
            queue,
        ],
        inbox.unwrap_or(bottom),
    )
}

/// `area` split into the frames' body and the one-line key bar under it. A one-row `area` is all
/// key bar, and an empty one is neither.
fn split_key_bar(area: Rect) -> (Rect, Rect) {
    let body_h = area.height.saturating_sub(1);
    (
        Rect {
            height: body_h,
            ..area
        },
        Rect {
            y: area.y + body_h,
            height: area.height - body_h,
            ..area
        },
    )
}

/// The fewest rows the history frame is drawn in: two borders, the header and one row.
const HISTORY_MIN_ROWS: u16 = 4;

/// Rows the history frame asks for: two borders, the header, and one row per ended run (one for
/// the empty-state line). It never takes more than a third of `body_h`, and is absent (0) when
/// that third is under `HISTORY_MIN_ROWS`.
fn history_height(len: usize, body_h: u16) -> u16 {
    let want = u16::try_from(len.max(1).saturating_add(3)).unwrap_or(u16::MAX);
    let cap = body_h / 3;
    if cap < HISTORY_MIN_ROWS {
        0
    } else {
        want.min(cap)
    }
}

/// `body` split into the area the numbered frames and inbox share and the history band under
/// it, full width, in every layout preset. The band is `None` when the frame is hidden or has no
/// room.
fn split_history(body: Rect, app: &App) -> (Rect, Option<Rect>) {
    let len = app.snapshot().map_or(0, |s| s.history.len());
    let height = if app.regatta_history_visible() {
        history_height(len, body.height)
    } else {
        0
    };
    if height == 0 {
        return (body, None);
    }
    let rest = Rect {
        height: body.height - height,
        ..body
    };
    let band = Rect {
        y: body.y + rest.height,
        height,
        ..body
    };
    (rest, Some(band))
}

/// The history frame's rect for a terminal of `full`, as `render` draws it.
fn history_rect(full: Rect, app: &App) -> Option<Rect> {
    split_history(split_key_bar(full).0, app).1
}

/// The fewest rows the run cost frame is drawn in: two borders, two axis rows and four plot rows.
const RUN_COST_MIN_ROWS: u16 = 8;
/// The most rows the run cost frame takes.
const RUN_COST_MAX_ROWS: u16 = 12;
/// The shortest a frame of presets 1-3 is allowed to be pushed to: the canvas's top row height.
const SHARED_FRAME_FLOOR: u16 = CANVAS_TOP_ROWS;

/// Rows the numbered frames and the inbox need above the run cost band, so the band never
/// shrinks one of them below what it holds. The canvas counts exact content: the runs frame
/// must stay under its half-of-what-is-left cap and the bottom row must hold the inbox floor or
/// the queue's rows. Presets 1-3 size frames by ratio, so each visible frame keeps the floor.
fn rows_above_run_cost(app: &App) -> u16 {
    let visible = app.regatta_frames_visible();
    let (runs_len, queue_len) = app
        .snapshot()
        .map_or((0, 0), |s| (s.runs.len(), s.queue.len()));
    let rows_of = |shown: bool, rows: u16| if shown { rows } else { 0 };
    let shown = u16::try_from(visible.iter().filter(|v| **v).count()).unwrap_or(0);
    let floor = SHARED_FRAME_FLOOR;
    match app.regatta_layout_preset() {
        0 => {
            let runs = u16::try_from(runs_len.saturating_add(3)).unwrap_or(u16::MAX);
            let queue = u16::try_from(queue_len.saturating_add(2)).unwrap_or(u16::MAX);
            let bottom = CANVAS_INBOX_MIN.max(rows_of(visible[5], queue));
            let left = if visible[4] {
                runs.saturating_mul(2).max(runs.saturating_add(bottom))
            } else {
                bottom
            };
            rows_of(visible[0] || visible[1], CANVAS_TOP_ROWS)
                .saturating_add(rows_of(visible[2] || visible[3], CANVAS_MIDDLE_ROWS))
                .saturating_add(left)
        }
        1 => (floor * shown.div_ceil(2)).saturating_add(CANVAS_INBOX_MIN),
        2 => (floor * shown).saturating_add(CANVAS_INBOX_MIN),
        _ => (floor * shown.saturating_sub(1).max(shown.min(1))).saturating_add(CANVAS_INBOX_MIN),
    }
}

/// `body` split into the area the numbered frames and inbox share and the run cost band under
/// it, full width, in every layout preset. The band takes only the rows `rows_above_run_cost`
/// leaves spare, and is `None` when the frame is hidden or those are under `RUN_COST_MIN_ROWS`.
fn split_run_cost(body: Rect, app: &App) -> (Rect, Option<Rect>) {
    let spare = body.height.saturating_sub(rows_above_run_cost(app));
    let height = if app.regatta_run_cost_visible() && spare >= RUN_COST_MIN_ROWS {
        spare.min(RUN_COST_MAX_ROWS)
    } else {
        0
    };
    if height == 0 {
        return (body, None);
    }
    let rest = Rect {
        height: body.height - height,
        ..body
    };
    let band = Rect {
        y: body.y + rest.height,
        height,
        ..body
    };
    (rest, Some(band))
}

/// The run cost frame's rect for a terminal of `full`, as `render` draws it: under the numbered
/// frames and above the history band.
fn run_cost_rect(full: Rect, app: &App) -> Option<Rect> {
    split_run_cost(split_history(split_key_bar(full).0, app).0, app).1
}

fn layout_rects(full: Rect, app: &App) -> ([Option<Rect>; 6], Rect) {
    let (area, _) = split_run_cost(split_history(split_key_bar(full).0, app).0, app);
    if app.regatta_layout_preset() == 0 {
        let runs_len = app.snapshot().map_or(0, |s| s.runs.len());
        return canvas_layout(area, app.regatta_frames_visible(), runs_len);
    }
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
        1 => grid_layout(grid_area, &visible),
        2 => stacked_layout(grid_area, &visible),
        _ => sidebar_layout(grid_area, &visible),
    };
    for (slot, rect) in visible.into_iter().zip(placed) {
        rects[slot] = Some(rect);
    }
    (rects, inbox_area)
}

/// Preset 1: a 2-column grid, filled row by row; an odd frame out spans the full row.
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

/// Preset 2: one stacked column, one full-width row per visible frame.
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

/// Preset 3: a wide main area (the first visible frame) plus the rest stacked in a right
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

/// A rounded, titled block: the border is `theme.border`, or `theme.border_focus` when
/// `focused`, and the title is always `theme.border_focus`, bold, padded with one space each
/// side. `title` is unpadded.
fn framed(title: &str, theme: &Theme, focused: bool) -> Block<'static> {
    let border_color = if focused {
        theme.border_focus
    } else {
        theme.border
    };
    Block::default()
        .title(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(theme.border_focus)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .style(base_style(theme))
}

/// A numbered frame block, titled like ` 1 chair `; `focused` marks the list Left/Right last
/// focused.
fn numbered_block(n: usize, name: &str, theme: &Theme, focused: bool) -> Block<'static> {
    framed(&format!("{n} {name}"), theme, focused)
}

/// `Some(app.selected())` when `focus` is the page's currently focused list, `None` otherwise
/// so an unfocused frame's render helper knows to draw no row as selected.
fn frame_selection(app: &App, focus: Focus) -> Option<usize> {
    (app.focus() == focus).then(|| app.selected())
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
        0 => render_chair(f, rect, snapshot, theme),
        1 => render_spend(f, rect, snapshot, app, theme),
        2 => render_machines(
            f,
            rect,
            snapshot,
            frame_selection(app, Focus::Machines),
            theme,
        ),
        3 => render_lanes(f, rect, app, theme),
        4 => render_runs(f, rect, snapshot, frame_selection(app, Focus::Runs), theme),
        5 => render_queue(f, rect, snapshot, frame_selection(app, Focus::Queue), theme),
        _ => unreachable!("regatta frame index out of range: {idx}"),
    }
}

/// The liveness word's colour: `live` is done-green, anything else is failed-red.
fn liveness_color(liveness: &str, theme: &Theme) -> Color {
    if liveness == "live" {
        theme.status_done
    } else {
        theme.status_failed
    }
}

/// `n`, or `-` when the feed does not carry the field.
fn or_dash<T: std::fmt::Display>(n: Option<T>) -> String {
    n.map_or_else(|| "-".to_string(), |n| n.to_string())
}

/// Line two of the chair frame. `None` is a field the feed does not carry yet.
fn chair_counts_text(
    lands: u32,
    phases: Option<u32>,
    needs_you: u32,
    drafts: Option<u32>,
    housekeeping_h: Option<u64>,
) -> String {
    let housekeeping = housekeeping_h.map_or_else(|| "-".to_string(), |h| format!("{h}h ago"));
    format!(
        "lands today {lands} ({} phases) \u{b7} needs you {needs_you} \u{b7} drafts {} \u{b7} housekeeping {housekeeping}",
        or_dash(phases),
        or_dash(drafts),
    )
}

/// The chair card's two canvas lines. A stale beat is drawn in `meter_high`: the theme has no
/// warning role.
fn chair_lines(chair: &Chair, theme: &Theme) -> Vec<Line<'static>> {
    let beat_style = match beat_freshness(chair.beat_age_s, TICK_INTERVAL_S) {
        Freshness::Stale => Style::default().fg(theme.meter_high),
        Freshness::Fresh => Style::default(),
    };
    let holder_line = Line::from(vec![
        Span::styled("\u{25cf} ", Style::default().fg(theme.live_dot)),
        Span::raw(format!("{} @ ", chair.holder)),
        Span::styled(chair.host.clone(), Style::default().fg(theme.accent)),
        Span::raw(format!(" \u{b7} epoch {} \u{b7} ", chair.epoch)),
        Span::styled(format!("beat {}s", chair.beat_age_s), beat_style),
        Span::raw(" \u{b7} "),
        Span::styled(
            chair.liveness.clone(),
            Style::default().fg(liveness_color(&chair.liveness, theme)),
        ),
    ]);
    // The feed carries no phases, drafts or housekeeping age yet, so those draw `-`.
    let counts_line = Line::from(chair_counts_text(
        chair.today.lands,
        None,
        chair.today.needs_chair_open,
        None,
        None,
    ));
    vec![holder_line, counts_line]
}

fn render_chair(f: &mut Frame, rect: Rect, snapshot: &FeedSnapshot, theme: &Theme) {
    let block = numbered_block(1, FRAME_NAMES[0], theme, false);
    let paragraph = Paragraph::new(chair_lines(&snapshot.chair, theme))
        .block(block)
        .style(base_style(theme));
    f.render_widget(paragraph, rect);
}

/// The meter colour at `position`, a fraction of the meter's width: low below 60%, mid from
/// 60%, high from 85%.
fn gradient_color(theme: &Theme, position: f64) -> Color {
    if position < 0.6 {
        theme.meter_low
    } else if position < 0.85 {
        theme.meter_mid
    } else {
        theme.meter_high
    }
}

/// A meter `width` cells wide. The first `round(fraction * width)` cells are filled, each in the
/// gradient colour of its own position; the rest are track. `tick` draws the stop tick over
/// that cell.
fn meter_cells(
    fraction: f64,
    width: usize,
    tick: Option<usize>,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let filled = (fraction.clamp(0.0, 1.0) * width as f64).round() as usize;
    (0..width)
        .map(|i| match (tick == Some(i), i < filled) {
            (true, _) => Span::styled("\u{2502}", Style::default().fg(theme.stop_tick)),
            (false, true) => Span::styled(
                "\u{2588}",
                Style::default().fg(gradient_color(theme, i as f64 / width as f64)),
            ),
            (false, false) => Span::styled("\u{2591}", Style::default().fg(theme.track)),
        })
        .collect()
}

/// The meter cell the stop tick sits on, clamped into the meter.
fn stop_tick_index(hard_stop_fraction: f64, width: usize) -> usize {
    ((hard_stop_fraction.clamp(0.0, 1.0) * width as f64) as usize).min(width.saturating_sub(1))
}

/// The meter width: `METER_WIDTH`, or less when `room` cannot hold the label, the text and a
/// space either side of the meter. Never below `METER_MIN`.
fn meter_width(room: usize, label: usize, text: usize) -> usize {
    room.saturating_sub(label + text + 2)
        .clamp(METER_MIN, METER_WIDTH)
}

/// A 12-hour clock in `offset`, with minutes only when they are not zero: `12:20 AM`, `4 AM`.
/// An unparseable string comes back unchanged. `weekday` puts the day before it: `Sun 4 AM`.
fn reset_text(utc: &str, offset: FixedOffset, weekday: bool) -> String {
    let Ok(at) = DateTime::parse_from_rfc3339(utc) else {
        return utc.to_string();
    };
    let at = at.with_timezone(&offset);
    let clock = if at.minute() == 0 {
        at.format("%-I %p")
    } else {
        at.format("%-I:%M %p")
    };
    if weekday {
        format!("{} {clock}", at.format("%a"))
    } else {
        clock.to_string()
    }
}

/// One spend row: the label, the meter, then `text` beside it.
fn spend_row(
    label: &str,
    fraction: f64,
    tick: Option<f64>,
    text: String,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let tick = tick.map(|stop| stop_tick_index(stop, width));
    let mut spans = vec![Span::raw(format!("{label:<SPEND_LABEL_WIDTH$} "))];
    spans.extend(meter_cells(fraction, width, tick, theme));
    spans.push(Span::raw(format!(" {text}")));
    Line::from(spans)
}

fn render_spend(f: &mut Frame, rect: Rect, snapshot: &FeedSnapshot, app: &App, theme: &Theme) {
    let spend = &snapshot.spend;
    let block = numbered_block(2, FRAME_NAMES[1], theme, false);
    let room = usize::from(block.inner(rect).width);
    let offset = app.utc_offset();
    let five_hour_text = format!(
        "{:.0}% \u{b7} resets {}",
        spend.five_hour_fraction * 100.0,
        reset_text(&spend.five_hour_resets_at, offset, false)
    );
    let week_text = format!(
        "{:.0}% of {:.0}% stop \u{b7} {}",
        spend.weekly_fraction * 100.0,
        spend.hard_stop_fraction * 100.0,
        reset_text(&spend.weekly_resets_at, offset, true)
    );
    // Both meters share one width, so the text columns line up.
    let longest = five_hour_text
        .chars()
        .count()
        .max(week_text.chars().count());
    let width = meter_width(room, SPEND_LABEL_WIDTH, longest);
    let five_hour = spend_row(
        "5 hour",
        spend.five_hour_fraction,
        None,
        five_hour_text,
        width,
        theme,
    );
    let week = spend_row(
        "week",
        spend.weekly_fraction,
        Some(spend.hard_stop_fraction),
        week_text,
        width,
        theme,
    );
    let paragraph = Paragraph::new(vec![five_hour, week])
        .block(block)
        .style(base_style(theme));
    f.render_widget(paragraph, rect);
}

/// One machine row: marker, name in its `accent`, one block per lane, `in_use/capacity`, the
/// login dot, then the beat age. A lane block is the machine's `accent` in use, `theme.track` free.
fn machine_line(m: &Machine, accent: Color, selected: bool, theme: &Theme) -> Line<'static> {
    let marker = if selected { "\u{25b6} " } else { "  " };
    let (login, login_color) = if m.login_ok {
        ("\u{25cf} login ok", theme.landed)
    } else {
        ("\u{25cf} login lapsed", theme.quarantined)
    };
    let lanes = (0..m.capacity).map(|lane| {
        let color = if lane < m.lanes_in_use {
            accent
        } else {
            theme.track
        };
        Span::styled("\u{25a0}", Style::default().fg(color))
    });
    let head = [
        Span::styled(marker, Style::default().fg(theme.accent)),
        Span::styled(m.name.clone(), Style::default().fg(accent)),
        Span::raw(" "),
    ];
    let tail = [
        Span::raw(format!(" {}/{} ", m.lanes_in_use, m.capacity)),
        Span::styled(login, Style::default().fg(login_color)),
        Span::raw(format!(" {}", short_age(m.beat_age_s))),
    ];
    Line::from(
        head.into_iter()
            .chain(lanes)
            .chain(tail)
            .collect::<Vec<Span<'static>>>(),
    )
}

fn render_machines(
    f: &mut Frame,
    rect: Rect,
    snapshot: &FeedSnapshot,
    selected: Option<usize>,
    theme: &Theme,
) {
    let lines: Vec<Line> = snapshot
        .machines
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let accent = theme.machine_accents[i % theme.machine_accents.len()];
            machine_line(m, accent, selected == Some(i), theme)
        })
        .collect();
    let paragraph = Paragraph::new(lines)
        .block(numbered_block(3, FRAME_NAMES[2], theme, selected.is_some()))
        .style(base_style(theme));
    f.render_widget(paragraph, rect);
}

/// The most lanes in use in any sample, 0 for an empty history.
fn lane_peak(history: &[(String, u32)]) -> u32 {
    history.iter().map(|&(_, lanes)| lanes).max().unwrap_or(0)
}

/// Hours the lanes chart spans, ending at the newest sample.
const LANES_WINDOW_HOURS: i64 = 24;

/// The parsed samples; a timestamp that fails to parse is dropped. Order is not assumed.
fn lane_samples(history: &[(String, u32)]) -> Vec<(DateTime<FixedOffset>, u32)> {
    history
        .iter()
        .filter_map(|(at, lanes)| Some((DateTime::parse_from_rfc3339(at).ok()?, *lanes)))
        .collect()
}

/// The chart's time window: the 24 hours ending at the newest sample, none for no samples.
fn lane_window(
    history: &[(String, u32)],
) -> Option<(DateTime<FixedOffset>, DateTime<FixedOffset>)> {
    let newest = lane_samples(history).into_iter().map(|(at, _)| at).max()?;
    Some((newest - chrono::Duration::hours(LANES_WINDOW_HOURS), newest))
}

/// `cols` counts spaced evenly across [`lane_window`]. Each is the count of the latest sample at
/// or before its time, 0 before the first sample. No samples give zeros.
fn lane_columns(history: &[(String, u32)], cols: usize) -> Vec<u32> {
    let samples = lane_samples(history);
    let Some((start, end)) = lane_window(history) else {
        return vec![0; cols];
    };
    let span_ms = (end - start).num_milliseconds();
    let last_col = i64::try_from(cols.saturating_sub(1))
        .unwrap_or(i64::MAX)
        .max(1);
    (0..cols)
        .map(|col| {
            let at = start + chrono::Duration::milliseconds(span_ms * col as i64 / last_col);
            samples
                .iter()
                .filter(|(sampled, _)| *sampled <= at)
                .max_by_key(|(sampled, _)| *sampled)
                .map_or(0, |&(_, lanes)| lanes)
        })
        .collect()
}

/// Braille dots for the left and right column of a cell, indexed top to bottom.
const BRAILLE_LEFT: [u32; 4] = [0x01, 0x02, 0x04, 0x40];
const BRAILLE_RIGHT: [u32; 4] = [0x08, 0x10, 0x20, 0x80];

/// Braille rows, top first, for an area chart: two columns of `counts` per cell, each filled
/// from the bottom to `count / peak` of the height. A non-zero count lights at least one dot.
fn braille_area(counts: &[u32], peak: u32, rows: usize) -> Vec<String> {
    let dots = rows * 4;
    let heights: Vec<usize> = counts
        .iter()
        .map(|&n| match (n, peak) {
            (0, _) | (_, 0) => 0,
            _ => ((n as usize * dots + peak as usize / 2) / peak as usize).clamp(1, dots),
        })
        .collect();
    (0..rows)
        .map(|row| {
            heights
                .chunks(2)
                .map(|pair| {
                    let lit = |side: usize, dot: usize| {
                        let from_bottom = (rows - 1 - row) * 4 + (3 - dot);
                        u32::from(pair.get(side).is_some_and(|&h| from_bottom < h))
                    };
                    let mask = (0..4).fold(0, |mask, dot| {
                        mask | (lit(0, dot) * BRAILLE_LEFT[dot])
                            | (lit(1, dot) * BRAILLE_RIGHT[dot])
                    });
                    match mask {
                        0 => ' ',
                        _ => char::from_u32(0x2800 + mask).unwrap_or(' '),
                    }
                })
                .collect()
        })
        .collect()
}

/// The axis line under the chart: `start` at the left, `lanes in use · peak N` centred and
/// `end` at the right, cut to `width` cells.
fn lane_axis_line(start: &str, peak: u32, end: &str, width: usize) -> String {
    let label = format!("lanes in use \u{b7} peak {peak}");
    let used = start.chars().count() + label.chars().count() + end.chars().count();
    let line = match width.checked_sub(used) {
        Some(room) if room >= 2 => {
            let left = room / 2;
            format!(
                "{start}{}{label}{}{end}",
                " ".repeat(left),
                " ".repeat(room - left)
            )
        }
        _ => format!("{start} {label} {end}"),
    };
    line.chars().take(width).collect()
}

fn render_lanes(f: &mut Frame, rect: Rect, app: &App, theme: &Theme) {
    let block = numbered_block(4, FRAME_NAMES[3], theme, false);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let history = app.lanes_history();
    let peak = lane_peak(history);
    let offset = app.utc_offset();
    let stamp = |at: DateTime<FixedOffset>| local_time(&at.to_rfc3339(), offset);
    let (start, end) = lane_window(history)
        .map_or_else(Default::default, |(start, end)| (stamp(start), stamp(end)));
    let axis = lane_axis_line(&start, peak, &end, usize::from(inner.width));
    let chart = braille_area(
        &lane_columns(history, usize::from(inner.width) * 2),
        peak,
        usize::from(inner.height - 1),
    );
    let lines: Vec<Line> = chart
        .into_iter()
        .map(|row| Line::styled(row, Style::default().fg(theme.accent)))
        .chain(std::iter::once(Line::styled(
            axis,
            Style::default().fg(theme.dim),
        )))
        .collect();
    f.render_widget(Paragraph::new(lines).style(base_style(theme)), inner);
}

/// A run's status word's color. The four terminal-ish statuses have their own roles; anything
/// else is waiting.
fn status_color(theme: &Theme, status: &str) -> Color {
    match status {
        "running" => theme.status_running,
        "approved" => theme.approved,
        "landed" => theme.landed,
        "quarantined" => theme.quarantined,
        "failed" => theme.status_failed,
        _ => theme.status_waiting,
    }
}

/// Widths of the runs columns, left to right: run, machine, phase, node. ATT and COST are
/// right-aligned in `ATT_WIDTH` and `COST_WIDTH`.
const RUN_COLUMNS: [usize; 4] = [12, 10, 22, 12];
const ATT_WIDTH: usize = 3;
const COST_WIDTH: usize = 7;

/// `text` cut to `width` chars with a trailing `…` when it was longer, then padded to `width`.
fn fit_cell(text: &str, width: usize) -> String {
    let cut: String = if text.chars().count() > width {
        text.chars()
            .take(width.saturating_sub(1))
            .chain(['\u{2026}'])
            .collect()
    } else {
        text.to_string()
    };
    format!("{cut:<width$}")
}

/// One runs line up to the status column; the header and the rows share it so they align.
fn run_columns(prefix: &str, cells: [&str; 4], att: &str, cost: &str) -> String {
    let [run, machine, phase, node] = cells;
    let [run_w, machine_w, phase_w, node_w] = RUN_COLUMNS;
    format!(
        "{prefix}{} {} {} {} {att:>ATT_WIDTH$} {cost:>COST_WIDTH$} ",
        fit_cell(run, run_w),
        fit_cell(machine, machine_w),
        fit_cell(phase, phase_w),
        fit_cell(node, node_w),
    )
}

fn run_header(theme: &Theme) -> Line<'static> {
    let columns = run_columns("  ", ["RUN", "MACHINE", "PHASE", "NODE"], "ATT", "COST");
    Line::styled(format!("{columns}STATUS"), Style::default().fg(theme.dim))
}

/// One run's row. A selected row is prefixed `▶` and sits on `theme.selected_row`.
fn run_row(r: &Run, selected: bool, theme: &Theme) -> Line<'static> {
    let prefix = if selected { "\u{25b6} " } else { "  " };
    let columns = run_columns(
        prefix,
        [&r.run, &r.machine, &r.phase, &r.node],
        &r.attempt.to_string(),
        &format!("{:.2}", r.cost),
    );
    let status = Span::styled(
        format!("\u{25cf} {}", r.status),
        Style::default().fg(status_color(theme, &r.status)),
    );
    let line = Line::from(vec![Span::raw(columns), status]);
    if selected {
        line.style(Style::default().fg(theme.accent).bg(theme.selected_row))
    } else {
        line
    }
}

fn render_runs(
    f: &mut Frame,
    rect: Rect,
    snapshot: &FeedSnapshot,
    selected: Option<usize>,
    theme: &Theme,
) {
    let block = numbered_block(5, FRAME_NAMES[4], theme, selected.is_some());
    let inner = block.inner(rect);
    let header = std::iter::once(run_header(theme));
    let rows = snapshot
        .runs
        .iter()
        .enumerate()
        .map(|(i, r)| run_row(r, selected == Some(i), theme));
    let paragraph = Paragraph::new(header.chain(rows).collect::<Vec<_>>())
        .block(block)
        .style(base_style(theme));
    f.render_widget(paragraph, rect);
    // The header takes the first inner row.
    paint_selected_row(f, inner, selected.map(|i| i + 1), theme);
}

/// Fills row `row` of `inner` with `theme.selected_row`, keeping what is drawn on it. A line's own
/// style stops at its text, so the rest of the row needs this.
fn paint_selected_row(f: &mut Frame, inner: Rect, row: Option<usize>, theme: &Theme) {
    let dy = row.and_then(|r| u16::try_from(r).ok());
    if let Some(dy) = dy.filter(|dy| *dy < inner.height) {
        let area = Rect {
            y: inner.y + dy,
            height: 1,
            ..inner
        };
        f.buffer_mut()
            .set_style(area, Style::default().bg(theme.selected_row));
    }
}

/// The outcome word's colour. Approved is cyan and stopped yellow, borrowed from the roles that
/// already carry those hues. A crash reads as a failure, an outcome this build does not know as
/// waiting.
fn outcome_color(theme: &Theme, outcome: Outcome) -> Color {
    match outcome {
        Outcome::Landed => theme.landed,
        Outcome::Approved => theme.status_running,
        Outcome::Quarantined => theme.quarantined,
        Outcome::Stopped => theme.meter_mid,
        Outcome::Crashed => theme.status_failed,
        Outcome::Unknown => theme.status_waiting,
    }
}

fn outcome_word(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Landed => "landed",
        Outcome::Approved => "approved",
        Outcome::Quarantined => "quarantined",
        Outcome::Stopped => "stopped",
        Outcome::Crashed => "crashed",
        Outcome::Unknown => "unknown",
    }
}

/// Width of the history outcome column: the word, then the cause or PR number.
const HISTORY_OUTCOME_WIDTH: usize = 28;

/// What follows the outcome word: a quarantined row's cause, a landed row's first PR number.
fn outcome_detail(row: &HistoryRow) -> String {
    match (row.outcome, row.cause.as_deref(), row.landed.first()) {
        (Outcome::Quarantined, Some(cause), _) => format!(" {cause}"),
        (Outcome::Landed, _, Some(task)) => format!(" #{}", task.pr),
        _ => String::new(),
    }
}

/// The history frame's top row: today's counts.
fn history_header(today: &HistoryToday, theme: &Theme) -> Line<'static> {
    Line::styled(
        format!(
            "today  {} lands  {} quarantines  ${:.2}  {} runs",
            today.lands, today.quarantines, today.cost_usd, today.runs
        ),
        Style::default().fg(theme.dim),
    )
}

/// One ended run: end time in `offset`, machine in `accent`, initiative, outcome word in its
/// colour with its cause or PR, then cost. A selected row is prefixed `▶` and sits on
/// `theme.selected_row`.
fn history_row(
    row: &HistoryRow,
    offset: FixedOffset,
    accent: Color,
    selected: bool,
    theme: &Theme,
) -> Line<'static> {
    let prefix = if selected { "\u{25b6} " } else { "  " };
    let word = outcome_word(row.outcome);
    let line = Line::from(vec![
        Span::raw(format!("{prefix}{} ", local_time(&row.ended_at, offset))),
        Span::styled(
            fit_cell(&row.machine, RUN_COLUMNS[1]),
            Style::default().fg(accent),
        ),
        Span::raw(format!(" {} ", fit_cell(&row.initiative, RUN_COLUMNS[2]))),
        Span::styled(word, Style::default().fg(outcome_color(theme, row.outcome))),
        Span::styled(
            fit_cell(&outcome_detail(row), HISTORY_OUTCOME_WIDTH - word.len()),
            Style::default().fg(theme.dim),
        ),
        Span::raw(format!(" {:>COST_WIDTH$}", format!("${:.2}", row.cost_usd))),
    ]);
    if selected {
        line.style(Style::default().fg(theme.accent).bg(theme.selected_row))
    } else {
        line
    }
}

/// The accent of the machine named `name`, by its place in the machines frame; `theme.dim` for a
/// machine the feed no longer lists.
fn machine_accent(snapshot: &FeedSnapshot, name: &str, theme: &Theme) -> Color {
    snapshot
        .machines
        .iter()
        .position(|m| m.name == name)
        .map_or(theme.dim, |i| {
            theme.machine_accents[i % theme.machine_accents.len()]
        })
}

/// The first history row drawn so that `selected` stays on screen in `rows` rows.
fn history_window_start(selected: Option<usize>, rows: usize) -> usize {
    selected.map_or(0, |s| (s + 1).saturating_sub(rows))
}

/// The history frame: today's counts, then one row per ended run in feed order, newest first.
fn render_history(
    f: &mut Frame,
    rect: Rect,
    snapshot: &FeedSnapshot,
    selected: Option<usize>,
    offset: FixedOffset,
    theme: &Theme,
) {
    let block = framed("history", theme, selected.is_some());
    let inner = block.inner(rect);
    let shown = selected.filter(|i| *i < snapshot.history.len());
    let rows = usize::from(inner.height).saturating_sub(1);
    let start = history_window_start(shown, rows);
    let body: Vec<Line<'static>> = if snapshot.history.is_empty() {
        vec![Line::styled(
            "no runs ended yet",
            Style::default().fg(theme.dim),
        )]
    } else {
        snapshot
            .history
            .iter()
            .enumerate()
            .skip(start)
            .take(rows)
            .map(|(i, r)| {
                let accent = machine_accent(snapshot, &r.machine, theme);
                history_row(r, offset, accent, shown == Some(i), theme)
            })
            .collect()
    };
    let lines: Vec<Line<'static>> = std::iter::once(history_header(&snapshot.history_today, theme))
        .chain(body)
        .collect();
    let paragraph = Paragraph::new(lines).block(block).style(base_style(theme));
    f.render_widget(paragraph, rect);
    // The header takes the first inner row.
    paint_selected_row(f, inner, shown.map(|i| i - start + 1), theme);
}

/// Cells in a queue progress bar.
const QUEUE_BAR_CELLS: usize = 12;

/// Filled cells of the queue bar for `landed` of `total` phases, rounded to the nearest cell and
/// never past the bar.
fn queue_bar_filled(landed: u32, total: u32) -> usize {
    if total == 0 {
        return 0;
    }
    let cells = QUEUE_BAR_CELLS as u64;
    let filled = (u64::from(landed) * cells + u64::from(total) / 2) / u64::from(total);
    (filled as usize).min(QUEUE_BAR_CELLS)
}

/// The bar's fill color: landed at 75% done or more, running before that.
fn queue_bar_color(theme: &Theme, landed: u32, total: u32) -> Color {
    if u64::from(landed) * 4 >= u64::from(total) * 3 {
        theme.landed
    } else {
        theme.status_running
    }
}

/// How many of `len` items to draw and how many to fold into `+ N more`, given at most
/// `max_items` and `rows` lines. A fold takes one line, so it costs one item.
fn overflow_split(len: usize, max_items: usize, rows: usize) -> (usize, usize) {
    if len <= max_items.min(rows) {
        (len, 0)
    } else {
        let shown = max_items.min(rows.saturating_sub(1));
        (shown, len - shown)
    }
}

fn more_line(more: usize, theme: &Theme) -> Line<'static> {
    Line::styled(format!("+ {more} more"), Style::default().fg(theme.dim))
}

fn queue_line(q: &QueueEntry, selected: bool, width: usize, theme: &Theme) -> Line<'static> {
    let prefix = if selected { "\u{25b6} " } else { "  " };
    let tail = format!(" {}/{} phases", q.phases_landed, q.phases_total);
    let initiative_w = width.saturating_sub(2 + 1 + QUEUE_BAR_CELLS + tail.chars().count());
    let filled = queue_bar_filled(q.phases_landed, q.phases_total);
    let bar_color = queue_bar_color(theme, q.phases_landed, q.phases_total);
    let bar = |cells: usize, color: Color| {
        Span::styled("\u{2588}".repeat(cells), Style::default().fg(color))
    };
    let line = Line::from(vec![
        Span::raw(format!(
            "{prefix}{} ",
            fit_cell(&q.initiative, initiative_w)
        )),
        bar(filled, bar_color),
        bar(QUEUE_BAR_CELLS - filled, theme.track),
        Span::raw(tail),
    ]);
    if selected {
        line.style(Style::default().fg(theme.accent).bg(theme.selected_row))
    } else {
        line
    }
}

fn render_queue(
    f: &mut Frame,
    rect: Rect,
    snapshot: &FeedSnapshot,
    selected: Option<usize>,
    theme: &Theme,
) {
    let block = numbered_block(6, FRAME_NAMES[5], theme, selected.is_some());
    let inner = block.inner(rect);
    let (shown, more) = overflow_split(snapshot.queue.len(), usize::MAX, usize::from(inner.height));
    let rows = snapshot
        .queue
        .iter()
        .take(shown)
        .enumerate()
        .map(|(i, q)| queue_line(q, selected == Some(i), usize::from(inner.width), theme));
    let lines: Vec<Line> = rows
        .chain((more > 0).then(|| more_line(more, theme)))
        .collect();
    let paragraph = Paragraph::new(lines).block(block).style(base_style(theme));
    f.render_widget(paragraph, rect);
    paint_selected_row(f, inner, selected.filter(|i| *i < shown), theme);
}

/// Inbox items drawn before the rest fold into `+ N more`.
const INBOX_ITEMS: usize = 3;

fn inbox_line(i: &InboxEntry, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::raw(format!("{} {} ", i.kind, i.target)),
        Span::styled(i.reason.clone(), Style::default().fg(theme.dim)),
    ])
}

fn render_inbox(f: &mut Frame, rect: Rect, snapshot: &FeedSnapshot, theme: &Theme) {
    let block = framed("inbox", theme, false);
    let rows = usize::from(block.inner(rect).height);
    let (shown, more) = overflow_split(snapshot.inbox.len(), INBOX_ITEMS, rows);
    let lines: Vec<Line> = if snapshot.inbox.is_empty() {
        vec![Line::styled(
            "\u{2713} nothing waiting on you",
            Style::default().fg(theme.landed),
        )]
    } else {
        snapshot
            .inbox
            .iter()
            .take(shown)
            .map(|i| inbox_line(i, theme))
            .chain((more > 0).then(|| more_line(more, theme)))
            .collect()
    };
    let paragraph = Paragraph::new(lines).block(block).style(base_style(theme));
    f.render_widget(paragraph, rect);
}

/// The frame's number key and title: it follows history (7) as the eighth frame.
const RUN_COST_NUMBER: usize = 8;
const RUN_COST_NAME: &str = "run cost";

/// `(at, dollars)` pairs as `(epoch seconds, dollars)`, dropping a pair whose `at` is not RFC 3339.
fn epoch_points<'a>(points: impl Iterator<Item = (&'a str, f64)>) -> Vec<(f64, f64)> {
    points
        .filter_map(|(at, usd)| {
            DateTime::parse_from_rfc3339(at)
                .ok()
                .map(|t| (t.timestamp() as f64, usd))
        })
        .collect()
}

/// `epoch` seconds as `%H:%M` in `offset`; empty for a value chrono cannot represent.
fn clock_label(epoch: f64, offset: FixedOffset) -> String {
    DateTime::from_timestamp(epoch as i64, 0).map_or_else(String::new, |t| {
        t.with_timezone(&offset).format("%H:%M").to_string()
    })
}

/// The run cost frame: one braille line per running run that has cost points, in its machine's
/// accent and named in the legend, over today's spend in `theme.dim`. With no point anywhere only
/// the titled block is drawn.
fn render_run_cost(
    f: &mut Frame,
    rect: Rect,
    snapshot: &FeedSnapshot,
    offset: FixedOffset,
    theme: &Theme,
) {
    let block = numbered_block(RUN_COST_NUMBER, RUN_COST_NAME, theme, false);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let running: Vec<&Run> = snapshot
        .runs
        .iter()
        .filter(|r| r.status == "running")
        .collect();
    let series: Vec<Vec<(f64, f64)>> = running
        .iter()
        .map(|r| {
            epoch_points(
                r.cost_series
                    .iter()
                    .map(|p| (p.at.as_str(), p.cumulative_cost_usd)),
            )
        })
        .collect();
    let spend = epoch_points(
        snapshot
            .spend_series
            .iter()
            .map(|p| (p.at.as_str(), p.cumulative_cost_usd)),
    );
    let Some(bounds) = chart_bounds(&series, &spend) else {
        return;
    };
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let line = |color: Color| {
        Dataset::default()
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(color))
    };
    let behind = (!spend.is_empty()).then(|| line(theme.dim).data(&spend));
    let datasets: Vec<Dataset> = behind
        .into_iter()
        .chain(drawable(&series).into_iter().map(|i| {
            line(machine_accent(snapshot, &running[i].machine, theme))
                .name(running[i].run.clone())
                .data(&series[i])
        }))
        .collect();
    let axis = Style::default().fg(theme.dim);
    let chart = Chart::new(datasets)
        .style(base_style(theme))
        .x_axis(
            Axis::default()
                .style(axis)
                .bounds([bounds.x_min, bounds.x_max])
                .labels(vec![
                    clock_label(bounds.x_min, offset),
                    clock_label(bounds.x_max, offset),
                ]),
        )
        .y_axis(
            Axis::default()
                .style(axis)
                .bounds([0.0, bounds.y_max])
                .labels(dollar_labels(bounds.y_max, 3)),
        )
        .hidden_legend_constraints((Constraint::Min(0), Constraint::Min(0)));
    f.render_widget(chart, inner);
}

/// Key and label pairs of the key bar: exactly the keys `input::handle_key` and `main` act on.
const KEY_BAR: [(&str, &str); 7] = [
    ("1-6", "frames"),
    ("\u{2190}\u{2192}", "focus"),
    ("\u{2191}\u{2193}", "select"),
    ("\u{23ce}", "drill down"),
    ("t", "theme"),
    ("p", "layout"),
    ("q", "quit"),
];

fn key_bar_line(theme: &Theme) -> Line<'static> {
    let key = Style::default().fg(theme.border_focus);
    let label = Style::default().fg(theme.dim);
    let spans = KEY_BAR.iter().enumerate().flat_map(|(i, (k, l))| {
        let sep = (i > 0).then(|| Span::styled(" \u{b7} ", label));
        sep.into_iter()
            .chain([Span::styled(*k, key), Span::styled(format!(" {l}"), label)])
    });
    Line::from(spans.collect::<Vec<_>>())
}

fn render_key_bar(f: &mut Frame, rect: Rect, theme: &Theme) {
    f.render_widget(
        Paragraph::new(key_bar_line(theme)).style(base_style(theme)),
        rect,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppPage, Focus, ThemeId};
    use ratatui::{Terminal, backend::TestBackend};

    const FIXTURE: &str = include_str!("../../tests/fixtures/dash_feed_v1.json");

    fn app_with_fixture() -> App {
        let snapshot = crate::feed::parse_snapshot(FIXTURE.trim()).expect("fixture should parse");
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.apply_snapshot(snapshot);
        app
    }

    /// A whole feed around one literal `chair` object. The feed's `at` is 2026-09-29T00:00:00Z.
    fn chair_feed(chair: &str) -> String {
        format!(
            r#"{{"schema": 1, "at": "2026-09-29T00:00:00Z", "chair": {chair},
"spend": {{"five_hour_fraction": 0.1, "five_hour_source": "meter", "weekly_fraction": 0.2, "weekly_source": "meter", "hard_stop_fraction": 0.9, "five_hour_resets_at": "2026-09-29T02:00:00Z", "weekly_resets_at": "2026-10-04T04:00:00Z"}},
"machines": [], "runs": [], "queue": [], "inbox": [], "watch": []}}"#
        )
    }

    fn draw_chair(chair: &str) -> Terminal<TestBackend> {
        let snapshot =
            crate::feed::parse_snapshot(&chair_feed(chair)).expect("literal feed should parse");
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let mut terminal = Terminal::new(TestBackend::new(80, 4)).expect("terminal");
        terminal
            .draw(|f| render_chair(f, f.area(), &snapshot, &theme))
            .expect("draw should not fail");
        terminal
    }

    /// The text of buffer row `y`. Every cell holds one char, so a char index is a column.
    fn row_text(terminal: &Terminal<TestBackend>, y: u16) -> String {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    /// The column of the first `needle` on row `y`.
    fn col_of(terminal: &Terminal<TestBackend>, y: u16, needle: &str) -> u16 {
        let row = row_text(terminal, y);
        let byte = row
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} is not on row {y}: {row:?}"));
        row[..byte].chars().count() as u16
    }

    const LOW: Color = Color::Rgb(0x56, 0xd3, 0x64);
    const MID: Color = Color::Rgb(0xe3, 0xb3, 0x41);
    const HIGH: Color = Color::Rgb(0xff, 0x7b, 0x72);
    const TRACK: Color = Color::Rgb(0x21, 0x26, 0x2d);
    const TICK: Color = Color::Rgb(0xc9, 0xd1, 0xd9);

    fn cell_colors(cells: &[Span<'static>]) -> Vec<Option<Color>> {
        cells.iter().map(|c| c.style.fg).collect()
    }

    fn draw_spend(width: u16, height: u16) -> Terminal<TestBackend> {
        let app = app_with_fixture();
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|f| render_spend(f, f.area(), app.snapshot().expect("snapshot"), &app, &theme))
            .expect("draw should not fail");
        terminal
    }

    const MID_LAND: &str = r#"{"holder": "chair@omarchy:12345", "host": "omarchy", "epoch": 7, "liveness": "live", "beat_age_s": 4, "last_tick_at": "2026-09-28T23:59:00Z", "last_status": "chair 09-28 19:59 EDT | landed 2, launched 1, waiting on api-runners review before the next land", "current_action": {"kind": "land_phase", "target": "api-runners/runner-parity", "since": "2026-09-28T23:58:30Z"}, "today": {"lands": 2, "launches": 1, "refused_or_failed": 0, "needs_chair_open": 0}}"#;

    const IDLE: &str = r#"{"holder": "chair@omarchy:12345", "host": "omarchy", "epoch": 7, "liveness": "live", "beat_age_s": 20, "last_tick_at": "2026-09-28T23:58:00Z", "last_status": "chair idle", "current_action": null, "today": {"lands": 1, "launches": 0, "refused_or_failed": 3, "needs_chair_open": 1}}"#;

    const STALE: &str = r#"{"holder": "chair@omarchy:12345", "host": "omarchy", "epoch": 7, "liveness": "stale", "beat_age_s": 130, "last_tick_at": "09-28 19:59 EDT", "last_status": "no tick since the beat went quiet", "current_action": null, "today": {"lands": 0, "launches": 0, "refused_or_failed": 0, "needs_chair_open": 0}}"#;

    #[test]
    fn chair_card_mid_land() {
        let terminal = draw_chair(MID_LAND);
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn chair_card_idle_between_ticks() {
        let terminal = draw_chair(IDLE);
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn chair_card_stale_beat() {
        let terminal = draw_chair(STALE);
        insta::assert_snapshot!(terminal.backend().to_string());
        let beat = col_of(&terminal, 1, "beat 130s");
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(beat, 1)].fg, HIGH);
        assert_eq!(buffer[(beat + 8, 1)].fg, HIGH);
    }

    #[test]
    fn a_fresh_beat_is_not_drawn_in_the_warning_colour() {
        let terminal = draw_chair(MID_LAND);
        let beat = col_of(&terminal, 1, "beat 4s");
        assert_ne!(terminal.backend().buffer()[(beat, 1)].fg, HIGH);
    }

    #[test]
    fn the_chair_dot_is_the_live_color_and_the_host_the_accent() {
        let terminal = draw_chair(MID_LAND);
        assert!(row_text(&terminal, 1).contains(
            "\u{25cf} chair@omarchy:12345 @ omarchy \u{b7} epoch 7 \u{b7} beat 4s \u{b7} live"
        ));
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(1, 1)].symbol(), "\u{25cf}");
        assert_eq!(buffer[(1, 1)].fg, LOW);
        let host = col_of(&terminal, 1, " @ ") + 3;
        assert_eq!(buffer[(host, 1)].fg, Color::Rgb(0x79, 0xc0, 0xff));
        assert_eq!(buffer[(host + 6, 1)].fg, Color::Rgb(0x79, 0xc0, 0xff));
        assert_ne!(buffer[(host + 7, 1)].fg, Color::Rgb(0x79, 0xc0, 0xff));
    }

    #[test]
    fn the_chair_counts_line_dashes_the_fields_the_feed_lacks() {
        let terminal = draw_chair(IDLE);
        assert!(row_text(&terminal, 2).contains(
            "lands today 1 (- phases) \u{b7} needs you 1 \u{b7} drafts - \u{b7} housekeeping -"
        ));
    }

    #[test]
    fn chair_counts_text_fills_every_field_when_present() {
        assert_eq!(
            chair_counts_text(2, Some(3), 1, Some(4), Some(5)),
            "lands today 2 (3 phases) \u{b7} needs you 1 \u{b7} drafts 4 \u{b7} housekeeping 5h ago"
        );
    }

    #[test]
    fn gradient_color_is_low_below_sixty_mid_below_eighty_five_then_high() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        assert_eq!(gradient_color(&theme, 0.2), LOW);
        assert_eq!(gradient_color(&theme, 0.6), MID);
        assert_eq!(gradient_color(&theme, 0.7), MID);
        assert_eq!(gradient_color(&theme, 0.85), HIGH);
        assert_eq!(gradient_color(&theme, 0.95), HIGH);
    }

    #[test]
    fn a_meter_at_20_percent_fills_five_low_cells_then_track() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let colors = cell_colors(&meter_cells(0.20, 26, None, &theme));
        assert_eq!(colors[..5], [Some(LOW); 5]);
        assert_eq!(colors[5..], [Some(TRACK); 21]);
    }

    #[test]
    fn a_meter_at_70_percent_runs_low_then_mid_to_cell_17() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let colors = cell_colors(&meter_cells(0.70, 26, None, &theme));
        assert_eq!(colors[..16], [Some(LOW); 16]);
        assert_eq!(colors[16..18], [Some(MID); 2]);
        assert_eq!(colors[18..], [Some(TRACK); 8]);
    }

    #[test]
    fn a_meter_at_95_percent_runs_low_mid_then_high_to_cell_24() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let colors = cell_colors(&meter_cells(0.95, 26, None, &theme));
        assert_eq!(colors[..16], [Some(LOW); 16]);
        assert_eq!(colors[16..23], [Some(MID); 7]);
        assert_eq!(colors[23..25], [Some(HIGH); 2]);
        assert_eq!(colors[25], Some(TRACK));
    }

    #[test]
    fn the_stop_tick_index_is_the_hard_stop_cell_clamped_into_the_meter() {
        assert_eq!(stop_tick_index(0.93, 26), 24);
        assert_eq!(stop_tick_index(0.98, 26), 25);
        assert_eq!(stop_tick_index(1.5, 26), 25);
        assert_eq!(stop_tick_index(-0.2, 26), 0);
        assert_eq!(stop_tick_index(0.5, 0), 0);
    }

    #[test]
    fn the_stop_tick_replaces_one_cell_in_the_stop_tick_color() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let cells = meter_cells(0.95, 26, Some(24), &theme);
        assert_eq!(cells[24].content, "\u{2502}");
        assert_eq!(cells[24].style.fg, Some(TICK));
        assert_eq!(cells[23].content, "\u{2588}");
        assert_eq!(cells.iter().filter(|c| c.content == "\u{2502}").count(), 1);
    }

    #[test]
    fn meter_width_shrinks_to_fit_the_text_and_stays_between_four_and_26() {
        assert_eq!(meter_width(58, 6, 26), 24);
        assert_eq!(meter_width(200, 6, 26), 26);
        assert_eq!(meter_width(10, 6, 26), 4);
    }

    #[test]
    fn reset_text_formats_a_twelve_hour_clock_in_the_given_offset() {
        let utc = FixedOffset::east_opt(0).unwrap();
        let et = FixedOffset::west_opt(4 * 3600).unwrap();
        assert_eq!(reset_text("2026-09-29T00:20:00Z", utc, false), "12:20 AM");
        assert_eq!(reset_text("2026-09-29T13:05:00Z", utc, false), "1:05 PM");
        assert_eq!(reset_text("2026-10-04T04:00:00Z", utc, true), "Sun 4 AM");
        assert_eq!(reset_text("2026-10-04T08:00:00Z", et, true), "Sun 4 AM");
        assert_eq!(reset_text("soon", utc, true), "soon");
    }

    #[test]
    fn the_spend_rows_put_text_beside_the_meter_and_the_tick_on_the_week_meter() {
        let terminal = draw_spend(60, 4);
        let five = row_text(&terminal, 1);
        let week = row_text(&terminal, 2);
        assert!(five.contains("5 hour"), "{five:?}");
        assert!(five.contains("16% \u{b7} resets 2 AM"), "{five:?}");
        assert!(week.contains("week"), "{week:?}");
        assert!(week.contains("35% of 93% stop \u{b7} Sun 4 AM"), "{week:?}");
        // Inner width 58: label 7 cells, a 24-cell meter from column 8, tick at 8 + 22.
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(30, 2)].symbol(), "\u{2502}");
        assert_eq!(buffer[(30, 2)].fg, TICK);
        assert_eq!(buffer[(32, 2)].symbol(), " ");
        assert_eq!(buffer[(8, 1)].fg, LOW);
        assert_eq!(buffer[(8 + 4, 1)].fg, TRACK);
        assert!((8..32).all(|x| buffer[(x, 1)].symbol() != "\u{2502}"));
        assert_eq!(col_of(&terminal, 1, "16%"), 33);
    }

    #[test]
    fn a_narrow_or_short_spend_frame_clips_without_panicking() {
        let _ = draw_spend(20, 3);
        let _ = draw_spend(3, 2);
    }

    fn history(samples: &[(&str, u32)]) -> Vec<(String, u32)> {
        samples
            .iter()
            .map(|&(at, lanes)| (at.to_string(), lanes))
            .collect()
    }

    #[test]
    fn lane_peak_is_the_history_maximum_and_zero_when_empty() {
        let samples = history(&[("2026-09-29T00:00:00Z", 2), ("2026-09-29T03:00:00Z", 7)]);
        assert_eq!(lane_peak(&samples), 7);
        assert_eq!(lane_peak(&[]), 0);
    }

    #[test]
    fn lane_columns_span_the_24h_ending_at_the_newest_sample_in_any_order() {
        let samples = history(&[("2026-09-29T00:00:00Z", 2), ("2026-09-29T03:00:00Z", 7)]);
        let reversed = history(&[("2026-09-29T03:00:00Z", 7), ("2026-09-29T00:00:00Z", 2)]);
        let every_3h = vec![0, 0, 0, 0, 0, 0, 0, 2, 7];
        assert_eq!(lane_columns(&samples, 9), every_3h);
        assert_eq!(lane_columns(&reversed, 9), every_3h);
        assert_eq!(lane_columns(&samples[..1], 3), vec![0, 0, 2]);
        assert_eq!(lane_columns(&[], 2), vec![0, 0]);
    }

    #[test]
    fn braille_area_fills_from_the_bottom_to_the_count_over_the_peak() {
        assert_eq!(braille_area(&[4, 4], 4, 1), vec!["\u{28ff}"]);
        assert_eq!(braille_area(&[4, 2], 4, 1), vec!["\u{28e7}"]);
        assert_eq!(braille_area(&[1, 0], 8, 1), vec!["\u{2840}"]);
        assert_eq!(braille_area(&[0, 0], 4, 2), vec![" ", " "]);
    }

    #[test]
    fn lane_axis_line_centres_the_label_between_the_two_times() {
        assert_eq!(
            lane_axis_line("00:00", 7, "06:00", 40),
            "00:00    lanes in use \u{b7} peak 7     06:00"
        );
        assert_eq!(lane_axis_line("00:00", 7, "06:00", 10), "00:00 lane");
    }

    fn app_with_lane_samples(samples: &[(&str, u32)]) -> App {
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        for &(at, lanes) in samples {
            let feed = FIXTURE
                .replacen(
                    "\"at\": \"2026-09-29T00:00:00Z\"",
                    &format!("\"at\": \"{at}\""),
                    1,
                )
                .replacen(
                    "\"lanes_in_use\": 2",
                    &format!("\"lanes_in_use\": {lanes}"),
                    1,
                );
            app.apply_snapshot(crate::feed::parse_snapshot(feed.trim()).expect("feed parses"));
        }
        app
    }

    #[test]
    fn the_lanes_frame_draws_a_sloped_area_and_labels_the_peak_and_the_24h_window() {
        let app = app_with_lane_samples(&[
            ("2026-09-29T00:00:00Z", 2),
            ("2026-09-29T03:00:00Z", 7),
            ("2026-09-29T06:00:00Z", 3),
        ]);
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let mut terminal = Terminal::new(TestBackend::new(44, 6)).expect("terminal");
        terminal
            .draw(|f| render_lanes(f, f.area(), &app, &theme))
            .expect("draw should not fail");
        // 84 dot columns over 09-28 06:00 to 09-29 06:00; 12 dot rows, so 2, 7 and 3 of a
        // peak of 7 stand 3, 12 and 5 dots high.
        let framed = |row: String| format!("\u{2502}{row}\u{2502}");
        assert_eq!(
            row_text(&terminal, 1),
            framed(format!(
                "{}\u{28b8}\u{28ff}\u{28ff}\u{28ff}\u{28ff}\u{2847}",
                " ".repeat(36)
            ))
        );
        assert_eq!(
            row_text(&terminal, 2),
            framed(format!(
                "{}\u{28b8}\u{28ff}\u{28ff}\u{28ff}\u{28ff}\u{28c7}",
                " ".repeat(36)
            ))
        );
        assert_eq!(
            row_text(&terminal, 3),
            framed(format!(
                "{}\u{28b0}{}\u{28fe}{}",
                " ".repeat(31),
                "\u{28f6}".repeat(4),
                "\u{28ff}".repeat(5)
            ))
        );
        assert_eq!(
            row_text(&terminal, 4),
            framed("06:00     lanes in use \u{b7} peak 7      06:00".to_string())
        );
    }

    const ACCENT: Color = Color::Rgb(0x79, 0xc0, 0xff);

    fn draw_machines(feed: &str) -> Terminal<TestBackend> {
        let snapshot = crate::feed::parse_snapshot(feed.trim()).expect("feed parses");
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let mut terminal = Terminal::new(TestBackend::new(60, 4)).expect("terminal");
        terminal
            .draw(|f| render_machines(f, f.area(), &snapshot, None, &theme))
            .expect("draw should not fail");
        terminal
    }

    fn fg_at(terminal: &Terminal<TestBackend>, y: u16, needle: &str) -> Option<Color> {
        let x = col_of(terminal, y, needle);
        terminal.backend().buffer()[(x, y)].style().fg
    }

    #[test]
    fn the_login_ok_dot_is_the_landed_color() {
        let terminal = draw_machines(FIXTURE);
        assert!(row_text(&terminal, 1).contains("\u{25cf} login ok 12s"));
        assert_eq!(fg_at(&terminal, 1, "\u{25cf}"), Some(LOW));
    }

    #[test]
    fn the_login_lapsed_dot_is_the_quarantined_color() {
        let terminal = draw_machines(&FIXTURE.replace("\"login_ok\": true", "\"login_ok\": false"));
        assert!(row_text(&terminal, 1).contains("\u{25cf} login lapsed 12s"));
        assert_eq!(fg_at(&terminal, 1, "\u{25cf}"), Some(HIGH));
    }

    #[test]
    fn an_in_use_lane_block_is_its_machine_accent_and_a_free_one_the_track() {
        let second = r#"{"name": "spare", "state": "active", "lanes_in_use": 1, "capacity": 2, "login_ok": true, "login_checked_at": "2026-09-28T23:50:00Z", "beat_age_s": 90, "checkouts": {}}"#;
        let feed = FIXTURE.replacen(
            "{\"behind_main\": 0}}}]",
            &format!("{{\"behind_main\": 0}}}}}}, {second}]"),
            1,
        );
        let terminal = draw_machines(&feed);
        let buffer = terminal.backend().buffer();
        let first = col_of(&terminal, 1, "\u{25a0}");
        assert_eq!(buffer[(first, 1)].style().fg, Some(ACCENT));
        assert_eq!(buffer[(first + 1, 1)].style().fg, Some(ACCENT));
        assert_eq!(buffer[(first + 2, 1)].style().fg, Some(TRACK));
        assert!(row_text(&terminal, 1).contains(" 2/3 "));
        // Machine 1's accent differs from theme.accent, so this tells the two apart.
        const SECOND_ACCENT: Color = Color::Rgb(0xff, 0xa6, 0x57);
        let name = col_of(&terminal, 2, "spare");
        let block = col_of(&terminal, 2, "\u{25a0}");
        assert_eq!(buffer[(name, 2)].style().fg, Some(SECOND_ACCENT));
        assert_eq!(buffer[(block, 2)].style().fg, Some(SECOND_ACCENT));
        assert_eq!(buffer[(block + 1, 2)].style().fg, Some(TRACK));
        assert!(row_text(&terminal, 2).contains(" 1/2 \u{25cf} login ok 1m"));
    }

    fn draw_page(app: &App, theme_id: ThemeId, width: u16, height: u16) -> Terminal<TestBackend> {
        let theme = crate::theme::resolve_for(theme_id, Some("truecolor"));
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|f| render(f, app, &theme))
            .expect("draw should not fail");
        terminal
    }

    #[test]
    fn renders_regatta_snapshot_at_120x40_in_the_regatta_theme() {
        let terminal = draw_page(&app_with_fixture(), ThemeId::Regatta, 120, 40);
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn renders_regatta_snapshot_at_120x40_in_the_harbor_light_theme() {
        let terminal = draw_page(&app_with_fixture(), ThemeId::HarborLight, 120, 40);
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn renders_regatta_snapshot_at_160x46_in_the_regatta_theme() {
        let terminal = draw_page(&app_with_fixture(), ThemeId::Regatta, 160, 46);
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn renders_regatta_snapshot_at_160x46_in_the_harbor_light_theme() {
        let terminal = draw_page(&app_with_fixture(), ThemeId::HarborLight, 160, 46);
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    fn title_cell_fg(theme_id: ThemeId) -> Color {
        let app = app_with_fixture();
        let theme = crate::theme::resolve_for(theme_id, Some("truecolor"));
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
    fn frame_two_title_uses_the_regatta_title_color() {
        assert_eq!(
            title_cell_fg(ThemeId::Regatta),
            crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor")).border_focus
        );
    }

    #[test]
    fn frame_two_title_uses_the_harbor_light_title_color() {
        assert_eq!(
            title_cell_fg(ThemeId::HarborLight),
            crate::theme::resolve_for(ThemeId::HarborLight, Some("truecolor")).border_focus
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

    /// A snapshot with two runs (`r0`, `r1`), for `row_at`'s tests below.
    fn app_with_two_runs() -> App {
        let json = r#"{"schema":1,"at":"2026-09-29T00:00:00Z","chair":{"holder":"h","host":"h","epoch":1,"liveness":"live","beat_age_s":0},"spend":{"five_hour_fraction":0.0,"five_hour_source":"meter","weekly_fraction":0.0,"weekly_source":"meter","hard_stop_fraction":0.0,"five_hour_resets_at":"2026-09-29T00:00:00Z","weekly_resets_at":"2026-09-29T00:00:00Z"},"machines":[{"name":"m0","state":"active","lanes_in_use":0,"capacity":3,"login_ok":true,"login_checked_at":"2026-09-29T00:00:00Z","beat_age_s":0,"checkouts":{}}],"runs":[{"run":"r0","machine":"m0","phase":"p","node":"n","attempt":1,"turns":1,"cost":0.0,"verdict":"ok","status":"running"},{"run":"r1","machine":"m0","phase":"p","node":"n","attempt":1,"turns":1,"cost":0.0,"verdict":"ok","status":"running"}],"queue":[],"inbox":[],"watch":[]}"#;
        let snapshot = crate::feed::parse_snapshot(json).expect("literal snapshot should parse");
        let mut app = App::default();
        app.apply_snapshot(snapshot);
        app
    }

    #[test]
    fn row_at_locates_the_runs_frames_second_row() {
        let app = app_with_two_runs();
        let area = Rect::new(0, 0, 120, 40);
        let runs_rect = frame_rects(area, &app)[4].expect("runs frame is visible");
        let (focus, row) = row_at(area, &app, runs_rect.x + 1, runs_rect.y + 2)
            .expect("the second row should hit");
        assert_eq!(focus, Focus::Runs);
        assert_eq!(row, 1);
    }

    #[test]
    fn row_at_misses_a_point_outside_every_rect() {
        let app = app_with_two_runs();
        let area = Rect::new(0, 0, 120, 40);
        assert_eq!(row_at(area, &app, 119, 39), None);
    }

    /// The runs frame's selected row is drawn in `theme.accent` and its border in
    /// `theme.border_focus` when the runs list is focused: read straight from `terminal.backend().buffer()` cells, since a
    /// text-only snapshot cannot show a style. `app_with_fixture` defaults to `Focus::Runs`
    /// with `selected() == 0`, so the fixture's one run is both focused and selected.
    fn runs_row_and_border_fg(theme_id: ThemeId) -> (Color, Color) {
        let app = app_with_fixture();
        assert_eq!(app.focus(), Focus::Runs);
        assert_eq!(app.selected(), 0);
        let theme = crate::theme::resolve_for(theme_id, Some("truecolor"));
        let area = Rect::new(0, 0, 120, 40);
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render(f, &app, &theme))
            .expect("draw should not fail");
        let runs_rect = frame_rects(area, &app)[4].expect("runs frame is visible");
        let buffer = terminal.backend().buffer();
        let row_fg = buffer[(runs_rect.x + 1, runs_rect.y + 2)].fg;
        let border_fg = buffer[(runs_rect.x, runs_rect.y)].fg;
        (row_fg, border_fg)
    }

    #[test]
    fn the_runs_frames_selected_row_is_accent_and_border_is_focus_in_the_regatta_theme() {
        let (row_fg, border_fg) = runs_row_and_border_fg(ThemeId::Regatta);
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        assert_eq!(row_fg, theme.accent);
        assert_eq!(border_fg, theme.border_focus);
    }

    #[test]
    fn the_runs_frames_selected_row_is_accent_and_border_is_focus_in_the_harbor_light_theme() {
        let (row_fg, border_fg) = runs_row_and_border_fg(ThemeId::HarborLight);
        let theme = crate::theme::resolve_for(ThemeId::HarborLight, Some("truecolor"));
        assert_eq!(row_fg, theme.accent);
        assert_eq!(border_fg, theme.border_focus);
    }

    const ALL: [bool; 6] = [true; 6];

    #[test]
    fn preset_zero_is_the_canvas_layout() {
        let app = App::default();
        assert_eq!(app.regatta_layout_preset(), 0);
        let area = Rect::new(0, 0, 120, 40);
        let (rest, _) = split_history(Rect::new(0, 0, 120, 39), &app);
        let (body, _) = split_run_cost(rest, &app);
        assert_eq!(layout_rects(area, &app), canvas_layout(body, ALL, 0));
    }

    #[test]
    fn the_key_bar_is_the_last_row_and_the_frames_keep_the_rest() {
        let (body, bar) = split_key_bar(Rect::new(0, 2, 120, 40));
        assert_eq!(body, Rect::new(0, 2, 120, 39));
        assert_eq!(bar, Rect::new(0, 41, 120, 1));
        assert_eq!(split_key_bar(Rect::new(0, 0, 9, 0)).1.height, 0);
    }

    #[test]
    fn canvas_layout_places_every_row_at_120x40_with_one_run() {
        let (rects, inbox) = canvas_layout(Rect::new(0, 0, 120, 40), ALL, 1);
        assert_eq!(rects[0], Some(Rect::new(0, 0, 60, 4)));
        assert_eq!(rects[1], Some(Rect::new(60, 0, 60, 4)));
        assert_eq!(rects[2], Some(Rect::new(0, 4, 48, 6)));
        assert_eq!(rects[3], Some(Rect::new(48, 4, 72, 6)));
        assert_eq!(rects[4], Some(Rect::new(0, 10, 120, 4)));
        assert_eq!(rects[5], Some(Rect::new(0, 14, 69, 26)));
        assert_eq!(inbox, Rect::new(69, 14, 51, 26));
    }

    #[test]
    fn canvas_layout_caps_runs_at_half_of_what_is_left() {
        let (rects, inbox) = canvas_layout(Rect::new(0, 0, 120, 40), ALL, 100);
        assert_eq!(rects[4], Some(Rect::new(0, 10, 120, 15)));
        assert_eq!(inbox.height, 15);
    }

    #[test]
    fn canvas_layout_keeps_the_inbox_at_five_rows_down_to_a_short_terminal() {
        for height in [24, 16, 12, 9] {
            let (rects, inbox) = canvas_layout(Rect::new(0, 0, 80, height), ALL, 100);
            assert!(inbox.height >= 5, "height {height}: inbox {inbox:?}");
            let bottom = rects
                .iter()
                .flatten()
                .chain([&inbox])
                .map(|r| r.y + r.height);
            assert_eq!(bottom.max(), Some(height));
        }
    }

    #[test]
    fn the_rendered_canvas_at_80x24_gives_the_inbox_at_least_five_rows() {
        let app = app_with_fixture();
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let area = Rect::new(0, 0, 80, 24);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
        terminal
            .draw(|f| render(f, &app, &theme))
            .expect("draw should not fail");
        let (_, inbox) = layout_rects(area, &app);
        assert!(inbox.height >= 5, "inbox {inbox:?}");
        let history = history_rect(area, &app).expect("history band");
        assert_eq!(inbox.y + inbox.height, history.y);
        assert_eq!(history.y + history.height, 23);
    }

    #[test]
    fn canvas_layout_gives_a_hidden_frames_share_to_its_neighbour() {
        let hidden = [false, true, false, true, true, false];
        let (rects, inbox) = canvas_layout(Rect::new(0, 0, 100, 30), hidden, 3);
        assert_eq!(rects[0], None);
        assert_eq!(rects[1], Some(Rect::new(0, 0, 100, 4)));
        assert_eq!(rects[2], None);
        assert_eq!(rects[3], Some(Rect::new(0, 4, 100, 6)));
        assert_eq!(rects[5], None);
        assert_eq!(inbox.width, 100);
    }

    #[test]
    fn titles_are_bold_padded_in_the_title_color_and_borders_use_the_border_color() {
        let app = app_with_fixture();
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("terminal");
        terminal
            .draw(|f| render(f, &app, &theme))
            .expect("draw should not fail");
        let buffer = terminal.backend().buffer();
        let chair = frame_rects(Rect::new(0, 0, 120, 40), &app)[0].expect("chair rect");
        let title: String = (1..=9)
            .map(|dx| buffer[(chair.x + dx, chair.y)].symbol())
            .collect();
        assert_eq!(title, " 1 chair ");
        let title_cell = &buffer[(chair.x + 2, chair.y)];
        assert_eq!(title_cell.fg, theme.border_focus);
        assert!(title_cell.modifier.contains(Modifier::BOLD));
        assert_eq!(buffer[(chair.x, chair.y)].fg, theme.border);
        assert_eq!(buffer[(chair.x, chair.y)].symbol(), "\u{256d}");
    }

    /// A feed with literal `runs`, `queue` and `inbox` arrays, in the default (Runs) focus.
    fn feed_app(runs: &str, queue: &str, inbox: &str) -> App {
        let json = format!(
            r#"{{"schema":1,"at":"2026-09-29T00:00:00Z","chair":{{"holder":"h","host":"h","epoch":1,"liveness":"live","beat_age_s":0}},"spend":{{"five_hour_fraction":0.0,"five_hour_source":"meter","weekly_fraction":0.0,"weekly_source":"meter","hard_stop_fraction":0.0,"five_hour_resets_at":"2026-09-29T00:00:00Z","weekly_resets_at":"2026-09-29T00:00:00Z"}},"machines":[],"runs":[{runs}],"queue":[{queue}],"inbox":[{inbox}],"watch":[]}}"#
        );
        let mut app = App::default();
        app.apply_snapshot(crate::feed::parse_snapshot(&json).expect("literal feed parses"));
        app
    }

    fn run_json(name: &str, status: &str) -> String {
        format!(
            r#"{{"run":"{name}","machine":"m0","phase":"p","node":"n","attempt":2,"turns":1,"cost":1.5,"verdict":"ok","status":"{status}"}}"#
        )
    }

    fn inbox_json(n: usize) -> String {
        (0..n)
            .map(|i| {
                format!(
                    r#"{{"kind":"needs_chair","target":"task-{i}","reason":"budget stop {i}"}}"#
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    }

    fn queue_json(n: usize, landed: u32, total: u32) -> String {
        (0..n)
            .map(|i| {
                format!(
                    r#"{{"initiative":"init-{i}","priority":1,"phases_landed":{landed},"phases_total":{total},"current_phase":"p"}}"#
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    }

    fn cell(terminal: &Terminal<TestBackend>, x: u16, y: u16) -> ratatui::buffer::Cell {
        terminal.backend().buffer()[(x, y)].clone()
    }

    #[test]
    fn each_status_dot_is_its_literal_color_in_the_regatta_theme() {
        let statuses = ["running", "approved", "landed", "quarantined"];
        let runs: Vec<String> = statuses.iter().map(|s| run_json(s, s)).collect();
        let app = feed_app(&runs.join(","), "", "");
        let terminal = draw_page(&app, ThemeId::Regatta, 120, 40);
        let runs_rect = frame_rects(Rect::new(0, 0, 120, 40), &app)[4].expect("runs rect");
        let expected = [
            Color::Rgb(0x56, 0xd4, 0xdd),
            Color::Rgb(0xd2, 0xa8, 0xff),
            Color::Rgb(0x56, 0xd3, 0x64),
            Color::Rgb(0xff, 0x7b, 0x72),
        ];
        for (i, (status, color)) in statuses.iter().zip(expected).enumerate() {
            let y = runs_rect.y + 2 + i as u16;
            assert!(row_text(&terminal, y).contains(&format!("\u{25cf} {status}")));
            assert_eq!(fg_at(&terminal, y, "\u{25cf}"), Some(color), "{status}");
        }
    }

    #[test]
    fn the_waiting_and_failed_statuses_take_their_own_colors() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        assert_eq!(status_color(&theme, "failed"), Color::Rgb(0xff, 0x7b, 0x72));
        assert_eq!(
            status_color(&theme, "pending"),
            Color::Rgb(0x8b, 0x94, 0x9e)
        );
    }

    #[test]
    fn the_selected_runs_row_is_on_the_selected_row_background_across_its_width() {
        let app = feed_app(
            &format!("{},{}", run_json("r0", "running"), run_json("r1", "landed")),
            "",
            "",
        );
        for (theme_id, bg) in [
            (ThemeId::Regatta, Color::Rgb(0x16, 0x1b, 0x22)),
            (ThemeId::HarborLight, Color::Rgb(0xef, 0xea, 0xdd)),
        ] {
            let terminal = draw_page(&app, theme_id, 120, 40);
            let rect = frame_rects(Rect::new(0, 0, 120, 40), &app)[4].expect("runs rect");
            let selected = rect.y + 2;
            let prefix: String = row_text(&terminal, selected)
                .chars()
                .skip(1)
                .take(2)
                .collect();
            assert_eq!(prefix, "\u{25b6} ");
            assert_eq!(cell(&terminal, rect.x + 1, selected).bg, bg);
            assert_eq!(cell(&terminal, rect.x + rect.width - 2, selected).bg, bg);
            assert_ne!(cell(&terminal, rect.x + 1, selected + 1).bg, bg);
        }
    }

    #[test]
    fn the_runs_header_is_dim_and_the_columns_line_up() {
        let app = feed_app(&run_json("r0", "running"), "", "");
        let terminal = draw_page(&app, ThemeId::Regatta, 120, 40);
        let rect = frame_rects(Rect::new(0, 0, 120, 40), &app)[4].expect("runs rect");
        let header = row_text(&terminal, rect.y + 1);
        let words: Vec<&str> = header.split_whitespace().skip(1).collect();
        assert_eq!(
            words[..7].join(" "),
            "RUN MACHINE PHASE NODE ATT COST STATUS"
        );
        assert_eq!(
            fg_at(&terminal, rect.y + 1, "RUN"),
            Some(Color::Rgb(0x8b, 0x94, 0x9e))
        );
        let row = row_text(&terminal, rect.y + 2);
        assert_eq!(
            col_of(&terminal, rect.y + 1, "STATUS"),
            row.find('\u{25cf}')
                .map(|b| row[..b].chars().count() as u16)
                .expect("dot")
        );
    }

    #[test]
    fn queue_bar_filled_rounds_to_the_nearest_of_twelve_cells() {
        assert_eq!(queue_bar_filled(0, 3), 0);
        assert_eq!(queue_bar_filled(1, 3), 4);
        assert_eq!(queue_bar_filled(2, 4), 6);
        assert_eq!(queue_bar_filled(3, 3), 12);
        assert_eq!(queue_bar_filled(0, 0), 0);
        assert_eq!(queue_bar_filled(9, 3), 12);
    }

    #[test]
    fn the_queue_bar_turns_landed_at_three_quarters_done() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        assert_eq!(queue_bar_color(&theme, 2, 4), Color::Rgb(0x56, 0xd4, 0xdd));
        assert_eq!(queue_bar_color(&theme, 3, 4), Color::Rgb(0x56, 0xd3, 0x64));
    }

    #[test]
    fn a_queue_row_draws_twelve_bar_cells_and_landed_over_total() {
        let app = feed_app("", &queue_json(1, 2, 4), "");
        let terminal = draw_page(&app, ThemeId::Regatta, 120, 40);
        let rect = frame_rects(Rect::new(0, 0, 120, 40), &app)[5].expect("queue rect");
        let y = rect.y + 1;
        let row = row_text(&terminal, y);
        assert!(row.contains("init-0"), "{row}");
        assert!(row.contains(" 2/4 phases"), "{row}");
        let x = col_of(&terminal, y, "\u{2588}");
        assert_eq!(cell(&terminal, x, y).fg, Color::Rgb(0x56, 0xd4, 0xdd));
        assert_eq!(cell(&terminal, x + 5, y).fg, Color::Rgb(0x56, 0xd4, 0xdd));
        assert_eq!(cell(&terminal, x + 6, y).fg, Color::Rgb(0x21, 0x26, 0x2d));
        assert_eq!(cell(&terminal, x + 11, y).fg, Color::Rgb(0x21, 0x26, 0x2d));
        assert_eq!(row.chars().filter(|c| *c == '\u{2588}').count(), 12);
    }

    #[test]
    fn overflow_split_folds_into_one_more_line_that_costs_an_item() {
        assert_eq!(overflow_split(3, 3, 10), (3, 0));
        assert_eq!(overflow_split(5, 3, 10), (3, 2));
        assert_eq!(overflow_split(4, 3, 3), (2, 2));
        assert_eq!(overflow_split(10, usize::MAX, 5), (4, 6));
        assert_eq!(overflow_split(2, 3, 0), (0, 2));
    }

    #[test]
    fn an_overflowing_queue_ends_on_a_dim_more_line() {
        let app = feed_app("", &queue_json(10, 0, 3), "");
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let mut terminal = Terminal::new(TestBackend::new(60, 5)).expect("terminal");
        terminal
            .draw(|f| render_queue(f, f.area(), app.snapshot().expect("snapshot"), None, &theme))
            .expect("draw should not fail");
        assert!(row_text(&terminal, 2).contains("init-1"));
        assert!(row_text(&terminal, 3).contains("+ 8 more"));
        assert_eq!(
            fg_at(&terminal, 3, "+ 8"),
            Some(Color::Rgb(0x8b, 0x94, 0x9e))
        );
    }

    fn inbox_rect(app: &App) -> Rect {
        layout_rects(Rect::new(0, 0, 120, 40), app).1
    }

    #[test]
    fn an_empty_inbox_says_nothing_waiting_in_the_landed_color() {
        let app = feed_app("", "", "");
        let terminal = draw_page(&app, ThemeId::Regatta, 120, 40);
        let y = inbox_rect(&app).y + 1;
        assert!(row_text(&terminal, y).contains("\u{2713} nothing waiting on you"));
        assert_eq!(
            fg_at(&terminal, y, "\u{2713}"),
            Some(Color::Rgb(0x56, 0xd3, 0x64))
        );
    }

    #[test]
    fn a_full_inbox_shows_three_items_with_dim_reasons_then_more() {
        let app = feed_app("", "", &inbox_json(5));
        let terminal = draw_page(&app, ThemeId::Regatta, 120, 40);
        let top = inbox_rect(&app).y + 1;
        for i in 0..3 {
            let row = row_text(&terminal, top + i);
            assert!(
                row.contains(&format!("needs_chair task-{i} budget stop {i}")),
                "{row}"
            );
        }
        assert_eq!(
            fg_at(&terminal, top, "budget"),
            Some(Color::Rgb(0x8b, 0x94, 0x9e))
        );
        assert_eq!(
            fg_at(&terminal, top, "task-0"),
            Some(Color::Rgb(0xc9, 0xd1, 0xd9))
        );
        assert!(row_text(&terminal, top + 3).contains("+ 2 more"));
        assert!(!row_text(&terminal, top + 3).contains("task-3"));
    }

    #[test]
    fn the_key_bar_is_the_last_row_with_keys_in_the_title_color_and_labels_dim() {
        let app = app_with_fixture();
        let terminal = draw_page(&app, ThemeId::Regatta, 120, 40);
        let row = row_text(&terminal, 39);
        assert_eq!(
            row.trim_end(),
            "1-6 frames \u{b7} \u{2190}\u{2192} focus \u{b7} \u{2191}\u{2193} select \u{b7} \u{23ce} drill down \u{b7} t theme \u{b7} p layout \u{b7} q quit"
        );
        assert_eq!(
            fg_at(&terminal, 39, "1-6"),
            Some(Color::Rgb(0x79, 0xc0, 0xff))
        );
        assert_eq!(
            fg_at(&terminal, 39, "frames"),
            Some(Color::Rgb(0x8b, 0x94, 0x9e))
        );
        assert_eq!(
            fg_at(&terminal, 39, "quit"),
            Some(Color::Rgb(0x8b, 0x94, 0x9e))
        );
    }

    #[test]
    fn every_key_on_the_key_bar_changes_the_app() {
        use crossterm::event::KeyCode;
        let before = app_with_fixture();
        let mut layout = app_with_fixture();
        crate::input::handle_key(&mut layout, KeyCode::Char('p'));
        assert_ne!(
            layout.regatta_layout_preset(),
            before.regatta_layout_preset()
        );
        let mut theme = app_with_fixture();
        crate::input::handle_key(&mut theme, KeyCode::Char('t'));
        assert_ne!(theme.theme(), before.theme());
        let mut frame = app_with_fixture();
        crate::input::handle_key(&mut frame, KeyCode::Char('6'));
        assert_ne!(
            frame.regatta_frames_visible(),
            before.regatta_frames_visible()
        );
        let mut focus = app_with_fixture();
        crate::input::handle_key(&mut focus, KeyCode::Right);
        assert_ne!(focus.focus(), before.focus());
    }

    const THEMES: [ThemeId; 2] = [ThemeId::Regatta, ThemeId::HarborLight];

    /// A feed with two machines, no runs, and the `HISTORY` and `TODAY` placeholders filled in.
    fn history_app(history: &str, today: &str) -> App {
        let json = r#"{"schema":1,"at":"2026-10-04T20:00:00Z","chair":{"holder":"h","host":"h","epoch":1,"liveness":"live","beat_age_s":0},"spend":{"five_hour_fraction":0.0,"five_hour_source":"meter","weekly_fraction":0.0,"weekly_source":"meter","hard_stop_fraction":0.0,"five_hour_resets_at":"2026-10-04T22:00:00Z","weekly_resets_at":"2026-10-11T04:00:00Z"},"machines":[{"name":"omarchy","state":"active","lanes_in_use":0,"capacity":3,"login_ok":true,"login_checked_at":"2026-10-04T19:00:00Z","beat_age_s":0,"checkouts":{}},{"name":"spare","state":"active","lanes_in_use":0,"capacity":2,"login_ok":true,"login_checked_at":"2026-10-04T19:00:00Z","beat_age_s":0,"checkouts":{}}],"runs":[],"queue":[],"inbox":[],"watch":[],"history":[HISTORY],"history_today":TODAY}"#
            .replace("HISTORY", history)
            .replace("TODAY", today);
        let snapshot = crate::feed::parse_snapshot(&json).expect("literal feed should parse");
        let mut app = App::default();
        app.apply_snapshot(snapshot);
        app
    }

    const TODAY: &str = r#"{"lands":3,"quarantines":1,"cost_usd":4.2,"runs":5}"#;

    /// One row of each outcome, newest first, ended 2026-10-04 between 08:00Z and 13:12Z.
    const MIXED: &str = r#"
{"run":"r6","machine":"omarchy","initiative":"dash-feed","ended_at":"2026-10-04T13:12:00Z","outcome":"landed","cost_usd":1.25,"landed":[{"task":"p1","pr":41}]},
{"run":"r5","machine":"omarchy","initiative":"dash-feed","ended_at":"2026-10-04T12:40:00Z","outcome":"approved","cost_usd":0.5},
{"run":"r4","machine":"spare","initiative":"history-frame","ended_at":"2026-10-04T11:05:00Z","outcome":"quarantined","cost_usd":2.1,"cause":"verify_failed"},
{"run":"r3","machine":"spare","initiative":"history-frame","ended_at":"2026-10-04T10:30:00Z","outcome":"stopped","cost_usd":0.3},
{"run":"r2","machine":"omarchy","initiative":"dash-feed","ended_at":"2026-10-04T09:15:00Z","outcome":"crashed","cost_usd":0.1},
{"run":"r1","machine":"spare","initiative":"history-frame","ended_at":"2026-10-04T08:00:00Z","outcome":"mystery","cost_usd":0.0}"#;

    fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color::Rgb(r, g, b)
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn draw_history_frame(app: &App, theme_id: ThemeId) -> Terminal<TestBackend> {
        let theme = crate::theme::resolve_for(theme_id, Some("truecolor"));
        let snapshot = app.snapshot().expect("snapshot");
        let mut terminal = Terminal::new(TestBackend::new(80, 9)).expect("terminal");
        terminal
            .draw(|f| render_history(f, f.area(), snapshot, None, app.utc_offset(), &theme))
            .expect("draw should not fail");
        terminal
    }

    #[test]
    fn history_frame_mixed_outcomes_in_the_regatta_theme() {
        let terminal = draw_history_frame(&history_app(MIXED, TODAY), ThemeId::Regatta);
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn history_frame_mixed_outcomes_in_the_harbor_light_theme() {
        let terminal = draw_history_frame(&history_app(MIXED, TODAY), ThemeId::HarborLight);
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn history_frame_empty_in_the_regatta_theme() {
        let terminal = draw_history_frame(&history_app("", TODAY), ThemeId::Regatta);
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn history_frame_empty_in_the_harbor_light_theme() {
        let terminal = draw_history_frame(&history_app("", TODAY), ThemeId::HarborLight);
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn outcome_words_take_their_status_colours_in_both_themes() {
        let expected = [
            (
                ThemeId::Regatta,
                [
                    rgb(0x56, 0xd3, 0x64),
                    rgb(0x56, 0xd4, 0xdd),
                    rgb(0xff, 0x7b, 0x72),
                    rgb(0xe3, 0xb3, 0x41),
                    rgb(0xff, 0x7b, 0x72),
                    rgb(0x8b, 0x94, 0x9e),
                ],
            ),
            (
                ThemeId::HarborLight,
                [
                    rgb(0x1a, 0x7f, 0x37),
                    rgb(0x0a, 0x6c, 0x74),
                    rgb(0xc2, 0x1d, 0x2a),
                    rgb(0x8a, 0x5a, 0x00),
                    rgb(0xc2, 0x1d, 0x2a),
                    rgb(0x57, 0x60, 0x6a),
                ],
            ),
        ];
        let app = history_app(MIXED, TODAY);
        let words = [
            "landed",
            "approved",
            "quarantined",
            "stopped",
            "crashed",
            "unknown",
        ];
        for (theme_id, colors) in expected {
            let terminal = draw_page(&app, theme_id, 120, 40);
            let rect = history_rect(Rect::new(0, 0, 120, 40), &app).expect("history rect");
            let buffer = terminal.backend().buffer();
            for (i, (word, color)) in words.iter().zip(colors).enumerate() {
                let y = rect.y + 2 + i as u16;
                let x = col_of(&terminal, y, word);
                assert_eq!(buffer[(x, y)].fg, color, "{theme_id:?} {word}");
                assert_eq!(buffer[(x + 2, y)].fg, color, "{theme_id:?} {word}");
            }
        }
    }

    #[test]
    fn history_rows_read_time_machine_initiative_outcome_and_cost() {
        let app = history_app(MIXED, TODAY);
        let terminal = draw_page(&app, ThemeId::Regatta, 120, 40);
        let rect = history_rect(Rect::new(0, 0, 120, 40), &app).expect("history rect");
        let row = |i: u16| row_text(&terminal, rect.y + 2 + i);
        assert!(row(0).contains("13:12 omarchy    dash-feed"), "{}", row(0));
        assert!(row(0).contains("landed #41"), "{}", row(0));
        assert!(row(0).contains("$1.25"), "{}", row(0));
        assert!(!row(1).contains('#'), "{}", row(1));
        assert!(row(2).contains("quarantined verify_failed"), "{}", row(2));
        assert!(row(5).contains("08:00 spare"), "{}", row(5));
    }

    #[test]
    fn history_machine_names_wear_the_machines_frame_accent() {
        let app = history_app(MIXED, TODAY);
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let terminal = draw_page(&app, ThemeId::Regatta, 120, 40);
        let rect = history_rect(Rect::new(0, 0, 120, 40), &app).expect("history rect");
        let buffer = terminal.backend().buffer();
        let omarchy = col_of(&terminal, rect.y + 2, "omarchy");
        let spare = col_of(&terminal, rect.y + 4, "spare");
        assert_eq!(buffer[(omarchy, rect.y + 2)].fg, theme.machine_accents[0]);
        assert_eq!(buffer[(spare, rect.y + 4)].fg, theme.machine_accents[1]);
        assert_ne!(theme.machine_accents[0], theme.machine_accents[1]);
    }

    #[test]
    fn the_end_time_is_converted_with_the_offset_the_app_carries() {
        let et = FixedOffset::west_opt(4 * 3600).unwrap();
        let app = history_app(MIXED, TODAY).with_utc_offset(et);
        let terminal = draw_page(&app, ThemeId::Regatta, 120, 40);
        let rect = history_rect(Rect::new(0, 0, 120, 40), &app).expect("history rect");
        assert!(row_text(&terminal, rect.y + 2).contains("09:12 omarchy"));
    }

    #[test]
    fn history_header_prints_todays_counts() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let today = HistoryToday {
            lands: 3,
            quarantines: 1,
            cost_usd: 4.2,
            runs: 5,
        };
        assert_eq!(
            line_text(&history_header(&today, &theme)),
            "today  3 lands  1 quarantines  $4.20  5 runs"
        );
        let app = history_app(MIXED, TODAY);
        let terminal = draw_page(&app, ThemeId::Regatta, 120, 40);
        let rect = history_rect(Rect::new(0, 0, 120, 40), &app).expect("history rect");
        assert!(
            row_text(&terminal, rect.y + 1)
                .contains("today  3 lands  1 quarantines  $4.20  5 runs")
        );
    }

    #[test]
    fn an_empty_history_says_no_runs_ended_yet_in_both_themes() {
        let app = history_app("", TODAY);
        for theme_id in THEMES {
            let terminal = draw_page(&app, theme_id, 120, 40);
            let rect = history_rect(Rect::new(0, 0, 120, 40), &app).expect("history rect");
            assert!(row_text(&terminal, rect.y + 2).contains("no runs ended yet"));
        }
    }

    #[test]
    fn the_selected_history_row_is_marked_and_painted_in_both_themes() {
        use crossterm::event::KeyCode;
        let mut app = history_app(MIXED, TODAY);
        for _ in 0..3 {
            crate::input::handle_key(&mut app, KeyCode::Right);
        }
        app.select_next();
        assert_eq!(app.focus(), Focus::History);
        assert_eq!(app.selected(), 1);
        for theme_id in THEMES {
            let theme = crate::theme::resolve_for(theme_id, Some("truecolor"));
            let terminal = draw_page(&app, theme_id, 120, 40);
            let rect = history_rect(Rect::new(0, 0, 120, 40), &app).expect("history rect");
            let buffer = terminal.backend().buffer();
            assert!(row_text(&terminal, rect.y + 3).contains("\u{25b6} 12:40"));
            assert_eq!(buffer[(rect.x + 3, rect.y + 3)].bg, theme.selected_row);
            assert_eq!(buffer[(rect.x + 3, rect.y + 2)].bg, theme.bg);
            assert_eq!(buffer[(rect.x, rect.y)].fg, theme.border_focus);
        }
    }

    #[test]
    fn history_window_keeps_the_selected_row_on_screen() {
        assert_eq!(history_window_start(None, 3), 0);
        assert_eq!(history_window_start(Some(2), 3), 0);
        assert_eq!(history_window_start(Some(3), 3), 1);
        assert_eq!(history_window_start(Some(9), 0), 10);
    }

    #[test]
    fn history_height_is_rows_plus_three_capped_at_a_third_of_the_body() {
        assert_eq!(history_height(0, 39), 4);
        assert_eq!(history_height(6, 39), 9);
        assert_eq!(history_height(50, 39), 13);
        assert_eq!(history_height(6, 11), 0);
    }

    #[test]
    fn the_history_rect_sits_above_the_key_bar_in_every_preset_and_hides_with_its_flag() {
        let area = Rect::new(0, 0, 120, 40);
        let mut app = history_app(MIXED, TODAY);
        for preset in 0..4 {
            assert_eq!(app.regatta_layout_preset(), preset);
            let history = history_rect(area, &app).expect("history is visible");
            assert_eq!(history, Rect::new(0, 30, 120, 9));
            let (rects, inbox) = layout_rects(area, &app);
            for rect in rects.iter().flatten().chain([&inbox]) {
                assert!(
                    rect.y + rect.height <= history.y,
                    "{rect:?} overlaps {history:?}"
                );
            }
            app.cycle_regatta_layout_preset();
        }
        app.toggle_history_frame();
        assert_eq!(history_rect(area, &app), None);
    }

    #[test]
    fn outcome_color_maps_every_outcome_to_an_existing_role() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        assert_eq!(outcome_color(&theme, Outcome::Landed), theme.landed);
        assert_eq!(
            outcome_color(&theme, Outcome::Approved),
            theme.status_running
        );
        assert_eq!(
            outcome_color(&theme, Outcome::Quarantined),
            theme.quarantined
        );
        assert_eq!(outcome_color(&theme, Outcome::Stopped), theme.meter_mid);
        assert_eq!(outcome_color(&theme, Outcome::Crashed), theme.status_failed);
        assert_eq!(
            outcome_color(&theme, Outcome::Unknown),
            theme.status_waiting
        );
    }

    /// Two machines and the `RUNS` and `SPEND` placeholders filled in. The feed's `at` is
    /// 2026-10-04T14:10:00Z.
    fn cost_app(runs: &str, spend: &str) -> App {
        let json = r#"{"schema":1,"at":"2026-10-04T14:10:00Z","chair":{"holder":"h","host":"h","epoch":1,"liveness":"live","beat_age_s":0},"spend":{"five_hour_fraction":0.0,"five_hour_source":"meter","weekly_fraction":0.0,"weekly_source":"meter","hard_stop_fraction":0.0,"five_hour_resets_at":"2026-10-04T22:00:00Z","weekly_resets_at":"2026-10-11T04:00:00Z"},"machines":[{"name":"omarchy","state":"active","lanes_in_use":1,"capacity":3,"login_ok":true,"login_checked_at":"2026-10-04T14:00:00Z","beat_age_s":0,"checkouts":{}},{"name":"spare","state":"active","lanes_in_use":1,"capacity":2,"login_ok":true,"login_checked_at":"2026-10-04T14:00:00Z","beat_age_s":0,"checkouts":{}}],"runs":[RUNS],"queue":[],"inbox":[],"watch":[],"spend_series":[SPEND]}"#
            .replace("RUNS", runs)
            .replace("SPEND", spend);
        let snapshot = crate::feed::parse_snapshot(&json).expect("literal feed should parse");
        let mut app = App::default();
        app.apply_snapshot(snapshot);
        app
    }

    fn cost_run(name: &str, machine: &str, series: &str) -> String {
        format!(
            r#"{{"run":"{name}","machine":"{machine}","phase":"p","node":"build","attempt":1,"turns":3,"cost":1.0,"verdict":"none","status":"running","cost_series":[{series}]}}"#
        )
    }

    const ALPHA_SERIES: &str = r#"["2026-10-04T14:00:00Z",0.10,"plan"],["2026-10-04T14:04:00Z",0.60,"build"],["2026-10-04T14:09:00Z",1.40,"build"]"#;
    const BETA_SERIES: &str = r#"["2026-10-04T14:02:00Z",0.20,"plan"],["2026-10-04T14:06:00Z",0.50,"build"],["2026-10-04T14:09:00Z",0.90,"build"]"#;
    const SPEND_SERIES: &str = r#"["2026-10-04T13:40:00Z",0.5],["2026-10-04T13:50:00Z",1.5],["2026-10-04T14:00:00Z",2.5],["2026-10-04T14:10:00Z",3.0]"#;

    fn two_run_app() -> App {
        let runs = format!(
            "{},{}",
            cost_run("r-alpha", "omarchy", ALPHA_SERIES),
            cost_run("r-beta", "spare", BETA_SERIES)
        );
        cost_app(&runs, SPEND_SERIES)
    }

    fn draw_cost(app: &App, theme_id: ThemeId) -> Terminal<TestBackend> {
        let theme = crate::theme::resolve_for(theme_id, Some("truecolor"));
        let snapshot = app.snapshot().expect("snapshot");
        let mut terminal = Terminal::new(TestBackend::new(80, 14)).expect("terminal");
        terminal
            .draw(|f| render_run_cost(f, f.area(), snapshot, app.utc_offset(), &theme))
            .expect("draw should not fail");
        terminal
    }

    /// The foreground of every drawn braille cell, in reading order.
    fn braille_fgs(terminal: &Terminal<TestBackend>) -> Vec<Color> {
        let buffer = terminal.backend().buffer();
        buffer
            .content()
            .iter()
            .filter(|c| {
                c.symbol()
                    .chars()
                    .any(|ch| ('\u{2801}'..='\u{28ff}').contains(&ch))
            })
            .map(|c| c.fg)
            .collect()
    }

    fn all_text(terminal: &Terminal<TestBackend>) -> String {
        (0..terminal.backend().buffer().area.height)
            .map(|y| row_text(terminal, y))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn epoch_points_drop_a_point_whose_at_does_not_parse() {
        let points = [("2026-10-04T14:00:00Z", 0.5), ("noon", 1.0)];
        assert_eq!(
            epoch_points(points.into_iter()),
            vec![(1_791_122_400.0, 0.5)]
        );
    }

    #[test]
    fn two_running_runs_draw_two_lines_in_the_regatta_theme() {
        let terminal = draw_cost(&two_run_app(), ThemeId::Regatta);
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn two_running_runs_draw_two_lines_in_the_harbor_light_theme() {
        let terminal = draw_cost(&two_run_app(), ThemeId::HarborLight);
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn each_line_wears_its_machines_accent_and_spend_is_dim_in_both_themes() {
        for theme_id in THEMES {
            let theme = crate::theme::resolve_for(theme_id, Some("truecolor"));
            let terminal = draw_cost(&two_run_app(), theme_id);
            let fgs = braille_fgs(&terminal);
            for color in [
                theme.machine_accents[0],
                theme.machine_accents[1],
                theme.dim,
            ] {
                assert!(
                    fgs.contains(&color),
                    "{theme_id:?}: no braille cell in {color:?}"
                );
            }
            let text = all_text(&terminal);
            assert!(
                text.contains("r-alpha") && text.contains("r-beta"),
                "{text}"
            );
        }
    }

    #[test]
    fn a_legend_name_is_drawn_in_its_runs_accent() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let terminal = draw_cost(&two_run_app(), ThemeId::Regatta);
        let fg_of = |needle: &str| {
            let y = (0..14)
                .find(|y| row_text(&terminal, *y).contains(needle))
                .unwrap_or_else(|| panic!("{needle} is not drawn"));
            cell(&terminal, col_of(&terminal, y, needle), y).fg
        };
        assert_eq!(fg_of("r-alpha"), theme.machine_accents[0]);
        assert_eq!(fg_of("r-beta"), theme.machine_accents[1]);
    }

    #[test]
    fn a_run_with_an_empty_series_draws_nothing_and_is_not_in_the_legend() {
        let runs = format!(
            "{},{}",
            cost_run("r-empty", "omarchy", ""),
            cost_run("r-beta", "spare", BETA_SERIES)
        );
        let app = cost_app(&runs, SPEND_SERIES);
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let terminal = draw_cost(&app, ThemeId::Regatta);
        let text = all_text(&terminal);
        assert!(!text.contains("r-empty"), "{text}");
        assert!(text.contains("r-beta"), "{text}");
        let fgs = braille_fgs(&terminal);
        assert!(fgs.contains(&theme.machine_accents[1]));
        assert!(!fgs.contains(&theme.machine_accents[0]));
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn with_no_point_anywhere_only_the_titled_block_is_drawn() {
        let app = cost_app(&cost_run("r-empty", "omarchy", ""), "");
        let terminal = draw_cost(&app, ThemeId::Regatta);
        assert!(row_text(&terminal, 0).contains(" 8 run cost "));
        assert!(braille_fgs(&terminal).is_empty());
        assert!(!all_text(&terminal).contains("r-empty"));
        assert_eq!(row_text(&terminal, 5).trim_matches(['\u{2502}', ' ']), "");
    }

    #[test]
    fn a_run_that_is_not_running_is_not_drawn() {
        let landed = cost_run("r-done", "omarchy", ALPHA_SERIES).replace("running", "landed");
        let terminal = draw_cost(&cost_app(&landed, ""), ThemeId::Regatta);
        assert!(braille_fgs(&terminal).is_empty());
    }

    #[test]
    fn the_run_cost_band_takes_only_spare_rows_at_120x40_and_160x46() {
        for (w, h) in [(120, 40), (160, 46)] {
            let full = Rect::new(0, 0, w, h);
            let mut app = two_run_app();
            let (with, with_inbox) = layout_rects(full, &app);
            let cost = run_cost_rect(full, &app).expect("the band has room");
            assert!((RUN_COST_MIN_ROWS..=RUN_COST_MAX_ROWS).contains(&cost.height));
            assert_eq!(cost.width, w);
            assert_eq!(with_inbox.y + with_inbox.height, cost.y);
            let history = history_rect(full, &app).expect("history band");
            assert_eq!(cost.y + cost.height, history.y);
            app.toggle_run_cost_frame();
            assert_eq!(run_cost_rect(full, &app), None);
            let (without, _) = layout_rects(full, &app);
            assert_eq!(with[..5], without[..5], "{w}x{h}");
            assert!(with_inbox.height >= CANVAS_INBOX_MIN);
        }
    }

    #[test]
    fn the_run_cost_band_is_absent_when_the_terminal_has_no_spare_rows() {
        assert_eq!(run_cost_rect(Rect::new(0, 0, 80, 24), &two_run_app()), None);
    }

    #[test]
    fn the_run_cost_frame_toggles_off_and_on_through_the_key_handler() {
        use crossterm::event::KeyCode;
        let mut app = two_run_app();
        let area = Rect::new(0, 0, 120, 40);
        let before = frame_rects(area, &app);
        let title = |app: &App| {
            let terminal = draw_page(app, ThemeId::Regatta, 120, 40);
            (0..40).any(|y| row_text(&terminal, y).contains(" 8 run cost "))
        };
        assert!(title(&app));
        crate::input::handle_key(&mut app, KeyCode::Char('8'));
        assert!(!title(&app));
        assert_eq!(run_cost_rect(area, &app), None);
        crate::input::handle_key(&mut app, KeyCode::Char('8'));
        assert!(title(&app));
        assert_eq!(frame_rects(area, &app)[..5], before[..5]);
    }
}
