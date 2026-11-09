//! The board's agents panel: a process list of the live agents, fastest token rate first. The
//! cell text comes from pure helpers over plain data; `draw` maps it to theme colours and lets
//! the table scroll to keep the selected row in view.

use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Cell, Paragraph, Row, Table, TableState},
};

use super::chrome::bar_parts;
use crate::app::{App, Focus};
use crate::feed::{LiveAgent, agents_by_rate};
use crate::theme::Theme;

/// Cells in the context-fill bar.
const BAR_W: usize = 6;
/// Context fill from which the bar turns the mid colour, and the high colour.
const FILL_MID: f64 = 0.6;
const FILL_HIGH: f64 = 0.85;

pub fn render(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let agents = app
        .snapshot()
        .map(|snap| agents_by_rate(&snap.live.agents))
        .unwrap_or_default();
    let selected = (app.focus() == Focus::BoardAgents).then(|| app.selected());
    draw(f, area, &agents, selected, theme);
}

/// Elapsed seconds as a clock: `m:ss` under an hour, `h:mm:ss` from then on.
fn elapsed_label(secs: u64) -> String {
    match (secs / 3600, secs / 60 % 60, secs % 60) {
        (0, m, s) => format!("{m}:{s:02}"),
        (h, m, s) => format!("{h}:{m:02}:{s:02}"),
    }
}

/// The context-fill bar as filled and empty cell counts. The fraction is clamped to 0..=1.
fn fill_cells(fill: f64) -> (usize, usize) {
    bar_parts(fill, BAR_W)
}

fn fill_color(fill: f64, theme: &Theme) -> ratatui::style::Color {
    if fill >= FILL_HIGH {
        theme.meter_high
    } else if fill >= FILL_MID {
        theme.meter_mid
    } else {
        theme.meter_low
    }
}

fn fill_bar<'a>(fill: f64, theme: &Theme) -> Line<'a> {
    let (filled, empty) = fill_cells(fill);
    Line::from(vec![
        Span::styled(
            "█".repeat(filled),
            Style::default().fg(fill_color(fill, theme)),
        ),
        Span::styled("░".repeat(empty), Style::default().fg(theme.track)),
    ])
}

fn agent_row<'a>(agent: &LiveAgent, theme: &Theme) -> Row<'a> {
    let dim = Style::default().fg(theme.dim);
    let serves = if agent.serves.is_empty() {
        Cell::from("-").style(dim)
    } else {
        Cell::from(agent.serves.clone())
    };
    Row::new([
        Cell::from(agent.kind.clone()),
        Cell::from(agent.model.clone()).style(dim),
        Cell::from(agent.host.clone()),
        serves,
        Cell::from(Line::from(format!("${:.2}", agent.cost_usd)).right_aligned()),
        Cell::from(Line::from(elapsed_label(agent.elapsed_s)).right_aligned()),
        Cell::from(Line::from(agent.tokens_per_min.to_string()).right_aligned()),
        Cell::from(fill_bar(agent.context_fill, theme)),
    ])
    .style(Style::default().fg(theme.fg))
}

/// Draws `agents` in the order given. `selected` is the highlighted row when the panel has focus.
fn draw(f: &mut Frame, area: Rect, agents: &[&LiveAgent], selected: Option<usize>, theme: &Theme) {
    let border = if selected.is_some() {
        theme.border_focus
    } else {
        theme.border
    };
    let block = Block::bordered()
        .title("agents")
        .border_style(Style::default().fg(border));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if agents.is_empty() {
        f.render_widget(
            Paragraph::new("no live agents").style(Style::default().fg(theme.dim)),
            inner,
        );
        return;
    }
    let header = Row::new([
        "kind", "model", "host", "serves", "cost", "elapsed", "tok/m", "ctx",
    ])
    .style(Style::default().fg(theme.dim));
    let table = Table::new(
        agents.iter().map(|agent| agent_row(agent, theme)),
        [
            Constraint::Length(6),
            Constraint::Fill(2),
            Constraint::Length(8),
            Constraint::Fill(3),
            Constraint::Length(6),
            Constraint::Length(7),
            Constraint::Length(6),
            Constraint::Length(BAR_W as u16),
        ],
    )
    .header(header)
    .row_highlight_style(Style::default().bg(theme.selected_row));
    let mut state = TableState::default().with_selected(selected);
    f.render_stateful_widget(table, inner, &mut state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ThemeId;
    use ratatui::{Terminal, backend::TestBackend};

    fn agent(id: &str, serves: &str, tokens_per_min: u64, context_fill: f64) -> LiveAgent {
        LiveAgent {
            id: id.to_string(),
            kind: "build".to_string(),
            model: "claude-sonnet-5-5".to_string(),
            host: "host-b".to_string(),
            serves: serves.to_string(),
            tokens_per_min,
            cost_usd: 0.25,
            elapsed_s: 125,
            context_fill,
        }
    }

    fn theme() -> Theme {
        crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"))
    }

    /// Draws the agents sorted by rate into a `height` row terminal.
    fn drawn(agents: &[LiveAgent], selected: Option<usize>, height: u16) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(110, height)).expect("terminal");
        terminal
            .draw(|f| draw(f, f.area(), &agents_by_rate(agents), selected, &theme()))
            .expect("draw should not fail");
        terminal
    }

    fn text(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn agents_sorted_by_token_rate() {
        let agents = [
            agent("a1", "board-view-keys", 900, 0.1),
            agent("a2", "board-view-load", 4200, 0.38),
            agent("a3", "", 3100, 0.9),
        ];
        insta::assert_snapshot!(text(&drawn(&agents, None, 7)));
    }

    #[test]
    fn the_selected_row_is_highlighted_only_when_a_row_is_selected() {
        let agents = [agent("a1", "x", 20, 0.1), agent("a2", "y", 10, 0.1)];
        let bg_of =
            |terminal: &Terminal<TestBackend>, y: u16| terminal.backend().buffer()[(2, y)].bg;
        let focused = drawn(&agents, Some(1), 6);
        assert_eq!(bg_of(&focused, 3), theme().selected_row);
        assert_ne!(bg_of(&focused, 2), theme().selected_row);
        let unfocused = drawn(&agents, None, 6);
        assert_ne!(bg_of(&unfocused, 3), theme().selected_row);
    }

    #[test]
    fn the_list_scrolls_to_keep_the_selected_row_in_view() {
        let agents: Vec<LiveAgent> = (0..6)
            .map(|n| agent(&format!("a{n}"), &format!("task-{n}"), 100 - n, 0.1))
            .collect();
        let shown = text(&drawn(&agents, Some(5), 5));
        assert!(shown.contains("task-5"), "{shown}");
        assert!(!shown.contains("task-0"), "{shown}");
    }

    #[test]
    fn an_empty_list_says_so() {
        let shown = text(&drawn(&[], None, 4));
        assert!(shown.contains("no live agents"), "{shown}");
    }

    #[test]
    fn elapsed_reads_as_a_clock() {
        assert_eq!(elapsed_label(125), "2:05");
        assert_eq!(elapsed_label(3725), "1:02:05");
    }

    #[test]
    fn the_fill_bar_clamps_to_its_width() {
        assert_eq!(fill_cells(0.0), (0, BAR_W));
        assert_eq!(fill_cells(0.5), (3, 3));
        assert_eq!(fill_cells(2.0), (BAR_W, 0));
        assert_eq!(fill_cells(-1.0), (0, BAR_W));
    }
}
