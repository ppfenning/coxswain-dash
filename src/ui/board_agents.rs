//! The board's agents panel: a btop-style process list of the live agents, fastest first.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};

use super::chrome::bar_parts;
use crate::app::{App, Focus};
use crate::feed::{LiveAgent, agents_by_rate};
use crate::theme::Theme;

const KIND_COLS: usize = 6;
const MODEL_COLS: usize = 18;
const HOST_COLS: usize = 10;
const COST_COLS: usize = 8;
const ELAPSED_COLS: usize = 7;
const RATE_COLS: usize = 8;
const BAR_COLS: usize = 8;
/// One space between each of the eight columns.
const GAPS: usize = 7;
const SERVES_MIN_COLS: usize = 8;

/// A run of row text and its foreground.
type Seg = (String, Color);

/// Whole seconds as `45s`, `2m` or `1h05m`.
fn elapsed_text(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs}s"),
        60..=3599 => format!("{}m", secs / 60),
        _ => format!("{}h{:02}m", secs / 3600, secs % 3600 / 60),
    }
}

/// Tokens per minute as `850`, `4.2k` or `1.5M`.
fn rate_text(tokens_per_min: u64) -> String {
    match tokens_per_min {
        0..=999 => tokens_per_min.to_string(),
        1_000..=999_999 => format!("{:.1}k", tokens_per_min as f64 / 1e3),
        _ => format!("{:.1}M", tokens_per_min as f64 / 1e6),
    }
}

/// `text` cut to `width` chars and padded with spaces on the right.
fn left(text: &str, width: usize) -> String {
    format!("{:<width$}", text.chars().take(width).collect::<String>())
}

/// `text` cut to `width` chars and padded with spaces on the left.
fn right(text: &str, width: usize) -> String {
    format!("{:>width$}", text.chars().take(width).collect::<String>())
}

fn fill_color(theme: &Theme, fraction: f64) -> Color {
    if fraction >= 0.85 {
        theme.meter_high
    } else if fraction >= 0.6 {
        theme.meter_mid
    } else {
        theme.meter_low
    }
}

/// The first row of the window that keeps `selected` in view, with it on the last line once the
/// list is longer than `height`. The panel keeps no scroll state.
fn scroll_offset(selected: usize, height: usize) -> usize {
    (selected + 1).saturating_sub(height)
}

fn gap() -> Seg {
    (" ".to_string(), Color::Reset)
}

/// The column titles, laid out as `agent_segs` lays out a row.
fn header_segs(serves_cols: usize, theme: &Theme) -> Vec<Seg> {
    [
        left("kind", KIND_COLS),
        left("model", MODEL_COLS),
        left("host", HOST_COLS),
        left("serves", serves_cols),
        right("cost", COST_COLS),
        right("elapsed", ELAPSED_COLS),
        right("tok/min", RATE_COLS),
        left("context", BAR_COLS),
    ]
    .into_iter()
    .map(|text| (text, theme.dim))
    .flat_map(|seg| [seg, gap()])
    .take(2 * 8 - 1)
    .collect()
}

fn agent_segs(agent: &LiveAgent, serves_cols: usize, theme: &Theme) -> Vec<Seg> {
    let fill = agent.context_fill;
    let (filled, empty) = bar_parts(fill, BAR_COLS);
    [
        (left(&agent.kind, KIND_COLS), theme.accent),
        (left(&agent.model, MODEL_COLS), theme.fg),
        (left(&agent.host, HOST_COLS), theme.dim),
        (left(&agent.serves, serves_cols), theme.fg),
        (
            right(&format!("${:.2}", agent.cost_usd), COST_COLS),
            theme.fg,
        ),
        (
            right(&elapsed_text(agent.elapsed_s), ELAPSED_COLS),
            theme.dim,
        ),
        (right(&rate_text(agent.tokens_per_min), RATE_COLS), theme.fg),
        ("█".repeat(filled), fill_color(theme, fill)),
        ("░".repeat(empty), theme.track),
    ]
    .into_iter()
    .enumerate()
    .flat_map(|(i, seg)| if i < 7 { vec![seg, gap()] } else { vec![seg] })
    .collect()
}

fn line(segs: Vec<Seg>, selected: bool, theme: &Theme) -> Line<'static> {
    let bg = if selected {
        Style::default().bg(theme.selected_row)
    } else {
        Style::default()
    };
    segs.into_iter()
        .map(|(text, color)| Span::styled(text, bg.fg(color)))
        .collect()
}

