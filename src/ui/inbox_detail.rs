//! Rendering for one inbox entry opened to its full text: kind, target, when it was raised, the
//! run it came from, the whole reason wrapped across lines, and a key line. The reason is never
//! truncated; a token wider than the frame is broken so every character appears.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::feed::InboxEntry;
use crate::theme::Theme;

/// Width of the label column ahead of each fact.
const LABEL_WIDTH: usize = 8;

/// Draws `entry` in `area`. `at` is shown as the feed gave it.
pub fn render(f: &mut Frame, area: Rect, entry: &InboxEntry, theme: &Theme) {
    let base = Style::default().fg(theme.fg).bg(theme.bg);
    let block = Block::default()
        .title(format!("Inbox {}", entry.kind))
        .borders(Borders::ALL)
        .style(base);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);

    let mut lines = vec![
        fact("kind", Some(&entry.kind), theme),
        fact("target", Some(&entry.target), theme),
        fact("raised", entry.at.as_deref(), theme),
        fact("run", entry.run.as_deref(), theme),
        Line::default(),
        Line::from(Span::styled("reason", Style::default().fg(theme.dim))),
    ];
    lines.extend(
        wrap_text(&entry.reason, usize::from(inner.width))
            .into_iter()
            .map(Line::from),
    );
    f.render_widget(Paragraph::new(lines).style(base), rows[0]);
    f.render_widget(Paragraph::new(key_line(theme)).style(base), rows[1]);
}

/// A label and its value, or a dim dash when the value is absent.
fn fact<'a>(label: &'a str, value: Option<&'a str>, theme: &Theme) -> Line<'a> {
    let dim = Style::default().fg(theme.dim);
    let value = match value {
        Some(text) => Span::raw(text),
        None => Span::styled("-", dim),
    };
    Line::from(vec![
        Span::styled(format!("{label:<LABEL_WIDTH$}"), dim),
        value,
    ])
}

fn key_line(theme: &Theme) -> Line<'static> {
    let key = Style::default().fg(theme.accent);
    let dim = Style::default().fg(theme.dim);
    Line::from(vec![
        Span::styled("a", key),
        Span::styled(" acknowledge  ", dim),
        Span::styled("d", key),
        Span::styled(" dismiss  ", dim),
        Span::styled("Esc", key),
        Span::styled(" back", dim),
    ])
}

/// Greedy word wrap to `width` characters, keeping the text's own line breaks. A word longer
/// than `width` is cut into `width`-character pieces. Nothing is dropped except the spaces at
/// a wrap. A `width` of 0 is treated as 1.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    text.lines()
        .flat_map(|line| wrap_line(line, width))
        .collect()
}

fn wrap_line(line: &str, width: usize) -> Vec<String> {
    let (mut done, current) = line.split_whitespace().fold(
        (Vec::<String>::new(), String::new()),
        |(mut done, current), word| {
            let word_len = word.chars().count();
            if word_len > width {
                if !current.is_empty() {
                    done.push(current);
                }
                let mut pieces = chunks(word, width);
                let last = pieces.pop().unwrap_or_default();
                done.extend(pieces);
                (done, last)
            } else if current.is_empty() {
                (done, word.to_string())
            } else if current.chars().count() + 1 + word_len <= width {
                (done, format!("{current} {word}"))
            } else {
                done.push(current);
                (done, word.to_string())
            }
        },
    );
    done.push(current);
    done
}

