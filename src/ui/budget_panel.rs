//! Pure line builders for the top of the budget view. Nothing here draws or reads app state.
//! `today_average_lines` and `no_limit_lines` take the feed's `spend.today` and `budget` blocks,
//! because only `SpendToday.cost` can be null. Above 60 columns the full wording shows; at or
//! below it the secondary words drop before the bar shrinks, and no line exceeds its width.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use crate::feed::{Budget, SpendToday, Verdict};
use crate::theme::SlipstreamPalette;

/// Widths above this show the full wording.
const WIDE_ABOVE: u16 = 60;
const BAR_MIN: usize = 8;
const BAR_MAX: usize = 24;

/// Rows `today_average_lines` returns.
const TODAY_ROWS: u16 = 1;
/// Rows `no_limit_lines` returns.
const NO_LIMIT_ROWS: u16 = 2;
/// Rows `budget_lines` returns when a month budget is set.
const BUDGET_ROWS: u16 = 3;

fn usd_or_dash(value: Option<f64>) -> String {
    value.map_or_else(|| "--".to_string(), |usd| format!("${usd:.2}"))
}

/// Cuts the spans to at most `width` columns. Every glyph used here is one column wide.
fn fit(spans: Vec<Span<'static>>, width: u16) -> Line<'static> {
    let mut left = usize::from(width);
    let kept = spans
        .into_iter()
        .filter_map(|span| {
            let n = span.content.chars().count();
            if left == 0 {
                None
            } else if n <= left {
                left -= n;
                Some(span)
            } else {
                let cut: String = span.content.chars().take(left).collect();
                left = 0;
                Some(Span::styled(cut, span.style))
            }
        })
        .collect::<Vec<_>>();
    Line::from(kept)
}

fn verdict_color(verdict: Verdict, p: &SlipstreamPalette) -> Color {
    match verdict {
        Verdict::OnTrack => p.done_chip,
        // `pending_chip` is a chip fill close to the ground, so over pace takes the warm accent.
        Verdict::OverPace => p.accents[1],
        Verdict::OverBudget => p.failed_chip,
    }
}

fn verdict_word(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::OnTrack => "on track",
        Verdict::OverPace => "over pace",
        Verdict::OverBudget => "over budget",
    }
}

/// Today's spend against the rolling daily average. A marker shows above or below; it is absent
/// when either figure is null or the two are equal.
pub fn today_average_lines(
    today: Option<&SpendToday>,
    budget: Option<&Budget>,
    width: u16,
    p: &SlipstreamPalette,
) -> Vec<Line<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let cost = today.and_then(|t| t.cost);
    let avg = budget.map(|b| b.daily_avg_usd);
    let (glyph, color, word) = match (cost, avg) {
        (Some(c), Some(a)) if c > a => ("▲", p.failed_chip, "above avg"),
        (Some(c), Some(a)) if c < a => ("▼", p.done_chip, "below avg"),
        _ => ("", p.dim, ""),
    };
    let marker = if glyph.is_empty() {
        String::new()
    } else if width > WIDE_ABOVE {
        format!("  {glyph} {word}")
    } else {
        format!("  {glyph}")
    };
    let label = Style::default().fg(p.dim);
    let value = Style::default().fg(p.text);
    vec![fit(
        vec![
            Span::styled("Today ", label),
            Span::styled(usd_or_dash(cost), value),
            Span::styled("  avg ", label),
            Span::styled(usd_or_dash(avg), value),
            Span::styled("/day", label),
            Span::styled(marker, Style::default().fg(color)),
        ],
        width,
    )]
}

