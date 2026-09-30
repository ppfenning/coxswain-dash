//! Rendering for the Slipstream page: one bordered card per run, showing its position in the
//! six-step pipeline (plan, build, handoff, review, arbitrate, land), plus a fixed-width right
//! rail summarizing spend, machines, inbox and chair. The card border and the current-step
//! highlight come from `themes/slipstream.toml`, a small palette this page owns on its own; the
//! rest of the colors (bg, fg, dim, status) still come from the active [`Theme`], so `t` still
//! switches the page between dark and light.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
};

use crate::app::App;
use crate::feed::{Chair, FeedSnapshot, InboxEntry, Machine, Run, Spend};
use crate::theme::Theme;

/// Width of the right rail, in columns.
const RAIL_WIDTH: u16 = 32;

/// Names of the six pipeline steps, in order.
const STEP_NAMES: [&str; 6] = ["plan", "build", "handoff", "review", "arbitrate", "land"];

/// The two colors `themes/slipstream.toml` carries. Read fresh on each render; no global cache.
#[derive(serde::Deserialize)]
struct Accent {
    accent: String,
    card_border: String,
}

/// Parses a `#rrggbb` string into a `Color::Rgb`. Invalid input is our own asset misparsing
/// itself, so a panic at this edge is the loud failure we want, not a silently wrong color.
fn parse_hex(hex: &str) -> Color {
    let hex = hex.trim_start_matches('#');
    let r = u8::from_str_radix(&hex[0..2], 16).expect("slipstream.toml color should be valid hex");
    let g = u8::from_str_radix(&hex[2..4], 16).expect("slipstream.toml color should be valid hex");
    let b = u8::from_str_radix(&hex[4..6], 16).expect("slipstream.toml color should be valid hex");
    Color::Rgb(r, g, b)
}

/// Loads `themes/slipstream.toml` and returns `(accent, card_border)`. A plain function, called
/// on every render: the asset is small and this page is its only reader.
fn slipstream_colors() -> (Color, Color) {
    let parsed: Accent = toml::from_str(include_str!("../../themes/slipstream.toml"))
        .expect("themes/slipstream.toml should parse");
    (parse_hex(&parsed.accent), parse_hex(&parsed.card_border))
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

pub fn render(f: &mut Frame, app: &App, theme: &Theme) {
    let area = f.area();
    let Some(snapshot) = app.snapshot() else {
        render_waiting(f, area, theme);
        return;
    };
    let (accent, card_border) = slipstream_colors();
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1), Constraint::Length(RAIL_WIDTH)])
        .split(area);
    render_cards(
        f,
        cols[0],
        snapshot,
        app.selected(),
        accent,
        card_border,
        theme,
    );
    render_rail(f, cols[1], snapshot, theme);
}

fn base_style(theme: &Theme) -> Style {
    Style::default().fg(theme.fg).bg(theme.bg)
}

fn render_waiting(f: &mut Frame, area: Rect, theme: &Theme) {
    let block = Block::default().title("Slipstream").borders(Borders::ALL);
    let paragraph = Paragraph::new("waiting for cox dash --feed")
        .block(block)
        .style(base_style(theme));
    f.render_widget(paragraph, area);
}

fn render_cards(
    f: &mut Frame,
    area: Rect,
    snapshot: &FeedSnapshot,
    selected: usize,
    accent: Color,
    card_border: Color,
    theme: &Theme,
) {
    let runs = &snapshot.runs;
    if runs.is_empty() {
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Length(3); runs.len()])
        .split(area);
    for (i, (rect, run)) in rows.iter().zip(runs.iter()).enumerate() {
        render_card(f, *rect, run, i == selected, accent, card_border, theme);
    }
}

/// The color for step `i`'s cell, given the run's current step (if its node maps to one).
/// Steps before the current one are done; the current one is the slipstream accent, or the
/// theme's failed color when the run is quarantined or failed; later steps (and every step,
/// when the node maps to none of them) are dim.
fn step_color(i: usize, current: Option<usize>, run: &Run, theme: &Theme, accent: Color) -> Color {
    match current {
        Some(cur) if i < cur => theme.status_done,
        Some(cur) if i == cur => {
            if run.status == "quarantined" || run.status == "failed" {
                theme.status_failed
            } else {
                accent
            }
        }
        _ => theme.dim,
    }
}

