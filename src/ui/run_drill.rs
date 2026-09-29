//! Rendering for the run drill-down board (`cox dash --detail run <id>`): a title line naming
//! the run, machine, initiative and phase; one row per pipeline step with a bar scaled to
//! turns, the turns and cost, and a verdict when there is one; why the run stopped, if it did;
//! the files it touched; its last tool calls; and as much of its log tail as fits below.

#![allow(dead_code)]

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::Line,
    widgets::{Block, BorderType, Borders, Paragraph},
};

use crate::detail::{RunDetail, StepDetail};
use crate::theme::Theme;

/// Width, in characters, of a fully-filled step bar.
const BAR_WIDTH: usize = 20;

pub fn render(f: &mut Frame, detail: &RunDetail, theme: &Theme, offset: chrono::FixedOffset) {
    let area = f.area();
    let block = Block::default()
        .title(format!("run {}", detail.run))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(base_style(theme));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let stopped_len = u16::from(detail.stopped_reason.is_some());
    let tool_calls_len = if detail.last_tool_calls.is_empty() {
        0
    } else {
        detail.last_tool_calls.len() as u16
    };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(detail.steps.len() as u16),
            Constraint::Length(stopped_len),
            Constraint::Length(detail.files.len() as u16),
            Constraint::Length(tool_calls_len),
            Constraint::Min(0),
        ])
        .split(inner);

    render_title(f, rows[0], detail, theme);
    render_steps(f, rows[1], detail, theme);
    if let Some(reason) = &detail.stopped_reason {
        render_stopped_reason(f, rows[2], reason, theme);
    }
    render_files(f, rows[3], detail, theme);
    render_tool_calls(f, rows[4], detail, theme, offset);
    render_log_tail(f, rows[5], detail, theme);
}

fn base_style(theme: &Theme) -> Style {
    Style::default().fg(theme.fg).bg(theme.bg)
}

fn render_title(f: &mut Frame, rect: Rect, detail: &RunDetail, theme: &Theme) {
    let line = Line::from(format!(
        "{} | {} | {} | {}",
        detail.run, detail.machine, detail.initiative, detail.phase
    ));
    let paragraph = Paragraph::new(line).style(Style::default().fg(theme.accent).bg(theme.bg));
    f.render_widget(paragraph, rect);
}

/// The color a step's row is drawn in: `status_done`/`status_running`/`status_failed` for
/// those statuses, `dim` for `pending` and anything else the feed might send.
fn step_color(theme: &Theme, status: &str) -> Color {
    match status {
        "done" => theme.status_done,
        "running" => theme.status_running,
        "failed" => theme.status_failed,
        _ => theme.dim,
    }
}

/// A bar of `#` scaled to `step.turns` against `max_turns`, padded to `BAR_WIDTH`. Empty when
/// every step (including this one) has zero turns.
fn step_bar(step: &StepDetail, max_turns: u32) -> String {
    if max_turns == 0 {
        return String::new();
    }
    let filled = (f64::from(step.turns) / f64::from(max_turns) * BAR_WIDTH as f64).round();
    "#".repeat((filled as usize).min(BAR_WIDTH))
}

fn render_steps(f: &mut Frame, rect: Rect, detail: &RunDetail, theme: &Theme) {
    let max_turns = detail
        .steps
        .iter()
        .map(|step| step.turns)
        .max()
        .unwrap_or(0);
    let lines: Vec<Line> = detail
        .steps
        .iter()
        .map(|step| {
            let bar = step_bar(step, max_turns);
            let node = &step.node;
            let turns = step.turns;
            let cost = step.cost;
            let verdict = step
                .verdict
                .as_deref()
                .map(|verdict| format!(" {verdict}"))
                .unwrap_or_default();
            Line::styled(
                format!("{node:<10} [{bar:<BAR_WIDTH$}] {turns:>3} ${cost:.2}{verdict}"),
                Style::default().fg(step_color(theme, &step.status)),
            )
        })
        .collect();
    let paragraph = Paragraph::new(lines).style(base_style(theme));
    f.render_widget(paragraph, rect);
}

