//! Rendering for the spend board (`cox dash --detail spend`), in the Slipstream's visual
//! language: rounded cards with spaced labels. Meters, trend, daily cost and where the spend
//! went stack in one column; cards drop out by priority (meters, daily, where, trend) when the
//! area is short.

use chrono::{DateTime, NaiveDate};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Padding, Paragraph},
};

use super::chrome::{
    bar_parts, base_style, below_first_row, card_block, clip, dim_style, first_row, render_label,
};
use crate::detail::{DayCost, Meter, MeterPoint, SpendDetail};
use crate::feed::{NodeSpend, SpendWindow};
use crate::theme::{SlipstreamPalette, Theme};

/// A meter turns `meter_mid` from 60% full and `meter_high` from 90%.
const METER_MID_FROM: f64 = 0.6;
const METER_HIGH_FROM: f64 = 0.9;
const LABEL_WIDTH: usize = 8;
const PCT_WIDTH: usize = 5;
const MIN_BAR: usize = 8;
const MAX_BAR: usize = 60;
const DAY_WIDTH: usize = 6;
const COST_WIDTH: usize = 9;
/// " 12345 calls" and " $1234.56" after a node's name.
const NODE_RIGHT: usize = 12 + 1 + COST_WIDTH;
const MAX_NODES: usize = 6;
/// Border rows plus the label row.
const CHROME: u16 = 3;
const SPARKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Cards in display order, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Card {
    Meters,
    Trend,
    Daily,
    Where,
}

type Named<'a> = Option<(&'a str, &'a SpendWindow)>;

fn fg(color: Color) -> Style {
    Style::default().fg(color)
}

fn meter_color(theme: &Theme, fraction: f64) -> Color {
    if fraction < METER_MID_FROM {
        theme.meter_low
    } else if fraction < METER_HIGH_FROM {
        theme.meter_mid
    } else {
        theme.meter_high
    }
}

/// Each card's `(card, min, want)` height, in priority order.
fn sizes(detail: &SpendDetail, window: Named) -> [(Card, u16, u16); 4] {
    let days = u16::try_from(detail.daily.len()).unwrap_or(u16::MAX);
    let daily = if days == 0 {
        (CHROME + 1, CHROME + 1)
    } else {
        (CHROME + 2, CHROME.saturating_add(days).saturating_add(1))
    };
    let wh = match window {
        None => (CHROME + 1, CHROME + 1),
        Some((_, w)) => {
            let nodes = w.by_node.len().clamp(1, MAX_NODES) as u16;
            let cpl = u16::from(w.cost_per_landed_task.is_some());
            (CHROME + 2, CHROME + 2 + nodes + cpl)
        }
    };
    let trend = CHROME + if detail.history.is_empty() { 1 } else { 2 };
    [
        (Card::Meters, CHROME + 2, CHROME + 2),
        (Card::Daily, daily.0, daily.1),
        (Card::Where, wh.0, wh.1),
        (Card::Trend, trend, trend),
    ]
}

/// Heights for the cards that fit in `height`, in display order. Every card gets its minimum in
/// priority order while it fits, then spare rows grow cards toward their wanted height, again in
/// priority order. A card whose minimum does not fit is left out.
fn plan(height: u16, sizes: &[(Card, u16, u16)]) -> Vec<(Card, u16)> {
    let kept: Vec<(Card, u16, u16)> = sizes
        .iter()
        .scan(height, |left, &(card, min, want)| {
            let fits = min <= *left;
            if fits {
                *left -= min;
            }
            Some(fits.then_some((card, min, want)))
        })
        .flatten()
        .collect();
    let spare = height - kept.iter().map(|k| k.1).sum::<u16>();
    let mut grown: Vec<(Card, u16)> = kept
        .iter()
        .scan(spare, |spare, &(card, min, want)| {
            let extra = (want - min).min(*spare);
            *spare -= extra;
            Some((card, min + extra))
        })
        .collect();
    grown.sort_by_key(|&(card, _)| card);
    grown
}

