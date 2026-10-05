//! Rendering for the Slipstream page: a header row, one rounded card per run showing its
//! position in the six-step pipeline (plan, build, handoff, review, arbitrate, land), a 40-column
//! right rail (spend, machines, inbox, chair) and a key bar. Every color comes from
//! `SlipstreamPalette`, dark or Harbor Light to follow the `t` theme toggle.

use chrono::{DateTime, FixedOffset};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

use super::chair_card::{Freshness, TICK_INTERVAL_S, beat_freshness, short_age};
use super::regatta::action_hints;
use super::{CollapsedRun, collapse_runs, count_suffix, end_chip};
use crate::app::{App, Focus};
use crate::feed::{Chair, FeedSnapshot, InboxEntry, Machine, Run, Spend};
use crate::theme::{SlipstreamPalette, Theme, resolve, slipstream_palette, slipstream_palette_for};

/// Width of the right rail, in columns.
pub const RAIL_WIDTH: u16 = 40;

/// Rows of the chair box: top border, two lines, the feed-error row, bottom border.
const CHAIR_HEIGHT: u16 = 5;

/// Rows of one run card: top border, two lines, bottom border.
const CARD_HEIGHT: u16 = 4;

/// Rows of one machine box: top border, name line, lane line, bottom border.
const MACHINE_HEIGHT: u16 = 4;

/// Names of the six pipeline steps, in order.
const STEP_NAMES: [&str; 6] = ["plan", "build", "handoff", "review", "arbitrate", "land"];

/// The keys `input::handle_key` and `main` bind that act on this page.
const KEYS: &str = "\u{2191}\u{2193} select   enter open   esc close   tab page   t theme   q quit";

