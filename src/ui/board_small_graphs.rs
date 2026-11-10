//! The board's small graphs panel: calls per minute, cache hit percent, and lands against
//! quarantines per hour, as three titled sparklines in a row. Each title carries the latest value.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Paragraph},
};

use super::status::{self, Status};
use crate::app::App;
use crate::feed::LivePoint;
use crate::theme::Theme;

/// Cache hit is a percentage, so its graph is always scaled to this.
const PERCENT_TOP: f64 = 100.0;
/// Bar levels: 0 is blank, 1 to 8 are the block glyphs `U+2581` to `U+2588`.
const LEVELS: [char; 9] = [
    ' ', '\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}', '\u{2587}',
    '\u{2588}',
];
/// A series shorter than this draws no bars.
const MIN_POINTS: usize = 2;

/// One bar level (0 to 8) per column for the last `width` values, scaled so `top` is level 8.
/// A bad value, or a top that is not above zero, is level 0.
fn bar_heights(values: &[f64], width: usize, top: f64) -> Vec<u8> {
    let shown = &values[values.len().saturating_sub(width)..];
    shown
        .iter()
        .map(|&v| {
            if top.is_nan() || top <= 0.0 || v.is_nan() {
                0
            } else {
                ((v / top).clamp(0.0, 1.0) * 8.0).round() as u8
            }
        })
        .collect()
}

fn level_glyph(level: u8) -> char {
    LEVELS[usize::from(level.min(8))]
}

/// The glyph row for a series. A series with fewer than two points is blank.
fn spark_row(values: &[f64], width: usize, top: f64) -> String {
    if values.len() < MIN_POINTS {
        return String::new();
    }
    bar_heights(values, width, top)
        .into_iter()
        .map(level_glyph)
        .collect()
}

fn values(points: &[LivePoint]) -> Vec<f64> {
    points.iter().map(|p| p.value).collect()
}

/// The largest value across both series, so lands and quarantines share one scale.
fn shared_top(a: &[f64], b: &[f64]) -> f64 {
    a.iter().chain(b).copied().fold(0.0, f64::max)
}

fn latest(values: &[f64]) -> Option<f64> {
    values.last().copied()
}

fn shown(value: Option<f64>, suffix: &str) -> String {
    value.map_or_else(|| "-".to_string(), |v| format!("{v:.0}{suffix}"))
}

fn graph_block(theme: &Theme, title: String) -> Block<'static> {
    Block::bordered()
        .border_style(Style::new().fg(theme.border))
        .title(title)
        .title_style(Style::new().fg(theme.border_focus))
}

fn spark(f: &mut Frame, inner: Rect, row: u16, text: String, color: Color) {
    if row >= inner.height || text.is_empty() {
        return;
    }
    let line = Rect::new(inner.x, inner.y + row, inner.width, 1);
    f.render_widget(Paragraph::new(text).style(Style::new().fg(color)), line);
}

