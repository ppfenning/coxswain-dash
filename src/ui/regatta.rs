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

use crate::actions::{Target, bindings};
use crate::app::{App, Focus, group_queue};
use crate::feed::{
    Chair, CurrentAction, FeedSnapshot, HistoryRow, HistoryToday, InboxEntry, Machine, Outcome,
    QueueEntry, Run, RunEnd,
};
use crate::form::{KEY_ADD_MACHINE, KEY_EDIT, KEY_NEW, KEY_REMOVE};
use crate::theme::Theme;

use super::chair_card::{Freshness, TICK_INTERVAL_S, beat_freshness, short_age};
use super::run_cost::{chart_bounds, dollar_labels, drawable};
use super::{CollapsedRun, collapse_runs, count_suffix, end_chip, local_time};

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

pub fn render(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
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
    if app.regatta_inbox_visible() {
        render_inbox(
            f,
            inbox_area,
            snapshot,
            frame_selection(app, Focus::Inbox),
            theme,
        );
    }
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
    render_key_bar(f, split_key_bar(area).1, app.focus(), theme);
}

/// The rects frames 1-6 render into (`None` for a hidden frame) on a screen of `area`. The page
/// gets only what a shown chair panel leaves, as `ui::render_page` draws it, so a mouse handler
/// hit-tests the frames that are on screen and no rect overlaps the panel.
pub fn frame_rects(area: Rect, app: &App) -> [Option<Rect>; 6] {
    page_frame_rects(super::split_page(area, app).0, app)
}

/// The rects frames 1-6 render into when the page itself is `page`; none on an empty page.
pub fn page_frame_rects(page: Rect, app: &App) -> [Option<Rect>; 6] {
    if page.width == 0 {
        [None; 6]
    } else {
        layout_rects(page, app).0
    }
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
        (4, Focus::Runs, collapse_runs(&snapshot.runs).len()),
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
const CANVAS_TOP_ROWS: u16 = 5;
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

/// Rows the inbox keeps while it is shown, and none once it is hidden.
fn inbox_floor(inbox_visible: bool) -> u16 {
    if inbox_visible { CANVAS_INBOX_MIN } else { 0 }
}

/// Preset 0, the canvas: chair|spend, machines|lanes, runs, then queue|inbox, top to bottom.
/// Returns the rects of frames 1-6 (`None` when hidden) and the inbox rect. A shown inbox is
/// never shorter than `CANVAS_INBOX_MIN` rows (unless `area` itself is), so the rows above it
/// give up height first, top to bottom. A hidden inbox has no floor, the queue takes the whole
/// bottom row, and the returned inbox rect is empty. `runs_len` is the runs list length; frame 5
/// is as tall as its rows, its header row and its borders need, at most half of the height left
/// after the first two rows.
fn canvas_layout(
    area: Rect,
    visible: [bool; 6],
    inbox_visible: bool,
    runs_len: usize,
) -> ([Option<Rect>; 6], Rect) {
    let rows_of = |shown: bool, rows: u16| if shown { rows } else { 0 };
    let budget = area.height.saturating_sub(inbox_floor(inbox_visible));
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
    let (queue, inbox) = split_pair(bottom, bottom.width * 58 / 100, visible[5], inbox_visible);
    (
        [
            chair,
            spend,
            machines,
            lanes,
            visible[4].then_some(runs),
            queue,
        ],
        inbox.unwrap_or(Rect {
            width: 0,
            height: 0,
            ..bottom
        }),
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

/// The fewest rows the history frame is drawn in: two borders, the summary, the column header and
/// one run.
const HISTORY_MIN_ROWS: u16 = 5;

/// Rows the history frame asks for: two borders, the summary, the column header and one row per
/// ended run. An empty history asks for the borders, the summary and one empty-state line, with
/// no column header. It never takes more than a third of `body_h`, and is absent (0) when that
/// third is under `HISTORY_MIN_ROWS`.
fn history_height(len: usize, body_h: u16) -> u16 {
    let rows = if len == 0 {
        1 + 3
    } else {
        len.saturating_add(4)
    };
    let want = u16::try_from(rows).unwrap_or(u16::MAX);
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
    let inbox_rows = inbox_floor(app.regatta_inbox_visible());
    match app.regatta_layout_preset() {
        0 => {
            let runs = u16::try_from(runs_len.saturating_add(3)).unwrap_or(u16::MAX);
            let queue = u16::try_from(queue_len.saturating_add(2)).unwrap_or(u16::MAX);
            let bottom = inbox_rows.max(rows_of(visible[5], queue));
            let left = if visible[4] {
                runs.saturating_mul(2).max(runs.saturating_add(bottom))
            } else {
                bottom
            };
            rows_of(visible[0] || visible[1], CANVAS_TOP_ROWS)
                .saturating_add(rows_of(visible[2] || visible[3], CANVAS_MIDDLE_ROWS))
                .saturating_add(left)
        }
        1 => (floor * shown.div_ceil(2)).saturating_add(inbox_rows),
        2 => (floor * shown).saturating_add(inbox_rows),
        _ => (floor * shown.saturating_sub(1).max(shown.min(1))).saturating_add(inbox_rows),
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
        return canvas_layout(
            area,
            app.regatta_frames_visible(),
            app.regatta_inbox_visible(),
            runs_len,
        );
    }
    let visible = visible_frame_indices(app);
    let mut rects: [Option<Rect>; 6] = [None; 6];
    let [grid_area, inbox_area] = {
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),
                Constraint::Length(inbox_floor(app.regatta_inbox_visible())),
            ])
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
        5 => render_queue(
            f,
            rect,
            snapshot,
            frame_selection(app, Focus::Queue),
            app.queue_grouped(),
            theme,
        ),
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

/// A tick older than this many tick intervals is stalled.
const STALLED_TICK_INTERVALS: u64 = 3;

fn tick_stalled(tick_age_s: Option<u64>) -> bool {
    tick_age_s.is_some_and(|age| age > STALLED_TICK_INTERVALS * TICK_INTERVAL_S)
}

/// The tick part of line three. `None` is a feed that does not carry `tick_age_s`.
fn tick_text(tick_age_s: Option<u64>) -> String {
    tick_age_s.map_or_else(
        || "tick -".to_string(),
        |age| format!("tick {} ago", short_age(age)),
    )
}

/// The action part of line three. `age_s` is `None` when the action's start time is unusable.
fn action_text(action: Option<&CurrentAction>, age_s: Option<u64>) -> String {
    action.map_or_else(
        || "idle".to_string(),
        |a| {
            format!(
                "{} {} \u{b7} {}",
                a.kind,
                a.target,
                age_s.map_or_else(|| "-".to_string(), short_age)
            )
        },
    )
}

/// Whole seconds from `since` to `at`, both RFC 3339; `None` when either is unparseable or
/// `since` is after `at`.
fn action_age_s(at: &str, since: &str) -> Option<u64> {
    let at = DateTime::parse_from_rfc3339(at).ok()?;
    let since = DateTime::parse_from_rfc3339(since).ok()?;
    u64::try_from((at - since).num_seconds()).ok()
}

/// The chair card's three canvas lines. A stale beat or tick is drawn in `meter_high`: the theme
/// has no warning role. `action_age_s` is the current action's age at the feed's `at`.
fn chair_lines(chair: &Chair, action_age_s: Option<u64>, theme: &Theme) -> Vec<Line<'static>> {
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
        chair.phases_today,
        chair.today.needs_chair_open,
        chair.drafts,
        chair.housekeeping_age_s.map(|s| s / 3600),
    ));
    let stale_style = Style::default().fg(theme.meter_high);
    let stalled = tick_stalled(chair.tick_age_s);
    let tick_style = if stalled {
        stale_style
    } else {
        Style::default()
    };
    let failed_style = if chair.today.refused_or_failed > 0 {
        Style::default().fg(theme.status_failed)
    } else {
        Style::default()
    };
    let tick_spans = [
        Some(Span::styled(tick_text(chair.tick_age_s), tick_style)),
        stalled.then(|| Span::styled(" stalled", stale_style)),
    ]
    .into_iter()
    .flatten();
    let tick_line = Line::from(
        tick_spans
            .chain([
                Span::raw(format!(
                    " \u{b7} {} \u{b7} ",
                    action_text(chair.current_action.as_ref(), action_age_s)
                )),
                Span::styled(
                    format!("{} refused or failed today", chair.today.refused_or_failed),
                    failed_style,
                ),
            ])
            .collect::<Vec<_>>(),
    );
    vec![holder_line, counts_line, tick_line]
}

fn render_chair(f: &mut Frame, rect: Rect, snapshot: &FeedSnapshot, theme: &Theme) {
    let block = numbered_block(1, FRAME_NAMES[0], theme, false);
    let age_s = snapshot
        .chair
        .current_action
        .as_ref()
        .and_then(|a| action_age_s(&snapshot.at, &a.since));
    let paragraph = Paragraph::new(chair_lines(&snapshot.chair, age_s, theme))
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
    let runs = app.snapshot().map_or(&[][..], |s| s.runs.as_slice());
    let list_w = lane_runs_width(inner.width, runs.len());
    // The run lines sit beside the chart rows only; the axis keeps the full width.
    let chart_w = inner.width - list_w;
    let history = app.lanes_history();
    let peak = lane_peak(history);
    let offset = app.utc_offset();
    let stamp = |at: DateTime<FixedOffset>| local_time(&at.to_rfc3339(), offset);
    let (start, end) = lane_window(history)
        .map_or_else(Default::default, |(start, end)| (stamp(start), stamp(end)));
    let axis = lane_axis_line(&start, peak, &end, usize::from(inner.width));
    let chart = braille_area(
        &lane_columns(history, usize::from(chart_w) * 2),
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
    if list_w > 0 {
        let list_area = Rect {
            x: inner.x + chart_w + 1,
            width: list_w - 1,
            height: inner.height - 1,
            ..inner
        };
        render_lane_runs(f, list_area, runs, theme);
    }
}

/// Width of a run line in the lanes frame: run, stage and cost.
const LANE_RUN_WIDTH: usize = RUN_COLUMNS[0] + 1 + RUN_COLUMNS[2] + 1 + COST_WIDTH;

/// The least width the lanes chart keeps when the run lines sit beside it.
const LANE_CHART_MIN: u16 = 24;

/// Columns the lanes frame gives its run lines, gap included: none when there are no runs or the
/// chart would drop under `LANE_CHART_MIN`.
fn lane_runs_width(inner_width: u16, runs: usize) -> u16 {
    let wanted = LANE_RUN_WIDTH as u16 + 1;
    if runs > 0 && inner_width >= LANE_CHART_MIN + wanted {
        wanted
    } else {
        0
    }
}

/// One run's line in the lanes frame: run, stage coloured by status, cost.
fn lane_run_line(r: &Run, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::raw(format!("{} ", fit_cell(&r.run, RUN_COLUMNS[0]))),
        stage_span(r, RUN_COLUMNS[2], theme),
        Span::raw(format!(" {:>COST_WIDTH$}", cost_text(r.cost))),
    ])
}

