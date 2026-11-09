//! The board's small graphs panel: calls per minute, cache hit percent, and lands with
//! quarantines per hour, three sparklines in a row.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Sparkline, SparklineBar},
};

use crate::app::App;
use crate::feed::LivePoint;
use crate::theme::Theme;

/// A sparkline cell resolves to eight levels, so a graph `rows` tall has `rows * 8` heights.
const LEVELS_PER_ROW: u64 = 8;
/// A series with fewer points than this draws an empty frame.
const MIN_POINTS: usize = 2;
/// The cache hit graph always spans 0 to 100 percent.
const PERCENT_TOP: f64 = 100.0;

/// The newest `width` values of `values` as bar heights in `0..=rows * 8`, scaled so `top` is
/// full height. A value above `top` clamps to full, a negative or non-finite one to zero, and
/// a `top` of zero or less gives all-zero bars.
fn bar_heights(values: &[f64], width: usize, top: f64, rows: u16) -> Vec<u64> {
    let full = u64::from(rows) * LEVELS_PER_ROW;
    let skip = values.len().saturating_sub(width);
    values[skip..]
        .iter()
        .map(|&v| {
            if top > 0.0 && v.is_finite() {
                ((v / top).clamp(0.0, 1.0) * full as f64).round() as u64
            } else {
                0
            }
        })
        .collect()
}

/// Alternates two bar series, `a` first, padding the shorter with zero bars.
fn interleave(a: &[u64], b: &[u64]) -> Vec<u64> {
    (0..a.len().max(b.len()))
        .flat_map(|i| {
            [
                a.get(i).copied().unwrap_or(0),
                b.get(i).copied().unwrap_or(0),
            ]
        })
        .collect()
}

fn values(points: &[LivePoint]) -> Vec<f64> {
    points.iter().map(|p| p.value).collect()
}

fn largest(values: &[f64]) -> f64 {
    values.iter().copied().fold(0.0, f64::max)
}

/// The last value of a series long enough to draw, else `None`.
fn latest(values: &[f64]) -> Option<f64> {
    values
        .last()
        .copied()
        .filter(|_| values.len() >= MIN_POINTS)
}

/// The latest value with `unit` after it, or a dash when there is none.
fn shown(latest: Option<f64>, unit: &str) -> String {
    latest.map_or_else(|| "-".to_string(), |v| format!("{v:.0}{unit}"))
}

fn frame<'a>(title: Line<'a>, theme: &Theme) -> Block<'a> {
    Block::bordered()
        .border_style(Style::new().fg(theme.border).bg(theme.panel))
        .style(Style::new().bg(theme.panel))
        .title(title)
}

fn title<'a>(name: &str, latest: Option<f64>, unit: &str, theme: &Theme) -> Line<'a> {
    Line::styled(
        format!("{name} {}", shown(latest, unit)),
        Style::new().fg(theme.fg),
    )
}

fn single(
    f: &mut Frame,
    area: Rect,
    name: &str,
    unit: &str,
    series: &[f64],
    top: f64,
    theme: &Theme,
) {
    let block = frame(title(name, latest(series), unit, theme), theme);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if series.len() < MIN_POINTS || inner.is_empty() {
        return;
    }
    let bars = bar_heights(series, usize::from(inner.width), top, inner.height);
    f.render_widget(
        Sparkline::default()
            .data(bars)
            .max(u64::from(inner.height) * LEVELS_PER_ROW)
            .style(Style::new().fg(theme.accent).bg(theme.panel)),
        inner,
    );
}

fn landings(f: &mut Frame, area: Rect, lands: &[f64], quarantines: &[f64], theme: &Theme) {
    let drawn = lands.len() >= MIN_POINTS || quarantines.len() >= MIN_POINTS;
    let (l, q) = (latest(lands), latest(quarantines));
    let title = Line::from(vec![
        Span::styled("land/quar ", Style::new().fg(theme.fg)),
        Span::styled(shown(l, ""), Style::new().fg(theme.landed)),
        Span::styled("/", Style::new().fg(theme.fg)),
        Span::styled(shown(q, ""), Style::new().fg(theme.quarantined)),
    ]);
    let block = frame(title, theme);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if !drawn || inner.is_empty() {
        return;
    }
    // Lands and quarantines share one axis and sit side by side, two columns per hour.
    let top = largest(lands).max(largest(quarantines));
    let per_series = usize::from(inner.width) / 2;
    let bars = interleave(
        &bar_heights(lands, per_series, top, inner.height),
        &bar_heights(quarantines, per_series, top, inner.height),
    );
    // `landed` and `quarantined` are the done green and dead red; `status_*` tokens belong to status.rs.
    let green = Style::new().fg(theme.landed).bg(theme.panel);
    let red = Style::new().fg(theme.quarantined).bg(theme.panel);
    let styled = bars
        .into_iter()
        .enumerate()
        .map(|(i, h)| SparklineBar::from(h).style(Some(if i % 2 == 0 { green } else { red })));
    f.render_widget(
        Sparkline::default()
            .data(styled)
            .max(u64::from(inner.height) * LEVELS_PER_ROW),
        inner,
    );
}

