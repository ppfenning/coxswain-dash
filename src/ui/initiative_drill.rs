//! Rendering for the initiative-drill board (`cox dash --detail initiative <id>`): a title
//! naming the initiative, one bordered block per phase showing its land time and tasks (each
//! task's needs edges and, when present, its error), and a trailing history block.
#![allow(dead_code)]

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
};

use crate::actions::Target;
use crate::detail::{HistoryEntry, InitiativeDetail, PhaseDetail};
use crate::theme::Theme;

fn base_style(theme: &Theme) -> Style {
    Style::default().fg(theme.fg).bg(theme.bg)
}

/// `done`/`landed` map to `status_done`, `running` to `status_running`, else `status_waiting`.
fn task_state_color(theme: &Theme, state: &str) -> Color {
    match state {
        "done" | "landed" => theme.status_done,
        "running" => theme.status_running,
        _ => theme.status_waiting,
    }
}

/// `<id> landed <local time>` once `landed_at` is set, else `<id> not landed`.
fn phase_title(phase: &PhaseDetail, offset: chrono::FixedOffset) -> String {
    match &phase.landed_at {
        Some(at) => format!("{} landed {}", phase.id, super::local_time(at, offset)),
        None => format!("{} not landed", phase.id),
    }
}

/// Per task: an `<id> <state>` line, then one `  <- <need>` line per need (its own line so a
/// long need id is not clipped alongside the task row), then `ERROR: <error>` when `error` is
/// `Some`.
fn phase_lines(phase: &PhaseDetail, theme: &Theme) -> Vec<Line<'static>> {
    phase
        .tasks
        .iter()
        .flat_map(|task| {
            let state_line = Line::styled(
                format!("{} {}", task.id, task.state),
                Style::default().fg(task_state_color(theme, &task.state)),
            );
            let need_lines = task
                .needs
                .iter()
                .map(|need| Line::from(format!("  <- {need}")));
            let error_line = task.error.as_ref().map(|error| {
                Line::styled(
                    format!("ERROR: {error}"),
                    Style::default().fg(theme.status_failed),
                )
            });
            std::iter::once(state_line)
                .chain(need_lines)
                .chain(error_line)
        })
        .collect()
}

/// `<local time> <event>` per entry, in the given order, which is newest last.
fn history_lines(history: &[HistoryEntry], offset: chrono::FixedOffset) -> Vec<Line<'static>> {
    history
        .iter()
        .map(|entry| {
            Line::from(format!(
                "{} {}",
                super::local_time(&entry.at, offset),
                entry.event
            ))
        })
        .collect()
}

fn render_phases(
    f: &mut Frame,
    area: Rect,
    phases: &[PhaseDetail],
    theme: &Theme,
    offset: chrono::FixedOffset,
) {
    let n = phases.len();
    if n == 0 {
        return;
    }
    let rects = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Ratio(1, n as u32); n])
        .split(area);
    for (phase, rect) in phases.iter().zip(rects.iter()) {
        let block = Block::default()
            .title(phase_title(phase, offset))
            .borders(Borders::ALL);
        let paragraph = Paragraph::new(phase_lines(phase, theme))
            .block(block)
            .style(base_style(theme));
        f.render_widget(paragraph, *rect);
    }
}

fn render_history(
    f: &mut Frame,
    area: Rect,
    detail: &InitiativeDetail,
    theme: &Theme,
    offset: chrono::FixedOffset,
) {
    let block = Block::default()
        .title("history")
        .title_bottom(super::footer_line(
            &Target::Initiative(detail.initiative.clone()),
            theme,
        ))
        .borders(Borders::ALL);
    let history = &detail.history;
    let paragraph = Paragraph::new(history_lines(history, offset))
        .block(block)
        .style(base_style(theme));
    f.render_widget(paragraph, area);
}

/// Draws the title, one block per phase, and a trailing history block over the whole frame.
pub fn render(
    f: &mut Frame,
    detail: &InitiativeDetail,
    theme: &Theme,
    offset: chrono::FixedOffset,
) {
    let area = f.area();
    let history_len = u16::try_from(detail.history.len()).unwrap_or(u16::MAX);
    let [title_area, phases_area, history_area] = {
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(3),
                Constraint::Length(history_len.saturating_add(2)),
            ])
            .split(area);
        [split[0], split[1], split[2]]
    };

    let title = Paragraph::new("")
        .block(
            Block::default()
                .title(format!("Initiative {}", detail.initiative))
                .borders(Borders::ALL),
        )
        .style(base_style(theme));
    f.render_widget(title, title_area);

    render_phases(f, phases_area, &detail.phases, theme, offset);
    render_history(f, history_area, detail, theme, offset);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ThemeId;
    use crate::detail::{DetailSnapshot, parse_detail};
    use ratatui::{Terminal, backend::TestBackend};

    const FIXTURE: &str = include_str!("../../tests/fixtures/dash_detail_initiative_v1.json");

    fn fixture_detail() -> InitiativeDetail {
        let snapshot = parse_detail(FIXTURE).expect("fixture should parse");
        let DetailSnapshot::Initiative(detail) = snapshot else {
            panic!("expected DetailSnapshot::Initiative");
        };
        detail
    }

    #[test]
    fn task_state_color_maps_landed_running_and_ready() {
        let theme = crate::theme::resolve(ThemeId::Regatta);
        assert_eq!(task_state_color(&theme, "landed"), theme.status_done);
        assert_eq!(task_state_color(&theme, "running"), theme.status_running);
        assert_eq!(task_state_color(&theme, "ready"), theme.status_waiting);
    }

    #[test]
    fn renders_initiative_snapshot_in_the_regatta_theme() {
        let detail = fixture_detail();
        let theme = crate::theme::resolve(ThemeId::Regatta);
        let offset = chrono::FixedOffset::east_opt(0).unwrap();
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render(f, &detail, &theme, offset))
            .expect("draw should not fail");
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn renders_initiative_snapshot_in_the_harbor_light_theme() {
        let detail = fixture_detail();
        let theme = crate::theme::resolve(ThemeId::HarborLight);
        let offset = chrono::FixedOffset::east_opt(0).unwrap();
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render(f, &detail, &theme, offset))
            .expect("draw should not fail");
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn error_task_error_text_is_status_failed_colored() {
        let detail = fixture_detail();
        let theme = crate::theme::resolve(ThemeId::Regatta);
        let offset = chrono::FixedOffset::east_opt(0).unwrap();
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render(f, &detail, &theme, offset))
            .expect("draw should not fail");
        let buffer = terminal.backend().buffer();
        let needle = "ERROR:";
        let width = 120u16;
        let height = 40u16;
        let mut found = None;
        'search: for y in 0..height {
            for x in 0..width {
                let mut matched = true;
                for (i, expected) in needle.chars().enumerate() {
                    let cx = x + i as u16;
                    if cx >= width || buffer[(cx, y)].symbol() != expected.to_string() {
                        matched = false;
                        break;
                    }
                }
                if matched {
                    found = Some((x, y));
                    break 'search;
                }
            }
        }
        let (x, y) = found.expect("ERROR: text should be rendered somewhere in the buffer");
        assert_eq!(buffer[(x, y)].fg, theme.status_failed);
    }
}