/// The run lines that fit `area`, then `+ N more` for the rest.
fn render_lane_runs(f: &mut Frame, area: Rect, runs: &[Run], theme: &Theme) {
    let (shown, more) = overflow_split(runs.len(), usize::MAX, usize::from(area.height));
    let lines: Vec<Line> = runs[..shown]
        .iter()
        .map(|r| lane_run_line(r, theme))
        .chain((more > 0).then(|| more_line(more, theme)))
        .collect();
    f.render_widget(Paragraph::new(lines).style(base_style(theme)), area);
}

/// Widths of the runs columns, left to right: run, machine, phase, node. The runs frame joins
/// phase and node into one stage column of `STAGE_WIDTH`. ATT and COST are right-aligned in
/// `ATT_WIDTH` and `COST_WIDTH`.
const RUN_COLUMNS: [usize; 4] = [12, 10, 22, 12];
const STAGE_WIDTH: usize = RUN_COLUMNS[2] + 1 + RUN_COLUMNS[3];
const ATT_WIDTH: usize = 3;
const COST_WIDTH: usize = 7;
/// The PROJECT column at full width; a narrow runs frame gives it less, down to nothing.
const PROJECT_WIDTH: usize = 14;
/// Columns a runs row spends left of the project cell and the end chip: the prefix, RUN, MACHINE,
/// the stage and the run tail, each with its separator. PROJECT never takes any of these.
const RUN_FIXED_WIDTH: usize =
    2 + RUN_COLUMNS[0] + 1 + RUN_COLUMNS[1] + 1 + STAGE_WIDTH + ATT_WIDTH + COST_WIDTH + 3;
/// Status words a run row can fall back to when its feed sends no end kind.
const STATUS_WORDS: [&str; 9] = [
    "running",
    "paused",
    "needs_person",
    "needs_chair",
    "approved",
    "landed",
    "quarantined",
    "failed",
    "waiting",
];

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

/// A run's stage: `phase · node`, whichever of the two is known, or `starting` when neither is.
fn stage_label(phase: Option<&str>, node: Option<&str>) -> String {
    match (phase, node) {
        (Some(phase), Some(node)) => format!("{phase} \u{b7} {node}"),
        (Some(only), None) | (None, Some(only)) => only.to_string(),
        (None, None) => "starting".to_string(),
    }
}

/// A cost in dollars and cents.
fn cost_text(dollars: f64) -> String {
    format!("${dollars:.2}")
}

/// The stage text's colour for a run status: running is cyan, paused yellow and needs a person
/// magenta, each borrowed from the role that already carries the hue. Any other status keeps the
/// frame's text colour. The feed names no paused or needs-a-person status yet, so those are
/// matched on the words `paused`, `needs_person` and `needs_chair`.
fn stage_color(theme: &Theme, status: &str) -> Option<Color> {
    match status {
        "running" => Some(theme.status_running),
        "paused" => Some(theme.meter_mid),
        "needs_person" | "needs_chair" => Some(theme.approved),
        _ => None,
    }
}

/// A run's stage padded to `width` and coloured by its status. The feed sends an unknown phase
/// or node as an empty string.
fn stage_span(r: &Run, width: usize, theme: &Theme) -> Span<'static> {
    let label = stage_label(
        Some(r.phase.as_str()).filter(|s| !s.is_empty()),
        Some(r.node.as_str()).filter(|s| !s.is_empty()),
    );
    let style = stage_color(theme, &r.status)
        .map_or_else(Style::default, |color| Style::default().fg(color));
    Span::styled(fit_cell(&label, width), style)
}

/// A project in one dim `width`-char cell: cut like RUN and MACHINE, `-` when the row has none.
fn project_cell(project: Option<&str>, width: usize, theme: &Theme) -> Span<'static> {
    let text = project.filter(|p| !p.is_empty()).unwrap_or("-");
    Span::styled(fit_cell(text, width), Style::default().fg(theme.dim))
}

/// Chars the widest end chip can draw: the longest `\u{25cf} label` over every end kind and every
/// status word, taken from `end_chip` itself so a new label cannot outgrow the reserve.
fn widest_end_chip(theme: &Theme) -> usize {
    let ends = [
        RunEnd::Running,
        RunEnd::Landed,
        RunEnd::Approved,
        RunEnd::Quarantined { cause: None },
        RunEnd::Idle,
        RunEnd::Died { cause: None },
        RunEnd::Stopped,
    ];
    let chips = ends
        .iter()
        .map(|end| end_chip(Some(end), "", theme))
        .chain(STATUS_WORDS.iter().map(|s| end_chip(None, s, theme)));
    chips
        .map(|chip| chip.spans.iter().map(|s| s.content.chars().count()).sum())
        .max()
        .unwrap_or(0)
}

/// The PROJECT column's width in a runs frame `inner_width` wide. It takes only what is left
/// after the fixed columns, the end chip's `chip_reserve` and its own separator, so it shrinks
/// first and can reach 0; RUN, MACHINE and the stage keep their widths.
fn project_width(inner_width: usize, chip_reserve: usize) -> usize {
    let spare = inner_width.saturating_sub(RUN_FIXED_WIDTH + chip_reserve);
    PROJECT_WIDTH.min(spare.saturating_sub(1))
}

/// The runs line up to the stage column; the header and the rows share it so they align. A
/// `project` cell sits between RUN and MACHINE with its own separator; `None` leaves it out.
fn run_lead(
    prefix: &str,
    run: &str,
    project: Option<Span<'static>>,
    machine: &str,
) -> Vec<Span<'static>> {
    let [run_w, machine_w, ..] = RUN_COLUMNS;
    std::iter::once(Span::raw(format!("{prefix}{} ", fit_cell(run, run_w))))
        .chain(project.into_iter().flat_map(|p| [p, Span::raw(" ")]))
        .chain([Span::raw(format!("{} ", fit_cell(machine, machine_w)))])
        .collect()
}

/// The runs line from the stage column to the status column.
fn run_tail(att: &str, cost: &str) -> String {
    format!(" {att:>ATT_WIDTH$} {cost:>COST_WIDTH$} ")
}

fn run_header(project_w: usize, theme: &Theme) -> Line<'static> {
    let project = (project_w > 0).then(|| project_cell(Some("PROJECT"), project_w, theme));
    let rest = format!(
        "{}{}STATUS",
        fit_cell("STAGE", STAGE_WIDTH),
        run_tail("ATT", "COST")
    );
    let spans = run_lead("  ", "RUN", project, "MACHINE")
        .into_iter()
        .chain([Span::raw(rest)])
        .collect::<Vec<_>>();
    Line::from(spans).style(Style::default().fg(theme.dim))
}

/// One initiative's row: its newest run, the run's end chip, and a dim `×N` when the initiative
/// had more than one run. A selected row is prefixed `▶` and sits on `theme.selected_row`.
fn run_row(row: &CollapsedRun, selected: bool, project_w: usize, theme: &Theme) -> Line<'static> {
    let r = row.run;
    let prefix = if selected { "\u{25b6} " } else { "  " };
    let project = (project_w > 0).then(|| project_cell(r.project.as_deref(), project_w, theme));
    let lead = run_lead(prefix, &r.run, project, &r.machine);
    let stage = [
        stage_span(r, STAGE_WIDTH, theme),
        Span::raw(run_tail(&r.attempt.to_string(), &cost_text(r.cost))),
    ];
    let chip = end_chip(r.end.as_ref(), &r.status, theme).spans;
    let line = Line::from(
        lead.into_iter()
            .chain(stage)
            .chain(chip)
            .chain(count_suffix(row.count, theme))
            .collect::<Vec<_>>(),
    );
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
    let collapsed = collapse_runs(&snapshot.runs);
    let count_w = collapsed
        .iter()
        .filter_map(|row| count_suffix(row.count, theme))
        .map(|s| s.content.chars().count())
        .max()
        .unwrap_or(0);
    let project_w = project_width(usize::from(inner.width), widest_end_chip(theme) + count_w);
    let header = std::iter::once(run_header(project_w, theme));
    let shown = selected_run_row(&snapshot.runs, selected);
    let rows = collapsed
        .iter()
        .enumerate()
        .map(|(i, row)| run_row(row, shown == Some(i), project_w, theme));
    let paragraph = Paragraph::new(header.chain(rows).collect::<Vec<_>>())
        .block(block)
        .style(base_style(theme));
    f.render_widget(paragraph, rect);
    // The header takes the first inner row.
    paint_selected_row(f, inner, shown.map(|i| i + 1), theme);
}

