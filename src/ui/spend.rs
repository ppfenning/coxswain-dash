//! The Spend page: the three-way split for the selected window on top, the per-node table
//! under it, and cost per landed task on the last line.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::Line,
    widgets::{Cell, Paragraph, Row, Table},
};

use crate::app::{App, SpendWindow as WindowChoice};
use crate::feed::SpendWindow;
use crate::theme::Theme;

use super::regatta::{feed_now, framed};
use super::run_cost::window_lines;

/// Shown in place of the table when the selected window carries no `by_node` rows.
const NO_BREAKDOWN: &str = "no per-role breakdown from this cox";

const HEADER: [&str; 7] = ["Role", "Tier", "Model", "Calls", "Turns", "Tokens", "Cost"];

/// One cell per column of [`HEADER`], one row per `by_node` entry, in feed order.
fn node_rows(w: &SpendWindow) -> Vec<[String; 7]> {
    w.by_node
        .iter()
        .map(|n| {
            [
                n.role.clone(),
                n.tier.clone(),
                n.model.clone(),
                n.calls.to_string(),
                n.turns.to_string(),
                n.tokens.to_string(),
                format!("${:.2}", n.cost_usd),
            ]
        })
        .collect()
}

/// The bottom line; `--` stands in for an absent value.
fn cost_line(value: Option<f64>) -> String {
    match value {
        Some(usd) => format!("cost per landed task  ${usd:.2}"),
        None => "cost per landed task  --".to_string(),
    }
}

fn window_title(choice: WindowChoice) -> &'static str {
    match choice {
        WindowChoice::FiveHour => "Spend  five-hour",
        WindowChoice::Weekly => "Spend  weekly",
    }
}

fn table_widget<'a>(rows: Vec<[String; 7]>, theme: &Theme) -> Table<'a> {
    let cell = |i: usize, text: String| {
        if i >= 3 {
            Cell::from(Line::from(text).right_aligned())
        } else {
            Cell::from(text)
        }
    };
    let header = Row::new(
        HEADER
            .iter()
            .enumerate()
            .map(|(i, h)| cell(i, (*h).to_string())),
    )
    .style(Style::default().fg(theme.dim).add_modifier(Modifier::BOLD));
    let body = rows.into_iter().map(|r| {
        Row::new(r.into_iter().enumerate().map(|(i, t)| cell(i, t)))
            .style(Style::default().fg(theme.fg))
    });
    Table::new(
        body,
        [
            Constraint::Length(14),
            Constraint::Length(8),
            Constraint::Min(12),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Length(10),
            Constraint::Length(10),
        ],
    )
    .header(header)
}

pub fn render(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let block = framed(window_title(app.spend_window()), theme, false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let Some((snapshot, split)) = app
        .snapshot()
        .and_then(|s| s.spend_split.as_ref().map(|p| (s, p)))
    else {
        f.render_widget(
            Paragraph::new("no spend split").style(Style::default().fg(theme.dim)),
            inner,
        );
        return;
    };
    let (w, name) = match app.spend_window() {
        WindowChoice::FiveHour => (&split.five_hour, "five-hour"),
        WindowChoice::Weekly => (&split.weekly, "weekly"),
    };
    let [top, middle, bottom] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(inner);
    let lines: Vec<Line> = window_lines(name, w, feed_now(snapshot))
        .into_iter()
        .enumerate()
        .map(|(i, text)| {
            let color = if i == 0 { theme.dim } else { theme.fg };
            Line::styled(text, Style::default().fg(color))
        })
        .collect();
    f.render_widget(Paragraph::new(lines), top);
    if w.by_node.is_empty() {
        f.render_widget(
            Paragraph::new(NO_BREAKDOWN).style(Style::default().fg(theme.dim)),
            middle,
        );
    } else {
        f.render_widget(table_widget(node_rows(w), theme), middle);
    }
    f.render_widget(
        Paragraph::new(cost_line(w.cost_per_landed_task)).style(Style::default().fg(theme.fg)),
        bottom,
    );
}
