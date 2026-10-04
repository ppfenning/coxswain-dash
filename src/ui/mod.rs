//! Dispatches rendering to the current page's module. No layout logic lives here; each page
//! module owns its own `render`.

mod chair_card;
pub mod initiative_drill;
pub mod machine_drill;
mod regatta;
pub mod run_drill;
mod slipstream;

use chrono::{DateTime, FixedOffset};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

use crate::app::{App, AppPage, DetailKind};
use crate::detail::DetailSnapshot;
use crate::theme::Theme;

pub use regatta::frame_rects as regatta_frame_rects;

/// Drawn in every frame but the chair panel while the feed has failed before its first snapshot.
const NO_FEED: &str = "no feed yet";

/// Width of Slipstream's right rail. Mirrors `RAIL_WIDTH` in `slipstream.rs`, which is private;
/// the error line is drawn into the rail's chair block, so the two must agree.
const SLIPSTREAM_RAIL_WIDTH: u16 = 32;

pub fn render(f: &mut Frame, app: &App, theme: &Theme) {
    match (app.detail(), app.detail_error()) {
        (Some((kind, id, _)), Some(err)) => render_detail_error(f, *kind, id, err, theme),
        (Some((_, _, Some(DetailSnapshot::Run(detail)))), None) => {
            run_drill::render(f, detail, theme, app.utc_offset())
        }
        (Some((_, _, Some(DetailSnapshot::Initiative(detail)))), None) => {
            initiative_drill::render(f, detail, theme, app.utc_offset())
        }
        (Some((_, _, Some(DetailSnapshot::Machine(detail)))), None) => {
            machine_drill::render(f, detail, theme, app.utc_offset())
        }
        (Some((kind, id, None)), None) => render_loading(f, *kind, id, theme),
        (None, _) => render_page(f, app, theme),
    }
}

/// The page behind any detail view. A feed error with no snapshot yet replaces the waiting
/// text; an error beside a held snapshot is drawn over the finished page.
fn render_page(f: &mut Frame, app: &App, theme: &Theme) {
    match (app.snapshot(), app.feed_error()) {
        (None, Some(err)) => render_feed_failed(f, app, err, theme),
        (_, err) => {
            match app.page() {
                AppPage::Regatta => regatta::render(f, app, theme),
                AppPage::Slipstream => slipstream::render(f, app, theme),
            }
            if let Some(err) = err {
                render_error_line(f, last_row(chair_inner(f.area(), app)), err, theme);
            }
        }
    }
}

/// `text` cut to `width` chars, so a line drawn into a row of that width never wraps.
fn fit_line(text: &str, width: u16) -> String {
    text.chars().take(usize::from(width)).collect()
}

fn feed_error_text(err: &str) -> String {
    format!("feed error: {err}")
}

fn bordered_inner(rect: Rect) -> Rect {
    Block::default().borders(Borders::ALL).inner(rect)
}

fn first_row(rect: Rect) -> Rect {
    Rect {
        height: rect.height.min(1),
        ..rect
    }
}

fn last_row(rect: Rect) -> Rect {
    let height = rect.height.min(1);
    Rect {
        y: rect.y + rect.height - height,
        height,
        ..rect
    }
}

/// Regatta's chair frame, or the first visible frame when frame 1 is hidden.
fn regatta_chair(area: Rect, app: &App) -> Option<Rect> {
    let rects = regatta_frame_rects(area, app);
    rects[0].or_else(|| rects.into_iter().flatten().next())
}

/// The inside of the block the chair is drawn in: Regatta's chair frame (the first visible
/// frame when frame 1 is hidden) or Slipstream's rail chair block.
fn chair_inner(area: Rect, app: &App) -> Rect {
    let rect = match app.page() {
        AppPage::Regatta => regatta_chair(area, app),
        AppPage::Slipstream => {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Min(1),
                    Constraint::Length(SLIPSTREAM_RAIL_WIDTH),
                ])
                .split(area);
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Ratio(1, 4); 4])
                .split(cols[1]);
            Some(rows[3])
        }
    };
    bordered_inner(rect.unwrap_or(area))
}

