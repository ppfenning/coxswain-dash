//! The board's throughput panel: the system token series stacked in three layers, a legend with
//! the latest tokens per minute, and a one-line mini graph per host.

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

const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
const LAYER_NAMES: [&str; 3] = ["in", "out", "cache"];
const LABEL_WIDTH: usize = 10;
const GRAPH_MIN_ROWS: u16 = 2;

/// The newest `width` values, right aligned: a short series pads on the left with `None`.
fn window<T: Copy>(values: &[T], width: usize) -> Vec<Option<T>> {
    let kept = &values[values.len().saturating_sub(width)..];
    (0..width - kept.len())
        .map(|_| None)
        .chain(kept.iter().copied().map(Some))
        .collect()
}

/// A series cut to `width` cells and the largest value among the cells shown.
#[derive(Debug, PartialEq)]
struct Scaled {
    cells: Vec<Option<u64>>,
    max: u64,
}

fn scale(values: &[u64], width: usize) -> Scaled {
    let cells = window(values, width);
    let max = cells.iter().flatten().copied().max().unwrap_or(0);
    Scaled { cells, max }
}

/// `value` as eighths of a cell out of `total` eighths, rounded; zero when `max` is zero.
fn eighths(value: u64, max: u64, total: usize) -> usize {
    if max == 0 {
        0
    } else {
        ((u128::from(value) * total as u128 + u128::from(max) / 2) / u128::from(max)) as usize
    }
}

fn layers(p: &ThroughputPoint) -> [u64; 3] {
    [p.input, p.output, p.cache_read]
}

fn total(p: &ThroughputPoint) -> u64 {
    layers(p).iter().sum()
}

fn is_idle(points: &[ThroughputPoint]) -> bool {
    points.iter().all(|p| total(p) == 0)
}

fn glyph(eighths: usize) -> char {
    match eighths {
        0 => ' ',
        n => BLOCKS[n.min(8) - 1],
    }
}

/// One column of the stacked graph, top row first: each cell's glyph and the layer that owns it.
/// The stack is input at the bottom, then output, then cache read, scaled so `max` fills `rows`.
fn column(values: [u64; 3], max: u64, rows: usize) -> Vec<(char, usize)> {
    let total = rows * 8;
    let c0 = values[0];
    let c1 = c0 + values[1];
    let tops = [c0, c1, c1 + values[2]].map(|c| eighths(c, max, total));
    (0..rows)
        .rev()
        .map(|row| {
            let base = row * 8;
            let fill = tops[2].saturating_sub(base).min(8);
            let owner = tops
                .iter()
                .position(|&t| t > (base + fill).saturating_sub(1))
                .unwrap_or(2);
            (glyph(fill), owner)
        })
        .collect()
}

/// The stacked graph as `rows` rows, top first, each `width` cells of glyph and layer.
fn grid(points: &[ThroughputPoint], width: usize, rows: usize) -> Vec<Vec<(char, usize)>> {
    let values: Vec<[u64; 3]> = points.iter().map(layers).collect();
    let cols = window(&values, width);
    let max = cols
        .iter()
        .flatten()
        .map(|v| v.iter().sum())
        .max()
        .unwrap_or(0);
    let columns: Vec<Vec<(char, usize)>> = cols
        .iter()
        .map(|c| match c {
            Some(v) => column(*v, max, rows),
            None => vec![(' ', 0); rows],
        })
        .collect();
    (0..rows)
        .map(|row| columns.iter().map(|c| c[row]).collect())
        .collect()
}

/// A host's totals as one row of block glyphs scaled to its own maximum; missing cells are blank.
fn mini_line(values: &[u64], width: usize) -> String {
    let Scaled { cells, max } = scale(values, width);
    cells
        .iter()
        .map(|c| match c {
            Some(v) => BLOCKS[eighths(*v, max, 8).clamp(1, 8) - 1],
            None => ' ',
        })
        .collect()
}

