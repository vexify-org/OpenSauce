//! Semantic palette, resolved to the concrete colors opencode paints with.
//!
//! Every value below comes straight from opencode's canonical theme
//! (`packages/tui/src/theme/assets/opencode.json`) so the terminal reads byte
//! for byte like opencode. The only deliberate product difference is the
//! **mode accent** (`agentColor`):
//! - `Build` = `secondary` blue `#5c9cf5` — action, focus, execution.
//! - `Plan`  = `warning`  yellow `#e5c07b` — deliberation, clarity.
//!
//! That accent drives the user-message left border and the assistant `▣` mark.

pub use ratatui::style::Color;

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// Mode accent / agent color. `#5c9cf5` (opencode `secondary`) for Build,
    /// `#e5c07b` (opencode `warning`) for Plan.
    pub accent: Color,
    /// Brighter variant of the accent, used for emphasis text.
    pub accent_bright: Color,
    /// `textMuted` -> `#808080`
    pub dim: Color,
    /// `text` -> `#eeeeee`
    pub text: Color,
    /// Agent ribbon / `▣` mark color (same as `accent`).
    pub user: Color,
    /// `text` for assistant body
    pub assistant: Color,
    /// Tool-call accent (opencode `info` cyan `#56b6c2`)
    pub tool: Color,
    /// `error` -> `#e06c75`
    pub error: Color,
    /// `success` -> `#7fd88f`
    pub success: Color,
    /// `warning` -> `#f5a742`
    pub warning: Color,
    /// `background` -> `#0a0a0a` (terminal background)
    pub bg: Color,
    /// `backgroundPanel` -> `#141414`
    pub panel: Color,
    /// `backgroundElement` -> `#1e1e1e`
    pub element: Color,
    /// `backgroundMenu` -> `#1e1e1e`
    pub menu: Color,
    /// `border` -> `#484848`
    pub border: Color,
    /// `borderActive` -> `#606060`
    pub border_active: Color,
    /// `primary` -> `#fab283`
    pub primary: Color,
    /// `markdownText` / code text -> `#eeeeee`
    pub markdown_text: Color,
    /// `markdownCode` -> `#7fd88f` (green)
    pub markdown_code: Color,
    /// `markdownEmph` -> `#e5c07b` (yellow)
    pub markdown_emph: Color,
    /// `diffAdded` -> `#7fd88f`
    pub diff_added: Color,
    /// `diffRemoved` -> `#e06c75`
    pub diff_removed: Color,
}

use crate::mode::Mode;

impl Palette {
    pub fn for_mode(mode: Mode) -> Palette {
        let base = Self::neutral();
        match mode {
            Mode::Build => Palette {
                accent: Color::Rgb(0x5c, 0x9c, 0xf5),
                accent_bright: Color::Rgb(0x8f, 0xbc, 0xff),
                user: Color::Rgb(0x5c, 0x9c, 0xf5),
                ..base
            },
            Mode::Plan => Palette {
                accent: Color::Rgb(0xe5, 0xc0, 0x7b),
                accent_bright: Color::Rgb(0xf2, 0xd8, 0xa8),
                user: Color::Rgb(0xe5, 0xc0, 0x7b),
                ..base
            },
        }
    }

    pub const fn neutral() -> Palette {
        Palette {
            accent: Color::Rgb(0x5c, 0x9c, 0xf5),
            accent_bright: Color::Rgb(0x8f, 0xbc, 0xff),
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
            menu: Color::Rgb(0x1e, 0x1e, 0x1e),
            border: Color::Rgb(0x48, 0x48, 0x48),
            border_active: Color::Rgb(0x60, 0x60, 0x60),
            primary: Color::Rgb(0xfa, 0xb2, 0x83),
            markdown_text: Color::Rgb(0xee, 0xee, 0xee),
            markdown_code: Color::Rgb(0x7f, 0xd8, 0x8f),
            markdown_emph: Color::Rgb(0xe5, 0xc0, 0x7b),
            diff_added: Color::Rgb(0x7f, 0xd8, 0x8f),
            diff_removed: Color::Rgb(0xe0, 0x6c, 0x75),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_and_plan_accent_differ() {
        let b = Palette::for_mode(Mode::Build);
        let p = Palette::for_mode(Mode::Plan);
        assert_ne!(b.accent, p.accent);
    }

    #[test]
    fn opencode_neutral_base() {
        let n = Palette::neutral();
        assert_eq!(n.text, Color::Rgb(0xee, 0xee, 0xee));
        assert_eq!(n.dim, Color::Rgb(0x80, 0x80, 0x80));
        assert_eq!(n.bg, Color::Rgb(0x0a, 0x0a, 0x0a));
        assert_eq!(n.panel, Color::Rgb(0x14, 0x14, 0x14));
        assert_eq!(n.border, Color::Rgb(0x48, 0x48, 0x48));
    }
}