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

use crate::feed::{
    OperatorAgent, OperatorSession, OperatorSessions, SpendSources, SpendSplit, SpendWindow,
};

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
        row("sessions", w.chair_session_usd, w.chair_session_share),
        row("agents", w.chair_agents_usd, w.chair_agents_share),
        row("seats", w.cox_seats_usd, w.cox_seats_share),
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

/// Which meter window the session table lists.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SessionWindow {
    FiveHour,
    Weekly,
}

/// `text` cut to `max` characters, the last one an ellipsis when anything was dropped.
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else if max == 0 {
        String::new()
    } else {
        format!("{}…", text.chars().take(max - 1).collect::<String>())
    }
}

/// The sessions `window` lists.
pub(crate) fn window_sessions(
    sessions: &OperatorSessions,
    window: SessionWindow,
) -> &[OperatorSession] {
    match window {
        SessionWindow::FiveHour => &sessions.five_hour,
        SessionWindow::Weekly => &sessions.weekly,
    }
}

/// Lines the whole frame wants: the split, plus the full table when there is a split.
pub(crate) fn spend_frame_height(
    split: Option<&SpendSplit>,
    sessions: &OperatorSessions,
    window: SessionWindow,
    now: DateTime<Utc>,
) -> usize {
    let table = window_sessions(sessions, window)
        .iter()
        .map(|s| 1 + s.agents.len())
        .sum::<usize>()
        .max(1);
    spend_split_lines(split, now).len() + if split.is_some() { table } else { 0 }
}

/// One session: the first eight characters of its id, start as month-day hour:minute (UTC, as
/// sent; the raw text when it does not parse), model, cost and agent count.
pub(crate) fn session_row(s: &OperatorSession) -> String {
    let when = DateTime::parse_from_rfc3339(&s.started_at)
        .map(|t| t.format("%m-%d %H:%M").to_string())
        .unwrap_or_else(|_| s.started_at.clone());
    let id: String = s.session_id.chars().take(8).collect();
    let noun = if s.agent_count == 1 {
        "agent"
    } else {
        "agents"
    };
    format!(
        "{id} {when} {} ${:.2} {} {noun}",
        s.model, s.cost_usd, s.agent_count
    )
}

/// One agent under its session, indented: type, description cut to leave the frame `width`, cost.
pub(crate) fn agent_line(a: &OperatorAgent, width: usize) -> String {
    let head = format!("    {}  ", a.agent_type);
    let tail = format!("  ${:.2}", a.cost_usd);
    let room = width.saturating_sub(head.chars().count() + tail.chars().count());
    format!("{head}{}{tail}", truncate(&a.description, room))
}

/// The table under the split: a row per session in feed order, each followed by its agents.
/// When the lines do not all fit `max_lines`, the ones that fit are kept and the last line counts
/// those left out. No room at all gives nothing.
/// No sessions gives one line with the spend sources' counts.
pub(crate) fn session_lines(
    sessions: &[OperatorSession],
    sources: &SpendSources,
    width: usize,
    max_lines: usize,
) -> Vec<String> {
    if max_lines == 0 {
        return Vec::new();
    }
    if sessions.is_empty() {
        let priced = if sources.sessions_priced.is_empty() {
            "sessions unpriced".to_string()
        } else {
            format!("priced {}", sources.sessions_priced)
        };
        return vec![format!(
            "none found  found {}  {priced}",
            sources.sessions_found
        )];
    }
    let lines: Vec<String> = sessions
        .iter()
        .flat_map(|s| {
            std::iter::once(session_row(s)).chain(s.agents.iter().map(|a| agent_line(a, width)))
        })
        .collect();
    if lines.len() <= max_lines {
        return lines;
    }
    let shown = max_lines - 1;
    lines
        .iter()
        .take(shown)
        .cloned()
        .chain(std::iter::once(format!(
            "{} more lines",
            lines.len() - shown
        )))
        .collect()
}

/// The whole frame text: the split, then the session table for `window` in the rows left under
/// it. An absent split is the placeholder alone.
pub(crate) fn spend_frame_lines(
    split: Option<&SpendSplit>,
    sessions: &OperatorSessions,
    sources: &SpendSources,
    window: SessionWindow,
    area: Rect,
    now: DateTime<Utc>,
) -> Vec<String> {
    let head = spend_split_lines(split, now);
    if split.is_none() {
        return head;
    }
    let listed = window_sessions(sessions, window);
    let room = usize::from(area.height).saturating_sub(head.len());
    let table = session_lines(listed, sources, usize::from(area.width), room);
    [head, table].concat()
}

