//! The board's burn panel: one gauge row per meter. Each row shows dollars per minute, a bar of the
//! used fraction with a marker at the ceiling, the projected use at reset and the reset time.

use chrono::FixedOffset;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};

use super::status::{self, Status};
use crate::app::App;
use crate::feed::Meter;
use crate::theme::Theme;

/// A bar narrower than this is not worth drawing; the text after it is dropped first.
const MIN_BAR_WIDTH: u16 = 8;
/// The bar moves to the mid colour once used reaches this share of the ceiling.
const NEAR_CEILING: f64 = 0.8;
const MARKER: &str = "\u{2502}";

/// Whole bar cells filled by `fraction` of `width`. NaN fills none.
fn bar_cells(fraction: f64, width: u16) -> u16 {
    if fraction.is_nan() {
        return 0;
    }
    (fraction.clamp(0.0, 1.0) * f64::from(width)).round() as u16
}

/// The cell holding the ceiling marker, or None when there is no ceiling to mark.
fn marker_cell(ceiling: f64, width: u16) -> Option<u16> {
    if width == 0 || ceiling.is_nan() || ceiling <= 0.0 {
        return None;
    }
    Some(bar_cells(ceiling, width).min(width - 1))
}

/// Red when over the ceiling, mid once near it, low otherwise. A bad ceiling counts as low.
fn bar_color(theme: &Theme, used: f64, ceiling: f64, over: bool) -> Color {
    if over {
        status::color(theme, Status::Failed)
    } else if ceiling > 0.0 && used / ceiling >= NEAR_CEILING {
        theme.meter_mid
    } else {
        theme.meter_low
    }
}

fn percent(fraction: f64) -> String {
    if fraction.is_finite() {
        format!("{:.0}%", fraction * 100.0)
    } else {
        "-".to_string()
    }
}

/// The tail after the bar, longest first: projection and reset, projection alone, nothing.
fn tails(meter: &Meter, offset: FixedOffset) -> [String; 3] {
    let proj = format!(" proj {:>4}", percent(meter.projected_fraction));
    [
        format!("{proj} {}", super::local_time(&meter.resets_at, offset)),
        proj,
        String::new(),
    ]
}

fn gauge_line(
    theme: &Theme,
    label: &str,
    usd_per_min: f64,
    meter: &Meter,
    width: u16,
    offset: FixedOffset,
) -> Line<'static> {
    let head = format!("{label} ${usd_per_min:.2}/min ");
    let head_width = head.chars().count() as u16;
    let (tail, bar_width) = tails(meter, offset)
        .into_iter()
        .map(|t| {
            let bar = width.saturating_sub(head_width + t.chars().count() as u16);
            (t, bar)
        })
        .find(|(_, bar)| *bar >= MIN_BAR_WIDTH)
        .unwrap_or_else(|| (String::new(), width.saturating_sub(head_width)));
    let fill = bar_cells(meter.used_fraction, bar_width);
    let marker = marker_cell(meter.ceiling_fraction, bar_width);
    let fill_color = bar_color(
        theme,
        meter.used_fraction,
        meter.ceiling_fraction,
        meter.over_ceiling,
    );
    let mut spans = vec![Span::styled(head, Style::new().fg(theme.dim))];
    spans.extend((0..bar_width).map(|i| {
        if Some(i) == marker {
            Span::styled(MARKER, Style::new().fg(theme.fg))
        } else if i < fill {
            Span::styled("\u{2588}", Style::new().fg(fill_color))
        } else {
            Span::styled("\u{2591}", Style::new().fg(theme.track))
        }
    }));
    let tail_color = if meter.over_ceiling {
        status::color(theme, Status::Failed)
    } else {
        theme.fg
    };
    spans.push(Span::styled(tail, Style::new().fg(tail_color)));
    Line::from(spans)
}