fn gauge_line(
    label: &str,
    meter: &Meter,
    width: usize,
    theme: &Theme,
    p: &SlipstreamPalette,
    offset: chrono::FixedOffset,
) -> Line<'static> {
    let color = meter_color(theme, meter.fraction);
    let money = format!("  ${:.2} of ${:.2}", meter.used_usd, meter.ceiling_usd);
    let resets = format!("  resets {}", super::local_time(&meter.resets_at, offset));
    let fixed = LABEL_WIDTH + PCT_WIDTH;
    let (money, resets) = [
        (money.as_str(), resets.as_str()),
        (money.as_str(), ""),
        ("", ""),
    ]
    .into_iter()
    .find(|(m, r)| width.saturating_sub(fixed + m.chars().count() + r.chars().count()) >= MIN_BAR)
    .unwrap_or(("", ""));
    let bar = width
        .saturating_sub(fixed + money.chars().count() + resets.chars().count())
        .min(MAX_BAR);
    let (filled, empty) = bar_parts(meter.fraction, bar);
    Line::from(vec![
        Span::styled(format!("{label:<LABEL_WIDTH$}"), dim_style(p)),
        Span::styled("█".repeat(filled), fg(color)),
        Span::styled("░".repeat(empty), fg(p.card_border)),
        Span::styled(format!(" {:>3.0}%", meter.fraction * 100.0), fg(color)),
        Span::styled(money.to_string(), fg(p.text)),
        Span::styled(resets.to_string(), dim_style(p)),
    ])
}

/// One block character per cell: the newest `width` values, or the values stretched to fill.
fn sparkline(values: &[f64], width: usize) -> String {
    let shown = &values[values.len().saturating_sub(width)..];
    let cell = |v: f64| SPARKS[(v.clamp(0.0, 1.0) * (SPARKS.len() - 1) as f64).round() as usize];
    if shown.len() >= width {
        shown.iter().map(|&v| cell(v)).collect()
    } else {
        (0..width)
            .map(|i| cell(shown[i * shown.len() / width]))
            .collect()
    }
}

/// How long the history runs: `45m`, `6h` or `3d`. Empty when a stamp does not parse.
fn span_label(first: &str, last: &str) -> String {
    match (
        DateTime::parse_from_rfc3339(first),
        DateTime::parse_from_rfc3339(last),
    ) {
        (Ok(a), Ok(b)) => {
            let minutes = (b - a).num_minutes().max(0);
            match minutes {
                0..=119 => format!("{minutes}m"),
                120..=2879 => format!("{}h", minutes / 60),
                _ => format!("{}d", minutes / 1440),
            }
        }
        _ => String::new(),
    }
}

fn trend_lines(
    history: &[MeterPoint],
    width: usize,
    theme: &Theme,
    p: &SlipstreamPalette,
) -> Vec<Line<'static>> {
    let (Some(first), Some(last)) = (history.first(), history.last()) else {
        return vec![Line::styled("no meter history yet", dim_style(p))];
    };
    let span = span_label(&first.at, &last.at);
    let rows: [(&str, Vec<f64>, Color); 2] = [
        (
            "5-hour",
            history.iter().map(|h| h.five_hour).collect(),
            p.accents[0],
        ),
        (
            "weekly",
            history.iter().map(|h| h.weekly).collect(),
            p.accents[1],
        ),
    ];
    rows.into_iter()
        .map(|(label, values, color)| {
            let latest = values.last().copied().unwrap_or(0.0);
            let age = format!("  {span:>4}");
            let pct = format!("  {:>3.0}%", latest * 100.0);
            let cells =
                width.saturating_sub(LABEL_WIDTH + age.chars().count() + pct.chars().count());
            Line::from(vec![
                Span::styled(format!("{label:<LABEL_WIDTH$}"), dim_style(p)),
                Span::styled(sparkline(&values, cells), fg(color)),
                Span::styled(age, dim_style(p)),
                Span::styled(pct, fg(meter_color(theme, latest))),
            ])
        })
        .collect()
}

/// `2026-10-03` as `Sat 03`. A day that does not parse is clipped as it came.
fn day_label(day: &str) -> String {
    NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .map(|d| d.format("%a %d").to_string())
        .unwrap_or_else(|_| clip(day, DAY_WIDTH))
}