/// The key row: `KEYS`, then the focused list's action keys, then `:` for the palette.
fn keys_text(focus: Focus) -> String {
    let hints = action_hints(focus)
        .iter()
        .map(|(k, l)| format!("{k} {l}"))
        .collect::<Vec<_>>()
        .join("  ");
    [KEYS, hints.as_str(), ": palette"]
        .iter()
        .filter(|part| !part.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("   ")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChipState {
    Done,
    Current,
    Failed,
    Pending,
}

/// The palette for a `COLORTERM` value: truecolor as defined, else the nearest xterm-256 indexes.
fn for_colorterm(theme: crate::app::ThemeId, colorterm: Option<&str>) -> SlipstreamPalette {
    match colorterm {
        Some("truecolor") | Some("24bit") => slipstream_palette_for(theme),
        _ => slipstream_palette_for(theme).to_256(),
    }
}

/// The edge: the palette this terminal can draw. Also used by tests elsewhere in `ui`.
pub fn resolved_palette(theme: crate::app::ThemeId) -> SlipstreamPalette {
    for_colorterm(theme, std::env::var("COLORTERM").ok().as_deref())
}

/// Maps a run's `node` to its index (0-5) among the six pipeline steps. `None` for a node that
/// belongs to none of them.
fn step_index(node: &str) -> Option<usize> {
    match node {
        "plan" | "scope_epic" => Some(0),
        "build" | "validate_chunk" => Some(1),
        "handoff" => Some(2),
        "review_charter" | "review_adversary" => Some(3),
        "arbitrate" => Some(4),
        "validate_phase" | "land" => Some(5),
        _ => None,
    }
}

/// State of step `i` given the run's current step. A landed run has every step done; a failed or
/// quarantined run shows its current step failed. The feed carries no per-step progress, so the
/// node is the only source.
fn chip_state(i: usize, current: Option<usize>, run: &Run) -> ChipState {
    match (run.status.as_str(), current) {
        ("landed", _) => ChipState::Done,
        (_, Some(cur)) if i < cur => ChipState::Done,
        ("quarantined" | "failed", Some(cur)) if i == cur => ChipState::Failed,
        (_, Some(cur)) if i == cur => ChipState::Current,
        _ => ChipState::Pending,
    }
}

fn chip_style(state: ChipState, p: &SlipstreamPalette) -> Style {
    let (fg, bg) = match state {
        ChipState::Done => (p.done_chip_text, p.done_chip),
        ChipState::Current => (p.ground, p.accent),
        ChipState::Failed => (p.ground, p.failed_chip),
        ChipState::Pending => (p.pending_chip_text, p.pending_chip),
    };
    Style::default().fg(fg).bg(bg)
}

/// `text` cut to at most `room` chars, with an ellipsis when it was cut.
fn clip(text: &str, room: usize) -> String {
    if text.chars().count() > room {
        text.chars()
            .take(room.saturating_sub(1))
            .chain(std::iter::once('\u{2026}'))
            .take(room)
            .collect()
    } else {
        text.to_string()
    }
}

/// The card's first line: the name, then `mid` (the end chip and count), then the cost flush
/// right. The name is cut first when the line is short.
fn title_spans(name: &str, mid: Vec<Span<'static>>, cost: f64, width: usize) -> Vec<Span<'static>> {
    let cost = format!("${cost:.2}");
    let mid_len: usize = mid.iter().map(|s| s.content.chars().count()).sum();
    let name = clip(
        name,
        width.saturating_sub(mid_len + cost.chars().count() + 1),
    );
    let pad = width.saturating_sub(name.chars().count() + mid_len + cost.chars().count());
    std::iter::once(Span::raw(name))
        .chain(mid)
        .chain([Span::raw(" ".repeat(pad)), Span::raw(cost)])
        .collect()
}

/// The index of the collapsed row that holds `runs[selected]`, or `None` when `selected` is out
/// of range. Rows keep first-seen order, so the row is the one whose count grows when the
/// selected run joins the runs before it.
fn selected_row(runs: &[Run], selected: usize) -> Option<usize> {
    let after = collapse_runs(runs.get(..=selected)?);
    let before = collapse_runs(&runs[..selected]);
    after
        .iter()
        .enumerate()
        .position(|(i, row)| row.count != before.get(i).map_or(0, |b| b.count))
}

/// `Tue Sep 29 14:00 v0.1.0`: the day in `offset`, its local time, the version. A timestamp that
/// does not parse is printed as given, without a day.
fn header_right(at: &str, offset: FixedOffset, version: &str) -> String {
    let time = super::local_time(at, offset);
    match DateTime::parse_from_rfc3339(at) {
        Ok(t) => format!(
            "{} {time} v{version}",
            t.with_timezone(&offset).format("%a %b %d")
        ),
        Err(_) => format!("{time} v{version}"),
    }
}

/// `RUNS` as `R U N S`.
fn spaced(label: &str) -> String {
    label
        .chars()
        .map(String::from)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Filled and empty cell counts of a bar `width` cells wide. The fraction is clamped to 0..=1.
fn bar_parts(fraction: f64, width: usize) -> (usize, usize) {
    let filled = ((fraction.clamp(0.0, 1.0) * width as f64).round() as usize).min(width);
    (filled, width - filled)
}

/// One `(width, in_use)` segment per lane, one-cell gaps between them, the last segment taking the
/// remainder so the row spans exactly `width` cells.
fn lane_blocks(in_use: u32, capacity: u32, width: usize) -> Vec<(usize, bool)> {
    let count = capacity as usize;
    let room = width.saturating_sub(count.saturating_sub(1));
    (0..count)
        .map(|i| {
            let extra = if i + 1 == count { room % count } else { 0 };
            (room / count + extra, (i as u32) < in_use)
        })
        .collect()
}

/// `(lands, beat)` texts for the chair line.
fn lands_beat(lands: u32, beat_age_s: u64) -> (String, String) {
    (
        format!("{lands} lands today"),
        format!("beat {}", short_age(beat_age_s)),
    )
}

/// Index into the palette's two machine accents: the machine's place in the feed's list.
fn machine_accent(name: &str, machines: &[Machine], p: &SlipstreamPalette) -> Color {
    let index = machines.iter().position(|m| m.name == name).unwrap_or(0);
    p.accents[index % 2]
}

struct Regions {
    header: Rect,
    left: Rect,
    rail: Rect,
    keys: Rect,
}

fn regions(area: Rect) -> Regions {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1), Constraint::Length(RAIL_WIDTH)])
        .split(rows[1]);
    Regions {
        header: rows[0],
        left: cols[0],
        rail: cols[1],
        keys: rows[2],
    }
}

/// The bordered chair box: the bottom `CHAIR_HEIGHT` rows of the rail.
fn chair_rect(rail: Rect) -> Rect {
    let height = CHAIR_HEIGHT.min(rail.height);
    Rect {
        y: rail.y + rail.height - height,
        height,
        ..rail
    }
}

fn first_row(rect: Rect) -> Rect {
    Rect {
        height: rect.height.min(1),
        ..rect
    }
}

fn below_first_row(rect: Rect) -> Rect {
    Rect {
        y: rect.y + rect.height.min(1),
        height: rect.height.saturating_sub(1),
        ..rect
    }
}

fn base_style(p: &SlipstreamPalette) -> Style {
    Style::default().fg(p.text).bg(p.ground)
}

fn dim_style(p: &SlipstreamPalette) -> Style {
    Style::default().fg(p.dim)
}

fn card_block(border: Color, bg: Color, p: &SlipstreamPalette) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .style(Style::default().fg(p.text).bg(bg))
}