pub fn render(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let block = Block::bordered()
        .border_style(Style::new().fg(theme.border))
        .title(Span::styled("burn", Style::new().fg(theme.fg)));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let Some(snapshot) = app.snapshot() else {
        return;
    };
    let burn = &snapshot.live.burn;
    let offset = app.utc_offset();
    let rows = [
        ("5h", burn.usd_per_min_5m, &burn.meters.five_hour),
        ("wk", burn.usd_per_min_15m, &burn.meters.weekly),
    ];
    for (i, (label, usd, meter)) in rows.into_iter().enumerate() {
        let y = inner.y.saturating_add(i as u16);
        if i as u16 >= inner.height {
            break;
        }
        let row = Rect::new(inner.x, y, inner.width, 1);
        let line = gauge_line(theme, label, usd, meter, row.width, offset);
        f.render_widget(Paragraph::new(line), row);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::app::{AppPage, ThemeId};

    const BURN: &str = r#""live":{"burn":{"usd_per_min_5m":1.2,"usd_per_min_15m":0.42,"meters":{"five_hour":{"used_fraction":0.9,"projected_fraction":1.1,"ceiling_fraction":0.8,"resets_at":"2026-09-29T18:00:00Z","over_ceiling":true},"weekly":{"used_fraction":0.3,"projected_fraction":0.55,"ceiling_fraction":0.8,"resets_at":"2026-10-04T04:00:00Z","over_ceiling":false}}}}"#;

    fn app_with_burn() -> App {
        let json = format!(
            r#"{{"schema":1,"at":"2026-09-29T00:00:00Z","chair":{{"holder":"h","host":"h","epoch":1,"liveness":"live","beat_age_s":0}},"spend":{{"five_hour_fraction":0.0,"five_hour_source":"meter","weekly_fraction":0.0,"weekly_source":"meter","hard_stop_fraction":0.0,"five_hour_resets_at":"","weekly_resets_at":""}},"machines":[],"runs":[],"queue":[],"inbox":[],"watch":[],{BURN}}}"#
        );
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.apply_snapshot(crate::feed::parse_snapshot(&json).expect("literal snapshot parses"));
        app
    }

    fn draw(app: &App, theme: &Theme) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(60, 5)).expect("terminal");
        terminal
            .draw(|f| render(f, f.area(), app, theme))
            .expect("draw should not fail");
        terminal
    }

    fn truecolor() -> Theme {
        crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"))
    }

    fn meter(used: f64, projected: f64, over: bool) -> Meter {
        Meter {
            used_fraction: used,
            projected_fraction: projected,
            ceiling_fraction: 0.8,
            resets_at: "2026-09-29T18:00:00Z".to_string(),
            over_ceiling: over,
        }
    }

    fn text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn a_gauge_over_its_ceiling_is_red() {
        let terminal = draw(&app_with_burn(), &truecolor());
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn the_over_meter_bar_is_red_and_the_other_is_not() {
        let theme = truecolor();
        let terminal = draw(&app_with_burn(), &theme);
        let buffer = terminal.backend().buffer();
        let first_fill = |y: u16| {
            (0..60)
                .map(|x| &buffer[(x, y)])
                .find(|c| c.symbol() == "\u{2588}")
                .map(|c| c.style().fg)
        };
        assert_eq!(first_fill(1), Some(Some(theme.status_failed)));
        assert_eq!(first_fill(2), Some(Some(theme.meter_low)));
    }

    #[test]
    fn bar_cells_scales_clamps_and_survives_nan() {
        assert_eq!(bar_cells(0.5, 10), 5);
        assert_eq!(bar_cells(1.7, 10), 10);
        assert_eq!(bar_cells(-1.0, 10), 0);
        assert_eq!(bar_cells(f64::NAN, 10), 0);
    }

    #[test]
    fn the_marker_sits_at_the_ceiling_and_never_past_the_bar() {
        assert_eq!(marker_cell(0.8, 10), Some(8));
        assert_eq!(marker_cell(1.0, 10), Some(9));
        assert_eq!(marker_cell(0.0, 10), None);
        assert_eq!(marker_cell(f64::NAN, 10), None);
        assert_eq!(marker_cell(0.5, 0), None);
    }

    #[test]
    fn the_bar_colour_follows_the_ceiling() {
        let theme = truecolor();
        assert_eq!(bar_color(&theme, 0.9, 0.8, true), theme.status_failed);
        assert_eq!(bar_color(&theme, 0.7, 0.8, false), theme.meter_mid);
        assert_eq!(bar_color(&theme, 0.3, 0.8, false), theme.meter_low);
        assert_eq!(bar_color(&theme, 0.3, 0.0, false), theme.meter_low);
    }

    #[test]
    fn a_gauge_line_shows_rate_projection_and_local_reset_time() {
        let et = FixedOffset::west_opt(4 * 3600).unwrap();
        let line = gauge_line(&truecolor(), "5h", 0.42, &meter(0.5, 0.65, false), 60, et);
        let text = text(&line);
        assert!(text.starts_with("5h $0.42/min "), "{text}");
        assert!(text.ends_with(" proj  65% 14:00"), "{text}");
    }

    #[test]
    fn a_narrow_gauge_drops_the_reset_time_then_the_projection() {
        let utc = FixedOffset::east_opt(0).unwrap();
        let at = |w| {
            text(&gauge_line(
                &truecolor(),
                "5h",
                0.42,
                &meter(0.5, 0.65, false),
                w,
                utc,
            ))
        };
        assert!(at(34).ends_with(" proj  65%"), "{}", at(34));
        assert!(at(25).ends_with('\u{2591}') || at(25).ends_with('\u{2588}'));
        assert!(!at(25).contains("proj"));
    }
}