/// The newest days that fit in `rows` (one row is kept for the total), then the total.
fn daily_lines(
    daily: &[DayCost],
    width: usize,
    rows: usize,
    theme: &Theme,
    p: &SlipstreamPalette,
) -> Vec<Line<'static>> {
    if daily.is_empty() {
        return vec![Line::styled("no daily cost yet", dim_style(p))];
    }
    let largest = daily.iter().map(|d| d.cost).fold(0.0, f64::max);
    let total: f64 = daily.iter().map(|d| d.cost).sum();
    let bar = width.saturating_sub(DAY_WIDTH + 2 + COST_WIDTH);
    let shown = &daily[daily.len().saturating_sub(rows.saturating_sub(1))..];
    let day_line = |d: &DayCost| {
        let fraction = if largest > 0.0 { d.cost / largest } else { 0.0 };
        let (filled, empty) = bar_parts(fraction, bar);
        Line::from(vec![
            Span::styled(format!("{:<DAY_WIDTH$} ", day_label(&d.day)), dim_style(p)),
            Span::styled("█".repeat(filled), fg(meter_color(theme, fraction))),
            Span::styled("░".repeat(empty), fg(p.card_border)),
            Span::styled(
                format!(" {:>COST_WIDTH$}", format!("${:.2}", d.cost)),
                fg(p.text),
            ),
        ])
    };
    let left = clip(
        &format!("Total · {} days", daily.len()),
        width.saturating_sub(COST_WIDTH + 1),
    );
    shown
        .iter()
        .map(day_line)
        .chain(std::iter::once(Line::from(vec![
            Span::styled(
                format!("{left:<w$}", w = width.saturating_sub(COST_WIDTH + 1)),
                dim_style(p),
            ),
            Span::styled(
                format!(" {:>COST_WIDTH$}", format!("${total:.2}")),
                fg(p.text),
            ),
        ])))
        .collect()
}

/// Cell counts for three shares across `width`, summing to `width`; all zero for no spend.
fn segments(shares: [f64; 3], width: usize) -> [usize; 3] {
    let share = |s: &f64| s.max(0.0);
    let total: f64 = shares.iter().map(share).sum();
    if total <= 0.0 {
        return [0; 3];
    }
    let edge = |upto: usize| {
        let cum: f64 = shares[..upto].iter().map(share).sum();
        (cum / total * width as f64).round() as usize
    };
    [edge(1), edge(2) - edge(1), edge(3) - edge(2)]
}

fn legend_line(
    w: &SpendWindow,
    width: usize,
    colors: [Color; 3],
    p: &SlipstreamPalette,
) -> Line<'static> {
    let items = [
        (
            "chair session",
            "session",
            w.chair_session_usd,
            w.chair_session_share,
        ),
        (
            "chair agents",
            "agents",
            w.chair_agents_usd,
            w.chair_agents_share,
        ),
        ("cox seats", "seats", w.cox_seats_usd, w.cox_seats_share),
    ];
    let build = |short: bool| -> Vec<Span<'static>> {
        items
            .iter()
            .zip(colors)
            .enumerate()
            .flat_map(|(i, (&(full, brief, usd, share), color))| {
                let gap = if i == 0 { "" } else { "  " };
                let name = if short { brief } else { full };
                [
                    Span::raw(gap),
                    Span::styled("● ", fg(color)),
                    Span::styled(
                        format!("{name} ${usd:.2} {:.0}%", share * 100.0),
                        fg(p.text),
                    ),
                ]
            })
            .collect()
    };
    let size = |spans: &[Span]| {
        spans
            .iter()
            .map(|s| s.content.chars().count())
            .sum::<usize>()
    };
    let full = build(false);
    Line::from(if size(&full) <= width {
        full
    } else {
        build(true)
    })
}

/// The `n` costliest nodes, costliest first.
fn top_nodes(nodes: &[NodeSpend], n: usize) -> Vec<&NodeSpend> {
    let mut sorted: Vec<&NodeSpend> = nodes.iter().collect();
    sorted.sort_by(|a, b| b.cost_usd.total_cmp(&a.cost_usd));
    sorted.truncate(n);
    sorted
}

fn node_line(n: &NodeSpend, width: usize, p: &SlipstreamPalette) -> Line<'static> {
    let room = width.saturating_sub(NODE_RIGHT);
    let name = clip(&format!("{} · {}", n.role, n.model), room);
    Line::from(vec![
        Span::styled(format!("{name:<room$}"), fg(p.text)),
        Span::styled(format!(" {:>5} calls", n.calls), dim_style(p)),
        Span::styled(
            format!(" {:>COST_WIDTH$}", format!("${:.2}", n.cost_usd)),
            fg(p.text),
        ),
    ])
}

