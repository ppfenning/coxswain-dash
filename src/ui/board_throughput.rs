//! The board's throughput panel: the system token series as three overlaid layers, then one mini
//! graph per host. Scaling and bucketing are pure functions over plain data.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Paragraph},
};

use crate::app::App;
use crate::feed::ThroughputPoint;
use crate::theme::Theme;

/// Eighth-block glyphs, one per fill level 1..=8.
const RAMP: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
const HOST_LABEL_MAX: usize = 10;

type Pick = fn(&ThroughputPoint) -> u64;
/// Input, output and cache read, in drawing and legend order.
const LAYERS: [(&str, Pick); 3] = [
    ("input", |p| p.input),
    ("output", |p| p.output),
    ("cache", |p| p.cache_read),
];

/// The glyph for a cell filled `fill` eighths high. Zero is blank.
fn glyph(fill: usize) -> char {
    match fill {
        0 => ' ',
        n => RAMP[n.min(8) - 1],
    }
}

/// Heights in eighths of a row, 0..=`rows * 8`, one per column. The newest `width` values are kept
/// and a shorter series is right aligned, padded with zeros on the left. A nonzero value is at
/// least one eighth high. A zero `max` gives all zeros.
fn scale(values: &[u64], width: usize, max: u64, rows: usize) -> Vec<usize> {
    let total = rows * 8;
    let kept = &values[values.len().saturating_sub(width)..];
    let height = |v: u64| match max {
        0 => 0,
        m => ((u128::from(v) * total as u128).div_ceil(u128::from(m))).min(total as u128) as usize,
    };
    (0..width - kept.len())
        .map(|_| 0)
        .chain(kept.iter().map(|&v| height(v)))
        .collect()
}

/// One cell per column and row, top row first, as a glyph and the index of the layer that owns it.
/// Where layers overlap the lowest one is drawn, so the smaller layer shows in front of a taller.
fn graph_rows(layers: &[Vec<usize>; 3], rows: usize) -> Vec<Vec<(char, usize)>> {
    let width = layers[0].len();
    let cell = |x: usize, row: usize| {
        layers
            .iter()
            .enumerate()
            .map(|(i, heights)| (heights[x], i))
            .filter(|&(h, _)| h > row * 8)
            .min()
            .map_or((' ', 0), |(h, i)| (glyph(h - row * 8), i))
    };
    (0..rows)
        .rev()
        .map(|row| (0..width).map(|x| cell(x, row)).collect())
        .collect()
}

/// A one-line graph from heights already scaled to a single row.
fn spark(heights: &[usize]) -> String {
    heights.iter().map(|&h| glyph(h)).collect()
}

fn layer(points: &[ThroughputPoint], pick: Pick) -> Vec<u64> {
    points.iter().map(pick).collect()
}

/// The newest value of a layer, or zero for an empty series.
fn latest(points: &[ThroughputPoint], pick: Pick) -> u64 {
    points.last().map_or(0, pick)
}

fn total(p: &ThroughputPoint) -> u64 {
    p.input + p.output + p.cache_read
}

/// The largest single-layer value in the series.
fn series_max(points: &[ThroughputPoint]) -> u64 {
    LAYERS
        .iter()
        .flat_map(|&(_, pick)| points.iter().map(pick))
        .max()
        .unwrap_or(0)
}

fn is_idle(points: &[ThroughputPoint]) -> bool {
    series_max(points) == 0
}

fn fmt_tokens(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

fn layer_colors(theme: &Theme) -> [ratatui::style::Color; 3] {
    [
        theme.machine_accents[0],
        theme.machine_accents[1],
        theme.machine_accents[2],
    ]
}

fn legend(points: &[ThroughputPoint], theme: &Theme) -> Line<'static> {
    let spans = LAYERS
        .iter()
        .zip(layer_colors(theme))
        .flat_map(|(&(name, pick), color)| {
            [
                Span::styled("■ ", Style::new().fg(color)),
                Span::styled(
                    format!("{name} {}  ", fmt_tokens(latest(points, pick))),
                    Style::new().fg(theme.fg),
                ),
            ]
        })
        .collect::<Vec<_>>();
    Line::from(spans)
}

fn host_line(
    host: &str,
    series: &[ThroughputPoint],
    label_w: usize,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    let label: String = host.chars().take(label_w).collect();
    let graph_w = width.saturating_sub(label_w + 1);
    let totals = layer(series, total);
    let max = totals.iter().copied().max().unwrap_or(0);
    Line::from(vec![
        Span::styled(format!("{label:<label_w$} "), Style::new().fg(theme.dim)),
        Span::styled(
            spark(&scale(&totals, graph_w, max, 1)),
            Style::new().fg(theme.accent),
        ),
    ])
}

fn render_idle(f: &mut Frame, inner: Rect, theme: &Theme) {
    let middle = Rect {
        y: inner.y + inner.height / 2,
        height: inner.height.min(1),
        ..inner
    };
    f.render_widget(
        Paragraph::new(Span::styled("idle", Style::new().fg(theme.dim)))
            .alignment(Alignment::Center),
        middle,
    );
}

