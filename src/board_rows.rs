// The board tree flattened into rows for the Board frame. Pure: no drawing, no clock. The
// snapshot's own `at` field says what "today" is. Node ids are the feed's ids as sent, so a phase
// id such as "p1-data" that two initiatives share collapses and opens in both.

use std::collections::{HashMap, HashSet};

use crate::feed::{BoardInitiative, BoardPhase, BoardTask, Epic, FeedSnapshot, StatusCounts};

/// The eight task statuses and how many tasks sit in each.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub ready: u32,
    pub building: u32,
    pub approved: u32,
    pub merged: u32,
    pub landed: u32,
    pub quarantined: u32,
    pub dropped: u32,
    pub blocked: u32,
}

impl From<&StatusCounts> for Counts {
    fn from(c: &StatusCounts) -> Self {
        Counts {
            ready: c.ready,
            building: c.building,
            approved: c.approved,
            merged: c.merged,
            landed: c.landed,
            quarantined: c.quarantined,
            dropped: c.dropped,
            blocked: c.blocked,
        }
    }
}

/// What every parent row carries. An initiative's title is its `short_id` and a phase's is its id,
/// because the feed sends no title for either.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: String,
    pub depth: usize,
    pub title: String,
    pub status: String,
    pub cost_usd: f64,
    pub counts: Counts,
    pub collapsed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    Epic(Node),
    /// `today` sums its phases' landed-today counts, so it shows while the initiative is folded.
    Initiative {
        node: Node,
        today: u32,
    },
    Phase(Node),
    Task {
        id: String,
        depth: usize,
        status: String,
        host: String,
        cost_usd: f64,
        /// From the machine lane whose tasks list names this task.
        elapsed_s: Option<u64>,
    },
    /// The folded landed and merged tasks of one phase. `id` is the phase id.
    Landed {
        id: String,
        depth: usize,
        today: u32,
        earlier: u32,
        opened: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    Building,
    Approved,
    Done,
    Dead,
    Waiting,
}

/// Maps a task or parent status word to a kind. A word the build does not know (a parent's
/// "active", say) is Waiting, so every branch returns.
pub fn status_kind(word: &str) -> StatusKind {
    match word {
        "building" => StatusKind::Building,
        "approved" => StatusKind::Approved,
        "landed" | "merged" => StatusKind::Done,
        "quarantined" | "dropped" => StatusKind::Dead,
        _ => StatusKind::Waiting,
    }
}

struct Ctx<'a> {
    collapsed: &'a HashSet<String>,
    opened: &'a HashSet<String>,
    /// Ids of tasks that landed on the UTC date of the snapshot's `at`.
    today: HashSet<&'a str>,
    elapsed: HashMap<&'a str, u64>,
}

/// Rows in tree order. Everything is expanded unless its id is in `collapsed`; a phase's landed
/// tasks stay folded into one count row unless the phase id is in `opened_landed`.
pub fn flatten(
    snap: &FeedSnapshot,
    collapsed: &HashSet<String>,
    opened_landed: &HashSet<String>,
) -> Vec<Row> {
    let ctx = Ctx {
        collapsed,
        opened: opened_landed,
        today: landed_today(snap),
        elapsed: lane_elapsed(snap),
    };
    snap.board
        .epics
        .iter()
        .flat_map(|e| epic_rows(&ctx, e))
        .collect()
}

fn landed_today(snap: &FeedSnapshot) -> HashSet<&str> {
    snap.at
        .get(..10)
        .into_iter()
        .flat_map(|date| {
            snap.history
                .iter()
                .filter(move |h| h.ended_at.starts_with(date))
        })
        .flat_map(|h| h.landed.iter().map(|l| l.task.as_str()))
        .collect()
}

fn lane_elapsed(snap: &FeedSnapshot) -> HashMap<&str, u64> {
    snap.machines
        .iter()
        .flat_map(|m| m.lanes.iter())
        .flat_map(|l| l.tasks.iter().map(|t| (t.as_str(), l.elapsed_s)))
        .collect()
}

fn is_folded(status: &str) -> bool {
    matches!(status, "landed" | "merged")
}

fn node(
    ctx: &Ctx,
    depth: usize,
    id: &str,
    title: &str,
    status: &str,
    cost: f64,
    c: &StatusCounts,
) -> Node {
    Node {
        id: id.to_string(),
        depth,
        title: title.to_string(),
        status: status.to_string(),
        cost_usd: cost,
        counts: Counts::from(c),
        collapsed: ctx.collapsed.contains(id),
    }
}

fn epic_rows(ctx: &Ctx, e: &Epic) -> Vec<Row> {
    let n = node(ctx, 0, &e.id, &e.title, &e.status, e.cost_usd, &e.counts);
    let children = if n.collapsed {
        Vec::new()
    } else {
        e.initiatives
            .iter()
            .flat_map(|i| initiative_rows(ctx, i))
            .collect()
    };
    std::iter::once(Row::Epic(n)).chain(children).collect()
}