/// The collapsed row that holds `runs[selected]`. `selected` indexes the uncollapsed list. Rows keep
/// first-seen order, so the row is the one whose count grows when `runs[selected]` joins the runs
/// before it, or the new last row when its initiative has not appeared yet.
fn selected_run_row(runs: &[Run], selected: Option<usize>) -> Option<usize> {
    let sel = selected?;
    let with = collapse_runs(runs.get(..=sel)?);
    let without = collapse_runs(&runs[..sel]);
    with.iter()
        .enumerate()
        .position(|(i, w)| without.get(i).is_none_or(|o| o.count != w.count))
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
/// waiting, and a run that ended having built nothing (idle) as dim.
fn outcome_color(theme: &Theme, outcome: Outcome) -> Color {
    match outcome {
        Outcome::Landed => theme.landed,
        Outcome::Approved => theme.status_running,
        Outcome::Quarantined => theme.quarantined,
        Outcome::Stopped => theme.meter_mid,
        Outcome::Idle => theme.dim,
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
        Outcome::Idle => "idle",
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

/// Chars of the history time column, `HH:MM`.
const HISTORY_TIME_WIDTH: usize = 5;
/// Columns a history row spends outside PROJECT: the prefix, TIME, MACHINE, INITIATIVE, STATUS and
/// COST, each with its separator. PROJECT never takes any of these.
const HISTORY_FIXED_WIDTH: usize = 2
    + HISTORY_TIME_WIDTH
    + 1
    + RUN_COLUMNS[1]
    + 1
    + RUN_COLUMNS[2]
    + 1
    + HISTORY_OUTCOME_WIDTH
    + 1
    + COST_WIDTH;

/// The PROJECT column's width in a history frame `inner_width` wide: what is left after the fixed
/// columns and its own separator, up to `PROJECT_WIDTH`. 0 leaves the column out.
fn history_project_width(inner_width: usize) -> usize {
    let spare = inner_width.saturating_sub(HISTORY_FIXED_WIDTH);
    PROJECT_WIDTH.min(spare.saturating_sub(1))
}

/// The history column names, dim, over the cells `history_row` draws at the same widths. PROJECT
/// is left out when `project_w` is 0.
fn history_columns(project_w: usize, theme: &Theme) -> Line<'static> {
    let project = if project_w > 0 {
        format!("{} ", fit_cell("PROJECT", project_w))
    } else {
        String::new()
    };
    Line::styled(
        format!(
            "  {} {} {project}{} {} {:>COST_WIDTH$}",
            fit_cell("TIME", HISTORY_TIME_WIDTH),
            fit_cell("MACHINE", RUN_COLUMNS[1]),
            fit_cell("INITIATIVE", RUN_COLUMNS[2]),
            fit_cell("STATUS", HISTORY_OUTCOME_WIDTH),
            "COST"
        ),
        Style::default().fg(theme.dim),
    )
}

/// One ended run: end time in `offset`, machine in `accent`, a dim project when `project_w` is
/// above 0, initiative, outcome word in its colour with its cause or PR, then cost. A selected
/// row is prefixed `▶` and sits on `theme.selected_row`.
fn history_row(
    row: &HistoryRow,
    offset: FixedOffset,
    accent: Color,
    selected: bool,
    project_w: usize,
    theme: &Theme,
) -> Line<'static> {
    let prefix = if selected { "\u{25b6} " } else { "  " };
    let word = outcome_word(row.outcome);
    let line = Line::from(
        [
            Span::raw(format!("{prefix}{} ", local_time(&row.ended_at, offset))),
            Span::styled(
                fit_cell(&row.machine, RUN_COLUMNS[1]),
                Style::default().fg(accent),
            ),
            Span::raw(" "),
        ]
        .into_iter()
        .chain(
            (project_w > 0)
                .then(|| {
                    [
                        project_cell(row.project.as_deref(), project_w, theme),
                        Span::raw(" "),
                    ]
                })
                .into_iter()
                .flatten(),
        )
        .chain([
            Span::raw(fit_cell(&row.initiative, RUN_COLUMNS[2])),
            Span::raw(" "),
            Span::styled(word, Style::default().fg(outcome_color(theme, row.outcome))),
            Span::styled(
                fit_cell(&outcome_detail(row), HISTORY_OUTCOME_WIDTH - word.len()),
                Style::default().fg(theme.dim),
            ),
            Span::raw(format!(" {:>COST_WIDTH$}", format!("${:.2}", row.cost_usd))),
        ])
        .collect::<Vec<_>>(),
    );
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

/// Rows of the history frame above the runs: the summary and the column header.
const HISTORY_HEAD_ROWS: usize = 2;

/// The history frame: today's counts, the column header, then one row per ended run in feed
/// order, newest first. An empty history draws the counts and one line, with no column header.
fn render_history(
    f: &mut Frame,
    rect: Rect,
    snapshot: &FeedSnapshot,
    selected: Option<usize>,
    offset: FixedOffset,
    theme: &Theme,
) {
    let block = numbered_block(7, "history", theme, selected.is_some());
    let inner = block.inner(rect);
    let shown = selected.filter(|i| *i < snapshot.history.len());
    let rows = usize::from(inner.height).saturating_sub(HISTORY_HEAD_ROWS);
    let start = history_window_start(shown, rows);
    let project_w = history_project_width(usize::from(inner.width));
    let summary = history_header(&snapshot.history_today, theme);
    let lines: Vec<Line<'static>> = if snapshot.history.is_empty() {
        vec![
            summary,
            Line::styled("no runs ended yet", Style::default().fg(theme.dim)),
        ]
    } else {
        [summary, history_columns(project_w, theme)]
            .into_iter()
            .chain(
                snapshot
                    .history
                    .iter()
                    .enumerate()
                    .skip(start)
                    .take(rows)
                    .map(|(i, r)| {
                        let accent = machine_accent(snapshot, &r.machine, theme);
                        history_row(r, offset, accent, shown == Some(i), project_w, theme)
                    }),
            )
            .collect()
    };
    let paragraph = Paragraph::new(lines).block(block).style(base_style(theme));
    f.render_widget(paragraph, rect);
    // The selected row sits under the summary and the header; one scrolled out of view gets none.
    let at = shown
        .and_then(|i| i.checked_sub(start))
        .filter(|d| *d < rows)
        .map(|d| d + HISTORY_HEAD_ROWS);
    paint_selected_row(f, inner, at, theme);
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
    let line = Line::from(
        std::iter::once(Span::raw(prefix))
            .chain([
                Span::raw(fit_cell(&q.initiative, initiative_w)),
                Span::raw(" "),
                bar(filled, bar_color),
                bar(QUEUE_BAR_CELLS - filled, theme.track),
                Span::raw(tail),
            ])
            .collect::<Vec<_>>(),
    );
    if selected {
        line.style(Style::default().fg(theme.accent).bg(theme.selected_row))
    } else {
        line
    }
}

/// The queue's lines and the line the selected row lands on. Grouped, each project gets a dim
/// header line over its rows, with no-project rows last under `other`. `selected` indexes `rows`.
fn queue_lines(
    rows: &[QueueEntry],
    grouped: bool,
    selected: Option<usize>,
    width: usize,
    theme: &Theme,
) -> (Vec<Line<'static>>, Option<usize>) {
    let chosen = selected.and_then(|i| rows.get(i));
    let row = |q: &QueueEntry| {
        let is_chosen = chosen.is_some_and(|c| std::ptr::eq(c, q));
        (queue_line(q, is_chosen, width, theme), is_chosen)
    };
    let items: Vec<(Line<'static>, bool)> = if grouped {
        group_queue(rows)
            .into_iter()
            .flat_map(|(label, members)| {
                std::iter::once((
                    Line::styled(label.to_string(), Style::default().fg(theme.dim)),
                    false,
                ))
                .chain(members.into_iter().map(row))
                .collect::<Vec<_>>()
            })
            .collect()
    } else {
        rows.iter().map(row).collect()
    };
    let at = items.iter().position(|(_, is_chosen)| *is_chosen);
    (items.into_iter().map(|(line, _)| line).collect(), at)
}

fn render_queue(
    f: &mut Frame,
    rect: Rect,
    snapshot: &FeedSnapshot,
    selected: Option<usize>,
    grouped: bool,
    theme: &Theme,
) {
    let block = numbered_block(6, FRAME_NAMES[5], theme, selected.is_some());
    let inner = block.inner(rect);
    let (all, at) = queue_lines(
        &snapshot.queue,
        grouped,
        selected,
        usize::from(inner.width),
        theme,
    );
    let (shown, more) = overflow_split(all.len(), usize::MAX, usize::from(inner.height));
    let lines: Vec<Line> = all
        .into_iter()
        .take(shown)
        .chain((more > 0).then(|| more_line(more, theme)))
        .collect();
    let paragraph = Paragraph::new(lines).block(block).style(base_style(theme));
    f.render_widget(paragraph, rect);
    paint_selected_row(f, inner, at.filter(|i| *i < shown), theme);
}

/// Inbox items drawn before the rest fold into `+ N more`.
const INBOX_ITEMS: usize = 3;

fn inbox_line(i: &InboxEntry, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::raw(format!("{} {} ", i.kind, i.target)),
        Span::styled(i.reason.clone(), Style::default().fg(theme.dim)),
    ])
}