/// Draws `feed error: <err>` on one row, in the error status color, clearing what was there.
fn render_error_line(f: &mut Frame, row: Rect, err: &str, theme: &Theme) {
    let text = fit_line(&feed_error_text(err), row.width);
    f.render_widget(Clear, row);
    f.render_widget(
        Paragraph::new(text).style(Style::default().fg(theme.status_failed).bg(theme.bg)),
        row,
    );
}

/// The feed failed before any snapshot: the Regatta frame layout, the chair frame carrying the
/// error line and every other frame saying there is no feed. The word "waiting" is never drawn.
fn render_feed_failed(f: &mut Frame, app: &App, err: &str, theme: &Theme) {
    let area = f.area();
    let style = Style::default().fg(theme.fg).bg(theme.bg);
    let chair = regatta_chair(area, app);
    f.render_widget(Block::default().style(style), area);
    for rect in regatta_frame_rects(area, app).into_iter().flatten() {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .style(style);
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        if Some(rect) != chair {
            f.render_widget(Paragraph::new(NO_FEED).style(style), first_row(inner));
        }
    }
    let chair_row = first_row(bordered_inner(chair.unwrap_or(area)));
    render_error_line(f, chair_row, err, theme);
}

/// Drawn in place of a drill body when its detail child failed: the block, and one error row.
fn render_detail_error(f: &mut Frame, kind: DetailKind, id: &str, err: &str, theme: &Theme) {
    let area = f.area();
    let block = Block::default()
        .title(format!("{} {}", kind_label(kind), id))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default().fg(theme.fg).bg(theme.bg));
    let inner = block.inner(area);
    f.render_widget(block, area);
    render_error_line(f, first_row(inner), err, theme);
}

/// The lowercase noun a loading paragraph names a [`DetailKind`] with. `src/main.rs`'s
/// `kind_arg` picks the same three strings for the `cox dash --detail` argument.
fn kind_label(kind: DetailKind) -> &'static str {
    match kind {
        DetailKind::Run => "run",
        DetailKind::Initiative => "initiative",
        DetailKind::Machine => "machine",
    }
}

/// Drawn in place of a drill board while its detail snapshot is still in flight.
fn render_loading(f: &mut Frame, kind: DetailKind, id: &str, theme: &Theme) {
    let area = f.area();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded);
    let paragraph = Paragraph::new(format!("loading {} {}", kind_label(kind), id))
        .block(block)
        .style(Style::default().fg(theme.fg).bg(theme.bg));
    f.render_widget(paragraph, area);
}