pub fn render(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let block = Block::bordered()
        .title("throughput tok/min")
        .border_style(Style::new().fg(theme.border));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    let tp = app.snapshot().map(|s| &s.live.throughput);
    let Some(tp) = tp.filter(|t| !is_idle(&t.system)) else {
        return render_idle(f, inner, theme);
    };
    let width = usize::from(inner.width);
    let below_legend = inner.height - 1;
    let hosts_shown = tp
        .hosts
        .len()
        .min(usize::from(below_legend.saturating_sub(1)));
    let graph_h = below_legend - hosts_shown as u16;
    let [legend_area, graph_area, hosts_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(graph_h),
        Constraint::Length(hosts_shown as u16),
    ])
    .areas(inner);

    f.render_widget(Paragraph::new(legend(&tp.system, theme)), legend_area);

    let max = series_max(&tp.system);
    let layers =
        LAYERS.map(|(_, pick)| scale(&layer(&tp.system, pick), width, max, usize::from(graph_h)));
    let colors = layer_colors(theme);
    let rows = graph_rows(&layers, usize::from(graph_h))
        .into_iter()
        .map(|row| {
            Line::from(
                row.into_iter()
                    .map(|(c, i)| Span::styled(c.to_string(), Style::new().fg(colors[i])))
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    f.render_widget(Paragraph::new(rows), graph_area);

    let label_w = tp
        .hosts
        .iter()
        .take(hosts_shown)
        .map(|h| h.host.chars().count())
        .max()
        .unwrap_or(0)
        .min(HOST_LABEL_MAX);
    let lines = tp
        .hosts
        .iter()
        .take(hosts_shown)
        .map(|h| host_line(&h.host, &h.series, label_w, width, theme))
        .collect::<Vec<_>>();
    f.render_widget(Paragraph::new(lines), hosts_area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppPage, ThemeId};
    use ratatui::{Terminal, backend::TestBackend};

    const FEED_HEAD: &str = r#"{"schema":1,"at":"2026-09-29T00:00:00Z","chair":{"holder":"h","host":"h","epoch":1,"liveness":"live","beat_age_s":0},"spend":{"five_hour_fraction":0.0,"five_hour_source":"meter","weekly_fraction":0.0,"weekly_source":"meter","hard_stop_fraction":0.0,"five_hour_resets_at":"2026-09-29T00:00:00Z","weekly_resets_at":"2026-09-29T00:00:00Z"},"machines":[],"runs":[],"queue":[],"inbox":[],"watch":[],"live":"#;

    fn point(input: u64, output: u64, cache_read: u64) -> String {
        format!(r#"{{"at":"t","input":{input},"output":{output},"cache_read":{cache_read}}}"#)
    }

    fn series(points: &[(u64, u64, u64)]) -> String {
        let items: Vec<String> = points.iter().map(|&(i, o, c)| point(i, o, c)).collect();
        format!("[{}]", items.join(","))
    }

    fn app_with_live(system: &str, hosts: &str) -> App {
        let line =
            format!(r#"{FEED_HEAD}{{"throughput":{{"system":{system},"hosts":{hosts}}}}}}}"#);
        let mut app = App::new(AppPage::Board, ThemeId::Regatta);
        app.apply_snapshot(crate::feed::parse_snapshot(&line).expect("literal feed parses"));
        app
    }

    fn draw(app: &App, width: u16, height: u16) -> String {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|f| render(f, f.area(), app, &theme))
            .expect("draw should not fail");
        terminal.backend().to_string()
    }

    #[test]
    fn scale_of_a_zero_series_is_all_zero() {
        assert_eq!(scale(&[0, 0, 0], 5, 0, 2), vec![0, 0, 0, 0, 0]);
    }

    #[test]
    fn scale_of_a_single_point_sits_at_the_right_edge() {
        assert_eq!(scale(&[7], 4, 7, 2), vec![0, 0, 0, 16]);
    }

    #[test]
    fn scale_of_a_series_longer_than_the_width_keeps_the_newest_points() {
        assert_eq!(scale(&[100, 1, 4, 8], 3, 8, 1), vec![1, 4, 8]);
    }

    #[test]
    fn graph_rows_draws_the_lower_layer_in_front() {
        let layers = [vec![0, 16], vec![4, 8], vec![0, 0]];
        let rows = graph_rows(&layers, 2);
        assert_eq!(rows[0], vec![(' ', 0), ('█', 0)]);
        assert_eq!(rows[1], vec![('▄', 1), ('█', 1)]);
    }

    #[test]
    fn spark_and_summaries_of_literals() {
        assert_eq!(spark(&[0, 4, 8]), " ▄█");
        let pts = [ThroughputPoint {
            input: 9,
            output: 2,
            cache_read: 5,
            ..Default::default()
        }];
        assert_eq!((series_max(&pts), latest(&pts, |p| p.output)), (9, 2));
        assert!(is_idle(&[]) && !is_idle(&pts));
        assert_eq!(fmt_tokens(12_340), "12.3k");
    }

    #[test]
    fn the_system_graph_with_a_mini_graph_per_host() {
        let system = series(&[
            (100, 20, 900),
            (300, 60, 1200),
            (500, 80, 2400),
            (200, 40, 3000),
            (400, 100, 2000),
            (800, 160, 4200),
            (600, 120, 3600),
            (700, 140, 5200),
        ]);
        let hosts = format!(
            r#"[{{"host":"host-a","series":{}}},{{"host":"host-b","series":{}}}]"#,
            series(&[
                (50, 10, 400),
                (150, 30, 600),
                (350, 70, 2600),
                (250, 50, 3100)
            ]),
            series(&[(0, 0, 100), (100, 10, 300), (400, 90, 800)]),
        );
        let app = app_with_live(&system, &hosts);
        insta::assert_snapshot!(draw(&app, 48, 8));
    }

    #[test]
    fn an_all_zero_series_draws_an_empty_frame_saying_idle() {
        let app = app_with_live(&series(&[(0, 0, 0), (0, 0, 0)]), "[]");
        assert!(draw(&app, 48, 8).contains("idle"));
    }

    #[test]
    fn a_tiny_area_draws_without_panicking() {
        let app = app_with_live(&series(&[(1, 2, 3)]), r#"[{"host":"h","series":[]}]"#);
        (1..5).for_each(|n| {
            draw(&app, n, n);
        });
    }
}
