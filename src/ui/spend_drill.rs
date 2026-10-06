//! Rendering for the spend board (`cox dash --detail spend`): both usage meters as gauges,
//! their history as block-character rows, and one bar row per day of cost. Nothing calls
//! `render` yet, so unused-code warnings are silenced until the wiring phase.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::detail::{DayCost, Meter, MeterPoint, SpendDetail};
use crate::theme::Theme;

/// A meter turns `meter_mid` from 60% full and `meter_high` from 90%.
const METER_MID_FROM: f64 = 0.6;
const METER_HIGH_FROM: f64 = 0.9;
const GAUGE_WIDTH: usize = 20;
const LABEL_WIDTH: usize = 8;
const DAY_WIDTH: usize = 10;
const COST_WIDTH: usize = 9;
const SPARKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

fn fg(color: Color) -> Style {
    Style::default().fg(color)
}

fn meter_color(theme: &Theme, fraction: f64) -> Color {
    if fraction < METER_MID_FROM {
        theme.meter_low
    } else if fraction < METER_HIGH_FROM {
        theme.meter_mid
    } else {
        theme.meter_high
    }
}

/// How many of `width` cells `fraction` fills, clamped to `0..=width`.
fn filled_cells(fraction: f64, width: usize) -> usize {
    (fraction.clamp(0.0, 1.0) * width as f64).round() as usize
}

fn gauge_line(
    label: &str,
    meter: &Meter,
    theme: &Theme,
    offset: chrono::FixedOffset,
) -> Line<'static> {
    let color = meter_color(theme, meter.fraction);
    let filled = filled_cells(meter.fraction, GAUGE_WIDTH);
    Line::from(vec![
        Span::styled(format!("{label:<LABEL_WIDTH$}"), fg(theme.dim)),
        Span::styled("█".repeat(filled), fg(color)),
        Span::styled("░".repeat(GAUGE_WIDTH - filled), fg(theme.track)),
        Span::styled(format!(" {:>3.0}%", meter.fraction * 100.0), fg(color)),
        Span::styled(
            format!("  ${:.2} of ${:.2}", meter.used_usd, meter.ceiling_usd),
            fg(theme.fg),
        ),
        Span::styled(
            format!("  resets {}", super::local_time(&meter.resets_at, offset)),
            fg(theme.dim),
        ),
    ])
}

/// One block character per value in `0.0..=1.0`, keeping the newest `width`.
fn sparkline(values: &[f64], width: usize) -> String {
    let start = values.len().saturating_sub(width);
    values[start..]
        .iter()
        .map(|v| SPARKS[(v.clamp(0.0, 1.0) * (SPARKS.len() - 1) as f64).round() as usize])
        .collect()
}

fn history_line(label: &str, values: &[f64], width: usize, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<LABEL_WIDTH$}"), fg(theme.dim)),
        Span::styled(
            sparkline(values, width.saturating_sub(LABEL_WIDTH)),
            fg(theme.accent),
        ),
    ])
}

fn history_lines(history: &[MeterPoint], width: usize, theme: &Theme) -> Vec<Line<'static>> {
    if history.is_empty() {
        return vec![Line::styled("no meter history yet", fg(theme.dim))];
    }
    let five_hour: Vec<f64> = history.iter().map(|p| p.five_hour).collect();
    let weekly: Vec<f64> = history.iter().map(|p| p.weekly).collect();
    vec![
        history_line("5-hour", &five_hour, width, theme),
        history_line("weekly", &weekly, width, theme),
    ]
}

fn daily_lines(daily: &[DayCost], width: usize, theme: &Theme) -> Vec<Line<'static>> {
    if daily.is_empty() {
        return vec![Line::styled("no daily cost yet", fg(theme.dim))];
    }
    let largest = daily.iter().map(|d| d.cost).fold(0.0, f64::max);
    let bar_width = width.saturating_sub(DAY_WIDTH + COST_WIDTH + 2);
    daily
        .iter()
        .map(|d| {
            let filled = if largest > 0.0 {
                filled_cells(d.cost / largest, bar_width)
            } else {
                0
            };
            Line::from(vec![
                Span::styled(format!("{:<DAY_WIDTH$} ", d.day), fg(theme.dim)),
                Span::styled("█".repeat(filled), fg(theme.accent)),
                Span::styled(
                    format!(
                        "{}{:>COST_WIDTH$}",
                        " ".repeat(bar_width - filled + 1),
                        format!("${:.2}", d.cost)
                    ),
                    fg(theme.accent),
                ),
            ])
        })
        .collect()
}