/// Draws the spend split panel unstyled into `area`.
pub fn draw_spend_split(
    buf: &mut Buffer,
    area: Rect,
    split: Option<&SpendSplit>,
    sessions: &OperatorSessions,
    sources: &SpendSources,
    window: SessionWindow,
    now: DateTime<Utc>,
) {
    let lines: Vec<Line> = spend_frame_lines(split, sessions, sources, window, area, now)
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
        draw_spend_split(
            &mut buf,
            area,
            split,
            &OperatorSessions::default(),
            &SpendSources::default(),
            SessionWindow::Weekly,
            now(),
        );
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
            five[1].contains("sessions") && five[1].contains("$12.34") && five[1].contains("49%")
        );
        assert!(five[2].contains("agents") && five[2].contains("$8.00") && five[2].contains("32%"));
        assert!(five[3].contains("seats") && five[3].contains("$4.66") && five[3].contains("19%"));
    }

    #[test]
    fn the_weekly_window_renders_dollars_shares_meter_and_reset() {
        let text = rendered(Some(&fixture()));
        let weekly: Vec<&str> = text.lines().skip(4).take(4).collect();
        assert!(weekly[0].contains("weekly"), "{}", weekly[0]);
        assert!(weekly[0].contains("meter 62%"), "{}", weekly[0]);
        assert!(weekly[0].contains("resets in 1d 12h"), "{}", weekly[0]);
        assert!(
            weekly[1].contains("sessions")
                && weekly[1].contains("$51.00")
                && weekly[1].contains("51%")
        );
        assert!(
            weekly[2].contains("agents")
                && weekly[2].contains("$30.00")
                && weekly[2].contains("30%")
        );
        assert!(
            weekly[3].contains("seats")
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

    fn agent(kind: &str, description: &str, cost: f64) -> OperatorAgent {
        OperatorAgent {
            agent_type: kind.to_string(),
            description: description.to_string(),
            cost_usd: cost,
        }
    }

    fn session(id: &str, agents: Vec<OperatorAgent>) -> OperatorSession {
        OperatorSession {
            session_id: id.to_string(),
            started_at: "2026-11-05T14:02:11Z".to_string(),
            model: "claude-sonnet-5-5".to_string(),
            cost_usd: 30.0,
            agent_count: agents.len() as u64,
            agents,
        }
    }

    #[test]
    fn a_session_row_has_short_id_start_model_cost_and_agent_count() {
        let s = session("c81e4d57-92a0", vec![agent("Explore", "x", 1.0)]);
        assert_eq!(
            session_row(&s),
            "c81e4d57 11-05 14:02 claude-sonnet-5-5 $30.00 1 agent"
        );
    }

    #[test]
    fn an_agent_line_is_indented_and_cuts_the_description_to_the_width() {
        let a = agent("Explore", "Find the spend split callers", 0.18);
        assert_eq!(
            agent_line(&a, 60),
            "    Explore  Find the spend split callers  $0.18"
        );
        assert_eq!(agent_line(&a, 34), "    Explore  Find the spen…  $0.18");
    }

    #[test]
    fn a_table_taller_than_the_frame_ends_with_a_count_of_the_rest() {
        let two = || vec![agent("Explore", "a", 1.0), agent("Explore", "b", 2.0)];
        let sessions = vec![session("aaaaaaaa", two()), session("bbbbbbbb", two())];
        let none = SpendSources::default();
        let lines = session_lines(&sessions, &none, 60, 4);
        assert_eq!(lines.len(), 4);
        assert!(lines[0].starts_with("aaaaaaaa"), "{lines:?}");
        assert_eq!(lines[3], "3 more lines");
        assert_eq!(session_lines(&sessions, &none, 60, 6).len(), 6);
        assert_eq!(session_lines(&sessions, &none, 60, 1), ["6 more lines"]);
        let tiny = session_lines(&sessions, &none, 60, 2);
        assert_eq!(tiny.len(), 2);
        assert!(tiny[0].starts_with("aaaaaaaa"), "{tiny:?}");
        assert_eq!(tiny[1], "5 more lines");
        assert!(session_lines(&sessions, &none, 60, 0).is_empty());
        assert!(session_lines(&[], &none, 60, 0).is_empty());
    }

    #[test]
    fn no_sessions_says_none_found_with_the_source_counts() {
        let priced = SpendSources {
            sessions_found: 2,
            sessions_priced: "ok".to_string(),
            ..SpendSources::default()
        };
        assert_eq!(
            session_lines(&[], &priced, 60, 5),
            ["none found  found 2  priced ok"]
        );
        assert_eq!(
            session_lines(&[], &SpendSources::default(), 60, 5),
            ["none found  found 0  sessions unpriced"]
        );
    }
}
