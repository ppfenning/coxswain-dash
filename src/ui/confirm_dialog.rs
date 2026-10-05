//! Draws the confirm dialog over the current page: a centered box with the title, the exact
//! `$ <command>` line (hard-wrapped, never cut) and a `y run    n cancel` hint.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Padding, Paragraph},
};

use crate::confirm::ConfirmState;
use crate::theme::Theme;

/// Preferred box width in columns; the box shrinks to the area when the area is narrower.
const BOX_WIDTH: u16 = 60;
/// Columns the border and the one-column padding take from each side of the box.
const SIDE: u16 = 2;
/// Width of the `$ ` prefix, and of the indent that aligns continuation rows under it.
const PREFIX: u16 = 2;

/// Centers a `want_width` by `content_rows + 2` box in `area`, clamped to `area` on both axes.
fn box_rect(area: Rect, content_rows: u16, want_width: u16) -> Rect {
    let width = want_width.min(area.width);
    let height = content_rows.saturating_add(2).min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

/// Splits `command` into chunks of at most `width` characters. Nothing is trimmed or dropped,
/// so the chunks concatenate back to `command`. An empty command gives one empty chunk.
fn wrap_command(command: &str, width: usize) -> Vec<String> {
    let chars: Vec<char> = command.chars().collect();
    if chars.is_empty() {
        vec![String::new()]
    } else {
        chars
            .chunks(width.max(1))
            .map(|chunk| chunk.iter().collect())
            .collect()
    }
}

/// Draws the dialog centered in `area`. The command is shown exactly as held in `state`.
pub fn render(f: &mut Frame, area: Rect, theme: &Theme, state: &ConfirmState) {
    let box_width = BOX_WIDTH.min(area.width);
    let chunk_width = usize::from(
        box_width
            .saturating_sub(SIDE * 2)
            .saturating_sub(PREFIX)
            .max(1),
    );
    let chunks = wrap_command(&state.command, chunk_width);
    let rows = u16::try_from(chunks.len() + 2).unwrap_or(u16::MAX);
    let rect = box_rect(area, rows, BOX_WIDTH);

    let style = Style::default().fg(theme.fg).bg(theme.panel);
    let dim = Style::default().fg(theme.dim).bg(theme.panel);
    let block = Block::default()
        .title(Span::styled(
            state.title.clone(),
            Style::default()
                .fg(theme.accent)
                .bg(theme.panel)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border).bg(theme.panel))
        .padding(Padding::horizontal(1))
        .style(style);

    let mut lines: Vec<Line> = chunks
        .into_iter()
        .enumerate()
        .map(|(i, chunk)| {
            let lead = if i == 0 { "$ " } else { "  " };
            Line::from(vec![Span::styled(lead, dim), Span::raw(chunk)])
        })
        .collect();
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled(
            "y",
            Style::default().fg(theme.status_waiting).bg(theme.panel),
        ),
        Span::styled(" run    n cancel", dim),
    ]));

    f.render_widget(Clear, rect);
    f.render_widget(Paragraph::new(lines).block(block).style(style), rect);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ThemeId;
    use ratatui::{Terminal, backend::TestBackend};

    const SHORT: &str = "cox land t-1";
    const LONG: &str = "cox land coxtop-runs-actions-as-cox-commands-after-a-coxtop-actions-confirm-dialog --branch p3-widgets --no-ff --message 'ship it'";

    fn state(command: &str) -> ConfirmState {
        ConfirmState::new(
            "Land".to_string(),
            command.split(' ').map(str::to_string).collect(),
            command.to_string(),
        )
    }

    fn draw(id: ThemeId, command: &str) -> String {
        let theme = crate::theme::resolve_for(id, Some("truecolor"));
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
        terminal
            .draw(|f| render(f, f.area(), &theme, &state(command)))
            .expect("draw should not fail");
        terminal.backend().to_string()
    }

    /// The command text read back from the box rows, minus border, padding, prefix and the
    /// trailing blank and hint rows.
    fn command_from(screen: &str) -> String {
        let rows: Vec<&str> = screen
            .lines()
            // `TestBackend` prints each row between double quotes.
            .map(|l| l.trim().trim_matches('"').trim())
            .filter(|l| l.starts_with('│'))
            .collect();
        rows[..rows.len() - 2]
            .iter()
            .map(|l| {
                let inner = l.trim_start_matches('│').trim_end_matches('│');
                // One padding column, the two-column prefix, then 54 command columns.
                inner
                    .chars()
                    .skip(1 + usize::from(PREFIX))
                    .take(54)
                    .collect::<String>()
            })
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn box_rect_centers_in_a_roomy_area() {
        let area = Rect::new(0, 0, 80, 24);
        assert_eq!(box_rect(area, 3, 60), Rect::new(10, 9, 60, 5));
    }

    #[test]
    fn box_rect_clamps_to_a_small_area() {
        let area = Rect::new(4, 2, 20, 4);
        assert_eq!(box_rect(area, 9, 60), area);
    }

    #[test]
    fn wrap_command_keeps_every_character() {
        for (command, width) in [(SHORT, 54), (LONG, 54), ("abcd", 4), ("a b  c", 2)] {
            let chunks = wrap_command(command, width);
            assert_eq!(chunks.concat(), command);
            assert!(chunks.iter().all(|c| c.chars().count() <= width));
        }
        assert_eq!(wrap_command("", 5), vec![String::new()]);
    }

    #[test]
    fn short_command_is_shown_after_a_dollar_with_the_hint() {
        let screen = draw(ThemeId::Regatta, SHORT);
        assert!(screen.contains("$ cox land t-1"));
        assert!(screen.contains("y run    n cancel"));
    }

    #[test]
    fn long_command_wraps_and_loses_no_character() {
        let screen = draw(ThemeId::Regatta, LONG);
        assert_eq!(command_from(&screen), LONG);
    }

    #[test]
    fn a_tiny_area_draws_nothing_outside_it() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let area = Rect::new(5, 2, 20, 6);
        let mut terminal = Terminal::new(TestBackend::new(30, 10)).expect("terminal");
        terminal
            .draw(|f| render(f, area, &theme, &state(LONG)))
            .expect("draw should not fail");
        let buffer = terminal.backend().buffer();
        for y in 0..10 {
            for x in 0..30 {
                if !area.contains((x, y).into()) {
                    assert_eq!(buffer[(x, y)].symbol(), " ", "cell {x},{y} outside area");
                }
            }
        }
    }

    #[test]
    fn renders_a_short_command_in_the_regatta_theme() {
        insta::assert_snapshot!(draw(ThemeId::Regatta, SHORT));
    }

    #[test]
    fn renders_a_short_command_in_the_harbor_light_theme() {
        insta::assert_snapshot!(draw(ThemeId::HarborLight, SHORT));
    }

    #[test]
    fn renders_a_long_command_in_the_regatta_theme() {
        insta::assert_snapshot!(draw(ThemeId::Regatta, LONG));
    }

    #[test]
    fn renders_a_long_command_in_the_harbor_light_theme() {
        insta::assert_snapshot!(draw(ThemeId::HarborLight, LONG));
    }
}
