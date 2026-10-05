//! Rendering for the Regatta page: seven rounded, titled frames (chair, spend, machines,
//! lanes-over-24h, runs, queue, and an always-shown inbox) laid out by `app.regatta_layout_preset()`
//! and reflowed around whichever of frames 1-6 `app.regatta_frames_visible()` hides.
//! [`frame_rects`] is the pure layout core; [`render`] calls it so the rects it draws into and
//! the rects a mouse click is tested against never disagree.

use chrono::{DateTime, FixedOffset, Timelike};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Sparkline},
};

use crate::app::{App, Focus};
use crate::feed::{Chair, FeedSnapshot, Run};
use crate::theme::Theme;

use super::chair_card::{Freshness, TICK_INTERVAL_S, beat_freshness};

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
/// rows and borders need, at most half of the height left after the first two rows.
fn canvas_layout(area: Rect, visible: [bool; 6], runs_len: usize) -> ([Option<Rect>; 6], Rect) {
    let rows_of = |shown: bool, rows: u16| if shown { rows } else { 0 };
    let budget = area.height.saturating_sub(CANVAS_INBOX_MIN);
    let top_h = rows_of(visible[0] || visible[1], CANVAS_TOP_ROWS).min(budget);
    let middle_h = rows_of(visible[2] || visible[3], CANVAS_MIDDLE_ROWS).min(budget - top_h);
    let left = area.height - top_h - middle_h;
    let runs_need = u16::try_from(runs_len.saturating_add(2))
        .unwrap_or(u16::MAX)
        .max(3);
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

fn layout_rects(area: Rect, app: &App) -> ([Option<Rect>; 6], Rect) {
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
            let is_selected = selected == Some(i);
            let prefix = if is_selected { "\u{25b6} " } else { "  " };
            let color = if is_selected {
                theme.accent
            } else {
                theme.machine_accents[i % theme.machine_accents.len()]
            };
            let login = if m.login_ok { "login ok" } else { "login down" };
            let name = &m.name;
            let lanes_in_use = m.lanes_in_use;
            let capacity = m.capacity;
            Line::styled(
                format!("{prefix}{name} {lanes_in_use}/{capacity} {login}"),
                Style::default().fg(color),
            )
        })
        .collect();
    let paragraph = Paragraph::new(lines)
        .block(numbered_block(3, FRAME_NAMES[2], theme, selected.is_some()))
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
        .block(numbered_block(4, FRAME_NAMES[3], theme, false))
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

fn render_runs(
    f: &mut Frame,
    rect: Rect,
    snapshot: &FeedSnapshot,
    selected: Option<usize>,
    theme: &Theme,
) {
    let lines: Vec<Line> = snapshot
        .runs
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let is_selected = selected == Some(i);
            let prefix = if is_selected { "\u{25b6} " } else { "  " };
            let color = if is_selected {
                theme.accent
            } else {
                run_color(theme, r)
            };
            let run = &r.run;
            let machine = &r.machine;
            let phase = &r.phase;
            let node = &r.node;
            let cost = r.cost;
            Line::styled(
                format!("{prefix}{run} {machine} {phase} {node} {cost:.2}"),
                Style::default().fg(color),
            )
        })
        .collect();
    let paragraph = Paragraph::new(lines)
        .block(numbered_block(5, FRAME_NAMES[4], theme, selected.is_some()))
        .style(base_style(theme));
    f.render_widget(paragraph, rect);
}

fn render_queue(
    f: &mut Frame,
    rect: Rect,
    snapshot: &FeedSnapshot,
    selected: Option<usize>,
    theme: &Theme,
) {
    let lines: Vec<Line> = snapshot
        .queue
        .iter()
        .enumerate()
        .map(|(i, q)| {
            let is_selected = selected == Some(i);
            let prefix = if is_selected { "\u{25b6} " } else { "  " };
            let initiative = &q.initiative;
            let phases_landed = q.phases_landed;
            let phases_total = q.phases_total;
            let current_phase = &q.current_phase;
            let text =
                format!("{prefix}{initiative} {phases_landed}/{phases_total} {current_phase}");
            if is_selected {
                Line::styled(text, Style::default().fg(theme.accent))
            } else {
                Line::from(text)
            }
        })
        .collect();
    let paragraph = Paragraph::new(lines)
        .block(numbered_block(6, FRAME_NAMES[5], theme, selected.is_some()))
        .style(base_style(theme));
    f.render_widget(paragraph, rect);
}

fn render_inbox(f: &mut Frame, rect: Rect, snapshot: &FeedSnapshot, theme: &Theme) {
    let block = framed("inbox", theme, false);
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
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
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
        let theme = crate::theme::resolve_for(ThemeId::HarborLight, Some("truecolor"));
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render(f, &app, &theme))
            .expect("draw should not fail");
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
        let row_fg = buffer[(runs_rect.x + 1, runs_rect.y + 1)].fg;
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
        assert_eq!(layout_rects(area, &app), canvas_layout(area, ALL, 0));
    }

    #[test]
    fn canvas_layout_places_every_row_at_120x40_with_one_run() {
        let (rects, inbox) = canvas_layout(Rect::new(0, 0, 120, 40), ALL, 1);
        assert_eq!(rects[0], Some(Rect::new(0, 0, 60, 4)));
        assert_eq!(rects[1], Some(Rect::new(60, 0, 60, 4)));
        assert_eq!(rects[2], Some(Rect::new(0, 4, 48, 6)));
        assert_eq!(rects[3], Some(Rect::new(48, 4, 72, 6)));
        assert_eq!(rects[4], Some(Rect::new(0, 10, 120, 3)));
        assert_eq!(rects[5], Some(Rect::new(0, 13, 69, 27)));
        assert_eq!(inbox, Rect::new(69, 13, 51, 27));
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
        assert_eq!(inbox.y + inbox.height, 24);
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
}
