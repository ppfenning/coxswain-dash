//! The chair agents panel: a bordered block titled `Chair agents` with one row per background
//! agent the chair launched, newest first. The pure core (ordering, elapsed time, cell text) takes
//! plain data and the current time; the drawing maps those cells to theme colours. The clock is
//! never read here. Nothing calls `render` yet, so dead-code warnings are silenced until the
//! wiring phase.
#![allow(dead_code)]

use chrono::{DateTime, Utc};
use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::feed::{AgentState, ChairAgent};
use crate::theme::Theme;

/// Width of the state column: `running` and `unknown` are the longest words.
const STATE_W: usize = 7;
/// Width of the elapsed column, enough for `12h59m`.
const ELAPSED_W: usize = 6;
/// Width of the token column, right aligned.
const TOKENS_W: usize = 9;
/// Width of the cost column, right aligned.
const COST_W: usize = 8;
/// Cells every row spends on the separators between its six columns.
const GAPS: usize = 5;

/// One row's text, before colour.
#[derive(Debug, PartialEq)]
struct Cells {
    label: String,
    state: AgentState,
    state_word: &'static str,
    model: String,
    elapsed: String,
    tokens: String,
    cost: String,
}

fn parse(utc: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(utc)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

/// Agents by `started_at`, newest first. An agent whose start does not parse sorts last.
fn newest_first(agents: &[ChairAgent]) -> Vec<&ChairAgent> {
    let mut sorted: Vec<&ChairAgent> = agents.iter().collect();
    sorted.sort_by_key(|agent| std::cmp::Reverse(parse(&agent.started_at)));
    sorted
}

/// Seconds a running agent has been up (`now` minus start) or a finished one ran (finish minus
/// start). `None` when a needed time is missing or does not parse. A negative span is 0.
fn elapsed_secs(agent: &ChairAgent, now: DateTime<Utc>) -> Option<u64> {
    let start = parse(&agent.started_at)?;
    let end = match agent.state {
        AgentState::Running => now,
        _ => parse(agent.finished_at.as_deref()?)?,
    };
    Some((end - start).num_seconds().max(0) as u64)
}

/// `45s`, `5m45s`, `1h15m`: seconds under a minute, minutes and seconds under an hour, else
/// hours and minutes.
fn duration_text(seconds: u64) -> String {
    match seconds {
        0..=59 => format!("{seconds}s"),
        60..=3599 => format!("{}m{:02}s", seconds / 60, seconds % 60),
        _ => format!("{}h{:02}m", seconds / 3600, seconds % 3600 / 60),
    }
}

fn state_word(state: AgentState) -> &'static str {
    match state {
        AgentState::Running => "running",
        AgentState::Done => "done",
        AgentState::Failed => "failed",
        AgentState::Unknown => "unknown",
    }
}

fn cells(agent: &ChairAgent, now: DateTime<Utc>) -> Cells {
    Cells {
        label: agent.label.clone(),
        state: agent.state,
        state_word: state_word(agent.state),
        model: agent.model.clone(),
        elapsed: elapsed_secs(agent, now).map_or_else(|| "-".to_string(), duration_text),
        tokens: agent.tokens.to_string(),
        cost: format!("${:.2}", agent.cost_usd),
    }
}

/// Cuts `text` to `width` chars, ending in `…` when anything was dropped, and pads with
/// spaces up to `width`.
fn fit(text: &str, width: usize) -> String {
    let len = text.chars().count();
    if len <= width {
        format!("{text:<width$}")
    } else if width == 0 {
        String::new()
    } else {
        let head: String = text.chars().take(width - 1).collect();
        format!("{head}…")
    }
}

/// The label takes whatever width the other columns leave.
fn row_line(cells: &Cells, theme: &Theme, width: usize, model_w: usize) -> Line<'static> {
    let label_w = width
        .saturating_sub(STATE_W + model_w + ELAPSED_W + TOKENS_W + COST_W + GAPS)
        .max(1);
    Line::from(vec![
        Span::raw(fit(&cells.label, label_w)),
        Span::raw(" "),
        Span::styled(
            format!("{:<STATE_W$}", cells.state_word),
            Style::default().fg(super::status::color(
                theme,
                super::status::agent(cells.state),
            )),
        ),
        Span::raw(" "),
        Span::styled(
            format!("{:<model_w$}", cells.model),
            Style::default().fg(theme.dim),
        ),
        Span::raw(" "),
        Span::raw(format!("{:>ELAPSED_W$}", cells.elapsed)),
        Span::raw(" "),
        Span::raw(format!("{:>TOKENS_W$}", cells.tokens)),
        Span::raw(" "),
        Span::raw(format!("{:>COST_W$}", cells.cost)),
    ])
}

