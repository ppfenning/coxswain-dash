//! The board's machine strip: one bordered panel per host, side by side, each with a cell per lane
//! slot, a CPU bar and a memory bar. The bottom two rows of every panel belong to the schedule.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
};

use super::board_schedule;
use super::chair_card::short_age;
use super::status::{self, Status};
use crate::app::App;
use crate::feed::{Machine, MachineLoad};
use crate::theme::Theme;

/// A panel narrower than this cannot show a lane label and a bar.
const MIN_PANEL_WIDTH: u16 = 24;
/// Room kept at the right for the `+N` marker when hosts are cut.
const MARKER_WIDTH: u16 = 4;
/// Inner rows at the bottom of a panel left for `board_schedule`.
const SCHEDULE_ROWS: u16 = 2;
const BAR_LABEL_WIDTH: u16 = 4;
const BAR_NUMBER_WIDTH: usize = 7;

/// Whole bar cells filled by `value` of `total` across `width`. A bad or empty total fills none.
fn bar_fill(value: f64, total: f64, width: u16) -> u16 {
    if total.is_nan() || total <= 0.0 || value.is_nan() {
        return 0;
    }
    let ratio = (value / total).clamp(0.0, 1.0);
    (ratio * f64::from(width)).round() as u16
}

/// How many of `host_count` panels fit in `area_width`, and how many are cut at the right.
fn host_width_fit(area_width: u16, host_count: usize) -> (usize, usize) {
    let fits_all = host_count * usize::from(MIN_PANEL_WIDTH) <= usize::from(area_width);
    if fits_all {
        return (host_count, 0);
    }
    let shown = usize::from(area_width.saturating_sub(MARKER_WIDTH) / MIN_PANEL_WIDTH);
    (shown, host_count - shown)
}

/// One entry per lane slot up to capacity: the label of the matching lane, or None when free.
fn lane_cells(machine: &Machine) -> Vec<Option<String>> {
    (0..machine.capacity as usize)
        .map(|slot| {
            machine.lanes.get(slot).map(|lane| {
                format!(
                    "{} {} {}",
                    lane.initiative,
                    lane.phase,
                    short_age(lane.elapsed_s)
                )
            })
        })
        .collect()
}

/// A host beat that carried no load leaves every figure at zero.
fn no_load(load: &MachineLoad) -> bool {
    load.cpu_pct == 0.0
        && load.load_avg_per_core == 0.0
        && load.mem_used_gb == 0.0
        && load.mem_total_gb == 0.0
}

fn cut(text: &str, width: usize) -> String {
    text.chars().take(width).collect()
}

fn meter_color(theme: &Theme, ratio: f64) -> Color {
    if ratio >= 0.85 {
        theme.meter_high
    } else if ratio >= 0.6 {
        theme.meter_mid
    } else {
        theme.meter_low
    }
}

fn bar_line(
    theme: &Theme,
    label: &str,
    value: f64,
    total: f64,
    number: String,
    width: u16,
) -> Line<'static> {
    let bar_width = width.saturating_sub(BAR_LABEL_WIDTH + 1 + BAR_NUMBER_WIDTH as u16);
    let fill = bar_fill(value, total, bar_width);
    let ratio = if total > 0.0 { value / total } else { 0.0 };
    Line::from(vec![
        Span::styled(
            format!("{label:<w$}", w = usize::from(BAR_LABEL_WIDTH)),
            Style::new().fg(theme.dim),
        ),
        Span::styled(
            "\u{2588}".repeat(usize::from(fill)),
            Style::new().fg(meter_color(theme, ratio)),
        ),
        Span::styled(
            "\u{2591}".repeat(usize::from(bar_width - fill)),
            Style::new().fg(theme.track),
        ),
        Span::styled(
            format!(" {number:>w$}", w = BAR_NUMBER_WIDTH),
            Style::new().fg(theme.fg),
        ),
    ])
}

fn render_cells(f: &mut Frame, row: Rect, machine: &Machine, theme: &Theme) {
    let cells = lane_cells(machine);
    let slot_width = (row.width / cells.len().max(1) as u16).max(1);
    for (i, cell) in cells.iter().enumerate() {
        let x = row.x.saturating_add((i as u16).saturating_mul(slot_width));
        let rect = Rect::new(x, row.y, slot_width.saturating_sub(1).max(1), 1).intersection(row);
        if rect.is_empty() {
            continue;
        }
        let width = usize::from(rect.width);
        let widget = match cell {
            Some(label) => Paragraph::new(format!("{:<width$}", cut(label, width))).style(
                Style::new()
                    .fg(theme.bg)
                    .bg(status::color(theme, Status::Running)),
            ),
            None => Paragraph::new("\u{b7}").style(Style::new().fg(theme.dim)),
        };
        f.render_widget(widget, rect);
    }
}

fn render_bars(
    f: &mut Frame,
    cpu_row: Option<Rect>,
    mem_row: Option<Rect>,
    machine: &Machine,
    theme: &Theme,
) {
    let load = &machine.load;
    let bare = no_load(load);
    let number = |text: String| if bare { "-".to_string() } else { text };
    if let Some(row) = cpu_row {
        let value = if bare { 0.0 } else { load.cpu_pct };
        let line = bar_line(
            theme,
            "cpu",
            value,
            100.0,
            number(format!("{:.0}%", load.cpu_pct)),
            row.width,
        );
        f.render_widget(Paragraph::new(line), row);
    }
    if let Some(row) = mem_row {
        let line = bar_line(
            theme,
            "mem",
            load.mem_used_gb,
            load.mem_total_gb,
            number(format!("{:.0}/{:.0}G", load.mem_used_gb, load.mem_total_gb)),
            row.width,
        );
        f.render_widget(Paragraph::new(line), row);
    }
}

