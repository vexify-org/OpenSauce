//! Tool framework.

pub mod edit;
pub mod fs;
pub mod registry;
pub mod search;
pub mod shell;
pub mod todo;

pub use registry::{dispatch, Tool, ToolRegistry};

/// A tool call result payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutput {
    pub ok: bool,
    pub text: String,
}

impl ToolOutput {
    pub fn ok(text: impl Into<String>) -> Self {
        ToolOutput { ok: true, text: text.into() }
    }
    pub fn err(text: impl Into<String>) -> Self {
        ToolOutput { ok: false, text: text.into() }
    }
}