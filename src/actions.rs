//! The cox-command actions (pause, kill, move, lanes, drain, priority, inbox) as pure data.
//! This is the one place that knows which key means which `cox` command. Nothing here runs a
//! command, reads the clock or touches the terminal.

use crate::feed::FeedSnapshot;

/// What a key press applies to, carrying the id the command needs.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Run(String),
    Initiative(String),
    Machine(String),
    InboxItem(String),
}

impl Target {
    fn id(&self) -> &str {
        match self {
            Self::Run(id) | Self::Initiative(id) | Self::Machine(id) | Self::InboxItem(id) => id,
        }
    }
}

/// One verb per variant. Lane and priority variants carry the already computed new value.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    RunPause {
        run: String,
    },
    RunKill {
        run: String,
    },
    /// The 'm' key opens a prefilled palette, so nothing builds this outside tests yet.
    #[allow(dead_code)]
    RunMove {
        run: String,
        host: String,
    },
    LanesUp {
        machine: String,
        lanes: u32,
    },
    LanesDown {
        machine: String,
        lanes: u32,
    },
    MachineDrain {
        machine: String,
    },
    MachineActivate {
        machine: String,
    },
    PriorityUp {
        initiative: String,
        priority: u32,
    },
    PriorityDown {
        initiative: String,
        priority: u32,
    },
    InboxAccept {
        id: String,
    },
    InboxDeny {
        id: String,
    },
}

/// The verb a key selects, before a target and the feed supply the rest.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Verb {
    Pause,
    Kill,
    Move,
    LanesUp,
    LanesDown,
    Drain,
    Activate,
    PriorityUp,
    PriorityDown,
    Accept,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Binding {
    pub key: char,
    pub label: &'static str,
    pub verb: Verb,
}

const fn bind(key: char, label: &'static str, verb: Verb) -> Binding {
    Binding { key, label, verb }
}

/// The keys that act on this kind of target. Up and Down are selection, so they are not bound.
pub fn bindings(target: &Target) -> Vec<Binding> {
    match target {
        Target::Run(_) => vec![
            bind('p', "pause", Verb::Pause),
            bind('k', "kill", Verb::Kill),
            bind('m', "move", Verb::Move),
        ],
        Target::Machine(_) => vec![
            bind('+', "lanes up", Verb::LanesUp),
            bind('-', "lanes down", Verb::LanesDown),
            bind('d', "drain", Verb::Drain),
            bind('a', "activate", Verb::Activate),
        ],
        Target::Initiative(_) => vec![
            bind(']', "priority up", Verb::PriorityUp),
            bind('[', "priority down", Verb::PriorityDown),
        ],
        Target::InboxItem(_) => vec![
            bind('a', "accept", Verb::Accept),
            bind('x', "deny", Verb::Deny),
        ],
    }
}

fn capacity_of(snapshot: &FeedSnapshot, machine: &str) -> Option<u32> {
    snapshot
        .machines
        .iter()
        .find(|m| m.name == machine)
        .map(|m| m.capacity)
}

fn priority_of(snapshot: &FeedSnapshot, initiative: &str) -> Option<u32> {
    snapshot
        .queue
        .iter()
        .find(|q| q.initiative == initiative)
        .map(|q| q.priority)
}

/// The action a key press means on a target, or None for an unbound key, for move (it has no
/// host yet, see `move_prefill`), and for a machine or initiative the snapshot does not list.
pub fn action_for(key: char, target: &Target, snapshot: &FeedSnapshot) -> Option<Action> {
    let verb = bindings(target).into_iter().find(|b| b.key == key)?.verb;
    let id = target.id().to_string();
    match verb {
        Verb::Pause => Some(Action::RunPause { run: id }),
        Verb::Kill => Some(Action::RunKill { run: id }),
        Verb::Move => None,
        Verb::LanesUp => capacity_of(snapshot, &id).map(|n| Action::LanesUp {
            machine: id,
            lanes: n.saturating_add(1),
        }),
        Verb::LanesDown => capacity_of(snapshot, &id).map(|n| Action::LanesDown {
            machine: id,
            lanes: n.saturating_sub(1),
        }),
        Verb::Drain => Some(Action::MachineDrain { machine: id }),
        Verb::Activate => Some(Action::MachineActivate { machine: id }),
        Verb::PriorityUp => priority_of(snapshot, &id).map(|n| Action::PriorityUp {
            initiative: id,
            priority: n.saturating_add(1),
        }),
        Verb::PriorityDown => priority_of(snapshot, &id).map(|n| Action::PriorityDown {
            initiative: id,
            priority: n.saturating_sub(1),
        }),
        Verb::Accept => Some(Action::InboxAccept { id }),
        Verb::Deny => Some(Action::InboxDeny { id }),
    }
}

