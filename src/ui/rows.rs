//! The rows the runs and queue frames draw, as plain data. Arrow keys and mouse clicks read this
//! model instead of the feed's own order, so a selection lands on what is on screen: a collapsed
//! initiative is one line, a queue group header is a line nobody can select, and the runs frame
//! pins a column header above its body.

use super::{collapse_runs, initiative_of};
use crate::app::group_queue;
use crate::feed::{QueueEntry, Run};

/// The runs frame draws one column header line above its body.
const RUNS_PINNED: usize = 1;

/// One drawn body line. A header holds its group label. An entity line holds the index, in the
/// feed list, of the entity it opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Body {
    Header(String),
    Entity(usize),
}

impl Body {
    fn entity(&self) -> Option<usize> {
        match self {
            Body::Header(_) => None,
            Body::Entity(index) => Some(*index),
        }
    }
}

/// A frame's body lines in drawn order, the count of pinned lines above them, and for each feed
/// index the line that stands for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rows {
    lines: Vec<Body>,
    pinned: usize,
    owner: Vec<Option<usize>>,
}

/// One entity line per initiative, in the order the runs frame draws them. A collapsed row opens
/// the initiative's newest run. Older runs of the initiative are owned by that same line.
pub(crate) fn runs_rows(runs: &[Run]) -> Rows {
    let lines: Vec<Body> = collapse_runs(runs)
        .iter()
        .filter_map(|row| runs.iter().position(|r| std::ptr::eq(r, row.run)))
        .map(Body::Entity)
        .collect();
    let owner = runs
        .iter()
        .map(|run| {
            lines.iter().position(|line| {
                line.entity()
                    .is_some_and(|i| initiative_of(&runs[i].run) == initiative_of(&run.run))
            })
        })
        .collect();
    Rows {
        lines,
        pinned: RUNS_PINNED,
        owner,
    }
}

/// The queue's lines. Ungrouped they are the feed order. Grouped, each project gets a header line
/// over its rows, and rows with no project come last.
pub(crate) fn queue_rows(queue: &[QueueEntry], grouped: bool) -> Rows {
    let lines: Vec<Body> = if grouped {
        group_queue(queue)
            .into_iter()
            .flat_map(|(label, members)| {
                std::iter::once(Body::Header(label.to_string()))
                    .chain(
                        members
                            .into_iter()
                            .filter_map(|m| queue.iter().position(|q| std::ptr::eq(q, m)))
                            .map(Body::Entity),
                    )
                    .collect::<Vec<_>>()
            })
            .collect()
    } else {
        (0..queue.len()).map(Body::Entity).collect()
    };
    let owner = (0..queue.len())
        .map(|i| lines.iter().position(|line| line.entity() == Some(i)))
        .collect();
    Rows {
        lines,
        pinned: 0,
        owner,
    }
}

/// The first visible line, given the selected line, the body height and the line count. The
/// selection is the last visible line once it passes the first screenful.
fn window(selected: usize, height: usize, count: usize) -> usize {
    (selected + 1)
        .saturating_sub(height)
        .min(count.saturating_sub(height))
}

impl Rows {
    /// The feed index one entity line forward or back from the line holding `from`, skipping
    /// headers and stopping at the ends. A `from` no line holds lands on the first entity.
    pub(crate) fn step(&self, from: usize, forward: bool) -> Option<usize> {
        let at = self.owner.get(from).copied().flatten();
        let moved = if forward {
            self.lines
                .iter()
                .skip(at.map_or(0, |line| line + 1))
                .find_map(|line| line.entity())
        } else {
            at.and_then(|line| self.lines[..line].iter().rev().find_map(|l| l.entity()))
        };
        moved
            .or_else(|| at.and_then(|line| self.lines[line].entity()))
            .or_else(|| self.lines.iter().find_map(|line| line.entity()))
    }

    /// The feed index a click on `row` opens. `row` counts from the first line inside the frame's
    /// border, `inner_height` is the lines inside it, and `selected` is the feed index selected
    /// there, which fixes the window. `None` on a pinned line, a header or past the last line.
    pub(crate) fn hit(&self, selected: usize, inner_height: usize, row: usize) -> Option<usize> {
        let body = inner_height.checked_sub(self.pinned)?;
        let in_body = row.checked_sub(self.pinned).filter(|r| *r < body)?;
        let (top, _) = self.view(selected, body);
        self.lines.get(top + in_body).and_then(|line| line.entity())
    }

