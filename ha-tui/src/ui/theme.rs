//! A small warm, muted "amber terminal" palette shared across widgets, in
//! the style of retro dashboard TUIs - tan/cream borders and text, amber
//! accents for active/selected things, soft green for "on", dimmed gray
//! for unavailable.

use ratatui::style::Color;

pub const BORDER: Color = Color::Rgb(150, 130, 100);
pub const BORDER_DIM: Color = Color::Rgb(90, 80, 68);
pub const TEXT: Color = Color::Rgb(214, 200, 174);
pub const TEXT_DIM: Color = Color::Rgb(140, 128, 110);
pub const ACCENT: Color = Color::Rgb(224, 175, 92);
pub const ON: Color = Color::Rgb(130, 190, 120);
pub const UNAVAILABLE: Color = Color::Rgb(110, 100, 90);
pub const ERROR: Color = Color::Rgb(210, 120, 100);
pub const HIGHLIGHT_BG: Color = Color::Rgb(224, 175, 92);
pub const HIGHLIGHT_FG: Color = Color::Rgb(30, 26, 20);
/// Subtle background tint for the whole selected panel, so it reads as
/// "focused" even before you look at which row is highlighted inside it.
pub const PANEL_SELECTED_BG: Color = Color::Rgb(46, 40, 30);
/// Near-black base a panel's title badge text sits on (see
/// `ui::cards::render_card`) - deliberately not pure black, so it still
/// reads as part of this palette rather than a hard cutout.
pub const CRUST: Color = Color::Rgb(24, 21, 16);