/// Formats an RFC 3339 timestamp as `%H:%M` in `offset`. A string that does not parse as
/// RFC 3339 comes back unchanged. Pure: the offset is passed in, never read from the clock.
pub fn local_time(utc: &str, offset: FixedOffset) -> String {
    match DateTime::parse_from_rfc3339(utc) {
        Ok(at) => at.with_timezone(&offset).format("%H:%M").to_string(),
        Err(_) => utc.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_time_returns_unparseable_input_unchanged() {
        let utc = FixedOffset::east_opt(0).unwrap();
        assert_eq!(local_time("not a time", utc), "not a time");
    }

    #[test]
    fn local_time_formats_in_the_given_offset() {
        let et = FixedOffset::west_opt(4 * 3600).unwrap();
        assert_eq!(local_time("2026-09-29T18:00:00Z", et), "14:00");
    }

    use crate::app::ThemeId;
    use crate::detail;
    use ratatui::{Terminal, backend::TestBackend};

    const FEED_FIXTURE: &str = include_str!("../../tests/fixtures/dash_feed_v1.json");
    const RUN_DETAIL_FIXTURE: &str = include_str!("../../tests/fixtures/dash_detail_run_v1.json");

    /// An app on the Regatta page with the fixture's one run selected and its detail opened
    /// (so `app.detail()` is `Some((DetailKind::Run, "dash-feed-1", _))`).
    fn app_with_open_run_detail() -> App {
        let snapshot =
            crate::feed::parse_snapshot(FEED_FIXTURE.trim()).expect("fixture should parse");
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.apply_snapshot(snapshot);
        app.open_detail();
        app
    }

    #[test]
    fn renders_a_loading_paragraph_when_the_detail_snapshot_has_not_arrived_yet() {
        let app = app_with_open_run_detail();
        let theme = crate::theme::resolve(ThemeId::Regatta);
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render(f, &app, &theme))
            .expect("draw should not fail");
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn renders_the_run_drill_once_its_detail_snapshot_has_arrived() {
        let mut app = app_with_open_run_detail();
        let snapshot =
            detail::parse_detail(RUN_DETAIL_FIXTURE.trim()).expect("fixture should parse");
        app.apply_detail_snapshot(snapshot);
        let theme = crate::theme::resolve(ThemeId::Regatta);
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render(f, &app, &theme))
            .expect("draw should not fail");
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn fit_line_cuts_to_the_width_and_leaves_short_text_alone() {
        assert_eq!(fit_line("short", 10), "short");
        assert_eq!(fit_line("abcdefghij", 4), "abcd");
        assert_eq!(fit_line("abc", 0), "");
    }

    #[test]
    fn feed_error_text_prefixes_the_line() {
        assert_eq!(feed_error_text("boom"), "feed error: boom");
    }

    const TRACEBACK: &str = "printf 'Traceback (most recent call last):\\n  File \"x\"\\nValueError: boom\\n' >&2; exit 1";
    const ERROR_LINE: &str = "feed error: ValueError: boom";

    /// An app fed by a stub child run through `read_feed`, as `main` feeds the real one.
    fn app_from_stub(script: &str, page: AppPage) -> App {
        let value: serde_json::Value = serde_json::from_str(FEED_FIXTURE).expect("fixture is json");
        let line = serde_json::to_string(&value).expect("value serializes");
        let mut command = std::process::Command::new("sh");
        command.args(["-c", script]).env("LINE", line);
        let (tx, rx) = std::sync::mpsc::channel();
        crate::feed::read_feed(crate::feed::spawn_command(command), tx);
        rx.iter()
            .fold(App::new(page, ThemeId::Regatta), |mut app, message| {
                match message {
                    crate::feed::FeedMessage::Snapshot(snapshot) => app.apply_snapshot(*snapshot),
                    crate::feed::FeedMessage::Error(text) => app.apply_feed_error(text),
                }
                app
            })
    }

    fn draw(app: &App, theme: &Theme) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("terminal");
        terminal
            .draw(|f| render(f, app, theme))
            .expect("draw should not fail");
        terminal.backend().buffer().clone()
    }

    fn rows(buffer: &ratatui::buffer::Buffer) -> Vec<String> {
        let area = buffer.area;
        (area.y..area.y + area.height)
            .map(|y| {
                (area.x..area.x + area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect()
            })
            .collect()
    }

    fn screen(buffer: &ratatui::buffer::Buffer) -> String {
        rows(buffer).join("\n")
    }

    /// The `(x, y)` of the first cell of `needle`. Every cell here holds one char, so a char
    /// index in a row is its column.
    fn find(buffer: &ratatui::buffer::Buffer, needle: &str) -> Option<(u16, u16)> {
        rows(buffer).iter().enumerate().find_map(|(y, row)| {
            row.find(needle)
                .map(|byte| (row[..byte].chars().count() as u16, y as u16))
        })
    }

    fn assert_drawn_in(buffer: &ratatui::buffer::Buffer, text: &str, color: ratatui::style::Color) {
        let (x, y) = find(buffer, text).unwrap_or_else(|| panic!("{text:?} is not on screen"));
        let len = text.chars().count() as u16;
        assert!((x..x + len).all(|col| buffer[(col, y)].fg == color));
    }

    #[test]
    fn an_error_after_a_good_snapshot_draws_the_page_and_one_error_line() {
        let script = format!("printf '%s\\n' \"$LINE\"; {TRACEBACK}");
        let app = app_from_stub(&script, AppPage::Regatta);
        let theme = crate::theme::resolve(ThemeId::Regatta);
        let buffer = draw(&app, &theme);
        let text = screen(&buffer);
        assert!(text.contains("chair@omarchy:12345 omarchy live"));
        assert_drawn_in(&buffer, ERROR_LINE, theme.status_failed);
        assert_eq!(text.matches("feed error").count(), 1);
        for traceback in ["Traceback", "most recent call", "File \"x\""] {
            assert!(
                !text.contains(traceback),
                "{traceback:?} leaked into a cell"
            );
        }
    }

    #[test]
    fn the_error_line_lands_inside_slipstreams_rail_chair_block() {
        let script = format!("printf '%s\\n' \"$LINE\"; {TRACEBACK}");
        let app = app_from_stub(&script, AppPage::Slipstream);
        let theme = crate::theme::resolve(ThemeId::Regatta);
        let buffer = draw(&app, &theme);
        assert_drawn_in(&buffer, ERROR_LINE, theme.status_failed);
        let (x, _) = find(&buffer, ERROR_LINE).expect("error line is on screen");
        assert!(x >= 120 - SLIPSTREAM_RAIL_WIDTH);
    }

    #[test]
    fn a_later_good_snapshot_removes_the_error_line() {
        let script = format!("printf '%s\\n' \"$LINE\"; {TRACEBACK}");
        let mut app = app_from_stub(&script, AppPage::Regatta);
        let snapshot =
            crate::feed::parse_snapshot(FEED_FIXTURE.trim()).expect("fixture should parse");
        app.apply_snapshot(snapshot);
        let theme = crate::theme::resolve(ThemeId::Regatta);
        assert!(!screen(&draw(&app, &theme)).contains("feed error"));
    }

    #[test]
    fn an_error_before_any_snapshot_fills_the_chair_panel_and_never_says_waiting() {
        let app = app_from_stub(TRACEBACK, AppPage::Regatta);
        assert!(app.snapshot().is_none());
        let theme = crate::theme::resolve(ThemeId::Regatta);
        let buffer = draw(&app, &theme);
        let text = screen(&buffer);
        let chair = regatta_frame_rects(Rect::new(0, 0, 120, 40), &app)[0].expect("chair rect");
        let (x, y) = find(&buffer, ERROR_LINE).expect("error line is on screen");
        assert!(x > chair.x && x < chair.x + chair.width);
        assert!(y > chair.y && y < chair.y + chair.height);
        assert_drawn_in(&buffer, ERROR_LINE, theme.status_failed);
        assert_eq!(text.matches(NO_FEED).count(), 5);
        assert!(!text.to_lowercase().contains("waiting"));
        assert!(!text.contains("Traceback"));
    }

    #[test]
    fn a_detail_error_replaces_the_detail_body_with_one_line() {
        let mut app = app_with_open_run_detail();
        let snapshot =
            detail::parse_detail(RUN_DETAIL_FIXTURE.trim()).expect("fixture should parse");
        app.apply_detail_snapshot(snapshot);
        app.apply_detail_error("ValueError: detail boom".to_owned());
        let theme = crate::theme::resolve(ThemeId::Regatta);
        let buffer = draw(&app, &theme);
        let text = screen(&buffer);
        assert_drawn_in(
            &buffer,
            "feed error: ValueError: detail boom",
            theme.status_failed,
        );
        assert!(!text.contains("test result: ok"));
        assert!(!text.contains("src/feed.rs"));
    }

    #[test]
    fn a_detail_error_replaces_the_loading_paragraph() {
        let mut app = app_with_open_run_detail();
        app.apply_detail_error("ValueError: detail boom".to_owned());
        let theme = crate::theme::resolve(ThemeId::Regatta);
        let text = screen(&draw(&app, &theme));
        assert!(text.contains("feed error: ValueError: detail boom"));
        assert!(!text.contains("loading"));
    }
}
