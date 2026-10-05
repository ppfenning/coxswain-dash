//! The two themes `t` toggles between: literal, hard-coded truecolor sets, no I/O. `App::theme`
//! carries only which of the two is selected; `resolve_for` is the pure lookup from that id and
//! the `COLORTERM` value to the colors a page renders with, and `resolve` is the one thin edge
//! that reads the environment. A terminal that does not announce `truecolor` or `24bit` gets the
//! same palette mapped to the nearest xterm-256 index. `themes/slipstream.toml` (see
//! `crate::app::ThemeId`'s doc comment) is a separate asset the Slipstream page loads on its own;
//! `SlipstreamPalette` carries the canvas values for that page but nothing renders from it yet.

use ratatui::style::Color;

use crate::app::ThemeId;

// The canvas fields below are defined ahead of the renderers that will read them, and CI
// denies warnings, so the allow stays until regatta.rs and slipstream.rs are wired up.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub fg: Color,
    pub bg: Color,
    pub panel: Color,
    pub accent: Color,
    pub dim: Color,
    pub border: Color,
    /// Focused frame border and the frame-title color.
    pub border_focus: Color,
    pub track: Color,
    pub selected_row: Color,
    /// Low, mid (at 60%) and high stop of the meter gradient.
    pub gradient: [Color; 3],
    pub meter_low: Color,
    pub meter_mid: Color,
    pub meter_high: Color,
    pub stop_tick: Color,
    pub approved: Color,
    pub landed: Color,
    pub quarantined: Color,
    pub live_dot: Color,
    pub status_running: Color,
    pub status_done: Color,
    pub status_failed: Color,
    pub status_waiting: Color,
    pub machine_accents: [Color; 4],
}

/// The Slipstream page's own palette, separate from `Theme`: a dark and a Harbor Light variant.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlipstreamPalette {
    pub ground: Color,
    pub text: Color,
    pub dim: Color,
    pub card: Color,
    pub card_border: Color,
    pub selected_border: Color,
    pub accent: Color,
    pub done_chip: Color,
    pub done_chip_text: Color,
    pub pending_chip: Color,
    pub pending_chip_text: Color,
    pub failed_chip: Color,
    pub accents: [Color; 2],
}

const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

const REGATTA: Theme = Theme {
    fg: rgb(0xc9d1d9),
    bg: rgb(0x0d1117),
    panel: rgb(0x0d1117),
    accent: rgb(0x79c0ff),
    dim: rgb(0x8b949e),
    border: rgb(0x30363d),
    border_focus: rgb(0x79c0ff),
    track: rgb(0x21262d),
    selected_row: rgb(0x161b22),
    gradient: [rgb(0x56d364), rgb(0xe3b341), rgb(0xff7b72)],
    meter_low: rgb(0x56d364),
    meter_mid: rgb(0xe3b341),
    meter_high: rgb(0xff7b72),
    stop_tick: rgb(0xc9d1d9),
    approved: rgb(0xd2a8ff),
    landed: rgb(0x56d364),
    quarantined: rgb(0xff7b72),
    live_dot: rgb(0x56d364),
    status_running: rgb(0x56d4dd),
    status_done: rgb(0x56d364),
    status_failed: rgb(0xff7b72),
    status_waiting: rgb(0x8b949e),
    machine_accents: [rgb(0x79c0ff), rgb(0xffa657), rgb(0xd2a8ff), rgb(0x56d4dd)],
};

const HARBOR_LIGHT: Theme = Theme {
    fg: rgb(0x1f2328),
    bg: rgb(0xf7f5ef),
    panel: rgb(0xfbfaf6),
    accent: rgb(0x0550ae),
    dim: rgb(0x57606a),
    border: rgb(0xd6d0c0),
    border_focus: rgb(0x0550ae),
    track: rgb(0xe8e3d6),
    selected_row: rgb(0xefeadd),
    gradient: [rgb(0x1a7f37), rgb(0x8a5a00), rgb(0xc21d2a)],
    meter_low: rgb(0x1a7f37),
    meter_mid: rgb(0x8a5a00),
    meter_high: rgb(0xc21d2a),
    // The canvas leaves stop tick and live dot unstated: text and landed colors.
    stop_tick: rgb(0x1f2328),
    approved: rgb(0x7438c9),
    landed: rgb(0x1a7f37),
    quarantined: rgb(0xc21d2a),
    live_dot: rgb(0x1a7f37),
    status_running: rgb(0x0a6c74),
    status_done: rgb(0x1a7f37),
    status_failed: rgb(0xc21d2a),
    status_waiting: rgb(0x57606a),
    machine_accents: [rgb(0x0550ae), rgb(0xb04400), rgb(0x7438c9), rgb(0x0a6c74)],
};