pub fn render(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let empty = crate::feed::Live::default();
    let live = app.snapshot().map_or(&empty, |s| &s.live);
    let calls = values(&live.calls_per_min);
    let cache = values(&live.cache_hit_pct);
    let lands = values(&live.lands_per_hour);
    let quarantines = values(&live.quarantines_per_hour);

    let [calls_area, cache_area, landing_area] =
        Layout::horizontal([Constraint::Ratio(1, 3); 3]).areas(area);

    let block = graph_block(theme, format!(" calls/min {} ", shown(latest(&calls), "")));
    let inner = block.inner(calls_area);
    f.render_widget(block, calls_area);
    let top = shared_top(&calls, &[]);
    spark(
        f,
        inner,
        0,
        spark_row(&calls, usize::from(inner.width), top),
        theme.accent,
    );

    let block = graph_block(theme, format!(" cache hit {} ", shown(latest(&cache), "%")));
    let inner = block.inner(cache_area);
    f.render_widget(block, cache_area);
    spark(
        f,
        inner,
        0,
        spark_row(&cache, usize::from(inner.width), PERCENT_TOP),
        theme.accent,
    );

    let title = format!(
        " lands/quar per h {}/{} ",
        shown(latest(&lands), ""),
        shown(latest(&quarantines), "")
    );
    let block = graph_block(theme, title);
    let inner = block.inner(landing_area);
    f.render_widget(block, landing_area);
    let top = shared_top(&lands, &quarantines);
    let width = usize::from(inner.width);
    // Lands in the landed (done) green, quarantines in the quarantined (dead) red.
    spark(
        f,
        inner,
        0,
        spark_row(&lands, width, top),
        status::color(theme, Status::Landed),
    );
    spark(
        f,
        inner,
        1,
        spark_row(&quarantines, width, top),
        status::color(theme, Status::Quarantined),
    );
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::app::{AppPage, ThemeId};

    fn app_with_live(live_json: &str) -> App {
        let json = format!(
            r#"{{"schema":1,"at":"2026-09-29T00:00:00Z","chair":{{"holder":"h","host":"h","epoch":1,"liveness":"live","beat_age_s":0}},"spend":{{"five_hour_fraction":0.0,"five_hour_source":"meter","weekly_fraction":0.0,"weekly_source":"meter","hard_stop_fraction":0.0,"five_hour_resets_at":"2026-09-29T00:00:00Z","weekly_resets_at":"2026-09-29T00:00:00Z"}},"machines":[],"runs":[],"queue":[],"inbox":[],"watch":[],"live":{live_json}}}"#
        );
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.apply_snapshot(crate::feed::parse_snapshot(&json).expect("literal snapshot parses"));
        app
    }

    const LIVE: &str = r#"{"calls_per_min":[{"at":"a","value":2.0},{"at":"b","value":4.0},{"at":"c","value":8.0},{"at":"d","value":6.0},{"at":"e","value":10.0}],"cache_hit_pct":[{"at":"a","value":50.0},{"at":"b","value":75.0},{"at":"c","value":90.0},{"at":"d","value":100.0}],"lands_per_hour":[{"at":"a","value":1.0},{"at":"b","value":3.0},{"at":"c","value":6.0},{"at":"d","value":4.0}],"quarantines_per_hour":[{"at":"a","value":0.0},{"at":"b","value":1.0},{"at":"c","value":0.0},{"at":"d","value":2.0}]}"#;

    fn draw(app: &App, theme: &Theme) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(72, 6)).expect("terminal");
        terminal
            .draw(|f| render(f, f.area(), app, theme))
            .expect("draw should not fail");
        terminal
    }

    #[test]
    fn the_three_small_graphs() {
        let app = app_with_live(LIVE);
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let terminal = draw(&app, &theme);
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn lands_draw_in_the_done_colour_and_quarantines_in_the_failed_colour() {
        let app = app_with_live(LIVE);
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let terminal = draw(&app, &theme);
        let buffer = terminal.backend().buffer();
        // The landing graph is the last third: columns 48 to 71, lands on row 1, quarantines on row 2.
        let lands = &buffer[(49, 1)];
        let quarantines = &buffer[(52, 2)];
        assert_eq!(
            lands.style().fg,
            Some(status::color(&theme, Status::Landed))
        );
        assert_ne!(lands.symbol(), " ");
        assert_eq!(
            quarantines.style().fg,
            Some(status::color(&theme, Status::Quarantined))
        );
        assert_ne!(quarantines.symbol(), " ");
    }

    #[test]
    fn short_and_empty_series_draw_a_frame_with_no_bars() {
        let app = app_with_live(r#"{"calls_per_min":[{"at":"a","value":5.0}]}"#);
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let terminal = draw(&app, &theme);
        let text = terminal.backend().to_string();
        assert!(text.contains("calls/min 5"));
        assert!(text.contains("cache hit -"));
        assert!(!text.contains('\u{2588}'));
    }

    #[test]
    fn bar_heights_scale_to_the_top() {
        assert_eq!(bar_heights(&[0.0, 5.0, 10.0], 3, 10.0), vec![0, 4, 8]);
        assert_eq!(bar_heights(&[150.0, -3.0], 2, 100.0), vec![8, 0]);
    }

    #[test]
    fn a_zero_series_is_all_level_zero() {
        assert_eq!(bar_heights(&[0.0, 0.0, 0.0], 3, 0.0), vec![0, 0, 0]);
        assert_eq!(bar_heights(&[0.0, 0.0, 0.0], 3, 100.0), vec![0, 0, 0]);
        assert_eq!(bar_heights(&[3.0, f64::NAN], 2, f64::NAN), vec![0, 0]);
    }

    #[test]
    fn a_series_longer_than_the_width_keeps_the_latest_values() {
        assert_eq!(
            bar_heights(&[8.0, 8.0, 0.0, 4.0, 8.0], 3, 8.0),
            vec![0, 4, 8]
        );
        assert_eq!(bar_heights(&[1.0, 2.0], 0, 8.0), Vec::<u8>::new());
    }

    #[test]
    fn level_glyphs_run_from_blank_to_a_full_block() {
        assert_eq!(level_glyph(0), ' ');
        assert_eq!(level_glyph(1), '\u{2581}');
        assert_eq!(level_glyph(8), '\u{2588}');
        assert_eq!(level_glyph(200), '\u{2588}');
    }

    #[test]
    fn a_series_of_fewer_than_two_points_is_a_blank_row() {
        assert_eq!(spark_row(&[], 10, 10.0), "");
        assert_eq!(spark_row(&[9.0], 10, 10.0), "");
        assert_eq!(spark_row(&[0.0, 10.0], 10, 10.0), " \u{2588}");
    }
}
