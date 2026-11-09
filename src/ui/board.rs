//! The Board page. `regions` splits the area once; each region goes to its panel module.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
};

use super::{
    board_agents, board_burn, board_header, board_machines, board_small_graphs, board_throughput,
    board_timeline, board_tree,
};
use crate::app::App;
use crate::theme::Theme;

/// Below this width the live column drops away and only the tree shows.
const LIVE_MIN_WIDTH: u16 = 100;
const HEADER_HEIGHT: u16 = 1;
const MACHINES_HEIGHT: u16 = 5;
const THROUGHPUT_HEIGHT: u16 = 8;
const BURN_HEIGHT: u16 = 5;
const AGENTS_MIN_HEIGHT: u16 = 4;
const SMALL_GRAPHS_HEIGHT: u16 = 6;
const TIMELINE_HEIGHT: u16 = 5;
/// The tree's share of the body width when the live column shows.
const TREE_PERCENT: u16 = 60;

/// The right-hand column. Absent under `LIVE_MIN_WIDTH` columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Live {
    throughput: Rect,
    burn: Rect,
    agents: Rect,
    small_graphs: Rect,
    timeline: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Regions {
    header: Rect,
    machines: Rect,
    tree: Rect,
    live: Option<Live>,
}

/// Splits `area` into the board's regions. The tree takes the height left after the header and
/// the machine strip.
fn regions(area: Rect) -> Regions {
    let [header, machines, body] = Layout::vertical([
        Constraint::Length(HEADER_HEIGHT),
        Constraint::Length(MACHINES_HEIGHT),
        Constraint::Min(0),
    ])
    .areas(area);
    if area.width < LIVE_MIN_WIDTH {
        return Regions {
            header,
            machines,
            tree: body,
            live: None,
        };
    }
    let [tree, live] = Layout::horizontal([
        Constraint::Percentage(TREE_PERCENT),
        Constraint::Percentage(100 - TREE_PERCENT),
    ])
    .areas(body);
    let [throughput, burn, agents, small_graphs, timeline] = Layout::vertical([
        Constraint::Length(THROUGHPUT_HEIGHT),
        Constraint::Length(BURN_HEIGHT),
        Constraint::Min(AGENTS_MIN_HEIGHT),
        Constraint::Length(SMALL_GRAPHS_HEIGHT),
        Constraint::Length(TIMELINE_HEIGHT),
    ])
    .areas(live);
    Regions {
        header,
        machines,
        tree,
        live: Some(Live {
            throughput,
            burn,
            agents,
            small_graphs,
            timeline,
        }),
    }
}

pub fn render(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let r = regions(area);
    board_header::render(f, r.header, app, theme);
    board_machines::render(f, r.machines, app, theme);
    board_tree::render(f, r.tree, app, theme);
    if let Some(live) = r.live {
        board_throughput::render(f, live.throughput, app, theme);
        board_burn::render(f, live.burn, app, theme);
        board_agents::render(f, live.agents, app, theme);
        board_small_graphs::render(f, live.small_graphs, app, theme);
        board_timeline::render(f, live.timeline, app, theme);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(r: &Regions) -> Vec<Rect> {
        let mut rects = vec![r.header, r.machines, r.tree];
        if let Some(l) = r.live {
            rects.extend([l.throughput, l.burn, l.agents, l.small_graphs, l.timeline]);
        }
        rects
    }

    const AREAS: [Rect; 5] = [
        Rect::new(0, 0, 120, 40),
        Rect::new(3, 2, 140, 50),
        Rect::new(0, 0, 100, 30),
        Rect::new(0, 0, 99, 30),
        Rect::new(0, 0, 80, 24),
    ];

    #[test]
    fn regions_stay_inside_the_area() {
        for area in AREAS {
            for rect in all(&regions(area)) {
                assert_eq!(area.intersection(rect), rect, "{rect:?} escapes {area:?}");
            }
        }
    }

    #[test]
    fn regions_never_overlap() {
        for area in AREAS {
            let rects = all(&regions(area));
            for (i, a) in rects.iter().enumerate() {
                for b in &rects[i + 1..] {
                    assert!(!a.intersects(*b), "{a:?} overlaps {b:?} in {area:?}");
                }
            }
        }
    }

    #[test]
    fn the_live_column_shows_from_100_columns() {
        assert!(regions(Rect::new(0, 0, 100, 30)).live.is_some());
        assert!(regions(Rect::new(0, 0, 99, 30)).live.is_none());
        assert!(regions(Rect::new(0, 0, 80, 24)).live.is_none());
    }

    #[test]
    fn the_tree_takes_the_full_width_when_the_live_column_drops() {
        let r = regions(Rect::new(0, 0, 80, 24));
        assert_eq!(r.tree.width, 80);
    }

    #[test]
    fn the_tree_gets_the_remaining_height() {
        for area in [Rect::new(0, 0, 120, 40), Rect::new(3, 2, 140, 50)] {
            let r = regions(area);
            assert_eq!(r.header.height, 1);
            assert_eq!(r.machines.width, area.width);
            assert_eq!(r.tree.height, area.height - HEADER_HEIGHT - MACHINES_HEIGHT);
        }
    }

    #[test]
    fn the_timeline_sits_under_the_live_column_only() {
        let area = Rect::new(0, 0, 120, 40);
        let r = regions(area);
        let live = r.live.expect("live column at 120 columns");
        assert_eq!(live.timeline.bottom(), area.bottom());
        assert!(live.timeline.x > r.tree.x);
        assert!(live.timeline.y > live.small_graphs.y);
    }
}