pub fn render(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let focused = app.focus() == Focus::BoardAgents;
    let border = if focused {
        theme.border_focus
    } else {
        theme.border
    };
    let block = Block::bordered()
        .title("agents")
        .border_style(Style::default().fg(border));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let agents = app
        .snapshot()
        .map(|s| agents_by_rate(&s.live.agents))
        .unwrap_or_default();
    if agents.is_empty() {
        let empty = Line::styled("no live agents", Style::default().fg(theme.dim));
        f.render_widget(Paragraph::new(empty), inner);
        return;
    }
    let fixed = KIND_COLS + MODEL_COLS + HOST_COLS + COST_COLS + ELAPSED_COLS + RATE_COLS;
    let serves_cols = usize::from(inner.width)
        .saturating_sub(fixed + BAR_COLS + GAPS)
        .max(SERVES_MIN_COLS);
    let selected = app.selected_in(Focus::BoardAgents).min(agents.len() - 1);
    let height = usize::from(inner.height.saturating_sub(1));
    let rows = agents
        .iter()
        .enumerate()
        .skip(scroll_offset(selected, height))
        .take(height)
        .map(|(i, agent)| {
            line(
                agent_segs(agent, serves_cols, theme),
                focused && i == selected,
                theme,
            )
        });
    let lines: Vec<Line> = std::iter::once(line(header_segs(serves_cols, theme), false, theme))
        .chain(rows)
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppPage, ThemeId};
    use crate::feed::parse_snapshot;
    use crate::theme;
    use ratatui::{Terminal, backend::TestBackend};

    const BOARD_FIXTURE: &str = include_str!("../../tests/fixtures/dash_feed_board_v1.json");

    fn regatta() -> Theme {
        theme::resolve_for(ThemeId::Regatta, Some("truecolor"))
    }

    fn agent(id: &str, serves: &str, tokens_per_min: u64, context_fill: f64) -> LiveAgent {
        LiveAgent {
            id: id.into(),
            kind: "build".into(),
            model: "claude-sonnet-5-5".into(),
            host: "host-b".into(),
            serves: serves.into(),
            tokens_per_min,
            cost_usd: tokens_per_min.min(10_000) as f64 / 4000.0,
            elapsed_s: tokens_per_min.min(10_000) / 10 * 6,
            context_fill,
        }
    }

    fn app_with(agents: Vec<LiveAgent>, agents_focused: bool) -> App {
        let mut snap = parse_snapshot(BOARD_FIXTURE).expect("board fixture parses");
        snap.live.agents = agents;
        let mut app = App::new(AppPage::Board, ThemeId::Regatta);
        app.apply_snapshot(snap);
        if agents_focused {
            app.cycle_focus_next();
        }
        app
    }

    fn draw(app: &App, w: u16, h: u16) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("terminal");
        terminal
            .draw(|f| render(f, f.area(), app, &regatta()))
            .expect("draw should not fail");
        terminal
    }

    #[test]
    fn agents_sorted_by_token_rate() {
        let app = app_with(
            vec![
                agent("slow", "board-view-keys", 850, 0.2),
                agent("fast", "board-view-load", 1_500_000, 0.9),
                agent("mid", "stray-fix", 4200, 0.5),
            ],
            true,
        );
        insta::assert_snapshot!(draw(&app, 100, 7).backend().to_string());
    }

    #[test]
    fn the_selected_row_is_highlighted_only_when_the_agents_list_has_focus() {
        let agents = || vec![agent("a", "x", 2000, 0.1), agent("b", "y", 1000, 0.1)];
        // Row 0 is the border and row 1 the column titles, so the agents start on row 2.
        let bg_at = |app: &App, y: u16| draw(app, 100, 6).backend().buffer()[(2, y)].bg;
        let focused = app_with(agents(), true);
        assert_eq!(bg_at(&focused, 2), regatta().selected_row);
        assert_ne!(bg_at(&focused, 3), regatta().selected_row);
        assert_ne!(bg_at(&app_with(agents(), false), 2), regatta().selected_row);
    }

    #[test]
    fn the_list_scrolls_to_keep_the_selected_row_in_view() {
        let agents = (0..6)
            .map(|i| {
                agent(
                    &format!("a{i}"),
                    &format!("serves-{i}"),
                    6000 - i * 1000,
                    0.1,
                )
            })
            .collect();
        let mut app = app_with(agents, true);
        app.select_at(5);
        let text = draw(&app, 100, 5).backend().to_string();
        assert!(text.contains("serves-5"), "{text}");
        assert!(!text.contains("serves-0"), "{text}");
    }

    #[test]
    fn scroll_offset_puts_the_selection_on_the_last_line_once_it_overflows() {
        assert_eq!(scroll_offset(0, 3), 0);
        assert_eq!(scroll_offset(2, 3), 0);
        assert_eq!(scroll_offset(3, 3), 1);
        assert_eq!(scroll_offset(9, 0), 10);
    }

    #[test]
    fn elapsed_text_picks_seconds_minutes_or_hours() {
        assert_eq!(elapsed_text(45), "45s");
        assert_eq!(elapsed_text(120), "2m");
        assert_eq!(elapsed_text(3900), "1h05m");
    }

    #[test]
    fn rate_text_compacts_thousands_and_millions() {
        assert_eq!(rate_text(850), "850");
        assert_eq!(rate_text(4200), "4.2k");
        assert_eq!(rate_text(1_500_000), "1.5M");
    }

    #[test]
    fn the_context_bar_clamps_the_fill_and_is_always_the_same_width() {
        let width = |fill| {
            agent_segs(&agent("a", "x", 1, fill), 8, &regatta())
                .iter()
                .rev()
                .take(2)
                .map(|(t, _)| t.chars().count())
                .sum::<usize>()
        };
        assert_eq!([width(-1.0), width(0.5), width(7.0)], [BAR_COLS; 3]);
    }
}