fn words(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| p.to_string()).collect()
}

/// The `cox` words of a move up to and including `--to`; the host follows.
fn move_words(run: &str) -> Vec<String> {
    words(&["runs", "move", run, "--to"])
}

/// What the palette opens pre-filled for a move: the user types the destination host.
pub fn move_prefill(run: &str) -> String {
    format!("{} ", join_quoted(&move_words(run)))
}

fn quote(token: &str) -> String {
    let needs = token.is_empty()
        || token
            .chars()
            .any(|c| c.is_whitespace() || c == '\'' || c == '"');
    if needs {
        format!("'{}'", token.replace('\'', "'\\''"))
    } else {
        token.to_string()
    }
}

fn join_quoted(tokens: &[String]) -> String {
    tokens
        .iter()
        .map(|t| quote(t))
        .collect::<Vec<_>>()
        .join(" ")
}

impl Action {
    /// The full command line, starting with `cox`. The only place command spellings live.
    pub fn argv(&self) -> Vec<String> {
        let tail = match self {
            Self::RunPause { run } => words(&["runs", "pause", run]),
            Self::RunKill { run } => words(&["runs", "stop", run]),
            Self::RunMove { run, host } => [move_words(run), vec![host.clone()]].concat(),
            Self::LanesUp { machine, lanes } | Self::LanesDown { machine, lanes } => {
                words(&["host", "capacity", machine, &lanes.to_string()])
            }
            Self::MachineDrain { machine } => words(&["host", "drain", machine]),
            Self::MachineActivate { machine } => words(&["host", "activate", machine]),
            Self::PriorityUp {
                initiative,
                priority,
            }
            | Self::PriorityDown {
                initiative,
                priority,
            } => words(&["route", "priority", initiative, &priority.to_string()]),
            Self::InboxAccept { id } => words(&["inbox", "accept", id]),
            Self::InboxDeny { id } => words(&["inbox", "deny", id]),
        };
        [words(&["cox"]), tail].concat()
    }

    /// The argv as one shell line, quoting any token with whitespace or a quote.
    pub fn display(&self) -> String {
        join_quoted(&self.argv())
    }