/// Slipstream on Harbor Light: the light theme's ground, text, borders and status colours.
const SLIPSTREAM_LIGHT: SlipstreamPalette = SlipstreamPalette {
    ground: rgb(0xf7f5ef),
    text: rgb(0x1f2328),
    dim: rgb(0x57606a),
    card: rgb(0xfbfaf6),
    card_border: rgb(0xd6d0c0),
    selected_border: rgb(0x0a6c74),
    accent: rgb(0x0a6c74),
    done_chip: rgb(0x1a7f37),
    done_chip_text: rgb(0xfbfaf6),
    pending_chip: rgb(0xe8e3d6),
    pending_chip_text: rgb(0x57606a),
    failed_chip: rgb(0xc21d2a),
    accents: [rgb(0x0550ae), rgb(0xb04400)],
};

const SLIPSTREAM: SlipstreamPalette = SlipstreamPalette {
    ground: rgb(0x0b1020),
    text: rgb(0xd6deeb),
    dim: rgb(0x7f8ba6),
    card: rgb(0x111a30),
    card_border: rgb(0x1e2a44),
    selected_border: rgb(0x2dd4bf),
    accent: rgb(0x2dd4bf),
    done_chip: rgb(0x7ee787),
    done_chip_text: rgb(0x0b1020),
    pending_chip: rgb(0x17213a),
    pending_chip_text: rgb(0x7f8ba6),
    failed_chip: rgb(0xff8a80),
    accents: [rgb(0x82aaff), rgb(0xffb86c)],
};

/// The channel values of the xterm 6x6x6 color cube, indexed 0..6.
const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];

fn dist2(a: (u8, u8, u8), b: (u8, u8, u8)) -> u32 {
    let d = |x: u8, y: u8| u32::from(x.abs_diff(y)).pow(2);
    d(a.0, b.0) + d(a.1, b.1) + d(a.2, b.2)
}

/// Nearest xterm-256 index for an RGB color: the closer of the nearest cube entry (16..=231)
/// and the nearest grey-ramp entry (232..=255, levels 8 + 10 * step). A tie goes to the cube.
pub fn nearest_256(r: u8, g: u8, b: u8) -> u8 {
    let want = (r, g, b);
    let snap = |c: u8| {
        (0..CUBE.len())
            .min_by_key(|&i| CUBE[i].abs_diff(c))
            .unwrap_or(0)
    };
    let (ri, gi, bi) = (snap(r), snap(g), snap(b));
    let cube_rgb = (CUBE[ri], CUBE[gi], CUBE[bi]);
    let (grey_step, grey_rgb) = (0..24u8)
        .map(|step| (step, (8 + 10 * step, 8 + 10 * step, 8 + 10 * step)))
        .min_by_key(|&(_, grey)| dist2(want, grey))
        .unwrap_or((0, (8, 8, 8)));
    if dist2(want, cube_rgb) <= dist2(want, grey_rgb) {
        (16 + 36 * ri + 6 * gi + bi) as u8
    } else {
        232 + grey_step
    }
}

/// Maps `Color::Rgb` to its nearest `Color::Indexed`; every other variant is returned as is.
fn to_indexed(c: Color) -> Color {
    match c {
        Color::Rgb(r, g, b) => Color::Indexed(nearest_256(r, g, b)),
        other => other,
    }
}

impl Theme {
    /// The same theme with every color mapped to its nearest xterm-256 index.
    pub fn to_256(self) -> Theme {
        let [g0, g1, g2] = self.gradient;
        let [a0, a1, a2, a3] = self.machine_accents;
        Theme {
            fg: to_indexed(self.fg),
            bg: to_indexed(self.bg),
            panel: to_indexed(self.panel),
            accent: to_indexed(self.accent),
            dim: to_indexed(self.dim),
            border: to_indexed(self.border),
            border_focus: to_indexed(self.border_focus),
            track: to_indexed(self.track),
            selected_row: to_indexed(self.selected_row),
            gradient: [to_indexed(g0), to_indexed(g1), to_indexed(g2)],
            meter_low: to_indexed(self.meter_low),
            meter_mid: to_indexed(self.meter_mid),
            meter_high: to_indexed(self.meter_high),
            stop_tick: to_indexed(self.stop_tick),
            approved: to_indexed(self.approved),
            landed: to_indexed(self.landed),
            quarantined: to_indexed(self.quarantined),
            live_dot: to_indexed(self.live_dot),
            status_running: to_indexed(self.status_running),
            status_done: to_indexed(self.status_done),
            status_failed: to_indexed(self.status_failed),
            status_waiting: to_indexed(self.status_waiting),
            machine_accents: [
                to_indexed(a0),
                to_indexed(a1),
                to_indexed(a2),
                to_indexed(a3),
            ],
        }
    }
}

impl SlipstreamPalette {
    /// The same palette with every color mapped to its nearest xterm-256 index.
    #[allow(dead_code)]
    pub fn to_256(self) -> SlipstreamPalette {
        let [a0, a1] = self.accents;
        SlipstreamPalette {
            ground: to_indexed(self.ground),
            text: to_indexed(self.text),
            dim: to_indexed(self.dim),
            card: to_indexed(self.card),
            card_border: to_indexed(self.card_border),
            selected_border: to_indexed(self.selected_border),
            accent: to_indexed(self.accent),
            done_chip: to_indexed(self.done_chip),
            done_chip_text: to_indexed(self.done_chip_text),
            pending_chip: to_indexed(self.pending_chip),
            pending_chip_text: to_indexed(self.pending_chip_text),
            failed_chip: to_indexed(self.failed_chip),
            accents: [to_indexed(a0), to_indexed(a1)],
        }
    }
}