fn initiative_rows(ctx: &Ctx, i: &BoardInitiative) -> Vec<Row> {
    let n = node(ctx, 1, &i.id, &i.short_id, &i.status, i.cost_usd, &i.counts);
    let today = i.phases.iter().map(|p| phase_today(ctx, p)).sum();
    let children = if n.collapsed {
        Vec::new()
    } else {
        i.phases.iter().flat_map(|p| phase_rows(ctx, p)).collect()
    };
    std::iter::once(Row::Initiative { node: n, today })
        .chain(children)
        .collect()
}

fn phase_today(ctx: &Ctx, p: &BoardPhase) -> u32 {
    p.tasks
        .iter()
        .filter(|t| is_folded(&t.status) && ctx.today.contains(t.id.as_str()))
        .count() as u32
}

fn phase_rows(ctx: &Ctx, p: &BoardPhase) -> Vec<Row> {
    let n = node(ctx, 2, &p.id, &p.id, &p.status, p.cost_usd, &p.counts);
    if n.collapsed {
        return vec![Row::Phase(n)];
    }
    let (folded, live): (Vec<&BoardTask>, Vec<&BoardTask>) =
        p.tasks.iter().partition(|t| is_folded(&t.status));
    let today = phase_today(ctx, p);
    let opened = ctx.opened.contains(&p.id);
    let landed = if folded.is_empty() {
        Vec::new()
    } else {
        let listed = if opened {
            folded.iter().map(|t| task_row(ctx, t, 4)).collect()
        } else {
            Vec::new()
        };
        std::iter::once(Row::Landed {
            id: p.id.clone(),
            depth: 3,
            today,
            earlier: folded.len() as u32 - today,
            opened,
        })
        .chain(listed)
        .collect()
    };
    std::iter::once(Row::Phase(n))
        .chain(live.iter().map(|t| task_row(ctx, t, 3)))
        .chain(landed)
        .collect()
}

