//! Pure formatting rules for the chair card. Plain data in, strings and small enums out: no
//! `Chair` type, no clock, no time-zone conversion, no colours. The frame maps roles to theme
//! colours.
#![allow(dead_code)]

use std::iter::once;

/// Seconds between chair ticks. The feed carries no interval field, so this is the one place it is set.
pub const TICK_INTERVAL_S: u64 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    Fresh,
    Stale,
}

/// How a count is drawn. Failure and needs-chair apply only when the count is non-zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountRole {
    Plain,
    Failure,
    NeedsChair,
}

/// Compact age: seconds under a minute, whole minutes under an hour, else whole hours.
pub fn short_age(seconds: u64) -> String {
    match seconds {
        0..=59 => format!("{seconds}s"),
        60..=3599 => format!("{}m", seconds / 60),
        _ => format!("{}h", seconds / 3600),
    }
}

pub fn doing_line(label: &str, target: &str, age_s: u64) -> String {
    format!("doing: {label} {target} \u{b7} {}", short_age(age_s))
}

pub fn idle_line() -> String {
    "idle between ticks".to_string()
}

/// `local_time` arrives already localised; it is printed as given.
pub fn last_tick_line(local_time: &str, age_s: u64) -> String {
    format!("last tick {local_time} \u{b7} {} ago", short_age(age_s))
}

/// Stale only when the beat is older than twice the tick interval; exactly twice is still fresh.
pub fn beat_freshness(beat_age_s: u64, tick_interval_s: u64) -> Freshness {
    if beat_age_s > 2 * tick_interval_s {
        Freshness::Stale
    } else {
        Freshness::Fresh
    }
}

fn role_if_nonzero(count: u32, role: CountRole) -> CountRole {
    if count > 0 { role } else { CountRole::Plain }
}

/// The row's four labelled counts, in order, each with its role.
pub fn counts_parts(
    lands: u32,
    launches: u32,
    failures: u32,
    needs_chair: u32,
) -> [(String, CountRole); 4] {
    [
        (format!("lands {lands}"), CountRole::Plain),
        (format!("launches {launches}"), CountRole::Plain),
        (
            format!("failures {failures}"),
            role_if_nonzero(failures, CountRole::Failure),
        ),
        (
            format!("needs chair {needs_chair}"),
            role_if_nonzero(needs_chair, CountRole::NeedsChair),
        ),
    ]
}

/// A word longer than `width` is cut into `width`-sized pieces; any other word stays whole.
fn word_pieces(word: &str, width: usize) -> Vec<String> {
    word.chars()
        .collect::<Vec<char>>()
        .chunks(width)
        .map(|piece| piece.iter().collect())
        .collect()
}

/// Greedy wrap on whitespace to `width` characters. A zero width is treated as one.
pub fn wrap_status(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    text.split_whitespace()
        .flat_map(|word| word_pieces(word, width))
        .fold(Vec::new(), |lines: Vec<String>, piece| {
            let fits = lines
                .last()
                .is_some_and(|last| last.chars().count() + 1 + piece.chars().count() <= width);
            if fits {
                let last_index = lines.len() - 1;
                lines
                    .into_iter()
                    .enumerate()
                    .map(|(i, line)| {
                        if i == last_index {
                            format!("{line} {piece}")
                        } else {
                            line
                        }
                    })
                    .collect()
            } else {
                lines.into_iter().chain(once(piece)).collect()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_age_picks_seconds_minutes_or_hours() {
        assert_eq!(short_age(45), "45s");
        assert_eq!(short_age(59), "59s");
        assert_eq!(short_age(60), "1m");
        assert_eq!(short_age(180), "3m");
        assert_eq!(short_age(3599), "59m");
        assert_eq!(short_age(3600), "1h");
        assert_eq!(short_age(7200), "2h");
    }

    #[test]
    fn doing_line_joins_label_target_and_age() {
        assert_eq!(
            doing_line("land_phase", "api-runners/runner-parity", 180),
            "doing: land_phase api-runners/runner-parity \u{b7} 3m"
        );
    }

    #[test]
    fn idle_line_says_idle_between_ticks() {
        assert_eq!(idle_line(), "idle between ticks");
    }

    #[test]
    fn last_tick_line_prints_the_given_time_and_age() {
        assert_eq!(
            last_tick_line("12:01", 120),
            "last tick 12:01 \u{b7} 2m ago"
        );
    }

    #[test]
    fn exactly_twice_the_interval_is_fresh() {
        assert_eq!(beat_freshness(120, 60), Freshness::Fresh);
    }

    #[test]
    fn one_second_over_twice_the_interval_is_stale() {
        assert_eq!(beat_freshness(121, 60), Freshness::Stale);
    }

    #[test]
    fn zero_counts_are_all_plain() {
        let parts = counts_parts(3, 2, 0, 0);
        assert_eq!(
            parts,
            [
                ("lands 3".to_string(), CountRole::Plain),
                ("launches 2".to_string(), CountRole::Plain),
                ("failures 0".to_string(), CountRole::Plain),
                ("needs chair 0".to_string(), CountRole::Plain),
            ]
        );
    }

    #[test]
    fn non_zero_failures_and_needs_chair_take_their_roles() {
        let parts = counts_parts(3, 2, 1, 4);
        assert_eq!(
            parts,
            [
                ("lands 3".to_string(), CountRole::Plain),
                ("launches 2".to_string(), CountRole::Plain),
                ("failures 1".to_string(), CountRole::Failure),
                ("needs chair 4".to_string(), CountRole::NeedsChair),
            ]
        );
    }

    #[test]
    fn wrap_status_breaks_a_long_line_onto_three_lines() {
        assert_eq!(
            wrap_status("one two three four five six", 10),
            vec![
                "one two".to_string(),
                "three four".to_string(),
                "five six".to_string()
            ]
        );
    }

    #[test]
    fn wrap_status_returns_short_text_as_one_line() {
        assert_eq!(wrap_status("all quiet", 40), vec!["all quiet".to_string()]);
    }

    #[test]
    fn wrap_status_splits_only_a_word_longer_than_the_width() {
        assert_eq!(
            wrap_status("abcdefghij", 4),
            vec!["abcd".to_string(), "efgh".to_string(), "ij".to_string()]
        );
    }

    #[test]
    fn wrap_status_of_blank_text_is_empty() {
        assert_eq!(wrap_status("   ", 10), Vec::<String>::new());
    }
}