fn render_label(f: &mut Frame, rect: Rect, label: &str, p: &SlipstreamPalette) {
    f.render_widget(
        Paragraph::new(Span::styled(spaced(label), dim_style(p))),
        first_row(rect),
    );
}

pub fn render(f: &mut Frame, app: &App) {
    render_with(
        f,
        app,
        resolved_palette(app.theme()),
        env!("CARGO_PKG_VERSION"),
    );
}

fn render_with(f: &mut Frame, app: &App, p: SlipstreamPalette, version: &str) {
    let area = f.area();
    f.render_widget(Block::default().style(base_style(&p)), area);
    let Some(snapshot) = app.snapshot() else {
        render_waiting(f, area, &p);
        return;
    };
    let r = regions(area);
    render_header(f, r.header, snapshot, app.utc_offset(), version, &p);
    render_cards(
        f,
        r.left,
        snapshot,
        app.selected(),
        &p,
        &resolve(app.theme()),
    );
    render_rail(f, r.rail, snapshot, &p);
    f.render_widget(
        Paragraph::new(Span::styled(keys_text(app.focus()), dim_style(&p))),
        r.keys,
    );
    if let Some(err) = app.feed_error() {
        render_error_line(f, chair_rect(r.rail), err, &p);
    }
}

fn render_waiting(f: &mut Frame, area: Rect, p: &SlipstreamPalette) {
    let block = Block::default()
        .title("Slipstream")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(p.card_border));
    let paragraph = Paragraph::new("waiting for cox dash --feed")
        .block(block)
        .style(base_style(p));
    f.render_widget(paragraph, area);
}

fn render_header(
    f: &mut Frame,
    rect: Rect,
    snapshot: &FeedSnapshot,
    offset: FixedOffset,
    version: &str,
    p: &SlipstreamPalette,
) {
    let title = Style::default().fg(p.accent).add_modifier(Modifier::BOLD);
    f.render_widget(Paragraph::new(Span::styled("coxtop", title)), rect);
    let right = header_right(&snapshot.at, offset, version);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(right, dim_style(p))).right_aligned()),
        rect,
    );
}

fn render_cards(
    f: &mut Frame,
    area: Rect,
    snapshot: &FeedSnapshot,
    selected: usize,
    p: &SlipstreamPalette,
    theme: &Theme,
) {
    render_label(f, area, "RUNS", p);
    let collapsed = collapse_runs(&snapshot.runs);
    let selected_row = selected_row(&snapshot.runs, selected);
    let constraints: Vec<Constraint> = collapsed
        .iter()
        .map(|_| Constraint::Length(CARD_HEIGHT))
        .chain(std::iter::once(Constraint::Min(0)))
        .collect();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(below_first_row(area));
    for (i, (rect, row)) in rows.iter().zip(collapsed.iter()).enumerate() {
        let selected = selected_row == Some(i);
        render_card(f, *rect, row, &snapshot.machines, selected, p, theme);
    }
}