fn where_lines(
    w: &SpendWindow,
    width: usize,
    rows: usize,
    p: &SlipstreamPalette,
) -> Vec<Line<'static>> {
    let colors = [p.accents[0], p.accents[1], p.accent];
    let [a, b, c] = segments(
        [
            w.chair_session_share,
            w.chair_agents_share,
            w.cox_seats_share,
        ],
        width,
    );
    let cells = [a, b, c];
    let bar = if cells.iter().sum::<usize>() == 0 {
        Line::styled("░".repeat(width), fg(p.card_border))
    } else {
        Line::from(
            cells
                .iter()
                .zip(colors)
                .map(|(&n, color)| Span::styled("█".repeat(n), fg(color)))
                .collect::<Vec<_>>(),
        )
    };
    let spare = rows.saturating_sub(2);
    let cpl = w.cost_per_landed_task.filter(|_| spare > 0);
    let node_rows = spare - usize::from(cpl.is_some());
    let nodes = if w.by_node.is_empty() && node_rows > 0 {
        vec![Line::styled("no node breakdown", dim_style(p))]
    } else {
        top_nodes(&w.by_node, node_rows)
            .into_iter()
            .map(|n| node_line(n, width, p))
            .collect()
    };
    let cpl_line = cpl.map(|usd| {
        let left = width.saturating_sub(COST_WIDTH + 1);
        Line::from(vec![
            Span::styled(format!("{:<left$}", "cost per land"), dim_style(p)),
            Span::styled(
                format!(" {:>COST_WIDTH$}", format!("${usd:.2}")),
                fg(p.text),
            ),
        ])
    });
    [bar, legend_line(w, width, colors, p)]
        .into_iter()
        .chain(nodes)
        .chain(cpl_line)
        .collect()
}

/// Draws a card's frame and label, returning the rect left for its content.
fn draw_card(f: &mut Frame, rect: Rect, label: &str, right: &str, p: &SlipstreamPalette) -> Rect {
    let block = card_block(p.card_border, p.card, p).padding(Padding::horizontal(1));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    render_label(f, inner, label, p);
    if !right.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(right.to_string(), dim_style(p)))
                .alignment(Alignment::Right),
            first_row(inner),
        );
    }
    below_first_row(inner)
}

