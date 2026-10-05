//! Rendering for the health board (`cox dash --detail health`): one bordered block titled
//! `Health` with five labelled sections. Nothing calls `render` yet, so unused-code warnings
//! are silenced until the wiring phase.

use chrono::FixedOffset;
use ratatui::{
    Frame,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::detail::{ChairLease, HealthDetail, HostHealth, Housekeeping, LoginHealth, StoreHealth};
use crate::theme::Theme;

fn fg(color: Color) -> Style {
    Style::default().fg(color)
}

fn label(theme: &Theme, text: &str) -> Span<'static> {
    Span::styled(format!("{text} "), fg(theme.dim))
}

fn heading(theme: &Theme, text: &str) -> Line<'static> {
    Line::styled(text.to_string(), fg(theme.accent))
}

fn host_lines(hosts: &[HostHealth], theme: &Theme) -> Vec<Line<'static>> {
    hosts
        .iter()
        .map(|host| {
            let state_color = if host.state == "up" {
                theme.status_running
            } else {
                theme.status_failed
            };
            Line::from(vec![
                Span::raw(format!("  {:<12} ", host.name)),
                Span::styled(format!("{:<6}", host.state), fg(state_color)),
                label(theme, "lanes"),
                Span::raw(format!("{}/{}", host.lanes_in_use, host.capacity)),
            ])
        })
        .collect()
}

fn login_lines(logins: &[LoginHealth], theme: &Theme) -> Vec<Line<'static>> {
    logins
        .iter()
        .map(|login| {
            let color = if login.ok {
                theme.status_done
            } else {
                theme.status_failed
            };
            Line::from(vec![
                Span::raw(format!("  {:<10} {:<12} ", login.provider, login.host)),
                Span::styled(login.detail.clone(), fg(color)),
            ])
        })
        .collect()
}

fn store_lines(store: &StoreHealth, theme: &Theme) -> Vec<Line<'static>> {
    let (word, color) = if store.ok {
        ("ok", theme.status_done)
    } else {
        ("not ok", theme.status_failed)
    };
    let mut spans = vec![
        label(theme, "  status"),
        Span::styled(word, fg(color)),
        Span::raw(format!("  {}", store.detail)),
    ];
    if let Some(ms) = store.latency_ms {
        spans.push(label(theme, "  latency"));
        spans.push(Span::raw(format!("{ms} ms")));
    }
    vec![Line::from(spans)]
}

fn lease_lines(lease: &ChairLease, theme: &Theme, offset: FixedOffset) -> Vec<Line<'static>> {
    let holder = match &lease.holder {
        Some(name) => Span::raw(name.clone()),
        None => Span::styled("no holder", fg(theme.status_waiting)),
    };
    let mut spans = vec![label(theme, "  holder"), holder];
    if let Some(expires_at) = &lease.expires_at {
        spans.push(label(theme, "  expires"));
        spans.push(Span::raw(super::local_time(expires_at, offset)));
    }
    vec![Line::from(spans)]
}

fn housekeeping_lines(
    housekeeping: &Housekeeping,
    theme: &Theme,
    offset: FixedOffset,
) -> Vec<Line<'static>> {
    let mut spans = match &housekeeping.last_run_at {
        Some(at) => vec![
            label(theme, "  last run"),
            Span::raw(super::local_time(at, offset)),
        ],
        None => vec![Span::raw("  never run")],
    };
    if let Some(hours) = housekeeping.age_hours {
        spans.push(label(theme, "  age"));
        spans.push(Span::raw(format!("{hours:.1} h")));
    }
    vec![Line::from(spans)]
}