/// The month-to-date bar, the projection with days left, and the verdict. Empty without a month
/// budget, so a zero or missing budget draws no bar.
pub fn budget_lines(budget: &Budget, width: u16, p: &SlipstreamPalette) -> Vec<Line<'static>> {
    let Some(verdict) = budget.verdict().filter(|_| width > 0) else {
        return Vec::new();
    };
    let wide = width > WIDE_ABOVE;
    let color = verdict_color(verdict, p);
    let dim = Style::default().fg(p.dim);
    let text = Style::default().fg(p.text);

    let ratio = budget.spent_usd / budget.budget_usd;
    let pct = format!("{:.0}%", ratio * 100.0);
    let amounts = format!("${:.2}/${:.2}", budget.spent_usd, budget.budget_usd);
    let room = usize::from(width).saturating_sub(pct.len() + amounts.len() + 3);
    let (bar_w, show_amounts) = if room >= BAR_MIN {
        (room.min(BAR_MAX), true)
    } else {
        let bare = usize::from(width).saturating_sub(pct.len() + 1);
        (bare.min(BAR_MAX), false)
    };
    let filled = ((ratio.clamp(0.0, 1.0) * bar_w as f64).round() as usize).min(bar_w);
    let bar = vec![
        Span::styled("█".repeat(filled), Style::default().fg(color)),
        Span::styled("░".repeat(bar_w - filled), dim),
        Span::styled(format!(" {pct}"), text),
        Span::styled(
            if show_amounts {
                format!("  {amounts}")
            } else {
                String::new()
            },
            dim,
        ),
    ];

    let days = if wide {
        let unit = if budget.days_left == 1 { "day" } else { "days" };
        format!("{} {unit} left", budget.days_left)
    } else {
        format!("{}d", budget.days_left)
    };
    let projection = if wide {
        vec![
            Span::styled("Projected ", dim),
            Span::styled(format!("${:.2}", budget.projected_usd), text),
            Span::styled(" by month end  ·  ", dim),
            Span::styled(days, text),
        ]
    } else {
        vec![
            Span::styled("proj ", dim),
            Span::styled(format!("${:.2}", budget.projected_usd), text),
            Span::styled("  ", dim),
            Span::styled(days, text),
        ]
    };

    let over_by = budget.spent_usd - budget.budget_usd;
    let suffix = match (verdict, wide) {
        (Verdict::OnTrack, true) => " · projected within budget".to_string(),
        (Verdict::OverPace, true) => format!(
            " · ${:.2} over by month end",
            budget.projected_usd - budget.budget_usd
        ),
        (Verdict::OverBudget, _) if over_by >= 0.005 => format!(" by ${over_by:.2}, bar full"),
        (Verdict::OverBudget, _) => ", bar full".to_string(),
        _ => String::new(),
    };
    let verdict_line = vec![
        Span::styled(
            verdict_word(verdict),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(suffix, dim),
    ];

    vec![
        fit(bar, width),
        fit(projection, width),
        fit(verdict_line, width),
    ]
}

/// Today and the daily average, plus a line saying no limit is configured.
pub fn no_limit_lines(
    today: Option<&SpendToday>,
    budget: Option<&Budget>,
    width: u16,
    p: &SlipstreamPalette,
) -> Vec<Line<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let note = fit(
        vec![Span::styled(
            "no limit configured",
            Style::default().fg(p.dim),
        )],
        width,
    );
    today_average_lines(today, budget, width, p)
        .into_iter()
        .chain(std::iter::once(note))
        .collect()
}

/// The three builders, named so `lines_height` can answer for each.
pub enum Section<'a> {
    TodayAverage,
    Budget(Option<&'a Budget>),
    NoLimit,
}

