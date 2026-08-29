//! Accent / palette selection per mode.
//!
//! The concrete colors below mirror opencode's canonical `opencode` theme
//! (see `packages/tui/src/theme/assets/opencode.json`), so the terminal looks
//! and feels like opencode. The only deliberate brand difference is the accent
//! driven by the conversation mode:
//! - `Build` is cool **blue** — action, focus, execution.
//! - `Plan` is warm **yellow** — deliberation, clarity, warning-light.
//!
//! The palette maps semantic roles to concrete `ratatui::style::Color`s so
//! call sites stay semantic instead of hard-coding colors.

pub use ratatui::style::Color;

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// Mode accent: `#5c9cf5` (opencode `secondary`, blue) for Build,
    /// `#e5c07b` (opencode `markdownEmph`, yellow) for Plan.
    pub accent: Color,
    pub accent_bright: Color,
    /// `textMuted` -> `#808080`
    pub dim: Color,
    /// `text` -> `#eeeeee`
    pub text: Color,
    /// agent / user ribbon color (opencode `secondary` blue)
    pub user: Color,
    /// `text` for assistant body
    pub assistant: Color,
    /// tool call fg (opencode `info` cyan)
    pub tool: Color,
    /// `error` -> `#e06c75`
    pub error: Color,
    /// `success` -> `#7fd88f`
    pub success: Color,
    /// `warning` -> `#f5a742`
    pub warning: Color,
    /// `background` -> `#0a0a0a`
    pub bg: Color,
    /// `backgroundPanel` -> `#141414`
    pub panel: Color,
    /// `backgroundElement` -> `#1e1e1e`
    pub element: Color,
    /// `border` -> `#484848`
    pub border: Color,
    /// `primary` -> `#fab283`
    pub primary: Color,
}

use crate::mode::Mode;

impl Palette {
    pub fn for_mode(mode: Mode) -> Palette {
        let base = Self::neutral();
        match mode {
            Mode::Build => Palette {
                accent: Color::Rgb(0x5c, 0x9c, 0xf5),
                accent_bright: Color::Rgb(0x8f, 0xbc, 0xff),
                ..base
            },
            Mode::Plan => Palette {
                accent: Color::Rgb(0xe5, 0xc0, 0x7b),
                accent_bright: Color::Rgb(0xf2, 0xd8, 0xa8),
                ..base
            },
        }
    }

    pub const fn neutral() -> Palette {
        Palette {
            accent: Color::Rgb(0x9d, 0x7c, 0xd8),
            accent_bright: Color::Rgb(0xb9, 0x9e, 0xe8),
            dim: Color::Rgb(0x80, 0x80, 0x80),
            text: Color::Rgb(0xee, 0xee, 0xee),
            user: Color::Rgb(0x5c, 0x9c, 0xf5),
            assistant: Color::Rgb(0xee, 0xee, 0xee),
            tool: Color::Rgb(0x56, 0xb6, 0xc2),
            error: Color::Rgb(0xe0, 0x6c, 0x75),
            success: Color::Rgb(0x7f, 0xd8, 0x8f),
            warning: Color::Rgb(0xf5, 0xa7, 0x42),
            bg: Color::Rgb(0x0a, 0x0a, 0x0a),
            panel: Color::Rgb(0x14, 0x14, 0x14),
            element: Color::Rgb(0x1e, 0x1e, 0x1e),
            border: Color::Rgb(0x48, 0x48, 0x48),
            primary: Color::Rgb(0xfa, 0xb2, 0x83),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_and_plan_palettes_differ() {
        let b = Palette::for_mode(Mode::Build);
        let p = Palette::for_mode(Mode::Plan);
        assert_ne!(b.accent, p.accent);
    }

    #[test]
    fn opencode_neutral_base() {
        let n = Palette::neutral();
        assert_eq!(n.text, Color::Rgb(0xee, 0xee, 0xee));
        assert_eq!(n.error, Color::Rgb(0xe0, 0x6c, 0x75));
        assert_eq!(n.bg, Color::Rgb(0x0a, 0x0a, 0x0a));
    }
}