fn compact(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

fn label(host: &str) -> String {
    let name: String = host.chars().take(LABEL_WIDTH).collect();
    format!("{name:<LABEL_WIDTH$} ")
}

pub fn render(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let throughput = app.snapshot().map(|s| &s.live.throughput);
    let system = throughput.map_or(&[][..], |t| t.system.as_slice());
    let hosts = throughput.map_or(&[][..], |t| t.hosts.as_slice());
    let block = Block::bordered().title("throughput tok/min");
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height == 0 {
        return;
    }
    if is_idle(system) {
        let middle = Rect::new(inner.x, inner.y + inner.height / 2, inner.width, 1);
        f.render_widget(
            Paragraph::new("idle")
                .alignment(Alignment::Center)
                .style(Style::new().fg(theme.dim)),
            middle,
        );
        return;
    }
    let colors = [
        theme.machine_accents[0],
        theme.machine_accents[1],
        theme.machine_accents[2],
    ];
    let host_rows = hosts
        .len()
        .min(inner.height.saturating_sub(1 + GRAPH_MIN_ROWS) as usize) as u16;
    let [legend, graph, host_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(host_rows),
    ])
    .areas(inner);

    let latest = system.last().map_or([0; 3], layers);
    let legend_spans: Vec<Span> = (0..3)
        .flat_map(|i| {
            [
                Span::styled("■ ", Style::new().fg(colors[i])),
                Span::styled(
                    format!("{} {}  ", LAYER_NAMES[i], compact(latest[i])),
                    Style::new().fg(theme.fg),
                ),
            ]
        })
        .collect();
    f.render_widget(Paragraph::new(Line::from(legend_spans)), legend);

    let rows: Vec<Line> = grid(system, graph.width as usize, graph.height as usize)
        .into_iter()
        .map(|row| {
            Line::from(
                row.into_iter()
                    .map(|(g, layer)| Span::styled(g.to_string(), Style::new().fg(colors[layer])))
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    f.render_widget(Paragraph::new(rows), graph);

    let spark_width = (host_area.width as usize).saturating_sub(LABEL_WIDTH + 1);
    let host_lines: Vec<Line> = hosts
        .iter()
        .take(host_rows as usize)
        .map(|h| {
            let totals: Vec<u64> = h.series.iter().map(total).collect();
            Line::from(vec![
                Span::styled(label(&h.host), Style::new().fg(theme.dim)),
                Span::styled(
                    mini_line(&totals, spark_width),
                    Style::new().fg(theme.accent),
                ),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(host_lines), host_area);
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::app::{AppPage, ThemeId};

    fn app_with_live(live_json: &str) -> App {
        let json = format!(
            r#"{{"schema":1,"at":"2026-09-29T00:00:00Z","chair":{{"holder":"h","host":"h","epoch":1,"liveness":"live","beat_age_s":0}},"spend":{{"five_hour_fraction":0.0,"five_hour_source":"meter","weekly_fraction":0.0,"weekly_source":"meter","hard_stop_fraction":0.0,"five_hour_resets_at":"2026-09-29T00:00:00Z","weekly_resets_at":"2026-09-29T00:00:00Z"}},"machines":[],"runs":[],"queue":[],"inbox":[],"watch":[],"live":{live_json}}}"#
        );
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.apply_snapshot(crate::feed::parse_snapshot(&json).expect("literal snapshot parses"));
        app
    }

    const TWO_HOSTS: &str = r#"{"throughput":{"system":[
        {"at":"a","input":100,"output":50,"cache_read":400},
        {"at":"b","input":200,"output":80,"cache_read":900},
        {"at":"c","input":300,"output":120,"cache_read":1500},
        {"at":"d","input":250,"output":100,"cache_read":2600},
        {"at":"e","input":400,"output":150,"cache_read":3000},
        {"at":"f","input":150,"output":60,"cache_read":1200},
        {"at":"g","input":600,"output":240,"cache_read":4200},
        {"at":"h","input":520,"output":200,"cache_read":3900}],
      "hosts":[
        {"host":"atlas","series":[
          {"at":"a","input":50,"output":20,"cache_read":200},
          {"at":"b","input":120,"output":50,"cache_read":500},
          {"at":"c","input":300,"output":100,"cache_read":1900},
          {"at":"d","input":220,"output":90,"cache_read":1500}]},
        {"host":"borealis","series":[
          {"at":"c","input":0,"output":0,"cache_read":0},
          {"at":"d","input":300,"output":110,"cache_read":2000}]}]}}"#;

    fn draw(app: &App, theme: &Theme, w: u16, h: u16) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("terminal");
        terminal
            .draw(|f| render(f, f.area(), app, theme))
            .expect("draw");
        terminal
    }

    #[test]
    fn the_system_graph_with_a_mini_graph_per_host() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let terminal = draw(&app_with_live(TWO_HOSTS), &theme, 48, 8);
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn every_point_zero_draws_an_empty_frame_with_idle() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let app = app_with_live(
            r#"{"throughput":{"system":[{"at":"a","input":0,"output":0,"cache_read":0}]}}"#,
        );
        let text = draw(&app, &theme, 30, 6).backend().to_string();
        assert!(text.contains("idle"), "{text}");
        assert!(!text.contains("■"), "{text}");
    }

    #[test]
    fn a_zero_series_scales_to_zeros_with_max_zero() {
        assert_eq!(
            scale(&[0, 0, 0], 3),
            Scaled {
                cells: vec![Some(0), Some(0), Some(0)],
                max: 0
            }
        );
        assert_eq!(eighths(0, 0, 16), 0);
    }

    #[test]
    fn a_single_point_sits_at_the_right_edge() {
        assert_eq!(
            scale(&[7], 4),
            Scaled {
                cells: vec![None, None, None, Some(7)],
                max: 7
            }
        );
        assert_eq!(mini_line(&[7], 4), "   █");
    }

    #[test]
    fn a_series_longer_than_the_width_keeps_the_newest_points() {
        assert_eq!(
            scale(&[9, 1, 2, 3], 3),
            Scaled {
                cells: vec![Some(1), Some(2), Some(3)],
                max: 3
            }
        );
    }

    #[test]
    fn eighths_scale_to_the_maximum_and_round() {
        assert_eq!(eighths(10, 10, 16), 16);
        assert_eq!(eighths(5, 10, 16), 8);
        assert_eq!(eighths(1, 10, 8), 1);
    }

    #[test]
    fn a_column_stacks_input_then_output_then_cache_from_the_bottom() {
        // Two rows, 16 eighths: input 4, output 4, cache 4 of max 16.
        assert_eq!(column([4, 4, 4], 16, 2), vec![('▄', 2), ('█', 1)]);
    }

    #[test]
    fn compact_counts_use_k_and_m() {
        assert_eq!(compact(999), "999");
        assert_eq!(compact(1_500), "1.5k");
        assert_eq!(compact(2_500_000), "2.5M");
    }
}