/// Rows the builder for `section` returns at `width`. No line wraps, so only a zero width, which
/// draws nothing, changes the count.
pub fn lines_height(width: u16, section: Section<'_>) -> u16 {
    match (width, section) {
        (0, _) => 0,
        (_, Section::TodayAverage) => TODAY_ROWS,
        (_, Section::Budget(budget)) => budget.and_then(Budget::verdict).map_or(0, |_| BUDGET_ROWS),
        (_, Section::NoLimit) => NO_LIMIT_ROWS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::slipstream_palette;

    fn budget(spent: f64, limit: f64, projected: f64) -> Budget {
        Budget {
            period: "month".to_string(),
            spent_usd: spent,
            budget_usd: limit,
            fraction: if limit == 0.0 { 0.0 } else { spent / limit },
            daily_avg_usd: 10.0,
            projected_usd: projected,
            days_left: 12,
            daily_avg_7d_usd: 0.0,
            daily_avg_14d_usd: 0.0,
        }
    }

    fn today(cost: Option<f64>) -> SpendToday {
        SpendToday {
            date: "2026-11-04".to_string(),
            cost,
            landed: None,
            per_landed: None,
            first_try_rate: None,
            measured: None,
        }
    }

    fn text(lines: &[Line<'static>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn on_track_reads_in_full_at_70_columns() {
        let p = slipstream_palette();
        let lines = budget_lines(&budget(200.0, 500.0, 450.0), 70, &p);
        let t = text(&lines);
        assert!(t[0].contains("40%  $200.00/$500.00"), "{}", t[0]);
        assert_eq!(t[1], "Projected $450.00 by month end  ·  12 days left");
        assert_eq!(t[2], "on track · projected within budget");
        assert_eq!(lines[2].spans[0].style.fg, Some(p.done_chip));
    }

    #[test]
    fn over_pace_is_colored_and_names_the_overshoot() {
        let p = slipstream_palette();
        let lines = budget_lines(&budget(200.0, 500.0, 620.0), 70, &p);
        assert_eq!(text(&lines)[2], "over pace · $120.00 over by month end");
        assert_eq!(lines[2].spans[0].style.fg, Some(p.accents[1]));
    }

    #[test]
    fn every_verdict_and_bar_color_is_readable_in_both_themes() {
        use crate::app::ThemeId;
        use crate::theme::slipstream_palette_for;
        let cases = [
            budget(200.0, 500.0, 450.0),
            budget(200.0, 500.0, 620.0),
            budget(540.0, 500.0, 700.0),
        ];
        for id in [ThemeId::Regatta, ThemeId::HarborLight] {
            let p = slipstream_palette_for(id);
            let mut seen = Vec::new();
            for b in &cases {
                let lines = budget_lines(b, 70, &p);
                let word = lines[2].spans[0].style.fg.expect("verdict word is colored");
                let fill = lines[0].spans[0].style.fg.expect("bar fill is colored");
                assert_eq!(word, fill);
                for bad in [p.ground, p.card, p.dim, p.pending_chip] {
                    assert_ne!(word, bad, "{id:?}: verdict color is unreadable");
                }
                seen.push(word);
            }
            assert_ne!(seen[0], seen[1]);
            assert_ne!(seen[1], seen[2]);
            assert_ne!(seen[0], seen[2]);
        }
    }

    #[test]
    fn over_budget_says_bar_full_at_every_width_and_at_exactly_the_budget() {
        let p = slipstream_palette();
        let narrow = text(&budget_lines(&budget(540.0, 500.0, 700.0), 40, &p));
        assert_eq!(narrow[2], "over budget by $40.00, bar full");
        let exact = text(&budget_lines(&budget(500.0, 500.0, 700.0), 70, &p));
        assert_eq!(exact[2], "over budget, bar full");
    }

    #[test]
    fn over_budget_clamps_the_bar_and_says_so() {
        let p = slipstream_palette();
        let lines = budget_lines(&budget(540.0, 500.0, 700.0), 70, &p);
        let t = text(&lines);
        assert!(t[0].starts_with(&"█".repeat(BAR_MAX)), "{}", t[0]);
        assert!(!t[0].contains('░'));
        assert!(t[0].contains("108%"));
        assert_eq!(t[2], "over budget by $40.00, bar full");
        assert_eq!(lines[2].spans[0].style.fg, Some(p.failed_chip));
    }

    #[test]
    fn narrow_width_drops_secondary_words_but_keeps_the_bar() {
        let p = slipstream_palette();
        let t = text(&budget_lines(&budget(200.0, 500.0, 450.0), 40, &p));
        assert_eq!(t[1], "proj $450.00  12d");
        assert_eq!(t[2], "on track");
        assert_eq!(t[0].chars().filter(|c| matches!(c, '█' | '░')).count(), 19);
    }

    #[test]
    fn zero_or_missing_budget_draws_no_bar() {
        let p = slipstream_palette();
        let zero = budget(200.0, 0.0, 450.0);
        assert!(budget_lines(&zero, 70, &p).is_empty());
        assert_eq!(lines_height(70, Section::Budget(Some(&zero))), 0);
        assert_eq!(lines_height(70, Section::Budget(None)), 0);
    }

    #[test]
    fn a_null_cost_prints_a_dash_and_no_marker() {
        let p = slipstream_palette();
        let b = budget(200.0, 500.0, 450.0);
        let t = text(&today_average_lines(Some(&today(None)), Some(&b), 70, &p));
        assert_eq!(t, vec!["Today --  avg $10.00/day".to_string()]);
    }

    #[test]
    fn the_marker_follows_today_against_the_average() {
        let p = slipstream_palette();
        let b = budget(200.0, 500.0, 450.0);
        let above = text(&today_average_lines(
            Some(&today(Some(12.0))),
            Some(&b),
            70,
            &p,
        ));
        assert_eq!(above[0], "Today $12.00  avg $10.00/day  ▲ above avg");
        let below = text(&today_average_lines(
            Some(&today(Some(4.0))),
            Some(&b),
            40,
            &p,
        ));
        assert_eq!(below[0], "Today $4.00  avg $10.00/day  ▼");
    }

    #[test]
    fn no_limit_says_so_and_dashes_a_missing_average() {
        let p = slipstream_palette();
        let t = text(&no_limit_lines(Some(&today(Some(3.0))), None, 70, &p));
        assert_eq!(t, vec!["Today $3.00  avg --/day", "no limit configured"]);
    }

    #[test]
    fn no_line_exceeds_the_width_and_heights_match() {
        let p = slipstream_palette();
        let b = budget(1234.5, 5000.0, 6100.0);
        let over = budget(5400.0, 5000.0, 7000.0);
        let t = today(Some(120.0));
        for width in [0u16, 10, 40, 70] {
            let w = usize::from(width);
            let groups = [
                budget_lines(&b, width, &p),
                budget_lines(&over, width, &p),
                today_average_lines(Some(&t), Some(&b), width, &p),
                no_limit_lines(Some(&t), None, width, &p),
            ];
            for line in groups.iter().flatten() {
                assert!(line.width() <= w, "{width}: {:?}", line);
            }
            let heights = [
                lines_height(width, Section::Budget(Some(&b))),
                lines_height(width, Section::Budget(Some(&over))),
                lines_height(width, Section::TodayAverage),
                lines_height(width, Section::NoLimit),
            ];
            for (group, height) in groups.iter().zip(heights) {
                assert_eq!(group.len(), usize::from(height), "width {width}");
            }
        }
    }
}
