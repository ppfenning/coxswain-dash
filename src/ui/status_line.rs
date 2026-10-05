//! Draws the one-line status of the last action's outcome. `ui/mod.rs` silences unused-code
//! warnings on this module until the page that shows the line wires it in.

use ratatui::{Frame, layout::Rect, style::Style, widgets::Paragraph};

use crate::exec::StatusLevel;
use crate::theme::Theme;

/// `text` cut to `width` chars, the last of them `…` when anything was cut.
fn clip(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        text.to_owned()
    } else {
        text.chars()
            .take(width.saturating_sub(1))
            .chain(std::iter::once('…').take(width.min(1)))
            .collect()
    }
}

/// Draws the status on one line of `area`: `status_done` for Ok, `status_failed` for Failed,
/// blank for `None`.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    status: Option<&(StatusLevel, String)>,
) {
    let base = Style::default().fg(theme.fg).bg(theme.bg);
    let paragraph = match status {
        Some((level, text)) => {
            let color = match level {
                StatusLevel::Ok => theme.status_done,
                StatusLevel::Failed => theme.status_failed,
            };
            Paragraph::new(clip(text, usize::from(area.width))).style(base.fg(color))
        }
        None => Paragraph::new("").style(base),
    };
    frame.render_widget(paragraph, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ThemeId;
    use ratatui::{Terminal, backend::TestBackend};

    fn drawn(status: Option<&(StatusLevel, String)>) -> Terminal<TestBackend> {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let mut terminal = Terminal::new(TestBackend::new(40, 1)).expect("terminal");
        terminal
            .draw(|f| render(f, f.area(), &theme, status))
            .expect("draw should not fail");
        terminal
    }

    #[test]
    fn clip_cuts_to_the_width_with_an_ellipsis() {
        assert_eq!(clip("abcdef", 4), "abc…");
        assert_eq!(clip("abcd", 4), "abcd");
        assert_eq!(clip("abcd", 0), "");
    }

    #[test]
    fn renders_an_ok_line() {
        let status = (StatusLevel::Ok, "landed task p3-widgets".to_owned());
        insta::assert_snapshot!(drawn(Some(&status)).backend().to_string());
    }

    #[test]
    fn renders_a_failed_line() {
        let status = (
            StatusLevel::Failed,
            "failed (exit 2): no such run".to_owned(),
        );
        insta::assert_snapshot!(drawn(Some(&status)).backend().to_string());
    }

    #[test]
    fn renders_a_clipped_long_line() {
        let status = (StatusLevel::Ok, "word ".repeat(20));
        insta::assert_snapshot!(drawn(Some(&status)).backend().to_string());
    }

    #[test]
    fn renders_none_as_a_blank_line() {
        insta::assert_snapshot!(drawn(None).backend().to_string());
    }

    #[test]
    fn ok_and_failed_text_take_the_done_and_failed_colors() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let ok = (StatusLevel::Ok, "ok".to_owned());
        let failed = (StatusLevel::Failed, "bad".to_owned());
        assert_eq!(
            drawn(Some(&ok)).backend().buffer()[(0, 0)].fg,
            theme.status_done
        );
        assert_eq!(
            drawn(Some(&failed)).backend().buffer()[(0, 0)].fg,
            theme.status_failed
        );
    }
}