fn render_stopped_reason(f: &mut Frame, rect: Rect, reason: &str, theme: &Theme) {
    let paragraph = Paragraph::new(format!("stopped: {reason}"))
        .style(Style::default().fg(theme.status_failed).bg(theme.bg));
    f.render_widget(paragraph, rect);
}

fn render_files(f: &mut Frame, rect: Rect, detail: &RunDetail, theme: &Theme) {
    let lines: Vec<Line> = detail
        .files
        .iter()
        .map(|file| Line::from(file.clone()))
        .collect();
    let paragraph = Paragraph::new(lines).style(base_style(theme));
    f.render_widget(paragraph, rect);
}

/// Renders nothing when there are no tool calls to show, so the space goes to the log tail.
fn render_tool_calls(
    f: &mut Frame,
    rect: Rect,
    detail: &RunDetail,
    theme: &Theme,
    offset: chrono::FixedOffset,
) {
    if detail.last_tool_calls.is_empty() {
        return;
    }
    let lines: Vec<Line> = detail
        .last_tool_calls
        .iter()
        .map(|call| {
            let at = super::local_time(&call.at, offset);
            let tool = &call.tool;
            let summary = &call.summary;
            Line::from(format!("{at} {tool}: {summary}"))
        })
        .collect();
    let paragraph = Paragraph::new(lines).style(Style::default().fg(theme.dim).bg(theme.bg));
    f.render_widget(paragraph, rect);
}

/// The last lines of `detail.log_tail` that fit in `rect`'s height, oldest of the kept lines
/// first, so the most recent line is always the last one shown.
fn render_log_tail(f: &mut Frame, rect: Rect, detail: &RunDetail, theme: &Theme) {
    let fits = rect.height as usize;
    let start = detail.log_tail.len().saturating_sub(fits);
    let lines: Vec<Line> = detail.log_tail[start..]
        .iter()
        .map(|line| Line::from(line.clone()))
        .collect();
    let paragraph = Paragraph::new(lines).style(Style::default().fg(theme.dim).bg(theme.bg));
    f.render_widget(paragraph, rect);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ThemeId;
    use crate::detail::{DetailSnapshot, parse_detail};
    use ratatui::{Terminal, backend::TestBackend};

    const FIXTURE: &str = include_str!("../../tests/fixtures/dash_detail_run_v1.json");

    fn run_detail_from_fixture() -> RunDetail {
        let snapshot = parse_detail(FIXTURE).expect("fixture should parse");
        let DetailSnapshot::Run(detail) = snapshot else {
            panic!("expected DetailSnapshot::Run");
        };
        detail
    }

    fn draw_with_theme(theme_id: ThemeId) -> Terminal<TestBackend> {
        let detail = run_detail_from_fixture();
        let theme = crate::theme::resolve(theme_id);
        let offset = chrono::FixedOffset::east_opt(0).unwrap();
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render(f, &detail, &theme, offset))
            .expect("draw should not fail");
        terminal
    }

    #[test]
    fn renders_run_drill_snapshot_in_the_regatta_theme() {
        let terminal = draw_with_theme(ThemeId::Regatta);
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn renders_run_drill_snapshot_in_the_harbor_light_theme() {
        let terminal = draw_with_theme(ThemeId::HarborLight);
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn build_step_row_uses_the_status_running_color() {
        let terminal = draw_with_theme(ThemeId::Regatta);
        let theme = crate::theme::resolve(ThemeId::Regatta);
        // Inner area starts at (1, 1) inside the rounded border. The title takes inner row 0,
        // so the steps start at inner row 1 (absolute y = 2). `build` is the fixture's second
        // step (index 1), so its row is absolute y = 3; its first character sits at x = 1.
        let fg = terminal.backend().buffer()[(1, 3)].fg;
        assert_eq!(fg, theme.status_running);
    }
}