/// The Slipstream page palette in truecolor. Pure.
#[allow(dead_code)]
pub fn slipstream_palette() -> SlipstreamPalette {
    SLIPSTREAM
}

/// The Slipstream palette for a theme: the dark canvas palette, or the light one built from Harbor Light.
pub fn slipstream_palette_for(id: crate::app::ThemeId) -> SlipstreamPalette {
    match id {
        crate::app::ThemeId::Regatta => SLIPSTREAM,
        crate::app::ThemeId::HarborLight => SLIPSTREAM_LIGHT,
    }
}

/// Looks up the `Theme` for a `ThemeId`. Truecolor when `colorterm` is `truecolor` or `24bit`,
/// otherwise the same colors mapped to the nearest xterm-256 index. Pure.
pub fn resolve_for(id: ThemeId, colorterm: Option<&str>) -> Theme {
    let theme = match id {
        ThemeId::Regatta => REGATTA,
        ThemeId::HarborLight => HARBOR_LIGHT,
    };
    match colorterm {
        Some("truecolor") | Some("24bit") => theme,
        _ => theme.to_256(),
    }
}

/// The edge: reads `COLORTERM` and hands it to `resolve_for`.
pub fn resolve(id: ThemeId) -> Theme {
    resolve_for(id, std::env::var("COLORTERM").ok().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regatta_and_harbor_light_resolve_to_different_themes() {
        assert_ne!(resolve(ThemeId::Regatta), resolve(ThemeId::HarborLight));
    }

    #[test]
    fn nearest_256_maps_black_and_white_to_the_cube_corners() {
        assert_eq!(nearest_256(0, 0, 0), 16);
        assert_eq!(nearest_256(255, 255, 255), 231);
    }

    #[test]
    fn nearest_256_maps_a_mid_grey_to_the_grey_ramp() {
        assert_eq!(nearest_256(0x80, 0x80, 0x80), 244);
    }

    #[test]
    fn nearest_256_maps_79c0ff_to_the_cube() {
        assert_eq!(nearest_256(0x79, 0xc0, 0xff), 111);
    }

    #[test]
    fn nearest_256_picks_the_closer_of_cube_and_grey_for_off_cube_colors() {
        assert_eq!(nearest_256(0x12, 0x34, 0x56), 23);
        assert_eq!(nearest_256(100, 100, 100), 241);
    }

    #[test]
    fn resolve_for_keeps_truecolor_for_truecolor_and_24bit() {
        assert_eq!(resolve_for(ThemeId::Regatta, Some("truecolor")), REGATTA);
        assert_eq!(resolve_for(ThemeId::Regatta, Some("24bit")), REGATTA);
        assert_eq!(
            resolve_for(ThemeId::HarborLight, Some("truecolor")),
            HARBOR_LIGHT
        );
    }

    #[test]
    fn resolve_for_downgrades_when_unset_or_not_truecolor() {
        let down = resolve_for(ThemeId::Regatta, None);
        assert_eq!(down, REGATTA.to_256());
        assert_eq!(down.border_focus, Color::Indexed(111));
        assert_eq!(resolve_for(ThemeId::Regatta, Some("")), down);
        assert_eq!(resolve_for(ThemeId::Regatta, Some("256color")), down);
    }

    #[test]
    fn regatta_focus_and_gradient_match_the_canvas() {
        let t = resolve_for(ThemeId::Regatta, Some("truecolor"));
        assert_eq!(t.border_focus, Color::Rgb(0x79, 0xc0, 0xff));
        assert_eq!(
            t.gradient,
            [
                Color::Rgb(0x56, 0xd3, 0x64),
                Color::Rgb(0xe3, 0xb3, 0x41),
                Color::Rgb(0xff, 0x7b, 0x72)
            ]
        );
    }

    #[test]
    fn harbor_light_focus_and_gradient_match_the_canvas() {
        let t = resolve_for(ThemeId::HarborLight, Some("truecolor"));
        assert_eq!(t.border_focus, Color::Rgb(0x05, 0x50, 0xae));
        assert_eq!(
            t.gradient,
            [
                Color::Rgb(0x1a, 0x7f, 0x37),
                Color::Rgb(0x8a, 0x5a, 0x00),
                Color::Rgb(0xc2, 0x1d, 0x2a)
            ]
        );
    }

    #[test]
    fn the_slipstream_palette_downgrades_every_field() {
        let p = slipstream_palette();
        assert_eq!(p.accent, Color::Rgb(0x2d, 0xd4, 0xbf));
        let down = p.to_256();
        assert_eq!(down.accent, Color::Indexed(nearest_256(0x2d, 0xd4, 0xbf)));
        assert_eq!(
            down.accents[1],
            Color::Indexed(nearest_256(0xff, 0xb8, 0x6c))
        );
    }
}
