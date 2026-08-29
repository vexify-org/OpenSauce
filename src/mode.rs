//! Conversation modes: `Build` and `Plan`.
//!
//! `Build` executes — it calls tools, edits files and runs commands (blue).
//! `Plan` reasons first — it investigates, produces a plan and asks before
//! applying any change (yellow).

use serde::{Deserialize, Serialize};
use std::fmt;

/// The two conversation modes OpenSauce ships with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Execute freely: tools run, files are edited, commands are executed.
    Build,
    /// Investigate and propose; do not apply changes without confirmation.
    Plan,
}

impl Mode {
    pub const ALL: [Mode; 2] = [Mode::Build, Mode::Plan];

    pub fn label(&self) -> &'static str {
        match self {
            Mode::Build => "Build",
            Mode::Plan => "Plan",
        }
    }

    /// One-word verb used in UI hints.
    pub fn verb(&self) -> &'static str {
        match self {
            Mode::Build => "build",
            Mode::Plan => "plan",
        }
    }

    /// Whether tools are allowed to mutate the environment in this mode.
    pub fn permits_mutation(&self) -> bool {
        matches!(self, Mode::Build)
    }

    /// Whether the agent should ask for confirmation before mutating tools.
    pub fn requires_confirmation(&self) -> bool {
        matches!(self, Mode::Plan)
    }

    pub fn from_name(name: &str) -> Option<Mode> {
        match name.to_ascii_lowercase().as_str() {
            "build" => Some(Mode::Build),
            "plan" => Some(Mode::Plan),
            "b" => Some(Mode::Build),
            "p" => Some(Mode::Plan),
            _ => None,
        }
    }
}

impl Default for Mode {
    fn default() -> Self {
        Mode::Build
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_names_round_trip() {
        assert_eq!(Mode::from_name("build"), Some(Mode::Build));
        assert_eq!(Mode::from_name("PLAN"), Some(Mode::Plan));
        assert_eq!(Mode::from_name("x"), None);
        assert_eq!(Mode::Build.label(), "Build");
        assert!(!Mode::Plan.permits_mutation());
        assert!(Mode::Build.permits_mutation());
    }
}