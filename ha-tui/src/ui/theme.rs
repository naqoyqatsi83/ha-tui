//! A small warm, muted "amber terminal" palette shared across widgets, in
//! the style of retro dashboard TUIs - tan/cream borders and text, amber
//! accents for active/selected things, soft green for "on", dimmed gray
//! for unavailable.

use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::Span;

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
/// Near-black base a panel's title badge text sits on (see `badge`) -
/// deliberately not pure black, so it still reads as part of this palette
/// rather than a hard cutout.
pub const CRUST: Color = Color::Rgb(24, 21, 16);

/// Shared frame for every bordered panel/popup: chunky filled wedges at
/// the top corners with thin one-eighth-block lines everywhere else,
/// instead of ratatui's default uniform box-drawing characters - the look
/// exabind (https://github.com/junkdog/exabind) uses for its own panels.
pub const PANEL_BORDER: border::Set = border::Set {
    top_left: "\u{259f}",
    top_right: "\u{259c}",
    bottom_left: "\u{2594}",
    bottom_right: "\u{2594}",
    vertical_left: "\u{258f}",
    vertical_right: "\u{2595}",
    horizontal_top: "\u{2594}",
    horizontal_bottom: "\u{2594}",
};

/// A panel/popup title rendered as a solid badge in `color`, not colored
/// text sitting on the border line: normal fg/bg styling flipped with
/// `Modifier::REVERSED`, so the badge's visible background is `color` and
/// the text cuts through in `CRUST`.
pub fn badge(text: impl Into<String>, color: Color) -> Span<'static> {
    Span::styled(text.into(), Style::default().fg(color).bg(CRUST).add_modifier(Modifier::BOLD | Modifier::REVERSED))
}