fn board_lines(detail: &HealthDetail, theme: &Theme, offset: FixedOffset) -> Vec<Line<'static>> {
    let sections = [
        ("Hosts", host_lines(&detail.hosts, theme)),
        ("Logins", login_lines(&detail.logins, theme)),
        ("Store", store_lines(&detail.store, theme)),
        (
            "Chair lease",
            lease_lines(&detail.chair_lease, theme, offset),
        ),
        (
            "Housekeeping",
            housekeeping_lines(&detail.housekeeping, theme, offset),
        ),
    ];
    sections
        .into_iter()
        .enumerate()
        .flat_map(|(index, (title, rows))| {
            let gap = if index == 0 {
                None
            } else {
                Some(Line::raw(""))
            };
            gap.into_iter()
                .chain(std::iter::once(heading(theme, title)))
                .chain(rows)
        })
        .collect()
}

pub fn render(f: &mut Frame, detail: &HealthDetail, theme: &Theme, offset: FixedOffset) {
    let area = f.area();
    let base = Style::default().fg(theme.fg).bg(theme.bg);
    let block = Block::default()
        .title("Health")
        .title_bottom(Line::styled("Esc back", fg(theme.dim)))
        .borders(Borders::ALL)
        .style(base);
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(
        Paragraph::new(board_lines(detail, theme, offset)).style(base),
        inner,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ThemeId;
    use ratatui::{Terminal, backend::TestBackend};

    const FIXTURE: &str = include_str!("../../tests/fixtures/dash_detail_health_v1.json");

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    fn draw(
        detail: &HealthDetail,
        theme: &Theme,
        width: u16,
        height: u16,
    ) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
        terminal
            .draw(|f| render(f, detail, theme, utc()))
            .expect("draw should not fail");
        terminal
    }

    /// The foreground colour of the first cell of `needle` on the first row that holds it.
    fn fg_of(terminal: &Terminal<TestBackend>, needle: &str) -> Option<Color> {
        let buffer = terminal.backend().buffer();
        let width = buffer.area.width;
        (0..buffer.area.height).find_map(|y| {
            let row: Vec<&str> = (0..width).map(|x| buffer[(x, y)].symbol()).collect();
            let joined = row.concat();
            joined
                .find(needle)
                .map(|byte| joined[..byte].chars().count() as u16)
                .map(|x| buffer[(x, y)].fg)
        })
    }

    fn fixture() -> HealthDetail {
        serde_json::from_str(FIXTURE).expect("fixture should parse")
    }

    fn board(id: ThemeId) -> (Terminal<TestBackend>, Theme) {
        let theme = crate::theme::resolve_for(id, Some("truecolor"));
        (draw(&fixture(), &theme, 100, 28), theme)
    }

    #[test]
    fn renders_the_health_title() {
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let terminal = draw(&fixture(), &theme, 80, 20);
        assert!(terminal.backend().to_string().contains("Health"));
    }

    #[test]
    fn renders_the_health_board_in_the_regatta_theme() {
        let (terminal, theme) = board(ThemeId::Regatta);
        assert_eq!(fg_of(&terminal, "down"), Some(theme.status_failed));
        insta::with_settings!({snapshot_path => "snapshots/health_drill"}, {
            insta::assert_snapshot!(terminal.backend().to_string());
        });
    }

    #[test]
    fn renders_the_health_board_in_the_harbor_light_theme() {
        let (terminal, theme) = board(ThemeId::HarborLight);
        assert_eq!(fg_of(&terminal, "down"), Some(theme.status_failed));
        insta::with_settings!({snapshot_path => "snapshots/health_drill"}, {
            insta::assert_snapshot!(terminal.backend().to_string());
        });
    }

    #[test]
    fn no_holder_and_no_housekeeping_run_say_so() {
        let detail = HealthDetail {
            chair_lease: ChairLease {
                holder: None,
                expires_at: None,
                ok: false,
            },
            housekeeping: Housekeeping {
                last_run_at: None,
                age_hours: None,
            },
            ..fixture()
        };
        let theme = crate::theme::resolve_for(ThemeId::Regatta, Some("truecolor"));
        let terminal = draw(&detail, &theme, 100, 28);
        let text = terminal.backend().to_string();
        assert!(text.contains("no holder"));
        assert!(text.contains("never run"));
        assert_eq!(fg_of(&terminal, "no holder"), Some(theme.status_waiting));
    }
}
