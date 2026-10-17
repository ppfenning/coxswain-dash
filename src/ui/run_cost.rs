//! Pure chart helpers for the run cost frame. Plain data in, numbers and indexes out: time is
//! epoch seconds as `f64`, cost is dollars as `f64`. The chart helpers take no feed types, no
//! theme, no widget and no clock. The spend split panel at the end of the module is the exception:
//! a pure formatter over the feed's `SpendSplit` and a passed-in `now`, and a thin draw function.
#![allow(dead_code)]

use chrono::{DateTime, Utc};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::Line,
    widgets::{Paragraph, Widget},
};

use crate::feed::{SpendSplit, SpendWindow};

/// Axis extent for the cost chart. The y axis always starts at 0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub x_min: f64,
    pub x_max: f64,
    pub y_max: f64,
}

/// Rounds up to the next of 1, 2, 2.5, 5 or 10 times a power of ten; non-positive input gives 1.
fn round_up_step(y: f64) -> f64 {
    if y <= 0.0 {
        1.0
    } else {
        let magnitude = 10f64.powi(y.log10().floor() as i32);
        [1.0, 2.0, 2.5, 5.0, 10.0]
            .iter()
            .map(|f| f * magnitude)
            .find(|top| y <= *top)
            .unwrap_or(10.0 * magnitude)
    }
}

/// Bounds covering every point of every series and of spend; None when all are empty.
pub fn chart_bounds(series: &[Vec<(f64, f64)>], spend: &[(f64, f64)]) -> Option<Bounds> {
    let points = || series.iter().flatten().chain(spend.iter());
    points().next()?;
    let (x_min, x_max, y_top) = points().fold(
        (f64::INFINITY, f64::NEG_INFINITY, 0.0_f64),
        |(lo, hi, top), (x, y)| (lo.min(*x), hi.max(*x), top.max(*y)),
    );
    Some(Bounds {
        x_min,
        x_max: if x_max > x_min { x_max } else { x_min + 1.0 },
        y_max: round_up_step(y_top),
    })
}

/// `count` evenly spaced labels from $0 to `y_max`; a whole value drops its decimals, any other gets two.
pub fn dollar_labels(y_max: f64, count: usize) -> Vec<String> {
    let label = |v: f64| {
        if (v - v.round()).abs() < 1e-9 {
            format!("${}", v.round())
        } else {
            format!("${v:.2}")
        }
    };
    match count {
        0 => Vec::new(),
        1 => vec![label(0.0)],
        n => (0..n)
            .map(|i| label(y_max * i as f64 / (n - 1) as f64))
            .collect(),
    }
}

/// Indexes of the series that have at least one point.
pub fn drawable(series: &[Vec<(f64, f64)>]) -> Vec<usize> {
    series
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.is_empty())
        .map(|(i, _)| i)
        .collect()
}

/// Time from `now` to an RFC 3339 `resets_at`, as "resets in 2h 15m" or "resets in 1d 12h" from
/// a day up. At or past the reset it reads "resets now"; an unparseable string reads "reset unknown".
fn time_remaining(resets_at: &str, now: DateTime<Utc>) -> String {
    match DateTime::parse_from_rfc3339(resets_at) {
        Err(_) => "reset unknown".to_string(),
        Ok(at) => {
            let secs = (at.with_timezone(&Utc) - now).num_seconds();
            let (days, hours, mins) = (secs / 86_400, secs % 86_400 / 3_600, secs % 3_600 / 60);
            match (secs, days, hours) {
                (s, _, _) if s <= 0 => "resets now".to_string(),
                (_, 0, 0) => format!("resets in {mins}m"),
                (_, 0, h) => format!("resets in {h}h {mins}m"),
                (_, d, h) => format!("resets in {d}d {h}h"),
            }
        }
    }
}

/// A fraction of one as a whole percent, rounded.
fn percent(fraction: f64) -> String {
    format!("{}%", (fraction * 100.0).round() as i64)
}

pub(crate) fn window_lines(name: &str, w: &SpendWindow, now: DateTime<Utc>) -> Vec<String> {
    let row = |label: &str, usd: f64, share: f64| {
        format!(
            "  {label:<14}{:>9}{:>6}",
            format!("${usd:.2}"),
            percent(share)
        )
    };
    vec![
        format!(
            "{name}  meter {}  {}",
            percent(w.meter_fraction),
            time_remaining(&w.resets_at, now)
        ),
        row("chair session", w.chair_session_usd, w.chair_session_share),
        row("chair agents", w.chair_agents_usd, w.chair_agents_share),
        row("cox seats", w.cox_seats_usd, w.cox_seats_share),
    ]
}

/// The spend split panel's text, one string per line: both windows, or a placeholder when absent.
pub fn spend_split_lines(split: Option<&SpendSplit>, now: DateTime<Utc>) -> Vec<String> {
    match split {
        None => vec!["no spend split".to_string()],
        Some(s) => [
            window_lines("five-hour", &s.five_hour, now),
            window_lines("weekly", &s.weekly, now),
        ]
        .concat(),
    }
}