fn task_row(ctx: &Ctx, t: &BoardTask, depth: usize) -> Row {
    Row::Task {
        id: t.id.clone(),
        depth,
        status: t.status.clone(),
        host: t.host.clone(),
        cost_usd: t.cost_usd,
        elapsed_s: ctx.elapsed.get(t.id.as_str()).copied(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::parse_snapshot;
    use serde_json::json;

    const BOARD_FIXTURE: &str = include_str!("../tests/fixtures/dash_feed_board_v1.json");

    fn no_ids() -> HashSet<String> {
        HashSet::new()
    }

    fn ids(list: &[&str]) -> HashSet<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn fixture() -> FeedSnapshot {
        parse_snapshot(BOARD_FIXTURE).expect("board fixture parses")
    }

    /// The fixture plus a merged task in p1-data. Today's history row landed board-view-parse and
    /// yesterday's landed board-view-old.
    fn with_landed() -> FeedSnapshot {
        let mut snap = fixture();
        snap.board.epics[0].initiatives[0].phases[0]
            .tasks
            .push(BoardTask {
                id: "board-view-old".into(),
                status: "merged".into(),
                ..BoardTask::default()
            });
        let row = |run: &str, ended_at: &str, task: &str| {
            serde_json::from_value(json!({
                "run": run, "machine": "host-b", "initiative": "board-view",
                "ended_at": ended_at, "outcome": "landed", "cost_usd": 0.5,
                "landed": [{"task": task, "pr": 7}]
            }))
            .expect("history row parses")
        };
        snap.history = vec![
            row("board-0", "2026-11-09T08:00:00Z", "board-view-parse"),
            row("board-9", "2026-11-08T23:59:00Z", "board-view-old"),
        ];
        snap
    }

    fn label(row: &Row) -> String {
        match row {
            Row::Epic(n) => format!("epic {} collapsed={}", n.id, n.collapsed),
            Row::Initiative { node, today } => {
                format!(
                    "  initiative {} collapsed={} today={today}",
                    node.id, node.collapsed
                )
            }
            Row::Phase(n) => format!("    phase {} collapsed={}", n.id, n.collapsed),
            Row::Task { id, depth, .. } => format!("{}task {id}", " ".repeat(depth * 2)),
            Row::Landed {
                id,
                today,
                earlier,
                opened,
                ..
            } => {
                format!("      landed {id} today={today} earlier={earlier} opened={opened}")
            }
        }
    }

    fn labels(rows: &[Row]) -> Vec<String> {
        rows.iter().map(label).collect()
    }

    fn parent(id: &str, depth: usize, title: &str, cost: f64, counts: Counts) -> Node {
        Node {
            id: id.into(),
            depth,
            title: title.into(),
            status: "active".into(),
            cost_usd: cost,
            counts,
            collapsed: false,
        }
    }

    fn task(id: &str, status: &str, host: &str, cost: f64, elapsed_s: Option<u64>) -> Row {
        Row::Task {
            id: id.into(),
            depth: 3,
            status: status.into(),
            host: host.into(),
            cost_usd: cost,
            elapsed_s,
        }
    }

    #[test]
    fn everything_is_expanded_by_default() {
        let all = Counts {
            ready: 1,
            building: 1,
            landed: 1,
            quarantined: 1,
            ..Counts::default()
        };
        let stray = Counts {
            building: 1,
            blocked: 1,
            ..Counts::default()
        };
        let p1 = Counts {
            building: 1,
            landed: 1,
            ..Counts::default()
        };
        let p2 = Counts {
            ready: 1,
            quarantined: 1,
            ..Counts::default()
        };
        let want = vec![
            Row::Epic(parent("board-epic", 0, "Board shows epics", 1.75, all)),
            Row::Initiative {
                node: parent("board-view", 1, "bv", 1.75, all),
                today: 0,
            },
            Row::Phase(parent("p1-data", 2, "p1-data", 0.75, p1)),
            task("board-view-load", "building", "host-b", 0.25, Some(120)),
            Row::Landed {
                id: "p1-data".into(),
                depth: 3,
                today: 0,
                earlier: 1,
                opened: false,
            },
            Row::Phase(parent("p2-render", 2, "p2-render", 1.0, p2)),
            task("board-view-frame", "quarantined", "host-b", 1.0, None),
            task("board-view-keys", "ready", "", 0.0, None),
            Row::Epic(parent("", 0, "no epic", 0.5, stray)),
            Row::Initiative {
                node: parent("stray-fix", 1, "sf", 0.5, stray),
                today: 0,
            },
            Row::Phase(parent("p1-fix", 2, "p1-fix", 0.5, stray)),
            task("stray-fix-patch", "building", "host-b", 0.5, Some(300)),
            task("stray-fix-docs", "blocked", "", 0.0, None),
        ];
        assert_eq!(flatten(&fixture(), &no_ids(), &no_ids()), want);
    }

    #[test]
    fn a_collapsed_initiative_hides_its_phases_but_keeps_its_today_count() {
        let rows = flatten(&with_landed(), &ids(&["board-view"]), &no_ids());
        assert_eq!(
            labels(&rows),
            [
                "epic board-epic collapsed=false",
                "  initiative board-view collapsed=true today=1",
                "epic  collapsed=false",
                "  initiative stray-fix collapsed=false today=0",
                "    phase p1-fix collapsed=false",
                "      task stray-fix-patch",
                "      task stray-fix-docs",
            ]
        );
    }

    #[test]
    fn a_collapsed_phase_hides_its_tasks_and_landed_row() {
        let rows = flatten(&with_landed(), &ids(&["p1-data"]), &no_ids());
        let phases: Vec<_> = labels(&rows)
            .into_iter()
            .filter(|l| l.contains("p1-data") || l.contains("landed"))
            .collect();
        assert_eq!(phases, ["    phase p1-data collapsed=true"]);
    }

    #[test]
    fn landed_and_merged_tasks_fold_into_a_count_row() {
        let rows = flatten(&with_landed(), &no_ids(), &no_ids());
        assert_eq!(
            labels(&rows)[..6],
            [
                "epic board-epic collapsed=false",
                "  initiative board-view collapsed=false today=1",
                "    phase p1-data collapsed=false",
                "      task board-view-load",
                "      landed p1-data today=1 earlier=1 opened=false",
                "    phase p2-render collapsed=false",
            ]
        );
    }

    #[test]
    fn opening_the_landed_row_lists_its_tasks_beneath_it() {
        let rows = flatten(&with_landed(), &no_ids(), &ids(&["p1-data"]));
        assert_eq!(
            labels(&rows)[3..7],
            [
                "      task board-view-load",
                "      landed p1-data today=1 earlier=1 opened=true",
                "        task board-view-parse",
                "        task board-view-old",
            ]
        );
    }

    #[test]
    fn elapsed_comes_from_the_lane_that_names_the_task() {
        let rows = flatten(&fixture(), &no_ids(), &no_ids());
        let elapsed: Vec<_> = rows
            .iter()
            .filter_map(|r| match r {
                Row::Task { id, elapsed_s, .. } => Some((id.as_str(), *elapsed_s)),
                _ => None,
            })
            .collect();
        assert_eq!(
            elapsed,
            [
                ("board-view-load", Some(120)),
                ("board-view-frame", None),
                ("board-view-keys", None),
                ("stray-fix-patch", Some(300)),
                ("stray-fix-docs", None),
            ]
        );
    }

    #[test]
    fn every_status_word_maps_to_its_kind() {
        use StatusKind::*;
        let want = [
            ("building", Building),
            ("approved", Approved),
            ("landed", Done),
            ("merged", Done),
            ("quarantined", Dead),
            ("dropped", Dead),
            ("ready", Waiting),
            ("blocked", Waiting),
            ("active", Waiting),
        ];
        let got: Vec<_> = want.iter().map(|(w, _)| (*w, status_kind(w))).collect();
        assert_eq!(got, want);
    }
}
