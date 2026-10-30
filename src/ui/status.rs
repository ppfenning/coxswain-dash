//! The one status vocabulary and the one map from a status to its glyph and colour.
//!
//! Every pane maps its own domain word (run end, task state, agent state, liveness, host health,
//! lease) to a [`Status`] and asks [`style`] for the look. No other file under `src/ui/` names a
//! `status_*` theme token; the guard test at the bottom of this file enforces that.
//!
//! Merges, so that one look has one meaning:
//! - `Working` draws as `Running`, and `Live` and `Up` draw as `Landed`'s green. Each pair is one
//!   state wearing two names, kept apart only so a caller can say what it means.
//! - `Crashed` and `Fenced` share the `!` glyph and the failure red: a lease holder that was
//!   cut off and one that died both need a person.
//! - `Landed`, `Approved` and `Quarantined` use the `landed`, `approved` and `quarantined` theme
//!   roles, the colours run history has always drawn.
//! - `Blocked` draws as `Ready` in the waiting hue. That hue is already the dim text colour, so
//!   the two differ by meaning and placement, not by RGB.
//! - `Stale` is the warning yellow, `Dropped` and `Idle` are dim.

use ratatui::style::Color;

use crate::feed::{AgentState, Outcome, RunEnd};
use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ready,
    Blocked,
    Running,
    Approved,
    Landed,
    Quarantined,
    Failed,
    Dropped,
    Live,
    Stale,
    Fenced,
    Crashed,
    Up,
    Down,
    Idle,
    Working,
}

/// The glyph and colour of `status` under `theme`.
pub fn style(theme: &Theme, status: Status) -> (char, Color) {
    match status {
        Status::Landed => ('█', theme.landed),
        Status::Approved => ('▓', theme.approved),
        Status::Running | Status::Working => ('▒', theme.status_running),
        Status::Ready => ('░', theme.status_waiting),
        Status::Blocked => ('░', theme.dim),
        Status::Quarantined => ('✕', theme.quarantined),
        Status::Failed | Status::Down => ('✕', theme.status_failed),
        Status::Fenced | Status::Crashed => ('!', theme.status_failed),
        Status::Live | Status::Up => ('█', theme.status_done),
        Status::Stale => ('!', theme.meter_mid),
        Status::Dropped => ('✕', theme.dim),
        Status::Idle => ('·', theme.dim),
    }
}

/// Only the colour of `status`.
pub fn color(theme: &Theme, status: Status) -> Color {
    style(theme, status).1
}

/// The status a feed or detail word names. A word this build does not know is `Ready`, the
/// waiting look.
pub fn from_word(word: &str) -> Status {
    match word {
        "ready" => Status::Ready,
        "blocked" => Status::Blocked,
        "running" | "open" => Status::Running,
        "working" => Status::Working,
        "approved" => Status::Approved,
        "landed" | "done" | "merged" => Status::Landed,
        "quarantined" => Status::Quarantined,
        "failed" => Status::Failed,
        "dropped" | "closed" => Status::Dropped,
        "live" => Status::Live,
        "stale" => Status::Stale,
        "fenced" => Status::Fenced,
        "crashed" => Status::Crashed,
        "up" => Status::Up,
        "down" => Status::Down,
        "idle" => Status::Idle,
        _ => Status::Ready,
    }
}

/// Chair liveness: any word that is not live, stale or fenced is a crash.
pub fn liveness(word: &str) -> Status {
    match from_word(word) {
        s @ (Status::Live | Status::Stale | Status::Fenced) => s,
        _ => Status::Crashed,
    }
}

/// A host is up or it is down.
pub fn host(word: &str) -> Status {
    if word == "up" {
        Status::Up
    } else {
        Status::Down
    }
}

/// A healthy or unhealthy check, such as a login or the store.
pub fn health(ok: bool) -> Status {
    if ok { Status::Up } else { Status::Down }
}

pub fn agent(state: AgentState) -> Status {
    match state {
        AgentState::Running => Status::Working,
        AgentState::Done => Status::Landed,
        AgentState::Failed => Status::Failed,
        AgentState::Unknown => Status::Ready,
    }
}

pub fn run_end(end: &RunEnd) -> Status {
    match end {
        RunEnd::Running => Status::Running,
        RunEnd::Landed => Status::Landed,
        RunEnd::Approved => Status::Approved,
        RunEnd::Quarantined { .. } => Status::Quarantined,
        RunEnd::Idle => Status::Idle,
        RunEnd::Died { .. } => Status::Failed,
        RunEnd::Stopped => Status::Dropped,
    }
}