fn render_card(
    f: &mut Frame,
    rect: Rect,
    row: &CollapsedRun,
    machines: &[Machine],
    selected: bool,
    p: &SlipstreamPalette,
    theme: &Theme,
) {
    let run = row.run;
    let (border, card_bg) = if selected {
        (p.selected_border, p.card)
    } else {
        (p.card_border, p.ground)
    };
    let block = card_block(border, card_bg, p);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let current = step_index(&run.node);
    let head = [
        Span::styled(
            run.machine.clone(),
            Style::default().fg(machine_accent(&run.machine, machines, p)),
        ),
        Span::raw(" "),
        Span::styled(run.phase.clone(), dim_style(p)),
        Span::raw("  "),
    ];
    let chips = STEP_NAMES.iter().enumerate().flat_map(|(i, name)| {
        [
            Span::styled(
                format!(" {name} "),
                chip_style(chip_state(i, current, run), p),
            ),
            Span::raw(" "),
        ]
    });
    // The chip keeps its status colour; the card's own background replaces the theme's.
    let end = end_chip(run.end.as_ref(), &run.status, theme)
        .spans
        .into_iter()
        .map(|s| Span::styled(s.content, s.style.bg(card_bg)));
    let count = count_suffix(row.count, theme).map(|s| Span::styled(s.content, dim_style(p)));
    let mid: Vec<Span<'static>> = std::iter::once(Span::raw(" "))
        .chain(end)
        .chain(count)
        .collect();
    let lines = vec![
        Line::from(title_spans(
            &run.run,
            mid,
            run.cost,
            usize::from(inner.width),
        )),
        Line::from(head.into_iter().chain(chips).collect::<Vec<_>>()),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn render_rail(f: &mut Frame, rail: Rect, snapshot: &FeedSnapshot, p: &SlipstreamPalette) {
    let chair_box = chair_rect(rail);
    let top = Rect {
        height: chair_box.y.saturating_sub(rail.y + 1),
        ..rail
    };
    let machines_height = (snapshot.machines.len() as u16)
        .saturating_mul(MACHINE_HEIGHT)
        .saturating_add(1);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(machines_height),
            Constraint::Min(0),
        ])
        .split(top);
    render_rail_spend(f, sections[0], &snapshot.spend, p);
    render_rail_machines(f, sections[1], &snapshot.machines, p);
    render_rail_inbox(f, sections[2], &snapshot.inbox, p);
    let label_row = Rect {
        y: chair_box.y.saturating_sub(1).max(rail.y),
        ..rail
    };
    render_label(f, label_row, "CHAIR", p);
    render_rail_chair(f, chair_box, &snapshot.chair, p);
}

fn bar_line(
    label: &str,
    fraction: f64,
    width: usize,
    fill: Color,
    p: &SlipstreamPalette,
) -> Line<'static> {
    let (filled, empty) = bar_parts(fraction, width);
    Line::from(vec![
        Span::styled(format!("{label} "), dim_style(p)),
        Span::styled("\u{2588}".repeat(filled), Style::default().fg(fill)),
        Span::styled("\u{2591}".repeat(empty), dim_style(p)),
        Span::raw(format!(" {:>3.0}%", fraction * 100.0)),
    ])
}

fn render_rail_spend(f: &mut Frame, rect: Rect, spend: &Spend, p: &SlipstreamPalette) {
    render_label(f, rect, "SPEND", p);
    // "5h " before the bar and " 16%" after it.
    let width = usize::from(rect.width).saturating_sub(3 + 5);
    let lines = vec![
        bar_line("5h", spend.five_hour_fraction, width, p.accents[0], p),
        bar_line("wk", spend.weekly_fraction, width, p.accents[1], p),
    ];
    f.render_widget(Paragraph::new(lines), below_first_row(rect));
}

fn render_rail_machines(f: &mut Frame, rect: Rect, machines: &[Machine], p: &SlipstreamPalette) {
    render_label(f, rect, "MACHINES", p);
    let constraints: Vec<Constraint> = machines
        .iter()
        .map(|_| Constraint::Length(MACHINE_HEIGHT))
        .chain(std::iter::once(Constraint::Min(0)))
        .collect();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(below_first_row(rect));
    for (box_rect, machine) in rows.iter().zip(machines.iter()) {
        render_machine_box(f, *box_rect, machine, machines, p);
    }
}

fn render_machine_box(
    f: &mut Frame,
    rect: Rect,
    machine: &Machine,
    machines: &[Machine],
    p: &SlipstreamPalette,
) {
    let block = card_block(p.card_border, p.card, p);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let width = usize::from(inner.width);
    let count = format!("{}/{}", machine.lanes_in_use, machine.capacity);
    let pad = width.saturating_sub(machine.name.chars().count() + count.chars().count());
    let name_line = Line::from(vec![
        Span::styled(
            machine.name.clone(),
            Style::default().fg(machine_accent(&machine.name, machines, p)),
        ),
        Span::raw(" ".repeat(pad)),
        Span::styled(count, dim_style(p)),
    ]);
    let lane_spans: Vec<Span> = lane_blocks(machine.lanes_in_use, machine.capacity, width)
        .into_iter()
        .enumerate()
        .flat_map(|(i, (cells, used))| {
            let (glyph, color) = if used {
                ("\u{2588}", p.done_chip)
            } else {
                ("\u{2591}", p.pending_chip_text)
            };
            let gap = if i == 0 { "" } else { " " };
            [
                Span::raw(gap),
                Span::styled(glyph.repeat(cells), Style::default().fg(color)),
            ]
        })
        .collect();
    f.render_widget(
        Paragraph::new(vec![name_line, Line::from(lane_spans)]),
        inner,
    );
}