/// Draws the spend board: a `Spend` block with the two meters, their history, then the
/// per-day cost rows.
pub fn render(
    f: &mut Frame,
    area: Rect,
    detail: &SpendDetail,
    theme: &Theme,
    offset: chrono::FixedOffset,
) {
    let base = Style::default().fg(theme.fg).bg(theme.bg);
    let block = Block::default()
        .title("Spend")
        .title_bottom(Line::styled("Esc back", fg(theme.dim).bg(theme.bg)))
        .borders(Borders::ALL)
        .style(base);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(inner);
    let width = usize::from(inner.width);

    let meters = vec![
        gauge_line("5-hour", &detail.five_hour, theme, offset),
        gauge_line("weekly", &detail.weekly, theme, offset),
    ];
    f.render_widget(Paragraph::new(meters).style(base), rows[0]);
    f.render_widget(
        Paragraph::new(history_lines(&detail.history, width, theme)).style(base),
        rows[1],
    );
    f.render_widget(
        Paragraph::new(daily_lines(&detail.daily, width, theme)).style(base),
        rows[2],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ThemeId;
    use ratatui::{Terminal, backend::TestBackend};

    const FIXTURE: &str = include_str!("../../tests/fixtures/dash_detail_spend_v1.json");
    const THEMES: [ThemeId; 2] = [ThemeId::Regatta, ThemeId::HarborLight];

    fn fixture() -> SpendDetail {
        serde_json::from_str(FIXTURE).expect("fixture should parse")
    }

    fn draw(detail: &SpendDetail, theme: &Theme) -> Terminal<TestBackend> {
        let offset = chrono::FixedOffset::east_opt(0).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(100, 28)).expect("terminal");
        terminal
            .draw(|f| render(f, f.area(), detail, theme, offset))
            .expect("draw should not fail");
        terminal
    }

    /// The row index whose text starts, after the border, with `label`.
    fn row_of(terminal: &Terminal<TestBackend>, label: &str) -> u16 {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .find(|&y| {
                let text: String = (1..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect();
                text.starts_with(label)
            })
            .unwrap_or_else(|| panic!("no row starting with {label:?}"))
    }

    /// The fg of the first gauge-bar cell on the row labelled `label`.
    fn bar_fg(terminal: &Terminal<TestBackend>, label: &str) -> Color {
        let y = row_of(terminal, label);
        terminal.backend().buffer()[(1 + LABEL_WIDTH as u16, y)].fg
    }

    #[test]
    fn renders_spend_snapshot_in_the_regatta_theme() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let terminal = draw(&fixture(), &theme);
        insta::with_settings!({snapshot_path => "snapshots/spend_drill"}, {
            insta::assert_snapshot!(terminal.backend().to_string());
        });
    }

    #[test]
    fn renders_spend_snapshot_in_the_harbor_light_theme() {
        let theme = crate::theme::resolve_for(ThemeId::HarborLight, Some("truecolor"));
        let terminal = draw(&fixture(), &theme);
        insta::with_settings!({snapshot_path => "snapshots/spend_drill"}, {
            insta::assert_snapshot!(terminal.backend().to_string());
        });
    }

    #[test]
    fn the_weekly_gauge_is_meter_mid_and_the_five_hour_gauge_meter_low() {
        for id in THEMES {
            let theme = crate::theme::resolve_for(id, Some("truecolor"));
            let terminal = draw(&fixture(), &theme);
            assert_eq!(bar_fg(&terminal, "weekly"), theme.meter_mid);
            assert_eq!(bar_fg(&terminal, "5-hour"), theme.meter_low);
        }
    }

    #[test]
    fn a_meter_at_ninety_five_percent_is_meter_high() {
        for id in THEMES {
            let theme = crate::theme::resolve_for(id, Some("truecolor"));
            let detail = SpendDetail {
                five_hour: Meter {
                    fraction: 0.95,
                    used_usd: 47.5,
                    ceiling_usd: 50.0,
                    resets_at: "2026-10-05T15:00:00Z".to_string(),
                },
                ..fixture()
            };
            let terminal = draw(&detail, &theme);
            assert_eq!(bar_fg(&terminal, "5-hour"), theme.meter_high);
        }
    }

    #[test]
    fn thresholds_split_at_sixty_and_ninety_percent() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        assert_eq!(meter_color(&theme, 0.59), theme.meter_low);
        assert_eq!(meter_color(&theme, 0.6), theme.meter_mid);
        assert_eq!(meter_color(&theme, 0.9), theme.meter_high);
    }

    #[test]
    fn empty_history_and_daily_draw_one_dim_line_each() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let detail = SpendDetail {
            history: vec![],
            daily: vec![],
            ..fixture()
        };
        let terminal = draw(&detail, &theme);
        let y = row_of(&terminal, "no meter history yet");
        assert_eq!(terminal.backend().buffer()[(1, y)].fg, theme.dim);
        let y = row_of(&terminal, "no daily cost yet");
        assert_eq!(terminal.backend().buffer()[(1, y)].fg, theme.dim);
    }
}
