//! Draws the colon palette: a one-line prompt in input mode, and in frame mode a bordered `cox`
//! frame holding the command's output from the scroll offset.

use ratatui::{
    Frame,
    layout::{Position, Rect},
    style::Style,
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::palette::PaletteState;
use crate::theme::Theme;

const HINT: &str = "Up Down scroll    Esc close";

/// `text` cut to `width` chars, with no ellipsis.
fn clip_line(text: &str, width: usize) -> String {
    text.chars().take(width).collect()
}

/// `rows` lines of `output` from line `scroll`, each clipped to `width`.
fn visible_lines(output: &[String], scroll: usize, rows: usize, width: usize) -> Vec<String> {
    output
        .iter()
        .skip(scroll)
        .take(rows)
        .map(|line| clip_line(line, width))
        .collect()
}

/// Draws the palette in `area`: the prompt on its bottom row, or the output frame above it.
/// Both clear what they cover first, so nothing of the page underneath shows through.
pub fn render(frame: &mut Frame, area: Rect, theme: &Theme, state: &PaletteState) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    if state.in_frame_mode() {
        render_frame(frame, area, theme, state);
    } else {
        render_prompt(frame, area, theme, state);
    }
}

fn render_prompt(frame: &mut Frame, area: Rect, theme: &Theme, state: &PaletteState) {
    let row = Rect {
        y: area.bottom() - 1,
        height: 1,
        ..area
    };
    let width = usize::from(row.width);
    let text = clip_line(&format!(":{}", state.input()), width);
    frame.render_widget(Clear, row);
    frame.render_widget(
        Paragraph::new(text).style(Style::default().fg(theme.fg).bg(theme.bg)),
        row,
    );
    let offset = (1 + state.cursor()).min(width - 1);
    frame.set_cursor_position(Position::new(
        row.x + u16::try_from(offset).unwrap_or(0),
        row.y,
    ));
}

fn render_frame(frame: &mut Frame, area: Rect, theme: &Theme, state: &PaletteState) {
    let rect = Rect {
        x: area.x.saturating_add(1),
        y: area.y,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(1),
    };
    let base = Style::default().fg(theme.fg).bg(theme.bg);
    let block = Block::default()
        .borders(Borders::ALL)
        .title("cox")
        .title_style(Style::default().fg(theme.border_focus))
        .border_style(Style::default().fg(theme.border_focus))
        .style(base);
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);
    if inner.width > 0 && inner.height > 0 {
        render_body(frame, inner, theme, state);
    }
}

