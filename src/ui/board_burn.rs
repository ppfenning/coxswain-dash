//! The board's burn panel: one gauge row per meter, the 5-hour meter beside the dollars per minute
//! over five minutes and the weekly meter beside the rate over fifteen.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};

use super::local_time;
use super::status::{Status, color};
use crate::app::App;
use crate::feed::Meter;
use crate::theme::Theme;

/// Columns the row label takes, including the space after it.
const LABEL_WIDTH: u16 = 3;
/// Below this many columns a bar says nothing, so the row keeps only its text.
const MIN_BAR_WIDTH: u16 = 4;
/// A bar turns `meter_mid` once it has used this share of its ceiling.
const NEAR_CEILING: f64 = 0.6;
const FILLED: char = '█';
const EMPTY: char = '░';
const MARKER: char = '│';

/// Filled cells of a `width`-cell bar. The fraction is clamped to 0..=1 and a NaN counts as empty.
fn bar_cells(fraction: f64, width: u16) -> u16 {
    if fraction.is_nan() {
        return 0;
    }
    (fraction.clamp(0.0, 1.0) * f64::from(width)).round() as u16
}

/// The column of the ceiling marker, or `None` for a zero-width bar. Never past the last cell.
fn marker_cell(ceiling: f64, width: u16) -> Option<u16> {
    width
        .checked_sub(1)
        .map(|last| bar_cells(ceiling, width).min(last))
}

/// The failed red, taken from the one status map.
fn failed_red(theme: &Theme) -> Color {
    color(theme, Status::Failed)
}

/// The failed red over the ceiling. Under it, `meter_low`, and `meter_mid` from 60% of the
/// ceiling. `meter_high` is never used: a meter that is not over its ceiling is not failing.
fn bar_color(used: f64, ceiling: f64, over: bool, theme: &Theme) -> Color {
    let share = if ceiling > 0.0 { used / ceiling } else { used };
    if over {
        failed_red(theme)
    } else if share >= NEAR_CEILING {
        theme.meter_mid
    } else {
        theme.meter_low
    }
}

fn rate_text(usd_per_min: f64) -> String {
    format!("${usd_per_min:.2}/min")
}

fn projection_text(projected: f64, resets_local: &str) -> String {
    format!("proj {:.0}% @ {resets_local}", projected * 100.0)
}

/// One gauge row: label, bar with the ceiling marker, rate, then the projection at reset.
fn gauge_line(
    label: &str,
    usd_per_min: f64,
    meter: &Meter,
    width: u16,
    offset: chrono::FixedOffset,
    theme: &Theme,
) -> Line<'static> {
    let rate = rate_text(usd_per_min);
    let projection = projection_text(
        meter.projected_fraction,
        &local_time(&meter.resets_at, offset),
    );
    let text_width = (rate.chars().count() + 1 + projection.chars().count()) as u16;
    let bar_width = width.saturating_sub(LABEL_WIDTH + text_width + 1);
    let colour = bar_color(
        meter.used_fraction,
        meter.ceiling_fraction,
        meter.over_ceiling,
        theme,
    );
    let mut spans = vec![Span::styled(
        format!("{label:<w$}", w = usize::from(LABEL_WIDTH)),
        Style::default().fg(theme.dim),
    )];
    if bar_width >= MIN_BAR_WIDTH {
        let filled = bar_cells(meter.used_fraction, bar_width);
        let marker = marker_cell(meter.ceiling_fraction, bar_width);
        spans.extend((0..bar_width).map(|i| {
            if Some(i) == marker {
                Span::styled(MARKER.to_string(), Style::default().fg(theme.fg))
            } else if i < filled {
                Span::styled(FILLED.to_string(), Style::default().fg(colour))
            } else {
                Span::styled(EMPTY.to_string(), Style::default().fg(theme.track))
            }
        }));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled(rate, Style::default().fg(theme.fg)));
    let projection_colour = if meter.over_ceiling {
        failed_red(theme)
    } else {
        theme.fg
    };
    spans.push(Span::raw(" "));
    spans.push(Span::styled(
        projection,
        Style::default().fg(projection_colour),
    ));
    Line::from(spans)
}

