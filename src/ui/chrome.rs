//! Drawing helpers the Slipstream page and the spend board share: spaced section labels, rounded
//! cards, the base and dim styles, filled-cell counts for bars, and ellipsis clipping.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::Span,
    widgets::{Block, BorderType, Borders, Paragraph},
};

use crate::theme::SlipstreamPalette;

/// `text` cut to at most `room` chars, with an ellipsis when it was cut.
pub(super) fn clip(text: &str, room: usize) -> String {
    if text.chars().count() > room {
        text.chars()
            .take(room.saturating_sub(1))
            .chain(std::iter::once('\u{2026}'))
            .take(room)
            .collect()
    } else {
        text.to_string()
    }
}

/// `RUNS` as `R U N S`.
pub(super) fn spaced(label: &str) -> String {
    label
        .chars()
        .map(String::from)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Filled and empty cell counts of a bar `width` cells wide. The fraction is clamped to 0..=1.
pub(super) fn bar_parts(fraction: f64, width: usize) -> (usize, usize) {
    let filled = ((fraction.clamp(0.0, 1.0) * width as f64).round() as usize).min(width);
    (filled, width - filled)
}

pub(super) fn first_row(rect: Rect) -> Rect {
    Rect {
        height: rect.height.min(1),
        ..rect
    }
}

pub(super) fn below_first_row(rect: Rect) -> Rect {
    Rect {
        y: rect.y + rect.height.min(1),
        height: rect.height.saturating_sub(1),
        ..rect
    }
}

pub(super) fn base_style(p: &SlipstreamPalette) -> Style {
    Style::default().fg(p.text).bg(p.ground)
}

pub(super) fn dim_style(p: &SlipstreamPalette) -> Style {
    Style::default().fg(p.dim)
}

pub(super) fn card_block(border: Color, bg: Color, p: &SlipstreamPalette) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .style(Style::default().fg(p.text).bg(bg))
}

pub(super) fn render_label(f: &mut Frame, rect: Rect, label: &str, p: &SlipstreamPalette) {
    f.render_widget(
        Paragraph::new(Span::styled(spaced(label), dim_style(p))),
        first_row(rect),
    );
}