fn chunks(word: &str, width: usize) -> Vec<String> {
    let chars: Vec<char> = word.chars().collect();
    chars
        .chunks(width)
        .map(|piece| piece.iter().collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ThemeId;
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

    const LAND: &str = "cox land --run run-20261103-abcdef --task inbox-detail-view --repo /home/pfenningpat/repos/coxswain-dash --apply";

    fn entry(at: Option<&str>, run: Option<&str>) -> InboxEntry {
        InboxEntry {
            kind: "approved-not-landed".to_string(),
            target: "coxswain-dash/inbox-detail-view".to_string(),
            reason: format!("Approved but not landed. Run {LAND}"),
            at: at.map(str::to_string),
            run: run.map(str::to_string),
            initiative: None,
            stale: None,
        }
    }

    fn draw(entry: &InboxEntry, id: ThemeId, width: u16, height: u16) -> (Buffer, Theme) {
        let theme = crate::theme::resolve_for(id, Some("truecolor"));
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|f| render(f, f.area(), entry, &theme))
            .expect("draw should not fail");
        (terminal.backend().buffer().clone(), theme)
    }

    fn row(buffer: &Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    #[test]
    fn wrap_text_breaks_an_unbroken_token_without_losing_a_character() {
        let token = "0123456789".repeat(4);
        let lines = wrap_text(&token, 10);
        assert_eq!(lines.len(), 4);
        assert!(lines.iter().all(|l| l.chars().count() == 10));
        assert_eq!(lines.concat(), token);
    }

    #[test]
    fn wrap_text_wraps_at_words_and_keeps_line_breaks() {
        assert_eq!(wrap_text("aa bb cc", 5), vec!["aa bb", "cc"]);
        assert_eq!(wrap_text("aa\n\nbb", 5), vec!["aa", "", "bb"]);
    }

    #[test]
    fn the_full_land_command_is_recoverable_at_a_narrow_width() {
        let (buffer, _) = draw(
            &entry(Some("2026-11-03T09:00:00Z"), Some("run-1")),
            ThemeId::Regatta,
            40,
            20,
        );
        let text: String = (0..buffer.area.height)
            .map(|y| row(&buffer, y))
            .collect::<String>()
            .chars()
            .filter(|c| !c.is_whitespace() && !"│┌┐└┘─".contains(*c))
            .collect();
        let want: String = LAND.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(text.contains(&want), "command missing from: {text}");
        assert!(!text.contains("..."));
        assert!(!text.contains('…'));
    }

    #[test]
    fn a_missing_at_and_run_draw_dim_dashes() {
        let (buffer, theme) = draw(&entry(None, None), ThemeId::Regatta, 60, 14);
        for y in [3, 4] {
            let line = row(&buffer, y);
            assert!(line.contains("- "), "no dash on row {y}: {line}");
            let x = u16::try_from(line.chars().position(|c| c == '-').unwrap()).unwrap();
            assert_eq!(buffer[(x, y)].symbol(), "-");
            assert_eq!(buffer[(x, y)].fg, theme.dim);
        }
    }

    #[test]
    fn the_key_line_names_acknowledge_dismiss_and_back() {
        let (buffer, _) = draw(&entry(None, None), ThemeId::Regatta, 60, 14);
        let line = row(&buffer, 12);
        assert!(
            line.contains("a acknowledge  d dismiss  Esc back"),
            "{line}"
        );
    }

    #[test]
    fn renders_inbox_detail_snapshot_in_the_regatta_theme() {
        let e = entry(Some("2026-11-03T09:00:00Z"), Some("run-20261103-abcdef"));
        let mut terminal = Terminal::new(TestBackend::new(48, 16)).expect("terminal");
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        terminal
            .draw(|f| render(f, f.area(), &e, &theme))
            .expect("draw should not fail");
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn renders_inbox_detail_snapshot_in_the_harbor_light_theme() {
        let e = entry(Some("2026-11-03T09:00:00Z"), Some("run-20261103-abcdef"));
        let mut terminal = Terminal::new(TestBackend::new(48, 16)).expect("terminal");
        let theme = crate::theme::resolve_for(ThemeId::HarborLight, Some("truecolor"));
        terminal
            .draw(|f| render(f, f.area(), &e, &theme))
            .expect("draw should not fail");
        insta::assert_snapshot!(terminal.backend().to_string());
    }
}