pub fn render(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let block = Block::bordered()
        .title("burn")
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().fg(theme.fg).bg(theme.bg));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let Some(snapshot) = app.snapshot() else {
        return;
    };
    let burn = &snapshot.live.burn;
    let offset = app.utc_offset();
    let lines = vec![
        gauge_line(
            "5h",
            burn.usd_per_min_5m,
            &burn.meters.five_hour,
            inner.width,
            offset,
            theme,
        ),
        gauge_line(
            "wk",
            burn.usd_per_min_15m,
            &burn.meters.weekly,
            inner.width,
            offset,
            theme,
        ),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ThemeId;
    use ratatui::{Terminal, backend::TestBackend};

    fn truecolor() -> Theme {
        crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"))
    }

    #[test]
    fn bar_cells_scales_and_clamps_the_fraction() {
        assert_eq!(bar_cells(0.0, 20), 0);
        assert_eq!(bar_cells(0.5, 20), 10);
        assert_eq!(bar_cells(1.0, 20), 20);
        assert_eq!(bar_cells(1.7, 20), 20);
        assert_eq!(bar_cells(-0.2, 20), 0);
        assert_eq!(bar_cells(f64::NAN, 20), 0);
        assert_eq!(bar_cells(0.5, 0), 0);
    }

    #[test]
    fn marker_cell_stays_on_the_bar() {
        assert_eq!(marker_cell(0.9, 10), Some(9));
        assert_eq!(marker_cell(0.5, 10), Some(5));
        assert_eq!(marker_cell(1.0, 10), Some(9));
        assert_eq!(marker_cell(0.0, 10), Some(0));
        assert_eq!(marker_cell(0.5, 0), None);
    }

    #[test]
    fn bar_color_is_red_over_and_low_then_mid_under() {
        let theme = truecolor();
        assert_eq!(bar_color(0.1, 0.9, false, &theme), theme.meter_low);
        assert_eq!(bar_color(0.53, 0.9, false, &theme), theme.meter_low);
        assert_eq!(bar_color(0.54, 0.9, false, &theme), theme.meter_mid);
        assert_eq!(bar_color(0.89, 0.9, false, &theme), theme.meter_mid);
        assert_eq!(bar_color(0.95, 0.9, true, &theme), theme.status_failed);
        assert_ne!(theme.status_failed, theme.meter_mid);
    }

    #[test]
    fn the_texts_are_literal() {
        assert_eq!(rate_text(0.4213), "$0.42/min");
        assert_eq!(projection_text(0.854, "14:00"), "proj 85% @ 14:00");
    }

    const FEED: &str = r#"{"schema":1,"at":"2026-11-09T12:00:00Z","chair":{"holder":"h","host":"x","epoch":1,"liveness":"live","beat_age_s":1},"spend":{"five_hour_fraction":0.1,"five_hour_source":"meter","weekly_fraction":0.1,"weekly_source":"meter","hard_stop_fraction":0.9,"five_hour_resets_at":"2026-11-09T14:00:00Z","weekly_resets_at":"2026-11-14T08:00:00Z"},"machines":[],"runs":[],"queue":[],"inbox":[],"watch":[],"live":{"burn":{"usd_per_min_5m":0.42,"usd_per_min_15m":0.31,"meters":{"five_hour":{"used_fraction":0.95,"projected_fraction":1.2,"ceiling_fraction":0.9,"resets_at":"2026-11-09T14:00:00Z","over_ceiling":true},"weekly":{"used_fraction":0.3,"projected_fraction":0.55,"ceiling_fraction":0.9,"resets_at":"2026-11-14T08:00:00Z","over_ceiling":false}}}}}"#;

    fn drawn() -> Terminal<TestBackend> {
        let mut app = App::default();
        app.apply_snapshot(crate::feed::parse_snapshot(FEED).expect("literal feed parses"));
        let theme = truecolor();
        let mut terminal = Terminal::new(TestBackend::new(70, 5)).expect("terminal");
        terminal
            .draw(|f| render(f, f.area(), &app, &theme))
            .expect("draw should not fail");
        terminal
    }

    /// The foreground of the first cell where `needle` starts on `row`.
    fn fg_at(terminal: &Terminal<TestBackend>, row: u16, needle: &str) -> Color {
        let buffer = terminal.backend().buffer();
        let line: String = (0..buffer.area.width)
            .map(|x| buffer[(x, row)].symbol().chars().next().unwrap_or(' '))
            .collect();
        let x = line
            .find(needle)
            .map(|byte| line[..byte].chars().count())
            .unwrap_or_else(|| panic!("{needle:?} not on row {row}: {line:?}"));
        buffer[(x as u16, row)].fg
    }

    #[test]
    fn a_gauge_over_its_ceiling_is_red() {
        let terminal = drawn();
        insta::assert_snapshot!(terminal.backend().to_string());
        let theme = truecolor();
        // Row 1 is the 5-hour meter (over), row 2 the weekly meter (under).
        assert_eq!(fg_at(&terminal, 1, "█"), theme.status_failed);
        assert_eq!(fg_at(&terminal, 1, "proj"), theme.status_failed);
        assert_ne!(fg_at(&terminal, 2, "█"), theme.status_failed);
        assert_ne!(fg_at(&terminal, 2, "proj"), theme.status_failed);
        assert_eq!(fg_at(&terminal, 2, "█"), theme.meter_low);
    }
}
