//! The chair panel's decision card: tracks which open decisions it has shown, records the
//! selected option, and builds the `cox chair answer` argv. Nothing here runs a process or moves
//! focus; each key returns a `KeyOutcome` for the caller to enact.

// The chair-panel-integration task consumes these items; until it lands nothing calls them.
#![allow(dead_code)]

use std::collections::HashSet;

use chrono::FixedOffset;
use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
};

use crate::feed::Decision;
use crate::theme::Theme;
use crate::ui::local_time;

/// The binary the answer argv starts with, the same one `feed::spawn_feed` runs.
const PROGRAM: &str = "cox";

/// What one key did, for the caller to enact. The card never acts on it itself.
#[derive(Debug, PartialEq, Eq)]
pub enum KeyOutcome {
    /// A number key recorded this 0-based option index.
    Selected(usize),
    /// `y` confirmed: the argv for the caller to run.
    Answer(Vec<String>),
    /// `Esc` cleared the selection. No answer was sent and the decision stays open.
    Cancelled,
    /// `Tab` asks the caller to move focus to the session.
    FocusSession,
    /// The key did nothing: no selection to confirm, an option out of range, or another key.
    Ignored,
}

#[derive(Debug, Default)]
pub struct DecisionCard {
    decision: Option<Decision>,
    shown: HashSet<String>,
    selected: Option<usize>,
}

/// The new ids in `listed` that `shown` lacks, and the ids still listed. Pure.
fn track(shown: &HashSet<String>, listed: &[Decision]) -> (bool, HashSet<String>) {
    let still_listed: HashSet<String> = listed.iter().map(|d| d.id.clone()).collect();
    let has_new = still_listed.iter().any(|id| !shown.contains(id));
    (has_new, still_listed)
}

/// The argv that answers `decision` with its option at index `option`, or None when the index
/// is out of range. The option is sent as its text, not its index.
pub fn answer_argv(program: &str, decision: &Decision, option: usize) -> Option<Vec<String>> {
    decision.options.get(option).map(|text| {
        vec![
            program.to_string(),
            "chair".to_string(),
            "answer".to_string(),
            decision.id.clone(),
            text.clone(),
        ]
    })
}

fn copy_of(decision: &Decision) -> Decision {
    Decision {
        id: decision.id.clone(),
        question: decision.question.clone(),
        options: decision.options.clone(),
        context: decision.context.clone(),
        asked_at: decision.asked_at.clone(),
    }
}

impl DecisionCard {
    pub fn new() -> Self {
        Self::default()
    }

    /// The decision the card currently shows, if any.
    pub fn decision(&self) -> Option<&Decision> {
        self.decision.as_ref()
    }

    /// The 0-based option index recorded so far.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Compares `listed` against the ids already shown. Returns true when an id appears that
    /// was not shown before. An id the feed no longer lists is forgotten, and when it was the
    /// current decision the card moves to the first listed one, or empties.
    pub fn note_decisions(&mut self, listed: &[Decision]) -> bool {
        let (has_new, still_listed) = track(&self.shown, listed);
        let current_id = self.decision.as_ref().map(|d| d.id.as_str());
        let keeps_current = current_id.is_some_and(|id| still_listed.contains(id));
        if !keeps_current {
            self.decision = listed.first().map(copy_of);
            self.selected = None;
        }
        self.shown = still_listed;
        has_new
    }

    /// Handles one key. Returns what happened and touches nothing outside the card.
    pub fn on_key(&mut self, key: KeyCode) -> KeyOutcome {
        if key == KeyCode::Tab {
            return KeyOutcome::FocusSession;
        }
        let Some(decision) = self.decision.as_ref() else {
            return KeyOutcome::Ignored;
        };
        match key {
            KeyCode::Char(c @ '1'..='9') => {
                let index = c as usize - '1' as usize;
                if index < decision.options.len() {
                    self.selected = Some(index);
                    KeyOutcome::Selected(index)
                } else {
                    KeyOutcome::Ignored
                }
            }
            KeyCode::Char('y') => match self
                .selected
                .and_then(|index| answer_argv(PROGRAM, decision, index))
            {
                Some(argv) => KeyOutcome::Answer(argv),
                None => KeyOutcome::Ignored,
            },
            KeyCode::Esc => {
                self.selected = None;
                KeyOutcome::Cancelled
            }
            _ => KeyOutcome::Ignored,
        }
    }