    /// The body lines drawn under the pinned lines, and the position among them of the line
    /// holding `selected`. `None` when that line is scrolled out of view or `selected` has none.
    pub(crate) fn visible(
        &self,
        selected: usize,
        inner_height: usize,
    ) -> (Vec<Body>, Option<usize>) {
        let body = inner_height.saturating_sub(self.pinned);
        let (top, line) = self.view(selected, body);
        let shown: Vec<Body> = self.lines.iter().skip(top).take(body).cloned().collect();
        let at = self.owner.get(selected).copied().flatten().and(
            line.checked_sub(top)
                .filter(|position| *position < shown.len()),
        );
        (shown, at)
    }

    /// The first visible line and the selected line, for a body of `body` lines. A `selected`
    /// that no line holds counts as the first line. `hit` and `visible` both read this.
    fn view(&self, selected: usize, body: usize) -> (usize, usize) {
        let line = self.owner.get(selected).copied().flatten().unwrap_or(0);
        (window(line, body, self.lines.len()), line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs(ids: &[&str]) -> Vec<Run> {
        let runs = ids
            .iter()
            .map(|id| {
                format!(
                    r#"{{"run":"{id}","machine":"m0","phase":"p","node":"n","attempt":1,"turns":1,"cost":0.0,"verdict":"ok","status":"running"}}"#
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let json = format!(
            r#"{{"schema":1,"at":"2026-09-29T00:00:00Z","chair":{{"holder":"h","host":"h","epoch":1,"liveness":"live","beat_age_s":0}},"spend":{{"five_hour_fraction":0.0,"five_hour_source":"meter","weekly_fraction":0.0,"weekly_source":"meter","hard_stop_fraction":0.0,"five_hour_resets_at":"2026-09-29T00:00:00Z","weekly_resets_at":"2026-09-29T00:00:00Z"}},"machines":[],"runs":[{runs}],"queue":[],"inbox":[],"watch":[]}}"#
        );
        crate::feed::parse_snapshot(&json)
            .expect("literal snapshot should parse")
            .runs
    }

    fn queue(rows: &[(&str, Option<&str>)]) -> Vec<QueueEntry> {
        rows.iter()
            .map(|(initiative, project)| QueueEntry {
                initiative: initiative.to_string(),
                priority: 1,
                phases_landed: 0,
                phases_total: 1,
                current_phase: "p".to_string(),
                repo: String::new(),
                project: project.map(str::to_string),
                phases: None,
            })
            .collect()
    }

    #[test]
    fn a_collapsed_initiative_is_one_line_opening_its_newest_run() {
        let rows = runs_rows(&runs(&["a-1", "b-1", "a-2"]));
        assert_eq!(rows.lines, vec![Body::Entity(2), Body::Entity(1)]);
        assert_eq!(rows.pinned, 1);
        assert_eq!(rows.owner, vec![Some(0), Some(1), Some(0)]);
    }

    #[test]
    fn the_grouped_queue_is_headers_over_rows_with_other_last() {
        let q = queue(&[
            ("q0", Some("x")),
            ("q1", None),
            ("q2", Some("y")),
            ("q3", Some("x")),
        ]);
        let rows = queue_rows(&q, true);
        assert_eq!(
            rows.lines,
            vec![
                Body::Header("x".to_string()),
                Body::Entity(0),
                Body::Entity(3),
                Body::Header("y".to_string()),
                Body::Entity(2),
                Body::Header("other".to_string()),
                Body::Entity(1),
            ]
        );
        assert_eq!(rows.pinned, 0);
    }

    #[test]
    fn the_ungrouped_queue_keeps_feed_order() {
        let rows = queue_rows(&queue(&[("q0", Some("x")), ("q1", None)]), false);
        assert_eq!(rows.lines, vec![Body::Entity(0), Body::Entity(1)]);
    }

    #[test]
    fn step_skips_headers_and_clamps_at_the_ends() {
        let q = queue(&[("q0", Some("x")), ("q1", Some("y"))]);
        let rows = queue_rows(&q, true);
        assert_eq!(rows.step(0, true), Some(1));
        assert_eq!(rows.step(1, true), Some(1));
        assert_eq!(rows.step(1, false), Some(0));
        assert_eq!(rows.step(0, false), Some(0));
    }

    #[test]
    fn step_over_a_collapsed_row_moves_once() {
        let rows = runs_rows(&runs(&["a-1", "a-2", "b-1"]));
        assert_eq!(rows.step(1, true), Some(2));
        assert_eq!(rows.step(0, true), Some(2));
        assert_eq!(rows.step(2, false), Some(1));
    }

    #[test]
    fn step_from_an_empty_list_is_none() {
        assert_eq!(queue_rows(&[], true).step(0, true), None);
    }

    #[test]
    fn window_keeps_the_selection_in_view() {
        assert_eq!(window(0, 10, 30), 0);
        assert_eq!(window(9, 10, 30), 0);
        assert_eq!(window(10, 10, 30), 1);
        assert_eq!(window(29, 10, 30), 20);
        assert_eq!(window(2, 10, 4), 0);
        assert_eq!(window(0, 0, 0), 0);
    }

    #[test]
    fn hit_maps_a_click_through_the_pinned_line() {
        let rows = runs_rows(&runs(&["a-1", "a-2", "b-1"]));
        assert_eq!(rows.hit(0, 5, 0), None);
        assert_eq!(rows.hit(0, 5, 1), Some(1));
        assert_eq!(rows.hit(0, 5, 2), Some(2));
        assert_eq!(rows.hit(0, 5, 3), None);
    }

    #[test]
    fn hit_on_a_header_is_none_and_on_an_entity_is_that_entity() {
        let q = queue(&[("q0", Some("x")), ("q1", Some("y"))]);
        let rows = queue_rows(&q, true);
        assert_eq!(rows.hit(0, 6, 0), None);
        assert_eq!(rows.hit(0, 6, 1), Some(0));
        assert_eq!(rows.hit(0, 6, 2), None);
        assert_eq!(rows.hit(0, 6, 3), Some(1));
    }

    #[test]
    fn hit_follows_the_window_in_a_long_list() {
        let ids: Vec<String> = (0..30).map(|i| format!("i{i}-1")).collect();
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        let rows = runs_rows(&runs(&ids));
        assert_eq!(rows.hit(29, 11, 1), Some(20));
        assert_eq!(rows.hit(29, 11, 10), Some(29));
        assert_eq!(rows.hit(29, 11, 11), None);
    }

    #[test]
    fn visible_windows_a_long_list_on_the_selection() {
        let ids: Vec<String> = (0..30).map(|i| format!("i{i}-1")).collect();
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        let rows = runs_rows(&runs(&ids));
        let (shown, at) = rows.visible(24, 11);
        assert_eq!(shown.len(), 10);
        assert_eq!(shown.first(), Some(&Body::Entity(15)));
        assert_eq!(shown.last(), Some(&Body::Entity(24)));
        assert_eq!(at, Some(9));
    }

    #[test]
    fn visible_keeps_the_group_labels_in_the_headers() {
        let q = queue(&[("q0", Some("x")), ("q1", None), ("q2", Some("y"))]);
        let (shown, at) = queue_rows(&q, true).visible(2, 10);
        assert_eq!(
            shown,
            vec![
                Body::Header("x".to_string()),
                Body::Entity(0),
                Body::Header("y".to_string()),
                Body::Entity(2),
                Body::Header("other".to_string()),
                Body::Entity(1),
            ]
        );
        assert_eq!(at, Some(3));
    }

    #[test]
    fn visible_agrees_with_hit_on_every_row() {
        let q = queue(&[
            ("q0", Some("x")),
            ("q1", None),
            ("q2", Some("y")),
            ("q3", Some("x")),
        ]);
        let rows = queue_rows(&q, true);
        for (selected, height) in [(0, 4), (3, 4), (1, 5), (2, 3), (0, 20)] {
            let (shown, _) = rows.visible(selected, height);
            for (row, line) in shown.iter().enumerate() {
                assert_eq!(rows.hit(selected, height, row), line.entity());
            }
        }
    }
}