/// The output lines above a dim hint row, inside the frame.
fn render_body(frame: &mut Frame, inner: Rect, theme: &Theme, state: &PaletteState) {
    let base = Style::default().fg(theme.fg).bg(theme.bg);
    let rows = inner.height - 1;
    let width = usize::from(inner.width);
    let lines = visible_lines(
        state.output().unwrap_or(&[]),
        state.scroll(),
        usize::from(rows),
        width,
    );
    frame.render_widget(
        Paragraph::new(lines.into_iter().map(Line::from).collect::<Vec<_>>()).style(base),
        Rect {
            height: rows,
            ..inner
        },
    );
    frame.render_widget(
        Paragraph::new(clip_line(HINT, width)).style(base.fg(theme.dim)),
        Rect {
            y: inner.bottom() - 1,
            height: 1,
            ..inner
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, AppPage, ThemeId};
    use crossterm::event::KeyCode;
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

    const FEED_FIXTURE: &str = include_str!("../../tests/fixtures/dash_feed_v1.json");

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    fn with_output(items: &[&str]) -> PaletteState {
        let mut state = PaletteState::open();
        state.set_output(lines(items));
        state
    }

    fn sample_output() -> PaletteState {
        with_output(&[
            "alpha",
            "bravo",
            "charlie",
            "delta",
            "echo",
            "foxtrot",
            "a line that runs well past the right edge of the frame",
        ])
    }

    fn draw(theme_id: ThemeId, state: &PaletteState) -> Terminal<TestBackend> {
        let theme = crate::theme::resolve_for(theme_id, Some("truecolor"));
        let mut terminal = Terminal::new(TestBackend::new(40, 10)).expect("terminal");
        terminal
            .draw(|f| render(f, f.area(), &theme, state))
            .expect("draw should not fail");
        terminal
    }

    fn row_text(buf: &Buffer, x0: u16, x1: u16, y: u16) -> String {
        (x0..x1).map(|x| buf[(x, y)].symbol()).collect()
    }

    fn padded(text: &str, width: usize) -> String {
        format!("{text:<width$}")
    }

    #[test]
    fn clip_line_cuts_to_the_width_without_an_ellipsis() {
        assert_eq!(clip_line("abcdef", 4), "abcd");
    }

    #[test]
    fn clip_line_keeps_a_short_line_whole() {
        assert_eq!(clip_line("abc", 4), "abc");
    }

    #[test]
    fn visible_lines_skips_the_scroll_and_takes_the_rows() {
        let out = lines(&["one", "two", "three", "four", "five"]);
        assert_eq!(visible_lines(&out, 2, 2, 10), lines(&["three", "four"]));
    }

    #[test]
    fn visible_lines_clips_each_line_and_survives_a_scroll_past_the_end() {
        let out = lines(&["abcdef"]);
        assert_eq!(visible_lines(&out, 0, 3, 3), lines(&["abc"]));
        assert!(visible_lines(&out, 9, 3, 3).is_empty());
    }

    #[test]
    fn renders_the_prompt_with_typed_text_in_the_regatta_theme() {
        let state = PaletteState::prefilled("runs --json");
        insta::assert_snapshot!(draw(ThemeId::Regatta, &state).backend().to_string());
    }

    #[test]
    fn renders_the_output_frame_in_the_regatta_theme() {
        let state = sample_output();
        insta::assert_snapshot!(draw(ThemeId::Regatta, &state).backend().to_string());
    }

    #[test]
    fn renders_the_output_frame_scrolled_by_two_in_the_regatta_theme() {
        let mut state = sample_output();
        state.handle_key(KeyCode::Down);
        state.handle_key(KeyCode::Down);
        insta::assert_snapshot!(draw(ThemeId::Regatta, &state).backend().to_string());
    }

    #[test]
    fn renders_the_output_frame_in_the_harbor_light_theme() {
        let state = sample_output();
        insta::assert_snapshot!(draw(ThemeId::HarborLight, &state).backend().to_string());
    }

    #[test]
    fn the_hint_row_is_dim_and_the_title_takes_the_focus_color() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let terminal = draw(ThemeId::Regatta, &sample_output());
        let buf = terminal.backend().buffer();
        assert_eq!(buf[(2, 7)].symbol(), "U");
        assert_eq!(buf[(2, 7)].fg, theme.dim);
        assert_eq!(buf[(2, 0)].symbol(), "c");
        assert_eq!(buf[(2, 0)].fg, theme.border_focus);
    }

    #[test]
    fn a_zero_height_area_draws_nothing_and_does_not_panic() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let mut terminal = Terminal::new(TestBackend::new(20, 3)).expect("terminal");
        let empty = Rect::new(0, 0, 20, 0);
        terminal
            .draw(|f| {
                render(f, empty, &theme, &PaletteState::open());
                render(f, empty, &theme, &sample_output());
            })
            .expect("draw should not fail");
        assert_eq!(
            row_text(terminal.backend().buffer(), 0, 20, 0),
            " ".repeat(20)
        );
    }

    /// The Regatta page drawn full-size, then the palette over its middle rows, in one draw.
    fn page_with_palette(state: &PaletteState, area: Rect) -> Terminal<TestBackend> {
        let snapshot =
            crate::feed::parse_snapshot(FEED_FIXTURE.trim()).expect("fixture should parse");
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.apply_snapshot(snapshot);
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).expect("terminal");
        terminal
            .draw(|f| {
                crate::ui::render(f, &app, &theme);
                render(f, area, &theme, state);
            })
            .expect("draw should not fail");
        terminal
    }

    #[test]
    fn the_prompt_row_shows_no_page_glyph_after_the_typed_text() {
        let area = Rect::new(0, 10, 120, 20);
        let terminal = page_with_palette(&PaletteState::prefilled("runs"), area);
        let buf = terminal.backend().buffer();
        assert_eq!(row_text(buf, 0, 120, 29), padded(":runs", 120));
    }

    #[test]
    fn the_output_frame_shows_no_page_glyph_inside_its_margin() {
        let area = Rect::new(0, 10, 120, 20);
        let state = with_output(&["one", "two"]);
        let terminal = page_with_palette(&state, area);
        let buf = terminal.backend().buffer();
        assert_eq!(buf[(1, 10)].symbol(), "┌");
        assert_eq!(buf[(118, 28)].symbol(), "┘");
        let width = 116;
        let expected = [
            padded("one", width),
            padded("two", width),
            padded("", width),
        ];
        (11..28).for_each(|y| {
            let want = match usize::from(y - 11) {
                n if n < expected.len() => expected[n].clone(),
                16 => padded(HINT, width),
                _ => padded("", width),
            };
            assert_eq!(row_text(buf, 2, 118, y), want, "row {y}");
        });
    }
}