    pub fn title(&self) -> String {
        match self {
            Self::RunPause { run } => format!("Pause run {run}"),
            Self::RunKill { run } => format!("Kill run {run}"),
            Self::RunMove { run, host } => format!("Move run {run} to {host}"),
            Self::LanesUp { machine, lanes } | Self::LanesDown { machine, lanes } => {
                format!("Set lanes on {machine} to {lanes}")
            }
            Self::MachineDrain { machine } => format!("Drain machine {machine}"),
            Self::MachineActivate { machine } => format!("Activate machine {machine}"),
            Self::PriorityUp {
                initiative,
                priority,
            }
            | Self::PriorityDown {
                initiative,
                priority,
            } => format!("Set priority of {initiative} to {priority}"),
            Self::InboxAccept { id } => format!("Accept inbox item {id}"),
            Self::InboxDeny { id } => format!("Deny inbox item {id}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feed::parse_snapshot;

    const FIXTURE: &str = include_str!("../tests/fixtures/dash_feed_v1.json");
    const INITIATIVE: &str = "dash-feed-streams-a-versioned-json-snapshot-of";

    fn snapshot() -> FeedSnapshot {
        parse_snapshot(FIXTURE).expect("fixture parses")
    }

    fn s(v: &[&str]) -> Vec<String> {
        words(v)
    }

    #[test]
    fn every_action_maps_to_its_exact_argv() {
        let table = [
            (
                Action::RunPause { run: "r-12".into() },
                s(&["cox", "runs", "pause", "r-12"]),
            ),
            (
                Action::RunKill { run: "r-12".into() },
                s(&["cox", "runs", "stop", "r-12"]),
            ),
            (
                Action::RunMove {
                    run: "r-12".into(),
                    host: "mini".into(),
                },
                s(&["cox", "runs", "move", "r-12", "--to", "mini"]),
            ),
            (
                Action::LanesUp {
                    machine: "omarchy".into(),
                    lanes: 4,
                },
                s(&["cox", "host", "capacity", "omarchy", "4"]),
            ),
            (
                Action::LanesDown {
                    machine: "omarchy".into(),
                    lanes: 2,
                },
                s(&["cox", "host", "capacity", "omarchy", "2"]),
            ),
            (
                Action::MachineDrain {
                    machine: "omarchy".into(),
                },
                s(&["cox", "host", "drain", "omarchy"]),
            ),
            (
                Action::MachineActivate {
                    machine: "omarchy".into(),
                },
                s(&["cox", "host", "activate", "omarchy"]),
            ),
            (
                Action::PriorityUp {
                    initiative: "init-a".into(),
                    priority: 2,
                },
                s(&["cox", "route", "priority", "init-a", "2"]),
            ),
            (
                Action::PriorityDown {
                    initiative: "init-a".into(),
                    priority: 0,
                },
                s(&["cox", "route", "priority", "init-a", "0"]),
            ),
            (
                Action::InboxAccept { id: "t-1".into() },
                s(&["cox", "inbox", "accept", "t-1"]),
            ),
            (
                Action::InboxDeny { id: "t-1".into() },
                s(&["cox", "inbox", "deny", "t-1"]),
            ),
        ];
        for (action, argv) in table {
            assert_eq!(action.argv(), argv, "{action:?}");
        }
    }

    #[test]
    fn display_quotes_a_token_with_a_space() {
        let action = Action::RunMove {
            run: "r-12".into(),
            host: "my host".into(),
        };
        assert_eq!(action.display(), "cox runs move r-12 --to 'my host'");
    }

    #[test]
    fn display_escapes_an_embedded_single_quote() {
        let action = Action::InboxDeny { id: "it's".into() };
        assert_eq!(action.display(), "cox inbox deny 'it'\\''s'");
    }

    #[test]
    fn title_is_a_short_sentence() {
        let action = Action::RunPause { run: "r-12".into() };
        assert_eq!(action.title(), "Pause run r-12");
    }

    #[test]
    fn lanes_down_clamps_at_zero() {
        let mut snap = snapshot();
        snap.machines[0].capacity = 0;
        let got = action_for('-', &Target::Machine("omarchy".into()), &snap);
        assert_eq!(
            got,
            Some(Action::LanesDown {
                machine: "omarchy".into(),
                lanes: 0
            })
        );
    }

    #[test]
    fn priority_down_clamps_at_zero() {
        let mut snap = snapshot();
        snap.queue[0].priority = 0;
        let got = action_for('[', &Target::Initiative(INITIATIVE.into()), &snap);
        assert_eq!(
            got,
            Some(Action::PriorityDown {
                initiative: INITIATIVE.into(),
                priority: 0
            })
        );
    }

    #[test]
    fn lanes_and_priority_step_from_the_feed_values() {
        let snap = snapshot();
        let up = action_for('+', &Target::Machine("omarchy".into()), &snap);
        assert_eq!(
            up,
            Some(Action::LanesUp {
                machine: "omarchy".into(),
                lanes: 4
            })
        );
        let prio = action_for(']', &Target::Initiative(INITIATIVE.into()), &snap);
        assert_eq!(
            prio,
            Some(Action::PriorityUp {
                initiative: INITIATIVE.into(),
                priority: 2
            })
        );
    }

    #[test]
    fn a_machine_missing_from_the_feed_yields_no_action() {
        let got = action_for('+', &Target::Machine("ghost".into()), &snapshot());
        assert_eq!(got, None);
    }

    #[test]
    fn move_is_not_built_and_the_palette_prefill_is_literal() {
        assert_eq!(
            action_for('m', &Target::Run("r-12".into()), &snapshot()),
            None
        );
        assert_eq!(move_prefill("r-12"), "runs move r-12 --to ");
    }

    #[test]
    fn bindings_have_no_duplicate_key_within_a_target_kind() {
        let kinds = [
            Target::Run(String::new()),
            Target::Machine(String::new()),
            Target::Initiative(String::new()),
            Target::InboxItem(String::new()),
        ];
        for kind in kinds {
            let keys: Vec<char> = bindings(&kind).iter().map(|b| b.key).collect();
            let mut unique = keys.clone();
            unique.sort();
            unique.dedup();
            assert_eq!(keys.len(), unique.len(), "{kind:?}");
        }
    }
}
