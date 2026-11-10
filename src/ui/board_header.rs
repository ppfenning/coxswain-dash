//! The board's header: one line holding the chair card and the status line. The chair label,
//! its orchestration mode, the last tick and the next wake sit on the left. The current status
//! message sits at the right in its level colour. The feed-health line stays on the footer row.

use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use super::chair_card::{Freshness, TICK_INTERVAL_S, beat_freshness, last_tick_line, short_age};
use super::fit_line;
use super::status::{self, Status};
use crate::app::App;
use crate::exec::StatusLevel;
use crate::feed::Chair;
use crate::theme::Theme;

/// Blank columns kept between the chair text and the message.
const GAP: usize = 2;
const SEPARATOR: &str = " \u{b7} ";

/// The feed has no wake field: only a fresh beat and a tick under one interval old give a `~` estimate.
fn next_wake_text(tick_age_s: Option<u64>, beat_age_s: u64) -> String {
    match (tick_age_s, beat_freshness(beat_age_s, TICK_INTERVAL_S)) {
        (Some(age), Freshness::Fresh) if age < TICK_INTERVAL_S => {
            format!("next ~{}", short_age(TICK_INTERVAL_S - age))
        }
        _ => "next -".to_string(),
    }
}

/// `chair <host>`, or `chair` when the feed names no host.
fn chair_label(host: &str) -> String {
    format!("chair {}", host.trim()).trim_end().to_string()
}

/// The left side of the line, in order: label, mode when known, last tick, next wake.
fn header_text(chair: &Chair) -> String {
    let tick = match (chair.last_tick_at.as_deref(), chair.tick_age_s) {
        (Some(at), Some(age)) => last_tick_line(at, age),
        _ => "last tick -".to_string(),
    };
    [
        Some(chair_label(&chair.host)),
        chair.orchestration().map(|(mode, _)| mode.to_string()),
        Some(tick),
        Some(next_wake_text(chair.tick_age_s, chair.beat_age_s)),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(SEPARATOR)
}

/// `text` cut to `width` chars, the last of them `…` when anything was cut.
fn clip(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        text.to_string()
    } else {
        fit_line(
            text,
            u16::try_from(width.saturating_sub(1)).unwrap_or(u16::MAX),
        ) + "\u{2026}"
    }
}

/// The chair text padded to push the message to the right edge, and the message. The message
/// keeps its columns; the chair text gives way first.
fn compose(chair: &str, message: Option<&str>, width: usize) -> (String, String) {
    let message = message.map(|m| clip(m, width)).unwrap_or_default();
    let used = message.chars().count();
    let room = if used == 0 {
        width
    } else {
        width.saturating_sub(used + GAP)
    };
    let left = clip(chair, room);
    let pad = width.saturating_sub(left.chars().count() + used);
    (format!("{left}{}", " ".repeat(pad)), message)
}

pub fn render(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let base = Style::default().fg(theme.fg).bg(theme.bg);
    let message = app.status();
    let chair = app.snapshot().map(|s| header_text(&s.chair));
    let (left, right) = compose(
        chair.as_deref().unwrap_or(""),
        message.map(|(_, text)| text.as_str()),
        usize::from(area.width),
    );
    let color = message.map(|(level, _)| {
        status::color(
            theme,
            match level {
                StatusLevel::Ok => Status::Up,
                StatusLevel::Failed => Status::Failed,
            },
        )
    });
    let line = Line::from(vec![
        Span::raw(left),
        Span::styled(right, color.map_or(base, |c| base.fg(c))),
    ]);
    f.render_widget(Paragraph::new(line).style(base), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppPage, Origin, ThemeId};
    use crate::exec::ExecResult;
    use ratatui::{Terminal, backend::TestBackend};

    const FEED_FIXTURE: &str = include_str!("../../tests/fixtures/dash_feed_v1.json");

    fn app_with_chair(chair: serde_json::Value) -> App {
        let mut value: serde_json::Value =
            serde_json::from_str(FEED_FIXTURE).expect("fixture is json");
        value["chair"] = chair;
        let snapshot = crate::feed::parse_snapshot(&value.to_string()).expect("feed parses");
        let mut app = App::new(AppPage::Board, ThemeId::Regatta);
        app.apply_snapshot(snapshot);
        app
    }

    fn literal_chair() -> serde_json::Value {
        serde_json::json!({
            "holder": "chair@host-b:12345",
            "host": "host-b",
            "epoch": 7,
            "liveness": "live",
            "beat_age_s": 4,
            "last_tick_at": "09-28 19:59 EDT",
            "tick_age_s": 15,
            "orchestration_mode": "orchestrating",
            "orchestration_toggle_available": true,
        })
    }

    fn drawn(app: &App, width: u16) -> ratatui::buffer::Buffer {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let mut terminal = Terminal::new(TestBackend::new(width, 1)).expect("terminal");
        terminal
            .draw(|f| render(f, f.area(), app, &theme))
            .expect("draw should not fail");
        terminal.backend().buffer().clone()
    }

    fn row(buffer: &ratatui::buffer::Buffer) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, 0)].symbol())
            .collect()
    }

    fn failed_result() -> ExecResult {
        ExecResult {
            argv: vec!["cox".to_string()],
            code: Some(1),
            output: "boom\n".to_string(),
        }
    }

    #[test]
    fn chair_label_mode_tick_and_next_wake() {
        let buffer = drawn(&app_with_chair(literal_chair()), 100);
        insta::assert_snapshot!(row(&buffer).trim_end());
    }

    #[test]
    fn next_wake_counts_down_from_one_tick_interval() {
        assert_eq!(next_wake_text(Some(15), 4), "next ~45s");
        assert_eq!(next_wake_text(Some(60), 4), "next -");
        assert_eq!(next_wake_text(None, 4), "next -");
        assert_eq!(next_wake_text(Some(15), 121), "next -");
    }

    #[test]
    fn an_unknown_mode_and_a_missing_tick_are_left_out_or_dashed() {
        let chair = serde_json::json!({
            "holder": "h", "host": "host-b", "epoch": 1, "liveness": "live",
            "beat_age_s": 1, "orchestration_mode": "mystery",
        });
        let app = app_with_chair(chair);
        let text = header_text(&app.snapshot().expect("snapshot").chair);
        assert_eq!(text, "chair host-b \u{b7} last tick - \u{b7} next -");
    }

    #[test]
    fn a_failed_status_is_drawn_at_the_right_in_the_failed_colour() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let mut app = app_with_chair(literal_chair());
        app.apply_exec_result(failed_result(), Origin::Action);
        let buffer = drawn(&app, 100);
        let text = row(&buffer);
        let start = text.find("boom").expect("the message is on the line");
        let start = u16::try_from(text[..start].chars().count()).expect("column");
        assert!((start..start + 4).all(|x| buffer[(x, 0)].fg == theme.status_failed));
        assert!(text.trim_end().ends_with("boom"), "{text}");
        assert!(text.starts_with("chair host-b"), "{text}");
    }

    #[test]
    fn the_message_keeps_its_columns_when_the_line_is_narrow() {
        let (left, right) = compose("chair host-b \u{b7} last tick -", Some("boom"), 16);
        assert_eq!(right, "boom");
        assert_eq!(left.chars().count() + right.chars().count(), 16);
        assert!(left.starts_with("chair hos\u{2026}"), "{left:?}");
    }
}