fn render_panel(f: &mut Frame, area: Rect, app: &App, theme: &Theme, machine: &Machine) {
    let block = Block::bordered()
        .border_style(Style::new().fg(theme.border))
        .title(Span::styled(
            format!(" {} \u{b7} {} ", machine.name, machine.state),
            Style::new().fg(theme.fg),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let schedule_height = SCHEDULE_ROWS.min(inner.height);
    let content_height = inner.height - schedule_height;
    let row =
        |i: u16| (i < content_height).then(|| Rect::new(inner.x, inner.y + i, inner.width, 1));
    if let Some(cells) = row(0) {
        render_cells(f, cells, machine, theme);
    }
    render_bars(f, row(1), row(2), machine, theme);
    if schedule_height > 0 {
        let schedule = Rect::new(
            inner.x,
            inner.y + content_height,
            inner.width,
            schedule_height,
        );
        board_schedule::render_host(f, schedule, app, theme, &machine.name);
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let machines = app.snapshot().map(|s| s.machines.as_slice()).unwrap_or(&[]);
    if machines.is_empty() {
        f.render_widget(Block::bordered().title("machines"), area);
        return;
    }
    let (shown, hidden) = host_width_fit(area.width, machines.len());
    let marker_width = if hidden > 0 { MARKER_WIDTH } else { 0 };
    let [panels, marker] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(marker_width)]).areas(area);
    let rects = Layout::horizontal(vec![Constraint::Fill(1); shown]).split(panels);
    for (rect, machine) in rects.iter().zip(machines) {
        render_panel(f, *rect, app, theme, machine);
    }
    if hidden > 0 {
        f.render_widget(
            Paragraph::new(format!("+{hidden}")).style(Style::new().fg(theme.dim)),
            marker,
        );
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::app::{AppPage, ThemeId};

    fn app_with_machine(machine_json: &str) -> App {
        let json = format!(
            r#"{{"schema":1,"at":"2026-09-29T00:00:00Z","chair":{{"holder":"h","host":"h","epoch":1,"liveness":"live","beat_age_s":0}},"spend":{{"five_hour_fraction":0.0,"five_hour_source":"meter","weekly_fraction":0.0,"weekly_source":"meter","hard_stop_fraction":0.0,"five_hour_resets_at":"2026-09-29T00:00:00Z","weekly_resets_at":"2026-09-29T00:00:00Z"}},"machines":[{machine_json}],"runs":[],"queue":[],"inbox":[],"watch":[]}}"#
        );
        let mut app = App::new(AppPage::Regatta, ThemeId::Regatta);
        app.apply_snapshot(crate::feed::parse_snapshot(&json).expect("literal snapshot parses"));
        app
    }

    const TWO_OF_THREE: &str = r#"{"name":"atlas","state":"active","lanes_in_use":2,"capacity":3,"login_ok":true,"login_checked_at":"2026-09-29T00:00:00Z","beat_age_s":0,"checkouts":{},"lanes":[{"run":"r1","initiative":"towpath","phase":"p4","tasks":["t1"],"elapsed_s":725,"cost_usd":1.5},{"run":"r2","initiative":"coxdash","phase":"p1","tasks":["t2"],"elapsed_s":45,"cost_usd":0.2}],"load":{"cpu_pct":42.0,"load_avg_per_core":0.5,"mem_used_gb":6.0,"mem_total_gb":16.0}}"#;

    fn draw(app: &App, theme: &Theme) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(60, 8)).expect("terminal");
        terminal
            .draw(|f| render(f, f.area(), app, theme))
            .expect("draw should not fail");
        terminal
    }

    #[test]
    fn a_host_with_two_of_three_lanes_occupied_and_a_cpu_bar() {
        let app = app_with_machine(TWO_OF_THREE);
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let terminal = draw(&app, &theme);
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn an_occupied_cell_is_filled_in_the_running_colour() {
        let app = app_with_machine(TWO_OF_THREE);
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let terminal = draw(&app, &theme);
        let cell = &terminal.backend().buffer()[(1, 1)];
        assert_eq!(cell.style().bg, Some(theme.status_running));
    }

    #[test]
    fn bar_fill_scales_clamps_and_survives_a_bad_total() {
        assert_eq!(bar_fill(50.0, 100.0, 10), 5);
        assert_eq!(bar_fill(200.0, 100.0, 10), 10);
        assert_eq!(bar_fill(1.0, 0.0, 10), 0);
        assert_eq!(bar_fill(f64::NAN, 100.0, 10), 0);
    }

    #[test]
    fn hosts_that_do_not_fit_are_counted_as_hidden() {
        assert_eq!(host_width_fit(100, 3), (3, 0));
        assert_eq!(host_width_fit(52, 3), (2, 1));
    }

    #[test]
    fn lane_cells_fill_slots_in_order_and_leave_the_rest_free() {
        let app = app_with_machine(TWO_OF_THREE);
        let machine = &app.snapshot().expect("snapshot").machines[0];
        assert_eq!(
            lane_cells(machine),
            vec![
                Some("towpath p4 12m".to_string()),
                Some("coxdash p1 45s".to_string()),
                None
            ]
        );
    }
}