fn render_inbox(
    f: &mut Frame,
    rect: Rect,
    snapshot: &FeedSnapshot,
    selected: Option<usize>,
    theme: &Theme,
) {
    let block = numbered_block(9, "inbox", theme, false);
    let inner = block.inner(rect);
    let rows = usize::from(inner.height);
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
    paint_selected_row(f, inner, selected.filter(|i| *i < shown), theme);
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
const KEY_BAR: [(&str, &str); 8] = [
    ("1-9", "frames"),
    ("\u{2190}\u{2192}", "focus"),
    ("\u{2191}\u{2193}", "select"),
    ("\u{23ce}", "drill down"),
    ("t", "theme"),
    ("v", "layout"),
    ("g", "group queue"),
    ("q", "quit"),
];

/// The view keys, after the palette. Fixed by the initiative, not read from the keymap.
const KEY_VIEWS: [(&str, &str); 3] = [("w", "watch"), ("$", "spend"), ("h", "health")];

/// The action keys of the list in `focus`, from `actions::bindings`. History has no actions.
/// The placeholder id is never read: `bindings` keys on the kind of target alone.
pub(super) fn action_hints(focus: Focus) -> Vec<(char, &'static str)> {
    let target = match focus {
        Focus::Runs => Target::Run(String::new()),
        Focus::Machines => Target::Machine(String::new()),
        Focus::Queue => Target::Initiative(String::new()),
        Focus::Inbox => Target::InboxItem(String::new()),
        Focus::History => return Vec::new(),
    };
    bindings(&target)
        .into_iter()
        .map(|b| (b.key, b.label))
        .collect()
}

/// The key bar for a `width`-column row: the fixed keys, a `│`, the focused list's action keys,
/// `:` for the palette, then the view keys. The part after the `│` shows before the fixed keys do.
/// When the row is too narrow, fixed keys are dropped whole from the end. Only if the tail alone is
/// still too wide are action hints dropped whole from the end. A hint is never cut mid-word.
fn key_bar_line(focus: Focus, width: u16, theme: &Theme) -> Line<'static> {
    let key = Style::default().fg(theme.border_focus);
    let label = Style::default().fg(theme.dim);
    let pair = |k: String, l: &str| [Span::styled(k, key), Span::styled(format!(" {l}"), label)];
    let hints: Vec<Vec<Span<'static>>> = action_hints(focus)
        .into_iter()
        .chain((focus == Focus::Machines).then_some((KEY_ADD_MACHINE, "add machine")))
        .chain(
            (focus == Focus::Queue)
                .then_some([(KEY_NEW, "new"), (KEY_EDIT, "edit"), (KEY_REMOVE, "remove")])
                .into_iter()
                .flatten(),
        )
        .map(|(k, l)| pair(k.to_string(), l).to_vec())
        .collect();
    let tail_for = |n: usize| -> Vec<Span<'static>> {
        std::iter::once(Span::styled(" \u{2502} ", label))
            .chain(hints[..n].iter().enumerate().flat_map(|(i, hint)| {
                (i > 0)
                    .then(|| Span::styled("  ", label))
                    .into_iter()
                    .chain(hint.clone())
            }))
            .chain((n > 0).then(|| Span::styled(" \u{b7} ", label)))
            .chain(pair(":".to_string(), "palette"))
            .chain(KEY_VIEWS.iter().flat_map(|(k, l)| {
                std::iter::once(Span::styled("  ", label)).chain(pair((*k).to_string(), l))
            }))
            .collect()
    };
    let kept = (0..=hints.len())
        .rev()
        .find(|&n| Line::from(tail_for(n)).width() <= usize::from(width))
        .unwrap_or(0);
    let tail = tail_for(kept);
    let fixed: Vec<Vec<Span<'static>>> = KEY_BAR
        .iter()
        .enumerate()
        .map(|(i, (k, l))| {
            (i > 0)
                .then(|| Span::styled(" \u{b7} ", label))
                .into_iter()
                .chain(pair((*k).to_string(), l))
                .collect()
        })
        .collect();
    let room = usize::from(width).saturating_sub(Line::from(tail.clone()).width());
    let fit = fixed
        .iter()
        .scan(0, |used, item| {
            *used += Line::from(item.clone()).width();
            Some(*used)
        })
        .take_while(|used| *used <= room)
        .count();
    Line::from(
        fixed
            .into_iter()
            .take(fit)
            .flatten()
            .chain(tail)
            .collect::<Vec<_>>(),
    )
}