/// Draws the panel from the agent list alone. Rows past the frame height are cut; there is no
/// scrolling. `now` is the only clock the panel sees.
pub fn render(f: &mut Frame, area: Rect, agents: &[ChairAgent], now: DateTime<Utc>, theme: &Theme) {
    let base = Style::default().fg(theme.fg).bg(theme.bg);
    let block = Block::default()
        .title("Chair agents")
        .borders(Borders::ALL)
        .style(base);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let lines: Vec<Line> = if agents.is_empty() {
        vec![Line::styled(
            "no chair agents",
            Style::default().fg(theme.dim),
        )]
    } else {
        let rows: Vec<Cells> = newest_first(agents)
            .into_iter()
            .take(usize::from(inner.height))
            .map(|agent| cells(agent, now))
            .collect();
        let model_w = rows
            .iter()
            .map(|row| row.model.chars().count())
            .max()
            .unwrap_or(0);
        rows.iter()
            .map(|row| row_line(row, theme, usize::from(inner.width), model_w))
            .collect()
    };
    f.render_widget(Paragraph::new(lines).style(base), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ThemeId;
    use ratatui::{Terminal, backend::TestBackend};

    const FIXTURE: &str = include_str!("../../tests/fixtures/dash_activity_v1.json");

    #[derive(serde::Deserialize)]
    struct Activity {
        chair_agents: Vec<ChairAgent>,
    }

    fn fixture_agents() -> Vec<ChairAgent> {
        let activity: Activity = serde_json::from_str(FIXTURE).expect("fixture parses");
        activity.chair_agents
    }

    fn at(utc: &str) -> DateTime<Utc> {
        parse(utc).expect("test time parses")
    }

    /// The buffer as one string of rows.
    fn drawn(agents: &[ChairAgent], now: &str) -> String {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let mut terminal = Terminal::new(TestBackend::new(100, 8)).expect("terminal");
        terminal
            .draw(|f| render(f, f.area(), agents, at(now), &theme))
            .expect("draw should not fail");
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn row_with<'a>(text: &'a str, needle: &str) -> &'a str {
        text.lines()
            .find(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("{needle} should be drawn:\n{text}"))
    }

    #[test]
    fn the_fixture_agents_appear_newest_first_with_every_column() {
        let text = drawn(&fixture_agents(), "2026-10-16T18:42:32Z");
        let newer = text.find("Rebuild the intake tickets").expect("newer row");
        let older = text.find("Land the chair attach phase").expect("older row");
        assert!(newer < older, "newest first:\n{text}");
        let running = row_with(&text, "Rebuild the intake tickets");
        for want in ["running", "claude-sonnet-5-5", "2m30s", "120004", "$0.22"] {
            assert!(running.contains(want), "{want} in {running}");
        }
        let done = row_with(&text, "Land the chair attach phase");
        for want in ["done", "claude-sonnet-5-5", "5m45s", "195479", "$0.41"] {
            assert!(done.contains(want), "{want} in {done}");
        }
    }

    #[test]
    fn a_failed_agent_shows_failed() {
        let json = r#"[{"label": "Fix the build", "agent_id": "c1", "model": "claude-opus-5",
            "state": "failed", "started_at": "2026-10-16T17:00:00Z",
            "finished_at": "2026-10-16T17:00:45Z", "tokens": 900, "cost_usd": 0.05}]"#;
        let agents: Vec<ChairAgent> = serde_json::from_str(json).expect("agent parses");
        let text = drawn(&agents, "2026-10-16T19:00:00Z");
        let row = row_with(&text, "Fix the build");
        for want in ["failed", "claude-opus-5", "45s", "900", "$0.05"] {
            assert!(row.contains(want), "{want} in {row}");
        }
    }

    #[test]
    fn an_empty_list_renders_no_chair_agents() {
        let text = drawn(&[], "2026-10-16T19:00:00Z");
        assert!(text.contains("no chair agents"), "{text}");
    }

    #[test]
    fn a_running_agent_counts_from_now_and_a_finished_one_from_its_finish() {
        let agents = fixture_agents();
        let now = at("2026-10-16T18:40:32Z");
        assert_eq!(elapsed_secs(&agents[0], now), Some(345));
        assert_eq!(elapsed_secs(&agents[1], now), Some(30));
        assert_eq!(
            elapsed_secs(&agents[1], at("2026-10-16T18:00:00Z")),
            Some(0)
        );
    }

    #[test]
    fn durations_read_in_seconds_minutes_and_hours() {
        assert_eq!(duration_text(45), "45s");
        assert_eq!(duration_text(345), "5m45s");
        assert_eq!(duration_text(4500), "1h15m");
    }
}