fn render_card(
    f: &mut Frame,
    rect: Rect,
    run: &Run,
    selected: bool,
    accent: Color,
    card_border: Color,
    theme: &Theme,
) {
    let title = if selected {
        format!("\u{25b6} {} {}", run.run, run.machine)
    } else {
        format!("{} {}", run.run, run.machine)
    };
    let border_type = if selected {
        BorderType::Double
    } else {
        BorderType::Plain
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(border_type)
        .border_style(Style::default().fg(card_border))
        .style(base_style(theme));
    let current = step_index(&run.node);
    let spans: Vec<Span> = STEP_NAMES
        .iter()
        .enumerate()
        .map(|(i, name)| {
            Span::styled(
                format!(" {name} "),
                Style::default().fg(step_color(i, current, run, theme, accent)),
            )
        })
        .collect();
    let paragraph = Paragraph::new(Line::from(spans))
        .block(block)
        .style(base_style(theme));
    f.render_widget(paragraph, rect);
}

fn rail_block(title: &'static str, theme: &Theme) -> Block<'static> {
    Block::default()
        .title(Span::styled(title, Style::default().fg(theme.accent)))
        .borders(Borders::ALL)
        .style(base_style(theme))
}

fn render_rail(f: &mut Frame, area: Rect, snapshot: &FeedSnapshot, theme: &Theme) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Ratio(1, 4),
            Constraint::Ratio(1, 4),
            Constraint::Ratio(1, 4),
            Constraint::Ratio(1, 4),
        ])
        .split(area);
    render_rail_spend(f, rows[0], &snapshot.spend, theme);
    render_rail_machines(f, rows[1], &snapshot.machines, theme);
    render_rail_inbox(f, rows[2], &snapshot.inbox, theme);
    render_rail_chair(f, rows[3], &snapshot.chair, theme);
}

fn render_rail_spend(f: &mut Frame, rect: Rect, spend: &Spend, theme: &Theme) {
    let five_hour_pct = spend.five_hour_fraction * 100.0;
    let weekly_pct = spend.weekly_fraction * 100.0;
    let lines = vec![
        Line::from(format!("5h {five_hour_pct:.0}%")),
        Line::from(format!("wk {weekly_pct:.0}%")),
    ];
    let paragraph = Paragraph::new(lines)
        .block(rail_block("spend", theme))
        .style(base_style(theme));
    f.render_widget(paragraph, rect);
}

fn render_rail_machines(f: &mut Frame, rect: Rect, machines: &[Machine], theme: &Theme) {
    let lines: Vec<Line> = machines
        .iter()
        .map(|m| {
            let name = &m.name;
            let lanes_in_use = m.lanes_in_use;
            let capacity = m.capacity;
            Line::from(format!("{name} {lanes_in_use}/{capacity}"))
        })
        .collect();
    let paragraph = Paragraph::new(lines)
        .block(rail_block("machines", theme))
        .style(base_style(theme));
    f.render_widget(paragraph, rect);
}

fn render_rail_inbox(f: &mut Frame, rect: Rect, inbox: &[InboxEntry], theme: &Theme) {
    let lines: Vec<Line> = inbox
        .iter()
        .map(|i| {
            let kind = &i.kind;
            let target = &i.target;
            Line::from(format!("{kind} {target}"))
        })
        .collect();
    let paragraph = Paragraph::new(lines)
        .block(rail_block("inbox", theme))
        .style(base_style(theme));
    f.render_widget(paragraph, rect);
}

fn render_rail_chair(f: &mut Frame, rect: Rect, chair: &Chair, theme: &Theme) {
    let holder = &chair.holder;
    let liveness = &chair.liveness;
    let lines = vec![Line::from(format!("{holder} {liveness}"))];
    let paragraph = Paragraph::new(lines)
        .block(rail_block("chair", theme))
        .style(base_style(theme));
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
        let mut app = App::new(AppPage::Slipstream, ThemeId::Regatta);
        app.apply_snapshot(snapshot);
        app
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
    fn renders_slipstream_snapshot_in_the_regatta_theme() {
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
    fn renders_slipstream_snapshot_in_the_harbor_light_theme() {
        let app = app_with_fixture();
        let theme = crate::theme::resolve(ThemeId::HarborLight);
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render(f, &app, &theme))
            .expect("draw should not fail");
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn the_selected_cards_title_shows_the_selection_marker() {
        let app = app_with_fixture();
        assert_eq!(app.selected(), 0);
        let theme = crate::theme::resolve(ThemeId::Regatta);
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render(f, &app, &theme))
            .expect("draw should not fail");
        assert!(terminal.backend().to_string().contains('\u{25b6}'));
    }

    #[test]
    fn current_step_cell_uses_the_slipstream_accent_color() {
        let app = app_with_fixture();
        let theme = crate::theme::resolve(ThemeId::Regatta);
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render(f, &app, &theme))
            .expect("draw should not fail");
        let (accent, _) = slipstream_colors();
        // The fixture's one run is on `build` (step 1) and is `running`, not
        // quarantined/failed, so its step cell renders in the slipstream accent, not a
        // theme status color. The card is the first row of the main column: its inner text
        // starts at (1, 1); each earlier step span is its name padded by one space each
        // side, so step 1's first letter sits one cell after `"plan"`'s padded span.
        let step_1_start: u16 = STEP_NAMES[..1]
            .iter()
            .map(|name| name.len() as u16 + 2)
            .sum();
        let x = 1 + step_1_start + 1;
        let fg = terminal.backend().buffer()[(x, 1)].fg;
        assert_eq!(fg, accent);
    }
}