pub fn outcome(outcome: Outcome) -> Status {
    match outcome {
        Outcome::Landed => Status::Landed,
        Outcome::Approved => Status::Approved,
        Outcome::Quarantined => Status::Quarantined,
        Outcome::Stopped => Status::Dropped,
        Outcome::Idle => Status::Idle,
        Outcome::Crashed => Status::Crashed,
        Outcome::Unknown => Status::Ready,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ThemeId;

    const ALL: [Status; 16] = [
        Status::Ready,
        Status::Blocked,
        Status::Running,
        Status::Approved,
        Status::Landed,
        Status::Quarantined,
        Status::Failed,
        Status::Dropped,
        Status::Live,
        Status::Stale,
        Status::Fenced,
        Status::Crashed,
        Status::Up,
        Status::Down,
        Status::Idle,
        Status::Working,
    ];

    fn expected(t: &Theme, s: Status) -> (char, Color) {
        match s {
            Status::Landed => ('█', t.landed),
            Status::Approved => ('▓', t.approved),
            Status::Running | Status::Working => ('▒', t.status_running),
            Status::Ready => ('░', t.status_waiting),
            Status::Blocked => ('░', t.dim),
            Status::Quarantined => ('✕', t.quarantined),
            Status::Failed | Status::Down => ('✕', t.status_failed),
            Status::Fenced | Status::Crashed => ('!', t.status_failed),
            Status::Live | Status::Up => ('█', t.status_done),
            Status::Stale => ('!', t.meter_mid),
            Status::Dropped => ('✕', t.dim),
            Status::Idle => ('·', t.dim),
        }
    }

    #[test]
    fn every_status_has_its_glyph_and_colour_in_both_themes() {
        for id in [ThemeId::Regatta, ThemeId::HarborLight] {
            let theme = crate::theme::resolve_for(id, Some("truecolor"));
            for s in ALL {
                assert_eq!(style(&theme, s), expected(&theme, s), "{s:?} in {id:?}");
            }
        }
    }

    #[test]
    fn history_colours_are_the_run_end_reference() {
        for id in [ThemeId::Regatta, ThemeId::HarborLight] {
            let t = crate::theme::resolve_for(id, Some("truecolor"));
            assert_eq!(color(&t, run_end(&RunEnd::Landed)), t.landed);
            assert_eq!(color(&t, run_end(&RunEnd::Approved)), t.approved);
            let q = RunEnd::Quarantined { cause: None };
            assert_eq!(color(&t, run_end(&q)), t.quarantined);
            assert_eq!(
                color(&t, run_end(&RunEnd::Died { cause: None })),
                t.status_failed
            );
        }
    }

    #[test]
    fn the_same_word_gets_the_same_status_in_every_pane() {
        assert_eq!(from_word("approved"), Status::Approved);
        assert_eq!(from_word("quarantined"), Status::Quarantined);
        assert_eq!(from_word("merged"), Status::Landed);
        assert_eq!(from_word("closed"), Status::Dropped);
        assert_eq!(from_word("nonsense"), Status::Ready);
        assert_eq!(agent(AgentState::Done), outcome(Outcome::Landed));
        assert_eq!(liveness("live"), Status::Live);
        assert_eq!(liveness("gone"), Status::Crashed);
        assert_eq!(host("up"), Status::Up);
        assert_eq!(host("sideways"), Status::Down);
    }

    #[test]
    fn slipstream_takes_its_status_colours_from_the_map() {
        for id in [ThemeId::Regatta, ThemeId::HarborLight] {
            let t = crate::theme::resolve_for(id, Some("truecolor"));
            let p = crate::theme::slipstream_palette_for(id);
            assert_eq!(p.done_chip, color(&t, Status::Landed));
            assert_eq!(p.running_chip, color(&t, Status::Running));
            assert_eq!(p.failed_chip, color(&t, Status::Failed));
        }
    }

    /// Strips a file down to its non-test source: everything before `#[cfg(test)]`.
    fn source_part(text: &str) -> &str {
        text.split("#[cfg(test)]").next().unwrap_or(text)
    }

    #[test]
    fn no_other_ui_file_names_a_status_token() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/ui");
        let tokens = [
            "status_running",
            "status_done",
            "status_failed",
            "status_waiting",
        ];
        let offenders: Vec<String> = std::fs::read_dir(dir)
            .expect("src/ui is readable")
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "rs"))
            .filter(|p| p.file_name().is_some_and(|n| n != "status.rs"))
            .filter_map(|p| {
                let text = std::fs::read_to_string(&p).ok()?;
                let hit = tokens.iter().find(|t| source_part(&text).contains(**t))?;
                Some(format!("{} names {hit}", p.display()))
            })
            .collect();
        assert!(offenders.is_empty(), "{offenders:?}");
    }
}