/// Draws the spend board: the cards that fit, each in a rounded block, and an `Esc back` row.
/// `window` is the feed's spend split for the chosen meter window, with its name.
pub fn render(
    f: &mut Frame,
    area: Rect,
    detail: &SpendDetail,
    window: Named,
    theme: &Theme,
    p: &SlipstreamPalette,
    offset: chrono::FixedOffset,
) {
    f.render_widget(Block::default().style(base_style(p)), area);
    let body = Rect {
        height: area.height.saturating_sub(1),
        ..area
    };
    let footer = Rect {
        y: area.y + body.height,
        height: area.height.min(1),
        ..area
    };
    f.render_widget(
        Paragraph::new(Span::styled("Esc back", dim_style(p))),
        footer,
    );

    let heights = plan(body.height, &sizes(detail, window));
    let rects = Layout::vertical(heights.iter().map(|&(_, h)| Constraint::Length(h))).split(body);
    for (&(card, _), &rect) in heights.iter().zip(rects.iter()) {
        let (label, right) = match card {
            Card::Meters => ("SPEND", ""),
            Card::Trend => ("TREND", ""),
            Card::Daily => ("DAILY", ""),
            Card::Where => ("WHERE IT WENT", window.map_or("", |(name, _)| name)),
        };
        let content = draw_card(f, rect, label, right, p);
        let width = usize::from(content.width);
        let rows = usize::from(content.height);
        let lines = match card {
            Card::Meters => vec![
                gauge_line("5-hour", &detail.five_hour, width, theme, p, offset),
                gauge_line("weekly", &detail.weekly, width, theme, p, offset),
            ],
            Card::Trend => trend_lines(&detail.history, width, theme, p),
            Card::Daily => daily_lines(&detail.daily, width, rows, theme, p),
            Card::Where => match window {
                Some((_, w)) => where_lines(w, width, rows, p),
                None => vec![Line::styled("no spend split yet", dim_style(p))],
            },
        };
        f.render_widget(Paragraph::new(lines), content);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ThemeId;
    use crate::ui::slipstream::for_colorterm;
    use ratatui::{Terminal, backend::TestBackend};

    const THEMES: [ThemeId; 2] = [ThemeId::Regatta, ThemeId::HarborLight];

    fn detail() -> SpendDetail {
        let daily: Vec<String> = (1..=14)
            .map(|d| {
                format!(
                    r#"{{"day": "2026-10-{d:02}", "cost": {}}}"#,
                    10.0 + (d * 7 % 11) as f64 * 4.5
                )
            })
            .collect();
        let history: Vec<String> = (0..12)
            .map(|h| {
                format!(
                    r#"{{"at": "2026-10-14T{h:02}:00:00Z", "five_hour": {}, "weekly": {}}}"#,
                    0.05 + h as f64 * 0.03,
                    0.5 + h as f64 * 0.01
                )
            })
            .collect();
        let json = format!(
            r#"{{"schema": 1, "at": "2026-10-14T12:00:00Z",
            "five_hour": {{"fraction": 0.42, "used_usd": 21.0, "ceiling_usd": 50.0, "resets_at": "2026-10-14T15:00:00Z"}},
            "weekly": {{"fraction": 0.61, "used_usd": 183.0, "ceiling_usd": 300.0, "resets_at": "2026-10-18T04:00:00Z"}},
            "history": [{}], "daily": [{}]}}"#,
            history.join(","),
            daily.join(",")
        );
        serde_json::from_str(&json).expect("detail parses")
    }

    fn window(with_nodes: bool) -> SpendWindow {
        let tail = if with_nodes {
            r#","by_node": [
              {"role": "build", "tier": "mid", "model": "sonnet", "calls": 41, "turns": 300, "tokens": 900000, "cost_usd": 6.1},
              {"role": "plan", "tier": "high", "model": "opus", "calls": 9, "turns": 40, "tokens": 200000, "cost_usd": 8.4},
              {"role": "review", "tier": "mid", "model": "sonnet", "calls": 17, "turns": 90, "tokens": 300000, "cost_usd": 2.2}],
              "cost_per_landed_task": 2.5"#
        } else {
            ""
        };
        serde_json::from_str(&format!(
            r#"{{"chair_session_usd": 9.0, "chair_agents_usd": 5.0, "cox_seats_usd": 16.7,
            "total_usd": 30.7, "chair_session_share": 0.293, "chair_agents_share": 0.163,
            "cox_seats_share": 0.544, "meter_fraction": 0.42, "resets_at": "2026-10-14T15:00:00Z"{tail}}}"#
        ))
        .expect("window parses")
    }

    fn draw_with(
        detail: &SpendDetail,
        win: Named,
        id: ThemeId,
        colorterm: Option<&str>,
        size: (u16, u16),
    ) -> Terminal<TestBackend> {
        let theme = crate::theme::resolve_for(id, colorterm);
        let palette = for_colorterm(id, colorterm);
        let offset = chrono::FixedOffset::east_opt(0).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(size.0, size.1)).expect("terminal");
        terminal
            .draw(|f| render(f, f.area(), detail, win, &theme, &palette, offset))
            .expect("draw should not fail");
        terminal
    }

    fn draw(id: ThemeId, size: (u16, u16)) -> Terminal<TestBackend> {
        let w = window(true);
        draw_with(&detail(), Some(("5-hour", &w)), id, Some("truecolor"), size)
    }

    fn screen(terminal: &Terminal<TestBackend>) -> String {
        terminal.backend().to_string()
    }

    macro_rules! snapshot {
        ($name:ident, $id:expr, $size:expr) => {
            #[test]
            fn $name() {
                let terminal = draw($id, $size);
                insta::with_settings!({snapshot_path => "snapshots/spend_drill"}, {
                    insta::assert_snapshot!(screen(&terminal));
                });
            }
        };
    }

    snapshot!(
        renders_spend_snapshot_in_the_regatta_theme,
        ThemeId::Regatta,
        (120, 40)
    );
    snapshot!(
        renders_spend_snapshot_in_the_harbor_light_theme,
        ThemeId::HarborLight,
        (120, 40)
    );
    snapshot!(
        renders_spend_snapshot_at_100x30_in_the_regatta_theme,
        ThemeId::Regatta,
        (100, 30)
    );
    snapshot!(
        renders_spend_snapshot_at_100x30_in_the_harbor_light_theme,
        ThemeId::HarborLight,
        (100, 30)
    );
    snapshot!(
        renders_spend_snapshot_at_160x46_in_the_regatta_theme,
        ThemeId::Regatta,
        (160, 46)
    );
    snapshot!(
        renders_spend_snapshot_at_160x46_in_the_harbor_light_theme,
        ThemeId::HarborLight,
        (160, 46)
    );

    #[test]
    fn a_window_without_nodes_or_cost_per_land_draws_the_fallback() {
        let w = window(false);
        let terminal = draw_with(
            &detail(),
            Some(("weekly", &w)),
            ThemeId::Regatta,
            Some("truecolor"),
            (120, 40),
        );
        let text = screen(&terminal);
        assert!(text.contains("no node breakdown"), "{text}");
        assert!(!text.contains("cost per land"), "{text}");
    }

    #[test]
    fn a_window_with_nodes_lists_them_costliest_first_then_cost_per_land() {
        let text = screen(&draw(ThemeId::Regatta, (120, 40)));
        let at = |needle: &str| {
            text.find(needle)
                .unwrap_or_else(|| panic!("{needle} in {text}"))
        };
        assert!(at("plan · opus") < at("build · sonnet"));
        assert!(at("build · sonnet") < at("review · sonnet"));
        assert!(at("review · sonnet") < at("cost per land"));
    }

    #[test]
    fn no_split_in_the_feed_draws_a_dim_line_in_the_where_card() {
        let terminal = draw_with(
            &detail(),
            None,
            ThemeId::Regatta,
            Some("truecolor"),
            (120, 40),
        );
        assert!(screen(&terminal).contains("no spend split yet"));
    }

    #[test]
    fn plan_keeps_all_cards_when_there_is_room_and_drops_trend_first() {
        let sizes = sizes(&detail(), Some(("5-hour", &window(true))));
        let cards = |h: u16| {
            plan(h, &sizes)
                .into_iter()
                .map(|(c, _)| c)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            cards(45),
            [Card::Meters, Card::Trend, Card::Daily, Card::Where]
        );
        assert_eq!(cards(15), [Card::Meters, Card::Daily, Card::Where]);
        assert_eq!(cards(10), [Card::Meters, Card::Daily]);
        assert_eq!(cards(5), [Card::Meters]);
        assert!(cards(4).is_empty());
        assert!(plan(29, &sizes).iter().map(|c| c.1).sum::<u16>() <= 29);
    }

    #[test]
    fn day_label_names_the_weekday_and_keeps_an_unparseable_day() {
        assert_eq!(day_label("2026-10-03"), "Sat 03");
        assert_eq!(day_label("yesterday"), "yeste…");
    }

    #[test]
    fn segments_sum_to_the_width() {
        assert_eq!(
            segments([0.293, 0.163, 0.544], 40).iter().sum::<usize>(),
            40
        );
        assert_eq!(segments([1.0, 0.0, 0.0], 7), [7, 0, 0]);
        assert_eq!(segments([0.0, 0.0, 0.0], 7), [0, 0, 0]);
    }

    #[test]
    fn sparkline_fills_the_width_from_few_points_and_keeps_the_newest_of_many() {
        assert_eq!(sparkline(&[0.0, 1.0], 4), "▁▁██");
        assert_eq!(sparkline(&[1.0, 0.0, 1.0], 2), "▁█");
        assert_eq!(sparkline(&[0.5], 0), "");
    }

    #[test]
    fn span_label_reads_minutes_hours_and_days() {
        let at = |h: u32, d: u32| format!("2026-10-{d:02}T{h:02}:00:00Z");
        assert_eq!(span_label(&at(10, 5), &at(10, 5)), "0m");
        assert_eq!(span_label(&at(10, 5), &at(16, 5)), "6h");
        assert_eq!(span_label(&at(10, 5), &at(10, 8)), "3d");
        assert_eq!(span_label("x", &at(10, 8)), "");
    }

    #[test]
    fn trend_lines_fill_the_width_with_span_and_latest_value() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let p = for_colorterm(ThemeId::Regatta, Some("truecolor"));
        for width in [60, 100, 140] {
            for line in trend_lines(&detail().history, width, &theme, &p) {
                assert_eq!(line.width(), width);
                assert!(line.to_string().ends_with("%"));
            }
        }
    }

    #[test]
    fn the_daily_card_keeps_the_newest_days_when_short_and_totals_all_of_them() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let p = for_colorterm(ThemeId::Regatta, Some("truecolor"));
        let d = detail();
        let lines = daily_lines(&d.daily, 60, 4, &theme, &p);
        assert_eq!(lines.len(), 4);
        assert!(lines[2].to_string().starts_with("Wed 14"), "{}", lines[2]);
        let total: f64 = d.daily.iter().map(|d| d.cost).sum();
        assert!(lines[3].to_string().ends_with(&format!("${total:.2}")));
    }

    fn bar_fg(terminal: &Terminal<TestBackend>, label: &str) -> Color {
        let buffer = terminal.backend().buffer();
        let y = (0..buffer.area.height)
            .find(|&y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .contains(label)
            })
            .unwrap_or_else(|| panic!("no row with {label:?}"));
        (0..buffer.area.width)
            .map(|x| &buffer[(x, y)])
            .find(|c| c.symbol() == "█")
            .expect("a filled gauge cell")
            .fg
    }

    #[test]
    fn the_weekly_gauge_is_meter_mid_and_the_five_hour_gauge_meter_low() {
        for id in THEMES {
            let theme = crate::theme::resolve_for(id, Some("truecolor"));
            let terminal = draw(id, (120, 40));
            assert_eq!(bar_fg(&terminal, "weekly  █"), theme.meter_mid);
            assert_eq!(bar_fg(&terminal, "5-hour  █"), theme.meter_low);
        }
    }

    #[test]
    fn a_meter_at_ninety_five_percent_is_meter_high() {
        for id in THEMES {
            let theme = crate::theme::resolve_for(id, Some("truecolor"));
            let hot = SpendDetail {
                five_hour: Meter {
                    fraction: 0.95,
                    used_usd: 47.5,
                    ceiling_usd: 50.0,
                    resets_at: "2026-10-14T15:00:00Z".to_string(),
                },
                ..detail()
            };
            let terminal = draw_with(&hot, None, id, Some("truecolor"), (120, 40));
            assert_eq!(bar_fg(&terminal, "5-hour  █"), theme.meter_high);
        }
    }

    #[test]
    fn thresholds_split_at_sixty_and_ninety_percent() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        assert_eq!(meter_color(&theme, 0.59), theme.meter_low);
        assert_eq!(meter_color(&theme, 0.6), theme.meter_mid);
        assert_eq!(meter_color(&theme, 0.9), theme.meter_high);
    }

    #[test]
    fn empty_history_and_daily_draw_one_dim_line_each() {
        let empty = SpendDetail {
            history: vec![],
            daily: vec![],
            ..detail()
        };
        let terminal = draw_with(&empty, None, ThemeId::Regatta, Some("truecolor"), (120, 40));
        let text = screen(&terminal);
        assert!(text.contains("no meter history yet"), "{text}");
        assert!(text.contains("no daily cost yet"), "{text}");
    }

    #[test]
    fn small_areas_draw_without_panicking_and_keep_the_meters() {
        for size in [
            (100, 30),
            (80, 24),
            (40, 12),
            (20, 6),
            (10, 3),
            (1, 1),
            (0, 0),
        ] {
            let terminal = draw(ThemeId::Regatta, size);
            if size.1 >= 6 && size.0 >= 40 {
                assert!(screen(&terminal).contains("5-hour"), "{size:?}");
            }
        }
    }

    #[test]
    fn a_256_color_terminal_draws_no_rgb_cells() {
        let w = window(true);
        for id in THEMES {
            let terminal = draw_with(&detail(), Some(("5-hour", &w)), id, None, (120, 40));
            let buffer = terminal.backend().buffer();
            let rgb = buffer
                .content()
                .iter()
                .any(|c| [c.fg, c.bg].iter().any(|k| matches!(k, Color::Rgb(..))));
            assert!(!rgb, "{id:?} drew a truecolor cell without COLORTERM");
        }
    }
}