fn render_rail_inbox(f: &mut Frame, rect: Rect, inbox: &[InboxEntry], p: &SlipstreamPalette) {
    render_label(f, rect, "INBOX", p);
    let lines: Vec<Line> = inbox
        .iter()
        .flat_map(|entry| {
            [
                Line::from(vec![
                    Span::styled(entry.kind.clone(), Style::default().fg(p.accents[1])),
                    Span::raw(format!(" {}", entry.target)),
                ]),
                Line::from(Span::styled(format!("  {}", entry.reason), dim_style(p))),
            ]
        })
        .collect();
    f.render_widget(Paragraph::new(lines), below_first_row(rect));
}

fn render_rail_chair(f: &mut Frame, rect: Rect, chair: &Chair, p: &SlipstreamPalette) {
    let block = card_block(p.card_border, p.card, p);
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let dot = if chair.liveness == "live" {
        p.done_chip
    } else {
        p.failed_chip
    };
    let beat = match beat_freshness(chair.beat_age_s, TICK_INTERVAL_S) {
        Freshness::Fresh => dim_style(p),
        Freshness::Stale => Style::default().fg(p.failed_chip),
    };
    let (lands, beat_text) = lands_beat(chair.today.lands, chair.beat_age_s);
    let lines = vec![
        Line::from(vec![
            Span::styled("\u{25cf} ", Style::default().fg(dot)),
            Span::raw(format!("{} @ {}", chair.holder, chair.host)),
        ]),
        Line::from(vec![
            Span::styled(lands, dim_style(p)),
            Span::styled(" \u{b7} ", dim_style(p)),
            Span::styled(beat_text, beat),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

/// `feed error: <err>` on the chair box's last inner row, clearing what was there.
fn render_error_line(f: &mut Frame, chair_box: Rect, err: &str, p: &SlipstreamPalette) {
    let row = super::last_row(super::bordered_inner(chair_box));
    let text = super::fit_line(&super::feed_error_text(err), row.width);
    f.render_widget(Clear, row);
    f.render_widget(
        Paragraph::new(text).style(Style::default().fg(p.failed_chip).bg(p.card)),
        row,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppPage, ThemeId};
    use crate::theme::slipstream_palette;
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

    const FIXTURE: &str = include_str!("../../tests/fixtures/dash_feed_v1.json");

    fn app_from(json: &str, theme: ThemeId) -> App {
        let snapshot = crate::feed::parse_snapshot(json.trim()).expect("fixture should parse");
        let mut app = App::new(AppPage::Slipstream, theme);
        app.apply_snapshot(snapshot);
        app
    }

    fn draw(app: &App, width: u16, height: u16) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|f| render_with(f, app, slipstream_palette_for(app.theme()), "1.2.3"))
            .expect("draw should not fail");
        terminal
    }

    fn run(node: &str, status: &str, attempt: u32) -> Run {
        Run {
            run: "r".to_string(),
            machine: "m".to_string(),
            phase: "p".to_string(),
            node: node.to_string(),
            attempt,
            turns: 0,
            cost: 0.0,
            verdict: String::new(),
            status: status.to_string(),
            cost_series: Vec::new(),
            end: None,
        }
    }

    fn named(id: &str) -> Run {
        Run {
            run: id.to_string(),
            ..run("plan", "running", 1)
        }
    }

    /// The fixture with its second run renamed, so its two runs are two initiatives.
    fn two_initiatives() -> String {
        FIXTURE.replace("\"run\": \"dash-feed-2\"", "\"run\": \"other-2\"")
    }

    /// The fixture with `runs` in place of its own.
    fn with_runs(runs: Vec<serde_json::Value>) -> String {
        let mut feed: serde_json::Value = serde_json::from_str(FIXTURE).expect("fixture is json");
        feed["runs"] = serde_json::Value::Array(runs);
        feed.to_string()
    }

    /// One run for `id` that ended as `end`, a bare kind or a `{kind, cause}` object.
    fn run_json(id: &str, end: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "run": id, "machine": "omarchy", "phase": "p1", "node": "build", "attempt": 1,
            "turns": 1, "cost": 1.5, "verdict": "none", "status": "running",
            "cost_series": [], "end": end
        })
    }

    /// One run of each end kind, each its own initiative.
    fn end_kinds_json() -> String {
        use serde_json::json;
        with_runs(vec![
            run_json("alpha-1", json!("running")),
            run_json("bravo-1", json!("landed")),
            run_json("charlie-1", json!("approved")),
            run_json(
                "delta-1",
                json!({"kind": "quarantined", "cause": "budget stop"}),
            ),
            run_json("echo-1", json!("idle")),
            run_json("foxtrot-1", json!({"kind": "died", "cause": "chair gone"})),
            run_json("golf-1", json!("stopped")),
        ])
    }

    /// The cell where `needle` starts on row `y`.
    fn cell_of<'a>(buffer: &'a Buffer, y: u16, needle: &str) -> &'a ratatui::buffer::Cell {
        let row: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        let byte = row
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} not on row {y}"));
        &buffer[(row[..byte].chars().count() as u16, y)]
    }

    /// The first run card's second line: header, RUNS label, card border, line one, line two.
    const CHIP_ROW: u16 = 4;

    #[test]
    fn the_key_row_keeps_its_keys_and_adds_the_runs_actions_and_the_palette() {
        assert_eq!(
            keys_text(Focus::Runs),
            format!("{KEYS}   p pause  k kill  m move   : palette")
        );
        assert_eq!(
            keys_text(Focus::Inbox),
            format!("{KEYS}   a accept  x deny   : palette")
        );
        assert_eq!(keys_text(Focus::History), format!("{KEYS}   : palette"));
    }

    /// Slipstream lists only runs, so `app.focus()` reads Runs there. A Regatta focus left on
    /// the inbox must not leak its accept/deny hints onto this page.
    #[test]
    fn the_key_row_follows_app_focus_which_is_runs_on_this_page() {
        use crossterm::event::KeyCode;
        let snapshot = crate::feed::parse_snapshot(FIXTURE.trim()).expect("fixture should parse");
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.apply_snapshot(snapshot);
        (0..4).for_each(|_| crate::input::handle_key(&mut app, KeyCode::Right));
        assert_eq!(app.focus(), Focus::Inbox);
        crate::input::handle_key(&mut app, KeyCode::Tab);
        assert_eq!(app.page(), AppPage::Slipstream);
        let terminal = draw(&app, 160, 46);
        let buffer = terminal.backend().buffer();
        let row: String = (0..buffer.area.width)
            .map(|x| buffer[(x, 45)].symbol())
            .collect();
        assert_eq!(row.trim_end(), keys_text(Focus::Runs));
    }

    #[test]
    fn the_key_row_snapshot() {
        let terminal = draw(&app_from(FIXTURE, ThemeId::Regatta), 160, 46);
        let buffer = terminal.backend().buffer();
        let row: String = (0..buffer.area.width)
            .map(|x| buffer[(x, 45)].symbol())
            .collect();
        insta::assert_snapshot!(row.trim_end());
    }

    #[test]
    fn step_index_maps_review_adversary_to_the_review_step() {
        assert_eq!(step_index("review_adversary"), Some(3));
    }

    #[test]
    fn step_index_rejects_an_empty_node() {
        assert_eq!(step_index(""), None);
    }

    #[test]
    fn header_right_prints_day_local_time_and_version() {
        let utc = FixedOffset::east_opt(0).unwrap();
        let et = FixedOffset::west_opt(4 * 3600).unwrap();
        assert_eq!(
            header_right("2026-09-29T00:00:00Z", utc, "0.1.0"),
            "Tue Sep 29 00:00 v0.1.0"
        );
        assert_eq!(
            header_right("2026-09-29T00:00:00Z", et, "0.1.0"),
            "Mon Sep 28 20:00 v0.1.0"
        );
        assert_eq!(header_right("garbage", utc, "0.1.0"), "garbage v0.1.0");
    }

    #[test]
    fn title_spans_puts_the_chip_after_the_name_and_the_cost_flush_right() {
        let text = |name, mid: &str, cost, width| -> String {
            title_spans(name, vec![Span::raw(mid.to_string())], cost, width)
                .iter()
                .map(|s| s.content.as_ref())
                .collect()
        };
        assert_eq!(
            text("dash-feed-1", " \u{25cf} landed", 0.84, 32),
            "dash-feed-1 \u{25cf} landed       $0.84"
        );
        assert_eq!(
            text("a-very-long-run-name", " \u{25cf} died", 12.5, 24),
            "a-very-lo\u{2026} \u{25cf} died $12.50"
        );
    }

    #[test]
    fn chip_state_marks_steps_before_at_and_after_the_current_one() {
        let running = run("build", "running", 1);
        let current = step_index(&running.node);
        assert_eq!(chip_state(0, current, &running), ChipState::Done);
        assert_eq!(chip_state(1, current, &running), ChipState::Current);
        assert_eq!(chip_state(2, current, &running), ChipState::Pending);
        let failed = run("build", "failed", 1);
        assert_eq!(chip_state(1, current, &failed), ChipState::Failed);
        let landed = run("land", "landed", 1);
        assert_eq!(chip_state(5, step_index("land"), &landed), ChipState::Done);
        assert_eq!(chip_state(0, None, &running), ChipState::Pending);
    }

    #[test]
    fn selected_row_finds_the_row_that_holds_a_hidden_relaunch() {
        let runs = [named("a-1"), named("b-1"), named("a-2"), named("c-1")];
        let rows = |i| selected_row(&runs, i);
        assert_eq!(
            [rows(0), rows(1), rows(2), rows(3), rows(4)],
            [Some(0), Some(1), Some(0), Some(2), None]
        );
    }

    #[test]
    fn bar_parts_clamps_and_rounds() {
        assert_eq!(bar_parts(0.16, 30), (5, 25));
        assert_eq!(bar_parts(1.5, 10), (10, 0));
        assert_eq!(bar_parts(-1.0, 10), (0, 10));
    }

    #[test]
    fn lane_blocks_span_the_width_with_one_cell_gaps() {
        assert_eq!(
            lane_blocks(2, 3, 38),
            vec![(12, true), (12, true), (12, false)]
        );
        assert_eq!(lane_blocks(3, 3, 10), vec![(2, true), (2, true), (4, true)]);
        assert_eq!(lane_blocks(0, 0, 38), vec![]);
    }

    #[test]
    fn lands_beat_reads_the_chair_fields() {
        assert_eq!(
            lands_beat(2, 4),
            ("2 lands today".to_string(), "beat 4s".to_string())
        );
    }

    #[test]
    fn for_colorterm_keeps_truecolor_and_downgrades_otherwise() {
        use crate::app::ThemeId;
        assert_eq!(
            for_colorterm(ThemeId::Regatta, Some("truecolor")),
            slipstream_palette()
        );
        assert_eq!(
            for_colorterm(ThemeId::Regatta, None),
            slipstream_palette().to_256()
        );
        assert_eq!(
            for_colorterm(ThemeId::HarborLight, Some("truecolor")),
            slipstream_palette_for(ThemeId::HarborLight)
        );
        assert_ne!(
            slipstream_palette_for(ThemeId::HarborLight),
            slipstream_palette()
        );
    }

    #[test]
    fn a_done_chip_is_drawn_in_the_done_chip_colors() {
        let terminal = draw(&app_from(&two_initiatives(), ThemeId::Regatta), 120, 40);
        let cell = cell_of(terminal.backend().buffer(), CHIP_ROW, "plan");
        let p = slipstream_palette();
        assert_eq!((cell.fg, cell.bg), (p.done_chip_text, p.done_chip));
    }

    #[test]
    fn the_current_chip_is_drawn_in_the_accent() {
        let terminal = draw(&app_from(&two_initiatives(), ThemeId::Regatta), 120, 40);
        let cell = cell_of(terminal.backend().buffer(), CHIP_ROW, "build");
        assert_eq!(cell.bg, slipstream_palette().accent);
    }

    #[test]
    fn a_pending_chip_is_drawn_in_the_pending_chip_colors() {
        let terminal = draw(&app_from(&two_initiatives(), ThemeId::Regatta), 120, 40);
        let cell = cell_of(terminal.backend().buffer(), CHIP_ROW, "handoff");
        let p = slipstream_palette();
        assert_eq!((cell.fg, cell.bg), (p.pending_chip_text, p.pending_chip));
    }

    #[test]
    fn the_harbor_light_theme_draws_the_light_slipstream_palette() {
        let terminal = draw(&app_from(FIXTURE, ThemeId::HarborLight), 120, 40);
        let cell = cell_of(terminal.backend().buffer(), CHIP_ROW, "handoff");
        let p = slipstream_palette_for(ThemeId::HarborLight);
        assert_eq!((cell.fg, cell.bg), (p.pending_chip_text, p.pending_chip));
        assert_ne!(p.ground, slipstream_palette().ground);
    }

    #[test]
    fn a_failed_chip_is_drawn_in_the_failed_chip_color() {
        let json = two_initiatives().replace("\"status\": \"running\"", "\"status\": \"failed\"");
        let terminal = draw(&app_from(&json, ThemeId::Regatta), 120, 40);
        let cell = cell_of(terminal.backend().buffer(), CHIP_ROW, "build");
        assert_eq!(cell.bg, slipstream_palette().failed_chip);
    }

    #[test]
    fn the_selected_cards_border_is_drawn_in_the_selected_border_color() {
        let app = app_from(FIXTURE, ThemeId::Regatta);
        assert_eq!(app.selected(), 0);
        let terminal = draw(&app, 120, 40);
        let corner = &terminal.backend().buffer()[(0, 2)];
        let p = slipstream_palette();
        assert_eq!(corner.symbol(), "\u{256d}");
        assert_eq!((corner.fg, corner.bg), (p.selected_border, p.card));
    }

    /// The screen as plain rows with trailing blanks trimmed.
    fn snapshot_text(theme: ThemeId, width: u16, height: u16) -> String {
        let terminal = draw(&app_from(FIXTURE, theme), width, height);
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                let row: String = (0..width).map(|x| buffer[(x, y)].symbol()).collect();
                row.trim_end().to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The card column only, as plain rows: seven cards, one per end kind.
    fn end_kinds_text(theme: ThemeId) -> String {
        let terminal = draw(&app_from(&end_kinds_json(), theme), 120, 34);
        let buffer = terminal.backend().buffer();
        (0..34)
            .map(|y| {
                let row: String = (0..80).map(|x| buffer[(x, y)].symbol()).collect();
                row.trim_end().to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn card_end_kinds_in_the_regatta_theme() {
        insta::assert_snapshot!(end_kinds_text(ThemeId::Regatta));
    }

    #[test]
    fn card_end_kinds_in_the_harbor_light_theme() {
        insta::assert_snapshot!(end_kinds_text(ThemeId::HarborLight));
    }

    #[test]
    fn a_died_end_chip_is_drawn_in_the_status_failed_color_on_the_card() {
        let terminal = draw(&app_from(&end_kinds_json(), ThemeId::Regatta), 120, 34);
        let cell = cell_of(terminal.backend().buffer(), 2 + 5 * 4 + 1, "died");
        assert_eq!(cell.fg, resolve(ThemeId::Regatta).status_failed);
        assert_eq!(cell.bg, slipstream_palette().ground);
    }

    #[test]
    fn relaunches_of_one_initiative_draw_one_card_with_a_dim_count() {
        let json = with_runs(
            ["foo-1", "foo-2", "foo-3"]
                .map(|id| run_json(id, serde_json::json!("landed")))
                .to_vec(),
        );
        let terminal = draw(&app_from(&json, ThemeId::Regatta), 120, 20);
        let buffer = terminal.backend().buffer();
        let name = cell_of(buffer, 3, "foo-3");
        let count = cell_of(buffer, 3, "\u{d7}3");
        assert_eq!(name.symbol(), "f");
        assert_eq!(count.fg, slipstream_palette().dim);
        let rows: String = (0..20)
            .flat_map(|y| (0..80).map(move |x| (x, y)))
            .map(|(x, y)| buffer[(x, y)].symbol())
            .collect();
        assert_eq!(rows.matches("foo-").count(), 1);
    }

    #[test]
    fn renders_slipstream_snapshot_in_the_regatta_theme() {
        insta::assert_snapshot!(snapshot_text(ThemeId::Regatta, 120, 40));
    }

    #[test]
    fn renders_slipstream_snapshot_in_the_harbor_light_theme() {
        insta::assert_snapshot!(snapshot_text(ThemeId::HarborLight, 120, 40));
    }

    #[test]
    fn renders_slipstream_snapshot_at_160x46_in_the_regatta_theme() {
        insta::assert_snapshot!(snapshot_text(ThemeId::Regatta, 160, 46));
    }

    #[test]
    fn renders_slipstream_snapshot_at_160x46_in_the_harbor_light_theme() {
        insta::assert_snapshot!(snapshot_text(ThemeId::HarborLight, 160, 46));
    }
}