fn render_key_bar(f: &mut Frame, rect: Rect, focus: Focus, theme: &Theme) {
    f.render_widget(
        Paragraph::new(key_bar_line(focus, rect.width, theme)).style(base_style(theme)),
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

    fn draw_chair_in(chair: &str, id: ThemeId, width: u16) -> Terminal<TestBackend> {
        let snapshot =
            crate::feed::parse_snapshot(&chair_feed(chair)).expect("literal feed should parse");
        let theme = crate::theme::resolve_for(id, Some("truecolor"));
        let mut terminal = Terminal::new(TestBackend::new(width, 5)).expect("terminal");
        terminal
            .draw(|f| render_chair(f, f.area(), &snapshot, &theme))
            .expect("draw should not fail");
        terminal
    }

    fn draw_chair(chair: &str) -> Terminal<TestBackend> {
        draw_chair_in(chair, ThemeId::Regatta, 80)
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

    const TICK_FRESH: &str = r#"{"holder": "chair@omarchy:12345", "host": "omarchy", "epoch": 7, "liveness": "live", "beat_age_s": 4, "tick_age_s": 12, "last_tick_at": "2026-09-28T23:59:48Z", "last_status": "landing", "current_action": {"kind": "land_phase", "target": "api-runners/runner-parity", "since": "2026-09-28T23:58:30Z"}, "today": {"lands": 2, "launches": 1, "refused_or_failed": 0, "needs_chair_open": 0}}"#;

    const TICK_STALLED: &str = r#"{"holder": "chair@omarchy:12345", "host": "omarchy", "epoch": 7, "liveness": "live", "beat_age_s": 20, "tick_age_s": 400, "last_tick_at": "2026-09-28T23:53:20Z", "last_status": "chair idle", "current_action": null, "today": {"lands": 1, "launches": 0, "refused_or_failed": 3, "needs_chair_open": 1}}"#;

    const BOTH_THEMES: [(ThemeId, &str); 2] = [
        (ThemeId::Regatta, "regatta"),
        (ThemeId::HarborLight, "harbor_light"),
    ];

    /// The foreground of the first cell of `needle` on row `y`.
    fn fg_of(terminal: &Terminal<TestBackend>, y: u16, needle: &str) -> Color {
        terminal.backend().buffer()[(col_of(terminal, y, needle), y)].fg
    }

    #[test]
    fn chair_tick_fresh_with_a_current_action() {
        for (id, name) in BOTH_THEMES {
            let theme = crate::theme::resolve_for(id, Some("truecolor"));
            let terminal = draw_chair_in(TICK_FRESH, id, 100);
            insta::assert_snapshot!(
                format!("chair_tick_fresh_{name}"),
                terminal.backend().to_string()
            );
            assert!(row_text(&terminal, 3).contains("tick 12s ago"));
            assert!(!row_text(&terminal, 3).contains("stalled"));
            assert!(
                row_text(&terminal, 3).contains("land_phase api-runners/runner-parity \u{b7} 1m")
            );
            assert_ne!(fg_of(&terminal, 3, "tick 12s ago"), theme.meter_high);
            assert_ne!(fg_of(&terminal, 3, "0 refused"), theme.status_failed);
        }
    }

    #[test]
    fn chair_tick_stalled_is_drawn_in_the_stale_colour_and_failures_in_the_failed_colour() {
        for (id, name) in BOTH_THEMES {
            let theme = crate::theme::resolve_for(id, Some("truecolor"));
            let terminal = draw_chair_in(TICK_STALLED, id, 100);
            insta::assert_snapshot!(
                format!("chair_tick_stalled_{name}"),
                terminal.backend().to_string()
            );
            assert!(row_text(&terminal, 3).contains("tick 6m ago stalled \u{b7} idle"));
            assert_eq!(fg_of(&terminal, 3, "tick 6m ago"), theme.meter_high);
            assert_eq!(fg_of(&terminal, 3, "stalled"), theme.meter_high);
            assert_eq!(fg_of(&terminal, 3, "3 refused"), theme.status_failed);
        }
    }

    #[test]
    fn chair_tick_absent_starts_with_a_dash_and_does_not_claim_stalled() {
        for (id, name) in BOTH_THEMES {
            let terminal = draw_chair_in(IDLE, id, 100);
            insta::assert_snapshot!(
                format!("chair_tick_absent_{name}"),
                terminal.backend().to_string()
            );
            assert!(row_text(&terminal, 3).starts_with("\u{2502}tick - \u{b7} idle"));
            assert!(!row_text(&terminal, 3).contains("stalled"));
        }
    }

    #[test]
    fn a_tick_is_stalled_only_past_three_tick_intervals() {
        assert!(!tick_stalled(None));
        assert!(!tick_stalled(Some(180)));
        assert!(tick_stalled(Some(181)));
    }

    #[test]
    fn tick_text_reads_the_age_or_a_dash() {
        assert_eq!(tick_text(Some(12)), "tick 12s ago");
        assert_eq!(tick_text(Some(400)), "tick 6m ago");
        assert_eq!(tick_text(None), "tick -");
    }

    #[test]
    fn action_text_reads_kind_target_and_age_or_idle() {
        let action = CurrentAction {
            kind: "land_phase".to_string(),
            target: "a/b".to_string(),
            since: "2026-09-28T23:58:30Z".to_string(),
        };
        assert_eq!(
            action_text(Some(&action), Some(90)),
            "land_phase a/b \u{b7} 1m"
        );
        assert_eq!(action_text(Some(&action), None), "land_phase a/b \u{b7} -");
        assert_eq!(action_text(None, Some(90)), "idle");
    }

    #[test]
    fn action_age_is_the_seconds_from_since_to_at_or_none() {
        let at = "2026-09-29T00:00:00Z";
        assert_eq!(action_age_s(at, "2026-09-28T23:58:30Z"), Some(90));
        assert_eq!(action_age_s(at, "2026-09-29T00:00:10Z"), None);
        assert_eq!(action_age_s(at, "not a time"), None);
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
            .draw(|f| render(f, f.area(), app, &theme))
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
            .draw(|f| render(f, f.area(), &app, &theme))
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
            .draw(|f| render(f, f.area(), &app, &theme))
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
        assert_eq!(layout_rects(area, &app), canvas_layout(body, ALL, true, 0));
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
        let (rects, inbox) = canvas_layout(Rect::new(0, 0, 120, 40), ALL, true, 1);
        assert_eq!(rects[0], Some(Rect::new(0, 0, 60, 5)));
        assert_eq!(rects[1], Some(Rect::new(60, 0, 60, 5)));
        assert_eq!(rects[2], Some(Rect::new(0, 5, 48, 6)));
        assert_eq!(rects[3], Some(Rect::new(48, 5, 72, 6)));
        assert_eq!(rects[4], Some(Rect::new(0, 11, 120, 4)));
        assert_eq!(rects[5], Some(Rect::new(0, 15, 69, 25)));
        assert_eq!(inbox, Rect::new(69, 15, 51, 25));
    }

    #[test]
    fn canvas_layout_caps_runs_at_half_of_what_is_left() {
        let (rects, inbox) = canvas_layout(Rect::new(0, 0, 120, 40), ALL, true, 100);
        assert_eq!(rects[4], Some(Rect::new(0, 11, 120, 14)));
        assert_eq!(inbox.height, 15);
    }

    #[test]
    fn canvas_layout_keeps_the_inbox_at_five_rows_down_to_a_short_terminal() {
        for height in [24, 16, 12, 9] {
            let (rects, inbox) = canvas_layout(Rect::new(0, 0, 80, height), ALL, true, 100);
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
            .draw(|f| render(f, f.area(), &app, &theme))
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
        let (rects, inbox) = canvas_layout(Rect::new(0, 0, 100, 30), hidden, true, 3);
        assert_eq!(rects[0], None);
        assert_eq!(rects[1], Some(Rect::new(0, 0, 100, 5)));
        assert_eq!(rects[2], None);
        assert_eq!(rects[3], Some(Rect::new(0, 5, 100, 6)));
        assert_eq!(rects[5], None);
        assert_eq!(inbox.width, 100);
    }

    #[test]
    fn a_hidden_inbox_gives_the_queue_the_whole_bottom_row() {
        let area = Rect::new(0, 0, 120, 40);
        let mut app = app_with_fixture();
        let shown = frame_rects(area, &app)[5].expect("queue rect");
        assert!(shown.width < area.width, "{shown:?}");
        app.toggle_inbox_frame();
        let hidden = frame_rects(area, &app)[5].expect("queue rect");
        assert_eq!((hidden.x, hidden.width), (0, area.width));
        assert_eq!(hidden.y + hidden.height, shown.y + shown.height);
        app.toggle_inbox_frame();
        assert_eq!(frame_rects(area, &app)[5], Some(shown));
    }

    #[test]
    fn a_hidden_inbox_drops_its_five_row_floor() {
        let (rects, inbox) = canvas_layout(Rect::new(0, 0, 80, 8), ALL, false, 100);
        assert_eq!(inbox.height, 0);
        assert_eq!(rects[5].map(|r| r.width), Some(80));
        let (_, shown) = canvas_layout(Rect::new(0, 0, 80, 8), ALL, true, 100);
        assert_eq!(shown.height, CANVAS_INBOX_MIN);
    }

    #[test]
    fn every_frame_title_begins_with_its_toggle_digit() {
        let terminal = draw_page(&two_run_app(), ThemeId::Regatta, 120, 40);
        let screen: String = (0..40)
            .map(|y| row_text(&terminal, y))
            .collect::<Vec<_>>()
            .join("\n");
        let titles: Vec<&str> = screen.split('\u{256d}').skip(1).collect();
        assert_eq!(titles.len(), 9, "{screen}");
        let names = [
            "chair",
            "spend",
            "machines",
            "lanes over 24h",
            "runs",
            "queue",
            "history",
            "run cost",
            "inbox",
        ];
        for (i, name) in names.iter().enumerate() {
            let want = format!(" {} {name} ", i + 1);
            assert!(
                titles.iter().any(|t| t.starts_with(&want)),
                "no title {want:?} in {screen}"
            );
        }
    }

    #[test]
    fn titles_are_bold_padded_in_the_title_color_and_borders_use_the_border_color() {
        let app = app_with_fixture();
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("terminal");
        terminal
            .draw(|f| render(f, f.area(), &app, &theme))
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
            "RUN PROJECT MACHINE STAGE ATT COST STATUS"
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

    /// The runs frame alone, 120 wide, with nothing selected.
    fn draw_runs_frame(runs: &[String], theme_id: ThemeId, height: u16) -> Terminal<TestBackend> {
        let app = feed_app(&runs.join(","), "", "");
        let theme = crate::theme::resolve_for(theme_id, Some("truecolor"));
        let mut terminal = Terminal::new(TestBackend::new(120, height)).expect("terminal");
        terminal
            .draw(|f| render_runs(f, f.area(), app.snapshot().expect("feed"), None, &theme))
            .expect("draw should not fail");
        terminal
    }

    fn ended_run(name: &str, status: &str, end: &str) -> String {
        run_json(name, status).replace(r#""status""#, &format!(r#""end":{end},"status""#))
    }

    /// One run of each end kind, each its own initiative.
    fn one_run_of_each_end_kind() -> Vec<String> {
        vec![
            ended_run("a-1", "running", r#""running""#),
            ended_run("b-1", "landed", r#""landed""#),
            ended_run("c-1", "approved", r#""approved""#),
            ended_run(
                "d-1",
                "quarantined",
                r#"{"kind":"quarantined","cause":"tests failed"}"#,
            ),
            ended_run("e-1", "idle", r#""idle""#),
            ended_run("f-1", "failed", r#"{"kind":"died","cause":"oom"}"#),
            ended_run("g-1", "stopped", r#""stopped""#),
        ]
    }

    #[test]
    fn runs_end_kinds_in_the_regatta_theme() {
        let terminal = draw_runs_frame(&one_run_of_each_end_kind(), ThemeId::Regatta, 10);
        insta::assert_snapshot!("runs_end_kinds_regatta", terminal.backend().to_string());
    }

    #[test]
    fn runs_end_kinds_in_the_harbor_light_theme() {
        let terminal = draw_runs_frame(&one_run_of_each_end_kind(), ThemeId::HarborLight, 10);
        insta::assert_snapshot!(
            "runs_end_kinds_harbor_light",
            terminal.backend().to_string()
        );
    }

    #[test]
    fn three_runs_of_one_initiative_draw_one_row_with_a_count_of_three() {
        let runs = ["x-1", "x-2", "x-3"].map(|id| ended_run(id, "landed", r#""landed""#));
        let terminal = draw_runs_frame(&runs, ThemeId::Regatta, 6);
        let rows: Vec<String> = (2..5).map(|y| row_text(&terminal, y)).collect();
        let hits: Vec<&String> = rows.iter().filter(|r| r.contains("x-")).collect();
        assert_eq!(hits.len(), 1, "rows: {rows:?}");
        assert!(hits[0].contains("x-3"), "row: {:?}", hits[0]);
        assert!(hits[0].contains("\u{d7}3"), "row: {:?}", hits[0]);
    }

    #[test]
    fn stage_label_joins_phase_and_node_or_names_the_one_known() {
        assert_eq!(stage_label(Some("p1"), Some("build")), "p1 \u{b7} build");
        assert_eq!(stage_label(Some("p1"), None), "p1");
        assert_eq!(stage_label(None, Some("build")), "build");
        assert_eq!(stage_label(None, None), "starting");
    }

    #[test]
    fn cost_text_is_dollars_with_cents() {
        assert_eq!(cost_text(1.37), "$1.37");
        assert_eq!(cost_text(0.0), "$0.00");
        assert_eq!(cost_text(12.3456), "$12.35");
    }

    #[test]
    fn stage_color_is_cyan_yellow_or_magenta_by_status_and_none_otherwise() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        assert_eq!(
            stage_color(&theme, "running"),
            Some(Color::Rgb(0x56, 0xd4, 0xdd))
        );
        assert_eq!(stage_color(&theme, "paused"), Some(MID));
        assert_eq!(
            stage_color(&theme, "needs_person"),
            Some(Color::Rgb(0xd2, 0xa8, 0xff))
        );
        assert_eq!(stage_color(&theme, "landed"), None);
    }

    fn stage_run(name: &str, status: &str, phase: &str, node: &str, cost: f64) -> String {
        format!(
            r#"{{"run":"{name}","machine":"m0","phase":"{phase}","node":"{node}","attempt":1,"turns":3,"cost":{cost},"verdict":"none","status":"{status}","cost_series":[]}}"#
        )
    }

    /// The lanes frame over the runs frame, 120 wide, drawn from one literal feed.
    fn draw_lanes_and_runs(runs: &[String]) -> Terminal<TestBackend> {
        let app = feed_app(&runs.join(","), "", "");
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let mut terminal = Terminal::new(TestBackend::new(120, 13)).expect("terminal");
        terminal
            .draw(|f| {
                render_lanes(f, Rect::new(0, 0, 120, 6), &app, &theme);
                render_runs(
                    f,
                    Rect::new(0, 6, 120, 7),
                    app.snapshot().expect("feed"),
                    None,
                    &theme,
                );
            })
            .expect("draw should not fail");
        terminal
    }

    #[test]
    fn the_lanes_and_runs_frames_show_each_runs_stage_and_cost_coloured_by_status() {
        let terminal = draw_lanes_and_runs(&[
            stage_run("run-a", "running", "p1-build", "build", 1.37),
            stage_run("run-b", "paused", "p2-review", "review", 0.5),
            stage_run("run-c", "needs_person", "p3-land", "land", 12.0),
        ]);
        insta::assert_snapshot!(terminal.backend().to_string());
        let cyan = Some(Color::Rgb(0x56, 0xd4, 0xdd));
        let magenta = Some(Color::Rgb(0xd2, 0xa8, 0xff));
        // Lanes run lines sit on rows 1-3, runs rows on rows 8-10.
        for (lanes_row, runs_row, stage, color) in [
            (1, 8, "p1-build \u{b7} build", cyan),
            (2, 9, "p2-review \u{b7} review", Some(MID)),
            (3, 10, "p3-land \u{b7} land", magenta),
        ] {
            assert_eq!(fg_at(&terminal, lanes_row, stage), color);
            assert_eq!(fg_at(&terminal, runs_row, stage), color);
        }
        assert!(row_text(&terminal, 1).contains("$1.37"));
        assert!(row_text(&terminal, 10).contains("$12.00"));
    }

    #[test]
    fn a_run_with_no_cost_series_phase_or_node_shows_starting_and_zero_dollars() {
        let terminal = draw_lanes_and_runs(&[stage_run("run-a", "running", "", "", 0.0)]);
        insta::assert_snapshot!(terminal.backend().to_string());
        for y in [1, 8] {
            let row = row_text(&terminal, y);
            assert!(row.contains("starting"), "row {y}: {row:?}");
            assert!(row.contains("$0.00"), "row {y}: {row:?}");
        }
    }

    #[test]
    fn at_68_columns_the_lanes_axis_keeps_its_peak_and_end_time_beside_the_run_lines() {
        let app = app_with_fixture();
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let mut terminal = Terminal::new(TestBackend::new(70, 6)).expect("terminal");
        terminal
            .draw(|f| render_lanes(f, f.area(), &app, &theme))
            .expect("draw should not fail");
        assert!(row_text(&terminal, 1).contains("dash-feed-1"));
        let axis = row_text(&terminal, 4);
        assert!(axis.contains("lanes in use \u{b7} peak 2"), "{axis:?}");
        assert!(axis.ends_with("00:00\u{2502}"), "{axis:?}");
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
            .draw(|f| {
                render_queue(
                    f,
                    f.area(),
                    app.snapshot().expect("snapshot"),
                    None,
                    false,
                    &theme,
                );
            })
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
        let terminal = draw_page(&app, ThemeId::Regatta, 200, 40);
        let row = row_text(&terminal, 39);
        assert_eq!(
            row.trim_end(),
            "1-9 frames \u{b7} \u{2190}\u{2192} focus \u{b7} \u{2191}\u{2193} select \u{b7} \u{23ce} drill down \u{b7} t theme \u{b7} v layout \u{b7} g group queue \u{b7} q quit \u{2502} p pause  k kill  m move \u{b7} : palette  w watch  $ spend  h health"
        );
        assert_eq!(
            fg_at(&terminal, 39, "$ spend"),
            Some(Color::Rgb(0x79, 0xc0, 0xff))
        );
        assert_eq!(
            fg_at(&terminal, 39, "spend"),
            Some(Color::Rgb(0x8b, 0x94, 0x9e))
        );
        assert_eq!(
            fg_at(&terminal, 39, "1-9"),
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
        assert_eq!(
            fg_at(&terminal, 39, "k kill"),
            Some(Color::Rgb(0x79, 0xc0, 0xff))
        );
        assert_eq!(
            fg_at(&terminal, 39, "kill"),
            Some(Color::Rgb(0x8b, 0x94, 0x9e))
        );
        assert_eq!(
            fg_at(&terminal, 39, ": palette"),
            Some(Color::Rgb(0x79, 0xc0, 0xff))
        );
    }

    /// A feed with a run, a queue entry and two inbox items, focused on `focus` by pressing Right.
    fn app_focused_on(focus: Focus) -> App {
        use crossterm::event::KeyCode;
        let mut app = feed_app(
            &run_json("r0", "running"),
            &queue_json(1, 0, 2),
            &inbox_json(2),
        );
        for _ in 0..5 {
            if app.focus() != focus {
                crate::input::handle_key(&mut app, KeyCode::Right);
            }
        }
        assert_eq!(app.focus(), focus);
        app
    }

    /// The key bar row of a `width`x40 page.
    fn key_bar_row(focus: Focus, width: u16) -> String {
        let terminal = draw_page(&app_focused_on(focus), ThemeId::Regatta, width, 40);
        row_text(&terminal, 39).trim_end().to_string()
    }

    const FIXED_KEYS: &str = "1-9 frames \u{b7} \u{2190}\u{2192} focus \u{b7} \u{2191}\u{2193} select \u{b7} \u{23ce} drill down \u{b7} t theme \u{b7} v layout \u{b7} g group queue \u{b7} q quit";

    const FOCUS_ACTIONS: [(Focus, &str); 4] = [
        (Focus::Runs, "p pause  k kill  m move"),
        (
            Focus::Machines,
            "+ lanes up  - lanes down  d drain  a activate  A add machine",
        ),
        (
            Focus::Queue,
            "] priority up  [ priority down  n new  e edit  x remove",
        ),
        (Focus::Inbox, "a accept  x deny"),
    ];

    const VIEW_KEYS: &str = "w watch  $ spend  h health";

    #[test]
    fn at_120_columns_every_focus_shows_its_actions_the_palette_and_the_view_keys() {
        for (focus, actions) in FOCUS_ACTIONS {
            let row = key_bar_row(focus, 120);
            assert!(
                row.ends_with(&format!(
                    " \u{2502} {actions} \u{b7} : palette  {VIEW_KEYS}"
                )),
                "{row}"
            );
            assert!(row.starts_with("1-9 frames"), "{row}");
        }
    }

    #[test]
    fn a_fixed_key_that_does_not_fit_is_dropped_whole_from_the_end() {
        assert_eq!(
            key_bar_row(Focus::Machines, 120),
            "1-9 frames \u{2502} + lanes up  - lanes down  d drain  a activate  A add machine \u{b7} : palette  w watch  $ spend  h health"
        );
        assert_eq!(
            key_bar_row(Focus::Queue, 160),
            "1-9 frames \u{b7} \u{2190}\u{2192} focus \u{b7} \u{2191}\u{2193} select \u{b7} \u{23ce} drill down \u{b7} t theme \u{2502} ] priority up  [ priority down  n new  e edit  x remove \u{b7} : palette  w watch  $ spend  h health"
        );
    }

    #[test]
    fn the_queue_key_bar_at_80_columns_drops_actions_from_the_end_and_keeps_the_view_keys() {
        let row = key_bar_row(Focus::Queue, 80);
        assert!(row.chars().count() <= 80, "{row}");
        assert!(row.ends_with(&format!(" : palette  {VIEW_KEYS}")), "{row}");
        assert!(row.contains("] priority up"), "{row}");
    }

    #[test]
    fn every_key_bar_at_80_columns_is_one_row_with_the_view_keys() {
        for (focus, _) in FOCUS_ACTIONS.into_iter().chain([(Focus::History, "")]) {
            let row = key_bar_row(focus, 80);
            assert!(row.chars().count() <= 80, "{row}");
            ["w watch", "$ spend", "h health"]
                .iter()
                .for_each(|hint| assert!(row.contains(hint), "{hint} missing from {row}"));
        }
    }

    #[test]
    fn action_hints_name_each_lists_bindings() {
        assert_eq!(
            action_hints(Focus::Runs),
            [('p', "pause"), ('k', "kill"), ('m', "move")]
        );
        assert_eq!(
            action_hints(Focus::Machines),
            [
                ('+', "lanes up"),
                ('-', "lanes down"),
                ('d', "drain"),
                ('a', "activate")
            ]
        );
        assert_eq!(
            action_hints(Focus::Queue),
            [(']', "priority up"), ('[', "priority down")]
        );
        assert_eq!(action_hints(Focus::Inbox), [('a', "accept"), ('x', "deny")]);
        assert_eq!(action_hints(Focus::History), []);
    }

    #[test]
    fn the_key_bar_keeps_its_fixed_keys_and_adds_the_focused_lists_actions() {
        for (focus, actions) in FOCUS_ACTIONS {
            assert_eq!(
                key_bar_row(focus, 200),
                format!("{FIXED_KEYS} \u{2502} {actions} \u{b7} : palette  {VIEW_KEYS}")
            );
        }
        assert_eq!(
            key_bar_row(Focus::History, 200),
            format!("{FIXED_KEYS} \u{2502} : palette  {VIEW_KEYS}")
        );
    }

    #[test]
    fn the_key_bar_snapshot_with_runs_focus() {
        insta::assert_snapshot!(key_bar_row(Focus::Runs, 120));
    }

    #[test]
    fn the_key_bar_snapshot_with_machines_focus() {
        insta::assert_snapshot!(key_bar_row(Focus::Machines, 120));
    }

    #[test]
    fn the_key_bar_snapshot_with_queue_focus() {
        insta::assert_snapshot!(key_bar_row(Focus::Queue, 120));
    }

    #[test]
    fn the_key_bar_snapshot_with_inbox_focus() {
        insta::assert_snapshot!(key_bar_row(Focus::Inbox, 120));
    }

    fn bg_at_inbox_row(terminal: &Terminal<TestBackend>, app: &App, row: u16) -> Color {
        let rect = inbox_rect(app);
        terminal.backend().buffer()[(rect.x + 1, rect.y + 1 + row)].bg
    }

    #[test]
    fn the_inbox_highlights_its_selected_row_only_while_it_holds_focus() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let inbox = app_focused_on(Focus::Inbox);
        assert_eq!(inbox.selected(), 0);
        let terminal = draw_page(&inbox, ThemeId::Regatta, 120, 40);
        assert_eq!(bg_at_inbox_row(&terminal, &inbox, 0), theme.selected_row);
        assert_ne!(bg_at_inbox_row(&terminal, &inbox, 1), theme.selected_row);
        let runs = app_focused_on(Focus::Runs);
        let terminal = draw_page(&runs, ThemeId::Regatta, 120, 40);
        assert_ne!(bg_at_inbox_row(&terminal, &runs, 0), theme.selected_row);
    }

    #[test]
    fn the_inbox_highlight_follows_the_selection_down() {
        use crossterm::event::KeyCode;
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let mut app = app_focused_on(Focus::Inbox);
        crate::input::handle_key(&mut app, KeyCode::Down);
        assert_eq!(app.selected(), 1);
        let terminal = draw_page(&app, ThemeId::Regatta, 120, 40);
        assert_ne!(bg_at_inbox_row(&terminal, &app, 0), theme.selected_row);
        assert_eq!(bg_at_inbox_row(&terminal, &app, 1), theme.selected_row);
    }

    #[test]
    fn every_key_on_the_key_bar_changes_the_app() {
        use crossterm::event::KeyCode;
        let before = app_with_fixture();
        let mut layout = app_with_fixture();
        // `v` cycles the layout on any focus; `p` stays run pause.
        crate::input::handle_key(&mut layout, KeyCode::Char('v'));
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

    /// The history frame alone in a `width` x `height` terminal, `selected` being its selection.
    fn draw_history_at(
        app: &App,
        theme_id: ThemeId,
        width: u16,
        height: u16,
        selected: Option<usize>,
    ) -> Terminal<TestBackend> {
        let theme = crate::theme::resolve_for(theme_id, Some("truecolor"));
        let snapshot = app.snapshot().expect("snapshot");
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|f| render_history(f, f.area(), snapshot, selected, app.utc_offset(), &theme))
            .expect("draw should not fail");
        terminal
    }

    /// The history frame at 80 wide, as tall as it asks to be (at least 9 rows).
    fn draw_history_frame(app: &App, theme_id: ThemeId) -> Terminal<TestBackend> {
        let asks = app.snapshot().map_or(0, |s| s.history.len()) + 4;
        let height = u16::try_from(asks).expect("a short history").max(9);
        draw_history_at(app, theme_id, 80, height, None)
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

    /// `row_json` with a `"project"` key whose value is the JSON literal `project`.
    fn with_project(row_json: &str, project: &str) -> String {
        format!("{},\"project\":{project}}}", row_json.trim_end_matches('}'))
    }

    fn project_theme() -> Theme {
        crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"))
    }

    fn draw_project_runs_frame(app: &App, theme: &Theme, width: u16) -> Terminal<TestBackend> {
        let snapshot = app.snapshot().expect("snapshot");
        let mut terminal = Terminal::new(TestBackend::new(width, 6)).expect("terminal");
        terminal
            .draw(|f| render_runs(f, f.area(), snapshot, None, theme))
            .expect("draw should not fail");
        terminal
    }

    fn draw_queue_frame(app: &App, theme: &Theme) -> Terminal<TestBackend> {
        let snapshot = app.snapshot().expect("snapshot");
        let mut terminal = Terminal::new(TestBackend::new(60, 5)).expect("terminal");
        terminal
            .draw(|f| render_queue(f, f.area(), snapshot, None, app.queue_grouped(), theme))
            .expect("draw should not fail");
        terminal
    }

    fn runs_with_projects() -> App {
        let runs = [
            with_project(&run_json("alpha-1", "running"), r#""pat-skylight""#),
            with_project(&run_json("beta-1", "running"), "null"),
        ];
        feed_app(&runs.join(","), "", "")
    }

    #[test]
    fn the_runs_frame_shows_a_dim_project_column_between_run_and_machine() {
        let theme = project_theme();
        let terminal = draw_project_runs_frame(&runs_with_projects(), &theme, 120);
        insta::assert_snapshot!(terminal.backend().to_string());
        let header = col_of(&terminal, 1, "PROJECT");
        let x = col_of(&terminal, 2, "pat-skylight");
        assert_eq!(x, header);
        assert!(col_of(&terminal, 2, "alpha-1") < x && x < col_of(&terminal, 2, "m0"));
        assert_eq!(terminal.backend().buffer()[(x, 2)].fg, theme.dim);
        assert!(!row_text(&terminal, 2).trim_end().ends_with("pat-skylight"));
    }

    #[test]
    fn a_run_with_no_project_draws_a_dim_dash_in_the_project_column() {
        let theme = project_theme();
        let terminal = draw_project_runs_frame(&runs_with_projects(), &theme, 120);
        let x = col_of(&terminal, 1, "PROJECT");
        let cell = &terminal.backend().buffer()[(x, 3)];
        assert_eq!(cell.symbol(), "-");
        assert_eq!(cell.fg, theme.dim);
        assert_eq!(terminal.backend().buffer()[(x + 1, 3)].symbol(), " ");
    }

    /// The widths of the PROJECT and RUN columns, read off the header row. A cut header still
    /// starts `PROJE`.
    fn header_widths(terminal: &Terminal<TestBackend>) -> (usize, usize) {
        let run = col_of(terminal, 1, "RUN");
        let machine = col_of(terminal, 1, "MACHINE");
        if row_text(terminal, 1).contains("PROJE") {
            let project = col_of(terminal, 1, "PROJE");
            (
                usize::from(machine - project - 1),
                usize::from(project - run - 1),
            )
        } else {
            (0, usize::from(machine - run - 1))
        }
    }

    #[test]
    fn a_narrow_runs_frame_shrinks_project_first_and_keeps_the_longest_chip_whole() {
        let theme = project_theme();
        let runs = [
            with_project(&run_json("alpha-1", "needs_person"), r#""pat-skylight""#),
            with_project(&run_json("beta-1", "needs_person"), "null"),
        ];
        let app = feed_app(&runs.join(","), "", "");
        let chip = "\u{25cf} needs_person";
        let at = |width| draw_project_runs_frame(&app, &theme, width);
        let (wide, hundred, ninety) = (at(120), at(100), at(90));
        for terminal in [&wide, &hundred, &ninety] {
            assert!(row_text(terminal, 1).contains("STATUS"));
            assert!(row_text(terminal, 2).contains(chip));
            assert!(row_text(terminal, 3).contains(chip));
            assert_eq!(header_widths(terminal).1, RUN_COLUMNS[0]);
        }
        assert_eq!(header_widths(&wide).0, PROJECT_WIDTH);
        let narrow = header_widths(&hundred).0;
        assert!(0 < narrow && narrow < PROJECT_WIDTH);
        assert_eq!(
            col_of(&hundred, 2, "pat-sky"),
            col_of(&hundred, 1, "PROJECT")
        );
        assert_eq!(header_widths(&ninety).0, 0);
        assert!(!row_text(&ninety, 2).contains("pat-"));
    }

    #[test]
    fn the_count_suffix_is_reserved_beside_the_longest_chip() {
        let theme = project_theme();
        let runs = [
            with_project(&run_json("alpha-1", "needs_person"), r#""pat-skylight""#),
            with_project(&run_json("alpha-2", "needs_person"), r#""pat-skylight""#),
        ];
        let app = feed_app(&runs.join(","), "", "");
        let terminal = draw_project_runs_frame(&app, &theme, 100);
        assert!(row_text(&terminal, 2).contains("\u{25cf} needs_person \u{d7}2"));
        let (project, run) = header_widths(&terminal);
        assert!(project < PROJECT_WIDTH);
        assert_eq!(run, RUN_COLUMNS[0]);
    }

    #[test]
    fn the_widest_end_chip_covers_the_longest_status_word() {
        let theme = project_theme();
        assert!(widest_end_chip(&theme) >= "\u{25cf} needs_person".chars().count());
        assert!(widest_end_chip(&theme) >= "\u{25cf} quarantined".chars().count());
    }

    #[test]
    fn project_width_takes_only_what_is_left_after_the_fixed_columns_and_chip() {
        let chip = 14;
        let full = RUN_FIXED_WIDTH + chip;
        assert_eq!(project_width(full + 100, chip), PROJECT_WIDTH);
        assert_eq!(project_width(full + 10, chip), 9);
        assert_eq!(project_width(full + 10, chip + 3), 6);
        assert_eq!(project_width(full + 1, chip), 0);
        assert_eq!(project_width(full, chip), 0);
        assert_eq!(project_width(0, chip), 0);
    }

    #[test]
    fn project_cell_cuts_pads_dims_and_falls_back_to_a_dash() {
        let theme = project_theme();
        let cell = project_cell(Some("alpha"), 8, &theme);
        assert_eq!(cell.content, "alpha   ");
        assert_eq!(cell.style.fg, Some(theme.dim));
        assert_eq!(project_cell(None, 4, &theme).content, "-   ");
        assert_eq!(project_cell(Some(""), 4, &theme).content, "-   ");
        assert_eq!(
            project_cell(Some("pat-skylight"), 6, &theme).content,
            "pat-s\u{2026}"
        );
    }

    fn queue_with_projects() -> App {
        let entry = |name: &str, project: &str| {
            format!(
                r#"{{"initiative":"{name}","priority":1,"phases_landed":1,"phases_total":3,"current_phase":"p","project":{project}}}"#
            )
        };
        let queue = [
            entry("alpha", r#""pat-skylight""#),
            entry("beta", "null"),
            entry("gamma", r#""""#),
        ];
        feed_app("", &queue.join(","), "")
    }

    #[test]
    fn the_queue_line_draws_the_initiative_alone() {
        let theme = project_theme();
        let terminal = draw_queue_frame(&queue_with_projects(), &theme);
        insta::assert_snapshot!(terminal.backend().to_string());
        assert!(row_text(&terminal, 1).contains("alpha"));
        assert!(!row_text(&terminal, 1).contains("pat-skylight"));
        let bar = |y| col_of(&terminal, y, "\u{2588}");
        assert_eq!(bar(1), bar(2));
        assert_eq!(bar(2), bar(3));
        assert_eq!(
            row_text(&terminal, 2).trim_end(),
            {
                let plain = feed_app("", &queue_json(1, 1, 3), "");
                let again = draw_queue_frame(&plain, &theme);
                row_text(&again, 1).replace("init-0", "beta  ")
            }
            .trim_end()
        );
    }

    /// Four rows over two projects, interleaved, and one with no project.
    fn queue_with_two_projects_and_a_null() -> App {
        let entry = |name: &str, project: &str| {
            format!(
                r#"{{"initiative":"{name}","priority":1,"phases_landed":1,"phases_total":3,"current_phase":"p","project":{project}}}"#
            )
        };
        let queue = [
            entry("alpha", r#""pat-skylight""#),
            entry("beta", "null"),
            entry("gamma", r#""pat-towpath""#),
            entry("delta", r#""pat-skylight""#),
        ];
        feed_app("", &queue.join(","), "")
    }

    fn draw_tall_queue_frame(app: &App, theme: &Theme) -> Terminal<TestBackend> {
        let snapshot = app.snapshot().expect("snapshot");
        let mut terminal = Terminal::new(TestBackend::new(60, 9)).expect("terminal");
        terminal
            .draw(|f| render_queue(f, f.area(), snapshot, None, app.queue_grouped(), theme))
            .expect("draw should not fail");
        terminal
    }

    #[test]
    fn the_queue_frame_ungrouped_keeps_feed_order() {
        let theme = project_theme();
        let terminal = draw_tall_queue_frame(&queue_with_two_projects_and_a_null(), &theme);
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn the_queue_frame_grouped_has_a_header_per_project_and_other_last() {
        let theme = project_theme();
        let mut app = queue_with_two_projects_and_a_null();
        app.toggle_queue_grouped();
        let terminal = draw_tall_queue_frame(&app, &theme);
        insta::assert_snapshot!(terminal.backend().to_string());
        let x = col_of(&terminal, 1, "pat-skylight");
        assert_eq!(terminal.backend().buffer()[(x, 1)].fg, theme.dim);
    }

    #[test]
    fn grouped_queue_lines_follow_the_groups_and_move_the_selection_with_its_row() {
        let theme = project_theme();
        let app = queue_with_two_projects_and_a_null();
        let rows = &app.snapshot().expect("snapshot").queue;
        let text = |line: &Line| {
            line.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        };
        let (lines, at) = queue_lines(rows, true, Some(3), 60, &theme);
        let heads: Vec<String> = lines
            .iter()
            .map(text)
            .filter(|t| !t.starts_with(' ') && !t.starts_with('\u{25b6}'))
            .collect();
        assert_eq!(heads, ["pat-skylight", "pat-towpath", "other"]);
        assert_eq!(at, Some(2));
        assert!(text(&lines[2]).starts_with("\u{25b6} delta"));
        assert_eq!(queue_lines(rows, false, Some(3), 60, &theme).1, Some(3));
    }

    #[test]
    fn the_key_bar_lists_g_for_grouping_the_queue() {
        assert!(KEY_BAR.contains(&("g", "group queue")));
        assert!(key_bar_row(Focus::Queue, 200).contains("g group queue"));
    }

    /// Two history rows, one with a project and one with none.
    fn history_with_a_project() -> App {
        let rows = [
            with_project(
                r#"{"run":"r2","machine":"omarchy","initiative":"dash-feed","ended_at":"2026-10-04T13:12:00Z","outcome":"landed","cost_usd":1.25}"#,
                r#""pat-skylight""#,
            ),
            with_project(
                r#"{"run":"r1","machine":"spare","initiative":"history-frame","ended_at":"2026-10-04T12:40:00Z","outcome":"approved","cost_usd":0.5}"#,
                "null",
            ),
        ];
        history_app(&rows.join(","), TODAY)
    }

    /// The history frame's rows: 0 is the border, 1 the summary, 2 the column header, 3 the first run.
    #[test]
    fn the_history_frame_draws_the_project_in_its_own_dim_column_before_the_initiative() {
        let theme = project_theme();
        let app = history_with_a_project();
        let terminal = draw_history_at(&app, ThemeId::Regatta, 120, 9, None);
        insta::assert_snapshot!(terminal.backend().to_string());
        let x = col_of(&terminal, 2, "PROJECT");
        assert_eq!(col_of(&terminal, 3, "pat-skylight"), x);
        assert_eq!(terminal.backend().buffer()[(x, 3)].fg, theme.dim);
        assert!(col_of(&terminal, 2, "MACHINE") < x);
        assert!(x < col_of(&terminal, 2, "INITIATIVE"));
        assert_eq!(
            col_of(&terminal, 3, "landed"),
            col_of(&terminal, 4, "approved")
        );
    }

    #[test]
    fn a_missing_project_draws_a_dash_in_the_project_column() {
        let theme = project_theme();
        let terminal = draw_history_at(&history_with_a_project(), ThemeId::Regatta, 120, 9, None);
        let x = col_of(&terminal, 2, "PROJECT");
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(x, 4)].symbol(), "-");
        assert_eq!(buffer[(x, 4)].fg, theme.dim);
        assert_eq!(buffer[(x + 1, 4)].symbol(), " ");
    }

    #[test]
    fn the_history_initiative_cell_holds_the_initiative_alone() {
        let terminal = draw_history_at(&history_with_a_project(), ThemeId::Regatta, 120, 9, None);
        let x = usize::from(col_of(&terminal, 2, "INITIATIVE"));
        let cell = |y: u16| -> String {
            row_text(&terminal, y)
                .chars()
                .skip(x)
                .take(RUN_COLUMNS[2])
                .collect()
        };
        assert_eq!(cell(3), fit_cell("dash-feed", RUN_COLUMNS[2]));
        assert_eq!(cell(4), fit_cell("history-frame", RUN_COLUMNS[2]));
    }

    #[test]
    fn the_history_header_names_every_column_and_sits_under_the_summary() {
        let theme = project_theme();
        let names = ["TIME", "MACHINE", "PROJECT", "INITIATIVE", "STATUS", "COST"];
        let line = history_columns(PROJECT_WIDTH, &theme);
        let text = line_text(&line);
        let at: Vec<usize> = names.iter().map(|n| text.find(n).expect(n)).collect();
        assert!(at.windows(2).all(|w| w[0] < w[1]), "{text:?}");
        assert_eq!(line.style.fg, Some(theme.dim));
        let without = line_text(&history_columns(0, &theme));
        assert!(!without.contains("PROJECT"), "{without:?}");
        assert!(
            ["INITIATIVE", "STATUS", "COST"]
                .iter()
                .all(|n| without.contains(n))
        );
        let app = history_app(MIXED, TODAY);
        let terminal = draw_page(&app, ThemeId::Regatta, 120, 40);
        let rect = history_rect(Rect::new(0, 0, 120, 40), &app).expect("history rect");
        assert!(row_text(&terminal, rect.y + 1).contains("today  3 lands"));
        let header = row_text(&terminal, rect.y + 2);
        assert!(names.iter().all(|n| header.contains(n)), "{header:?}");
    }

    #[test]
    fn history_columns_and_rows_share_one_width_set() {
        let theme = project_theme();
        let app = history_with_a_project();
        let row = &app.snapshot().expect("snapshot").history[0];
        for project_w in [0, 3, PROJECT_WIDTH] {
            let header = line_text(&history_columns(project_w, &theme));
            let line = line_text(&history_row(
                row,
                app.utc_offset(),
                theme.dim,
                false,
                project_w,
                &theme,
            ));
            assert_eq!(header.chars().count(), line.chars().count(), "{project_w}");
        }
    }

    #[test]
    fn history_project_width_takes_only_what_is_left_after_the_fixed_columns() {
        assert_eq!(HISTORY_FIXED_WIDTH, 78);
        assert_eq!(history_project_width(78), 0);
        assert_eq!(history_project_width(79), 0);
        assert_eq!(history_project_width(80), 1);
        assert_eq!(history_project_width(78 + 1 + PROJECT_WIDTH), PROJECT_WIDTH);
        assert_eq!(history_project_width(118), PROJECT_WIDTH);
    }

    #[test]
    fn at_80_wide_the_project_column_is_left_out_and_every_other_column_draws() {
        let terminal = draw_history_at(&history_with_a_project(), ThemeId::Regatta, 80, 9, None);
        let header = row_text(&terminal, 2);
        let first = row_text(&terminal, 3);
        assert!(!header.contains("PROJ"), "{header:?}");
        assert!(!first.contains("pat-skylight"), "{first:?}");
        for name in ["TIME", "MACHINE", "INITIATIVE", "STATUS", "COST"] {
            assert!(header.contains(name), "{name} {header:?}");
        }
        for cell in ["13:12 omarchy", "dash-feed", "landed", "$1.25"] {
            assert!(first.contains(cell), "{cell} {first:?}");
        }
    }

    #[test]
    fn at_120_wide_the_project_column_is_drawn() {
        let terminal = draw_history_at(&history_with_a_project(), ThemeId::Regatta, 120, 9, None);
        assert!(row_text(&terminal, 2).contains("PROJECT"));
        assert!(row_text(&terminal, 3).contains("pat-skylight"));
    }

    #[test]
    fn a_one_run_history_in_the_smallest_frame_draws_that_run() {
        let app = history_app(
            r#"{"run":"r1","machine":"spare","initiative":"history-frame","ended_at":"2026-10-04T12:40:00Z","outcome":"approved","cost_usd":0.5}"#,
            TODAY,
        );
        assert_eq!(history_height(1, 3 * HISTORY_MIN_ROWS), HISTORY_MIN_ROWS);
        let terminal = draw_history_at(&app, ThemeId::Regatta, 120, HISTORY_MIN_ROWS, None);
        assert!(row_text(&terminal, 1).contains("today"));
        assert!(row_text(&terminal, 2).contains("TIME"));
        assert!(row_text(&terminal, 3).contains("12:40 spare"));
    }

    #[test]
    fn selecting_with_no_visible_rows_draws_no_highlight_and_does_not_panic() {
        let theme = project_theme();
        let app = history_app(MIXED, TODAY);
        for height in 2..=4 {
            let terminal = draw_history_at(&app, ThemeId::Regatta, 120, height, Some(3));
            let buffer = terminal.backend().buffer();
            assert!(
                buffer.content().iter().all(|c| c.bg != theme.selected_row),
                "height {height}"
            );
        }
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
                let y = rect.y + 3 + i as u16;
                let x = col_of(&terminal, y, word);
                assert_eq!(buffer[(x, y)].fg, color, "{theme_id:?} {word}");
                assert_eq!(buffer[(x + 2, y)].fg, color, "{theme_id:?} {word}");
            }
        }
    }

    #[test]
    fn history_rows_read_time_machine_project_initiative_outcome_and_cost() {
        let app = history_app(MIXED, TODAY);
        let terminal = draw_page(&app, ThemeId::Regatta, 120, 40);
        let rect = history_rect(Rect::new(0, 0, 120, 40), &app).expect("history rect");
        let row = |i: u16| row_text(&terminal, rect.y + 3 + i);
        let lead = format!(
            "13:12 {} {} dash-feed",
            fit_cell("omarchy", RUN_COLUMNS[1]),
            fit_cell("-", PROJECT_WIDTH)
        );
        assert!(row(0).contains(&lead), "{}", row(0));
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
        let omarchy = col_of(&terminal, rect.y + 3, "omarchy");
        let spare = col_of(&terminal, rect.y + 5, "spare");
        assert_eq!(buffer[(omarchy, rect.y + 3)].fg, theme.machine_accents[0]);
        assert_eq!(buffer[(spare, rect.y + 5)].fg, theme.machine_accents[1]);
        assert_ne!(theme.machine_accents[0], theme.machine_accents[1]);
    }

    #[test]
    fn the_end_time_is_converted_with_the_offset_the_app_carries() {
        let et = FixedOffset::west_opt(4 * 3600).unwrap();
        let app = history_app(MIXED, TODAY).with_utc_offset(et);
        let terminal = draw_page(&app, ThemeId::Regatta, 120, 40);
        let rect = history_rect(Rect::new(0, 0, 120, 40), &app).expect("history rect");
        assert!(row_text(&terminal, rect.y + 3).contains("09:12 omarchy"));
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
            assert!(row_text(&terminal, rect.y + 4).contains("\u{25b6} 12:40"));
            assert_eq!(buffer[(rect.x + 3, rect.y + 4)].bg, theme.selected_row);
            assert_eq!(buffer[(rect.x + 3, rect.y + 3)].bg, theme.bg);
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
    fn history_height_is_rows_plus_four_capped_at_a_third_of_the_body() {
        assert_eq!(history_height(0, 39), 4);
        assert_eq!(history_height(1, 39), 5);
        assert_eq!(history_height(6, 39), 10);
        assert_eq!(history_height(50, 39), 13);
        assert_eq!(history_height(6, 11), 0);
        assert_eq!(history_height(1, 15), 5);
        assert_eq!(history_height(1, 14), 0);
    }

    #[test]
    fn the_history_rect_sits_above_the_key_bar_in_every_preset_and_hides_with_its_flag() {
        let area = Rect::new(0, 0, 120, 40);
        let mut app = history_app(MIXED, TODAY);
        for preset in 0..4 {
            assert_eq!(app.regatta_layout_preset(), preset);
            let history = history_rect(area, &app).expect("history is visible");
            assert_eq!(history, Rect::new(0, 29, 120, 10));
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
        assert_eq!(outcome_color(&theme, Outcome::Idle), theme.dim);
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
