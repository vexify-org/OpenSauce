//! Accent / palette selection per mode.
//!
//! Each mode owns a small thematic palette so the UI underscores the intent:
//! - `Build` is cool **blue** — action, focus, execution.
//! - `Plan` is warm **yellow** — deliberation, clarity, warning-light.
//!
//! The palette maps semantic roles (title, accent, highlight, dim) to concrete
//! `ratatui::style::Color`s so call sites stay semantic instead of hard-coding
//! colors.

pub use ratatui::style::Color;

#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub accent: Color,
    pub accent_bright: Color,
    pub dim: Color,
    pub user: Color,
    pub assistant: Color,
    pub tool: Color,
    pub error: Color,
}

use crate::mode::Mode;

impl Palette {
    pub fn for_mode(mode: Mode) -> Palette {
        match mode {
            Mode::Build => {
                // Cool blues
                Palette {
                    accent: Color::Rgb(48, 122, 229),
                    accent_bright: Color::Rgb(120, 176, 255),
                    dim: Color::Rgb(90, 100, 120),
                    user: Color::Rgb(110, 160, 255),
                    assistant: Color::Rgb(200, 210, 230),
                    tool: Color::Rgb(120, 150, 200),
                    error: Color::Rgb(240, 90, 90),
                }
            }
            Mode::Plan => {
                // Warm ambers
                Palette {
                    accent: Color::Rgb(214, 158, 46),
                    accent_bright: Color::Rgb(240, 200, 90),
                    dim: Color::Rgb(120, 100, 60),
                    user: Color::Rgb(235, 190, 90),
                    assistant: Color::Rgb(230, 218, 190),
                    tool: Color::Rgb(200, 170, 100),
                    error: Color::Rgb(230, 90, 70),
                }
            }
        }
    }

    pub const fn neutral() -> Palette {
        Palette {
            accent: Color::Rgb(80, 90, 110),
            accent_bright: Color::Rgb(160, 170, 190),
            dim: Color::Rgb(90, 100, 120),
            user: Color::Rgb(150, 160, 180),
            assistant: Color::Rgb(200, 210, 230),
            tool: Color::Rgb(120, 150, 200),
            error: Color::Rgb(240, 90, 90),
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
}