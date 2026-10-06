//! Rendering for the machine-drill board (`cox dash --detail machine <id>`): a bordered
//! block titled `Machine <machine>` holding host facts, a lanes-in-use meter, and one row
//! per checkout in `detail.checkouts` (already sorted by repo, since it is a `BTreeMap`).
//! Nothing calls `render` yet, so unused-code warnings are silenced the way `src/detail.rs`
//! silences them until the page-dispatch task wires this board in.
#![allow(dead_code)]

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
};

use crate::actions::Target;
use crate::detail::MachineDetail;
use crate::theme::Theme;

fn base_style(theme: &Theme) -> Style {
    Style::default().fg(theme.fg).bg(theme.bg)
}

/// `meter_low` below 60% full, `meter_mid` below 85%, else `meter_high`.
fn meter_color(theme: &Theme, fraction: f64) -> Color {
    if fraction < 0.6 {
        theme.meter_low
    } else if fraction < 0.85 {
        theme.meter_mid
    } else {
        theme.meter_high
    }
}

/// `status_done` at zero commits behind, `meter_mid` from one to five, `meter_high` above.
fn checkout_color(theme: &Theme, behind_main: u32) -> Color {
    if behind_main == 0 {
        theme.status_done
    } else if behind_main <= 5 {
        theme.meter_mid
    } else {
        theme.meter_high
    }
}

/// Draws the machine drill-down: title, host facts, a lanes meter, then one line per
/// checkout. `offset` is threaded through for signature parity with the other drill boards;
/// this board shows no timestamp, so it goes unused.
pub fn render(
    f: &mut Frame,
    area: Rect,
    detail: &MachineDetail,
    theme: &Theme,
    offset: chrono::FixedOffset,
) {
    let _ = offset;
    let block = Block::default()
        .title(format!("Machine {}", detail.machine))
        .title_bottom(super::footer_line(
            &Target::Machine(detail.machine.clone()),
            theme,
        ))
        .borders(Borders::ALL)
        .style(base_style(theme));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);

    render_host_facts(f, rows[0], detail, theme);
    render_lanes(f, rows[1], detail, theme);
    render_checkouts(f, rows[2], detail, theme);
}

fn render_host_facts(f: &mut Frame, rect: Rect, detail: &MachineDetail, theme: &Theme) {
    let facts = &detail.host_facts;
    let line = format!(
        "{} {} cpus {:.1} GB",
        facts.os, facts.cpu_count, facts.mem_total_gb
    );
    let paragraph = Paragraph::new(line).style(base_style(theme));
    f.render_widget(paragraph, rect);
}

fn render_lanes(f: &mut Frame, rect: Rect, detail: &MachineDetail, theme: &Theme) {
    let fraction = if detail.capacity == 0 {
        0.0
    } else {
        f64::from(detail.lanes_in_use) / f64::from(detail.capacity)
    };
    let line = Line::styled(
        format!("{}/{}", detail.lanes_in_use, detail.capacity),
        Style::default()
            .fg(meter_color(theme, fraction))
            .bg(theme.bg),
    );
    let paragraph = Paragraph::new(line);
    f.render_widget(paragraph, rect);
}

fn render_checkouts(f: &mut Frame, rect: Rect, detail: &MachineDetail, theme: &Theme) {
    let lines: Vec<Line> = detail
        .checkouts
        .iter()
        .map(|(repo, checkout)| {
            Line::styled(
                format!(
                    "{repo} {} {} behind main",
                    checkout.branch, checkout.behind_main
                ),
                Style::default()
                    .fg(checkout_color(theme, checkout.behind_main))
                    .bg(theme.bg),
            )
        })
        .collect();
    let paragraph = Paragraph::new(lines).style(base_style(theme));
    f.render_widget(paragraph, rect);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ThemeId;
    use crate::detail::{DetailSnapshot, parse_detail};
    use ratatui::{Terminal, backend::TestBackend};

    const FIXTURE: &str = include_str!("../../tests/fixtures/dash_detail_machine_v1.json");

    fn machine_from_fixture() -> MachineDetail {
        let snapshot = parse_detail(FIXTURE).expect("fixture should parse");
        let DetailSnapshot::Machine(detail) = snapshot else {
            panic!("expected DetailSnapshot::Machine");
        };
        detail
    }

    #[test]
    fn renders_the_machine_title_from_the_fixture() {
        let detail = machine_from_fixture();
        let theme = crate::theme::resolve(ThemeId::Regatta);
        let offset = chrono::FixedOffset::east_opt(0).unwrap();
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render(f, f.area(), &detail, &theme, offset))
            .expect("draw should not fail");
        assert!(terminal.backend().to_string().contains("Machine omarchy"));
    }

    #[test]
    fn renders_machine_snapshot_in_the_regatta_theme() {
        let detail = machine_from_fixture();
        let theme = crate::theme::resolve(ThemeId::Regatta);
        let offset = chrono::FixedOffset::east_opt(0).unwrap();
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render(f, f.area(), &detail, &theme, offset))
            .expect("draw should not fail");
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn renders_machine_snapshot_in_the_harbor_light_theme() {
        let detail = machine_from_fixture();
        let theme = crate::theme::resolve(ThemeId::HarborLight);
        let offset = chrono::FixedOffset::east_opt(0).unwrap();
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render(f, f.area(), &detail, &theme, offset))
            .expect("draw should not fail");
        insta::assert_snapshot!(terminal.backend().to_string());
    }

    #[test]
    fn coxswain_dash_checkout_is_drawn_in_meter_mid() {
        let detail = machine_from_fixture();
        let theme = crate::theme::resolve(ThemeId::Regatta);
        let offset = chrono::FixedOffset::east_opt(0).unwrap();
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render(f, f.area(), &detail, &theme, offset))
            .expect("draw should not fail");
        // The fixture's `coxswain-dash` checkout has `behind_main: 3`, so it renders in
        // `meter_mid`, not `status_done` (0) or `meter_high` (above 5).
        let buffer = terminal.backend().buffer();
        let row = 1 + 1 + 1; // block top border + host-facts row + lanes row
        let line: String = (0..buffer.area.width)
            .map(|x| buffer[(x, row)].symbol().chars().next().unwrap_or(' '))
            .collect();
        assert!(line.contains("coxswain-dash"), "line was {line:?}");
        let x = line.find("coxswain-dash").expect("checkout line present") as u16;
        assert_eq!(buffer[(x, row)].fg, theme.meter_mid);
    }
}
