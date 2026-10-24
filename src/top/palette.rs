//! Kiso's dark theme tokens, with the console's gold accent, as RGB. Kiso
//! keeps terminals dark in both themes, so `top` paints its own background
//! and uses only the dark values. The values come from `@momoi-labs/kiso`
//! 0.21.0, `tokens/build/tokens.css`.
//!
//! Following `docs/kiso.md`, no color marks health: a long query, a lock
//! wait or a wraparound vacuum reads as text, not as red.

use std::sync::OnceLock;

use ratatui::style::{Color, Style};
use ratatui::widgets::Block;

/// `background`, dark.900.
const BACKGROUND: Rgb = (0x21, 0x1f, 0x1c);
/// `surface`, dark.800: inline panels.
const SURFACE: Rgb = (0x2d, 0x2b, 0x28);
/// `elevated-surface`, dark.700: popups. `muted` shares it: table headers.
const ELEVATED: Rgb = (0x3a, 0x38, 0x34);
/// `foreground`, dark.100.
const FOREGROUND: Rgb = (0xee, 0xec, 0xe9);
/// `muted-foreground`, dark.400.
const MUTED_FOREGROUND: Rgb = (0xab, 0xa9, 0xa5);
/// `subtle-foreground`, dark.450: non-text graphics only.
const SUBTLE_FOREGROUND: Rgb = (0x94, 0x92, 0x8e);
/// `border`, dark.600.
const BORDER: Rgb = (0x49, 0x47, 0x44);
/// `selected` for the gold accent.
const SELECTED: Rgb = (0x38, 0x2e, 0x17);
/// `primary` and `primary-foreground` for the gold accent.
const PRIMARY: Rgb = (0xf2, 0xc4, 0x38);
const PRIMARY_FOREGROUND: Rgb = (0x1b, 0x1a, 0x1e);
/// `link`: the gold ink, gold.base on dark.
const LINK: Rgb = (0xff, 0xd4, 0x5c);
/// `warning-on-dark`, `danger-on-dark`, `info-on-dark`.
const WARNING: Rgb = (0xef, 0xc0, 0x5b);
const DANGER: Rgb = (0xf5, 0x80, 0x7f);
const INFO: Rgb = (0x75, 0xc4, 0xfa);
/// `chart-1` to `chart-3`: categorical series, never health.
const CHART: [Rgb; 3] = [(0xa7, 0x93, 0xf4), (0x5b, 0xd2, 0x95), (0xef, 0xc0, 0x5b)];

type Rgb = (u8, u8, u8);

/// Whether the terminal takes 24-bit colors. Terminals that do say so in
/// `COLORTERM`; others, such as an older tmux, misread RGB codes as other
/// attributes, so they get the nearest of the 256 xterm colors.
fn truecolor() -> bool {
    static TRUECOLOR: OnceLock<bool> = OnceLock::new();
    *TRUECOLOR.get_or_init(|| {
        matches!(
            std::env::var("COLORTERM").as_deref(),
            Ok("truecolor" | "24bit")
        )
    })
}

fn color(rgb: Rgb) -> Color {
    if truecolor() {
        Color::Rgb(rgb.0, rgb.1, rgb.2)
    } else {
        Color::Indexed(xterm_256(rgb))
    }
}

/// The nearest xterm color: the 6x6x6 cube (16-231) or the grey ramp
/// (232-255), whichever is closer.
fn xterm_256((r, g, b): Rgb) -> u8 {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let nearest = |value: u8| {
        (0..6)
            .min_by_key(|&index| (i32::from(LEVELS[index]) - i32::from(value)).abs())
            .unwrap_or(0)
    };
    let distance = |(r2, g2, b2): (u8, u8, u8)| {
        let d = |a: u8, b: u8| (i32::from(a) - i32::from(b)).pow(2);
        d(r, r2) + d(g, g2) + d(b, b2)
    };
    let (ri, gi, bi) = (nearest(r), nearest(g), nearest(b));
    let cube = (LEVELS[ri], LEVELS[gi], LEVELS[bi]);
    let average = (u32::from(r) + u32::from(g) + u32::from(b)) / 3;
    let step = (average.saturating_sub(3) / 10).min(23) as u8;
    let grey = 8 + 10 * step;
    if distance((grey, grey, grey)) < distance(cube) {
        232 + step
    } else {
        16 + 36 * ri as u8 + 6 * gi as u8 + bi as u8
    }
}

/// A categorical series: active, idle in transaction, waiting.
pub fn series(slot: usize) -> Style {
    Style::new().fg(color(CHART[slot]))
}

/// The whole screen: text on the canvas.
pub fn base() -> Style {
    Style::new().fg(color(FOREGROUND)).bg(color(BACKGROUND))
}

pub fn strong() -> Style {
    Style::new().fg(color(FOREGROUND)).bold()
}

pub fn muted() -> Style {
    Style::new().fg(color(MUTED_FOREGROUND))
}

/// Tree branches and other marks that are not text.
pub fn subtle() -> Style {
    Style::new().fg(color(SUBTLE_FOREGROUND))
}

pub fn link() -> Style {
    Style::new().fg(color(LINK))
}

pub fn brand() -> Style {
    Style::new()
        .fg(color(PRIMARY_FOREGROUND))
        .bg(color(PRIMARY))
        .bold()
}

pub fn table_header() -> Style {
    Style::new()
        .fg(color(FOREGROUND))
        .bg(color(ELEVATED))
        .bold()
}

pub fn selected() -> Style {
    // The nearest xterm color to `selected` is a grey one step off the
    // background, too close to see, so 256-color terminals get a dark olive
    // in the gold's hue.
    let fill = if truecolor() {
        color(SELECTED)
    } else {
        Color::Indexed(58)
    };
    Style::new().fg(color(FOREGROUND)).bg(fill)
}

pub fn warning() -> Style {
    Style::new().fg(color(WARNING))
}

pub fn danger() -> Style {
    Style::new().fg(color(DANGER))
}

pub fn info() -> Style {
    Style::new().fg(color(INFO))
}

/// A panel inside the screen, such as the query preview.
pub fn panel() -> Block<'static> {
    Block::bordered()
        .style(Style::new().fg(color(FOREGROUND)).bg(color(SURFACE)))
        .border_style(Style::new().fg(color(BORDER)))
}

/// A popup over the screen.
pub fn popup() -> Block<'static> {
    Block::bordered()
        .style(Style::new().fg(color(FOREGROUND)).bg(color(ELEVATED)))
        .border_style(Style::new().fg(color(BORDER)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_maps_to_the_nearest_xterm_color() {
        assert_eq!(xterm_256((0x21, 0x1f, 0x1c)), 234);
        assert_eq!(xterm_256((0xee, 0xec, 0xe9)), 255);
        assert_eq!(xterm_256((0xff, 0xd4, 0x5c)), 221);
        assert_eq!(xterm_256((0x00, 0x00, 0x00)), 16);
    }
}