pub fn render(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let [calls_area, cache_area, lands_area] =
        Layout::horizontal([Constraint::Ratio(1, 3); 3]).areas(area);
    let empty = Default::default();
    let live = app.snapshot().map_or(&empty, |s| &s.live);
    let calls = values(&live.calls_per_min);
    let cache = values(&live.cache_hit_pct);
    single(
        f,
        calls_area,
        "calls/min",
        "",
        &calls,
        largest(&calls),
        theme,
    );
    single(f, cache_area, "cache hit", "%", &cache, PERCENT_TOP, theme);
    landings(
        f,
        lands_area,
        &values(&live.lands_per_hour),
        &values(&live.quarantines_per_hour),
        theme,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppPage, ThemeId};
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn bars_scale_against_the_top() {
        assert_eq!(bar_heights(&[0.0, 5.0, 10.0], 8, 10.0, 2), vec![0, 8, 16]);
    }

    #[test]
    fn a_zero_series_gives_zero_bars() {
        assert_eq!(bar_heights(&[0.0, 0.0, 0.0], 8, 0.0, 2), vec![0, 0, 0]);
        assert_eq!(bar_heights(&[3.0, 4.0], 8, 0.0, 2), vec![0, 0]);
    }

    #[test]
    fn a_series_longer_than_the_width_keeps_the_newest_values() {
        let series: Vec<f64> = (1..=10).map(f64::from).collect();
        assert_eq!(bar_heights(&series, 4, 10.0, 5), vec![28, 32, 36, 40]);
    }

    #[test]
    fn an_empty_series_gives_no_bars() {
        assert_eq!(bar_heights(&[], 8, 10.0, 2), Vec::<u64>::new());
        assert_eq!(bar_heights(&[1.0], 0, 10.0, 2), Vec::<u64>::new());
    }

    #[test]
    fn a_value_above_the_top_clamps_and_a_negative_one_is_zero() {
        assert_eq!(
            bar_heights(&[25.0, -3.0, f64::NAN], 8, 10.0, 2),
            vec![16, 0, 0]
        );
    }

    #[test]
    fn interleave_alternates_and_pads_the_shorter_series() {
        assert_eq!(interleave(&[1, 2], &[7]), vec![1, 7, 2, 0]);
    }

    #[test]
    fn the_latest_value_needs_two_points() {
        assert_eq!(shown(latest(&[1.0, 42.4]), ""), "42");
        assert_eq!(shown(latest(&[97.0, 96.6]), "%"), "97%");
        assert_eq!(shown(latest(&[5.0]), ""), "-");
        assert_eq!(shown(latest(&[]), "%"), "-");
    }

    const SNAPSHOT: &str = r#"{"schema":1,"at":"2026-11-09T00:00:00Z","chair":{"holder":"h","host":"h","epoch":1,"liveness":"live","beat_age_s":0},"spend":{"five_hour_fraction":0.0,"five_hour_source":"meter","weekly_fraction":0.0,"weekly_source":"meter","hard_stop_fraction":0.0,"five_hour_resets_at":"2026-11-09T00:00:00Z","weekly_resets_at":"2026-11-09T00:00:00Z"},"machines":[],"runs":[],"queue":[],"inbox":[],"watch":[],"live":{"calls_per_min":[{"at":"t1","value":4},{"at":"t2","value":9},{"at":"t3","value":14},{"at":"t4","value":6},{"at":"t5","value":18},{"at":"t6","value":12},{"at":"t7","value":20}],"cache_hit_pct":[{"at":"t1","value":40},{"at":"t2","value":55},{"at":"t3","value":70},{"at":"t4","value":62},{"at":"t5","value":80},{"at":"t6","value":88},{"at":"t7","value":94}],"lands_per_hour":[{"at":"t1","value":1},{"at":"t2","value":3},{"at":"t3","value":2},{"at":"t4","value":5},{"at":"t5","value":4},{"at":"t6","value":6}],"quarantines_per_hour":[{"at":"t1","value":0},{"at":"t2","value":1},{"at":"t3","value":0},{"at":"t4","value":2},{"at":"t5","value":1},{"at":"t6","value":3}]}}"#;

    fn draw(json: &str) -> Terminal<TestBackend> {
        let mut app = App::new(AppPage::Board, ThemeId::Regatta);
        app.apply_snapshot(crate::feed::parse_snapshot(json).expect("literal snapshot parses"));
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let mut terminal = Terminal::new(TestBackend::new(48, 6)).expect("terminal");
        terminal
            .draw(|f| render(f, f.area(), &app, &theme))
            .expect("draw should not fail");
        terminal
    }

    #[test]
    fn the_three_small_graphs() {
        insta::assert_snapshot!(draw(SNAPSHOT).backend().to_string());
    }

    #[test]
    fn an_empty_live_block_draws_three_empty_frames() {
        let json = SNAPSHOT.split(r#","live":"#).next().unwrap().to_string() + "}";
        let text = draw(&json).backend().to_string();
        assert!(text.contains("calls/min -") && text.contains("cache hit -"));
        assert!(text.contains("land/quar -/-"));
        assert!(!text.contains(['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█']));
    }
}