    /// Draws the question, context, asked-at time and numbered options into `area`.
    pub fn render(&self, f: &mut Frame, area: Rect, theme: &Theme, offset: FixedOffset) {
        let base = Style::default().fg(theme.fg).bg(theme.bg);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.accent))
            .title("decision");
        let lines = match &self.decision {
            Some(decision) => decision_lines(decision, self.selected, theme, offset),
            None => vec![Line::styled(
                "no open decision",
                Style::default().fg(theme.dim),
            )],
        };
        let paragraph = Paragraph::new(lines)
            .block(block)
            .style(base)
            .wrap(Wrap { trim: false });
        f.render_widget(paragraph, area);
    }
}

fn decision_lines<'a>(
    decision: &'a Decision,
    selected: Option<usize>,
    theme: &Theme,
    offset: FixedOffset,
) -> Vec<Line<'a>> {
    let header = vec![
        Line::styled(
            decision.question.as_str(),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Line::styled(decision.context.as_str(), Style::default().fg(theme.fg)),
        Line::styled(
            format!("asked {}", local_time(&decision.asked_at, offset)),
            Style::default().fg(theme.dim),
        ),
        Line::raw(""),
    ];
    let options = decision.options.iter().enumerate().map(|(index, text)| {
        let style = if selected == Some(index) {
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.fg)
        };
        Line::from(Span::styled(format!("{}. {}", index + 1, text), style))
    });
    let hint = Line::styled(
        "number selects, y confirms, Esc clears, Tab goes to the session",
        Style::default().fg(theme.dim),
    );
    header
        .into_iter()
        .chain(options)
        .chain([Line::raw(""), hint])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirm_builds_the_answer_argv_for_the_selected_option() {
        let decision = Decision {
            id: "dec-7".to_string(),
            question: "Ship it?".to_string(),
            options: vec!["yes".to_string(), "no".to_string()],
            context: "CI is green".to_string(),
            asked_at: "2026-10-04T12:00:00Z".to_string(),
        };
        let expected: Vec<String> = ["cox", "chair", "answer", "dec-7", "no"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(answer_argv("cox", &decision, 1), Some(expected));
    }

    fn decision(id: &str) -> Decision {
        Decision {
            id: id.to_string(),
            question: "Ship it?".to_string(),
            options: vec!["yes".to_string(), "no".to_string()],
            context: "CI is green".to_string(),
            asked_at: "2026-10-04T12:00:00Z".to_string(),
        }
    }

    #[test]
    fn note_decisions_is_true_once_per_id_and_forgets_ids_the_feed_dropped() {
        let mut card = DecisionCard::new();
        assert!(card.note_decisions(&[decision("a")]));
        assert!(!card.note_decisions(&[decision("a")]));
        assert!(!card.note_decisions(&[]));
        assert!(card.note_decisions(&[decision("a")]));
    }

    #[test]
    fn keys_return_outcomes_and_esc_keeps_the_decision_open() {
        let mut card = DecisionCard::new();
        assert_eq!(card.on_key(KeyCode::Tab), KeyOutcome::FocusSession);
        card.note_decisions(&[decision("a")]);
        assert_eq!(card.on_key(KeyCode::Char('y')), KeyOutcome::Ignored);
        assert_eq!(card.on_key(KeyCode::Char('2')), KeyOutcome::Selected(1));
        assert_eq!(
            card.on_key(KeyCode::Char('y')),
            KeyOutcome::Answer(vec![
                "cox".to_string(),
                "chair".to_string(),
                "answer".to_string(),
                "a".to_string(),
                "no".to_string(),
            ])
        );
        assert_eq!(card.on_key(KeyCode::Esc), KeyOutcome::Cancelled);
        assert_eq!(card.selected(), None);
        assert!(card.decision().is_some());
        assert_eq!(card.on_key(KeyCode::Tab), KeyOutcome::FocusSession);
    }
}