/// Draws the spend split panel unstyled into `area`.
pub fn draw_spend_split(
    buf: &mut Buffer,
    area: Rect,
    split: Option<&SpendSplit>,
    now: DateTime<Utc>,
) {
    let lines: Vec<Line> = spend_split_lines(split, now)
        .into_iter()
        .map(Line::from)
        .collect();
    Paragraph::new(lines).render(area, buf);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(usd: [f64; 3], share: [f64; 3], meter: f64, resets_at: &str) -> SpendWindow {
        SpendWindow {
            chair_session_usd: usd[0],
            chair_agents_usd: usd[1],
            cox_seats_usd: usd[2],
            total_usd: usd.iter().sum(),
            chair_session_share: share[0],
            chair_agents_share: share[1],
            cox_seats_share: share[2],
            meter_fraction: meter,
            resets_at: resets_at.to_string(),
            by_node: Vec::new(),
            cost_per_landed_task: None,
        }
    }

    fn fixture() -> SpendSplit {
        SpendSplit {
            five_hour: window(
                [12.34, 8.0, 4.66],
                [0.49, 0.32, 0.19],
                0.37,
                "2026-10-16T22:15:00Z",
            ),
            weekly: window(
                [51.0, 30.0, 19.0],
                [0.51, 0.30, 0.19],
                0.62,
                "2026-10-18T08:00:00Z",
            ),
        }
    }

    fn now() -> DateTime<Utc> {
        "2026-10-16T20:00:00Z".parse().unwrap()
    }

    fn rendered(split: Option<&SpendSplit>) -> String {
        let area = Rect::new(0, 0, 60, 10);
        let mut buf = Buffer::empty(area);
        draw_spend_split(&mut buf, area, split, now());
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_five_hour_window_renders_dollars_shares_meter_and_reset() {
        let text = rendered(Some(&fixture()));
        let five: Vec<&str> = text.lines().take(4).collect();
        assert!(five[0].contains("five-hour"), "{}", five[0]);
        assert!(five[0].contains("meter 37%"), "{}", five[0]);
        assert!(five[0].contains("resets in 2h 15m"), "{}", five[0]);
        assert!(
            five[1].contains("chair session")
                && five[1].contains("$12.34")
                && five[1].contains("49%")
        );
        assert!(
            five[2].contains("chair agents")
                && five[2].contains("$8.00")
                && five[2].contains("32%")
        );
        assert!(
            five[3].contains("cox seats") && five[3].contains("$4.66") && five[3].contains("19%")
        );
    }

    #[test]
    fn the_weekly_window_renders_dollars_shares_meter_and_reset() {
        let text = rendered(Some(&fixture()));
        let weekly: Vec<&str> = text.lines().skip(4).take(4).collect();
        assert!(weekly[0].contains("weekly"), "{}", weekly[0]);
        assert!(weekly[0].contains("meter 62%"), "{}", weekly[0]);
        assert!(weekly[0].contains("resets in 1d 12h"), "{}", weekly[0]);
        assert!(
            weekly[1].contains("chair session")
                && weekly[1].contains("$51.00")
                && weekly[1].contains("51%")
        );
        assert!(
            weekly[2].contains("chair agents")
                && weekly[2].contains("$30.00")
                && weekly[2].contains("30%")
        );
        assert!(
            weekly[3].contains("cox seats")
                && weekly[3].contains("$19.00")
                && weekly[3].contains("19%")
        );
    }

    #[test]
    fn an_absent_split_renders_the_placeholder() {
        let text = rendered(None);
        assert!(text.contains("no spend split"), "{text}");
        assert!(!text.contains("five-hour"), "{text}");
    }

    #[test]
    fn time_remaining_covers_minutes_hours_days_past_and_garbage() {
        let n = now();
        assert_eq!(time_remaining("2026-10-16T20:12:00Z", n), "resets in 12m");
        assert_eq!(
            time_remaining("2026-10-16T22:15:00Z", n),
            "resets in 2h 15m"
        );
        assert_eq!(time_remaining("2026-10-17T21:00:00Z", n), "resets in 1d 1h");
        assert_eq!(time_remaining("2026-10-16T19:00:00Z", n), "resets now");
        assert_eq!(time_remaining("soon", n), "reset unknown");
    }

    #[test]
    fn bounds_cover_two_series_and_spend() {
        let series = vec![
            vec![(10.0, 0.5), (20.0, 1.5)],
            vec![(5.0, 0.2), (15.0, 3.0)],
        ];
        let spend = [(30.0, 4.0)];
        assert_eq!(
            chart_bounds(&series, &spend),
            Some(Bounds {
                x_min: 5.0,
                x_max: 30.0,
                y_max: 5.0
            })
        );
    }

    #[test]
    fn bounds_are_none_when_everything_is_empty() {
        assert_eq!(chart_bounds(&[vec![], vec![]], &[]), None);
        assert_eq!(chart_bounds(&[], &[]), None);
    }

    #[test]
    fn a_single_point_series_widens_x() {
        let b = chart_bounds(&[vec![(100.0, 1.0)]], &[]).unwrap();
        assert_eq!((b.x_min, b.x_max), (100.0, 101.0));
    }

    #[test]
    fn y_max_stays_on_a_step_boundary() {
        assert_eq!(chart_bounds(&[vec![(0.0, 5.0)]], &[]).unwrap().y_max, 5.0);
    }

    #[test]
    fn y_max_rounds_up_just_above_a_step_boundary() {
        assert_eq!(chart_bounds(&[vec![(0.0, 5.01)]], &[]).unwrap().y_max, 10.0);
    }

    #[test]
    fn labels_use_whole_dollars_from_one_up() {
        assert_eq!(dollar_labels(5.0, 3), ["$0", "$2.50", "$5"]);
    }

    #[test]
    fn labels_use_cents_below_one_dollar() {
        assert_eq!(dollar_labels(0.5, 3), ["$0", "$0.25", "$0.50"]);
    }

    #[test]
    fn drawable_skips_an_empty_series() {
        let series = vec![vec![(0.0, 1.0)], vec![], vec![(0.0, 2.0), (1.0, 3.0)]];
        assert_eq!(drawable(&series), vec![0, 2]);
    }
}
