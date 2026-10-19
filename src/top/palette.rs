//! Kiso's dark theme tokens, with the console's gold accent, as RGB. Kiso
//! keeps terminals dark in both themes, so `top` paints its own background
//! and uses only the dark values. The values come from `@momoi-labs/kiso`
//! 0.21.0, `tokens/build/tokens.css`.
//!
//! Following `docs/kiso.md`, no color marks health: a long query, a lock
//! wait or a wraparound vacuum reads as text, not as red.

use ratatui::style::{Color, Style, Stylize};
use ratatui::widgets::Block;

/// `background`, dark.900.
const BACKGROUND: Color = Color::Rgb(0x21, 0x1f, 0x1c);
/// `surface`, dark.800: inline panels.
const SURFACE: Color = Color::Rgb(0x2d, 0x2b, 0x28);
/// `elevated-surface`, dark.700: popups. `muted` shares it: table headers.
const ELEVATED: Color = Color::Rgb(0x3a, 0x38, 0x34);
/// `foreground`, dark.100.
const FOREGROUND: Color = Color::Rgb(0xee, 0xec, 0xe9);
/// `muted-foreground`, dark.400.
const MUTED_FOREGROUND: Color = Color::Rgb(0xab, 0xa9, 0xa5);
/// `subtle-foreground`, dark.450: non-text graphics only.
const SUBTLE_FOREGROUND: Color = Color::Rgb(0x94, 0x92, 0x8e);
/// `border`, dark.600.
const BORDER: Color = Color::Rgb(0x49, 0x47, 0x44);
/// `selected` for the gold accent.
const SELECTED: Color = Color::Rgb(0x38, 0x2e, 0x17);
/// `primary` and `primary-foreground` for the gold accent.
const PRIMARY: Color = Color::Rgb(0xf2, 0xc4, 0x38);
const PRIMARY_FOREGROUND: Color = Color::Rgb(0x1b, 0x1a, 0x1e);
/// `link`: the gold ink, gold.base on dark.
const LINK: Color = Color::Rgb(0xff, 0xd4, 0x5c);
/// `warning-on-dark`, `danger-on-dark`, `info-on-dark`.
const WARNING: Color = Color::Rgb(0xef, 0xc0, 0x5b);
const DANGER: Color = Color::Rgb(0xf5, 0x80, 0x7f);
const INFO: Color = Color::Rgb(0x75, 0xc4, 0xfa);
/// `chart-1` to `chart-3`: categorical series, never health.
pub const CHART: [Color; 3] = [
    Color::Rgb(0xa7, 0x93, 0xf4),
    Color::Rgb(0x5b, 0xd2, 0x95),
    Color::Rgb(0xef, 0xc0, 0x5b),
];

/// The whole screen: text on the canvas.
pub fn base() -> Style {
    Style::new().fg(FOREGROUND).bg(BACKGROUND)
}

pub fn strong() -> Style {
    Style::new().fg(FOREGROUND).bold()
}

pub fn muted() -> Style {
    Style::new().fg(MUTED_FOREGROUND)
}

/// Tree branches and other marks that are not text.
pub fn subtle() -> Style {
    Style::new().fg(SUBTLE_FOREGROUND)
}

pub fn link() -> Style {
    Style::new().fg(LINK)
}

pub fn brand() -> Style {
    Style::new().fg(PRIMARY_FOREGROUND).bg(PRIMARY).bold()
}

pub fn table_header() -> Style {
    Style::new().fg(FOREGROUND).bg(ELEVATED).bold()
}

pub fn selected() -> Style {
    Style::new().fg(FOREGROUND).bg(SELECTED)
}

pub fn warning() -> Style {
    Style::new().fg(WARNING)
}

pub fn danger() -> Style {
    Style::new().fg(DANGER)
}

pub fn info() -> Style {
    Style::new().fg(INFO)
}

/// A panel inside the screen, such as the query preview.
pub fn panel() -> Block<'static> {
    Block::bordered()
        .style(Style::new().fg(FOREGROUND).bg(SURFACE))
        .border_style(Style::new().fg(BORDER))
}

/// A popup over the screen.
pub fn popup() -> Block<'static> {
    Block::bordered()
        .style(Style::new().fg(FOREGROUND).bg(ELEVATED))
        .border_style(Style::new().fg(BORDER))
}
