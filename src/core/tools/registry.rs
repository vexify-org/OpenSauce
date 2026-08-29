//! Tool trait + registry.

use super::edit::EditFile;
use super::fs::{ListDir, ReadFile, WriteFile, WorkspaceInfo};
use super::search::{GlobFiles, Grep, ListFiles};
use super::shell::RunCommand;
use super::todo::Todo;
use super::ToolOutput;
use crate::core::message::ToolCall;
use crate::mode::Mode;
use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// A capability the agent can invoke.
pub trait Tool: Send + Sync + std::fmt::Debug {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    /// Whether this tool is safe in read-only (`Plan`) mode.
    fn read_only(&self) -> bool;

    /// JSON schema (OpenAI-style) for the arguments object.
    fn schema(&self) -> serde_json::Value;

    /// Execute the tool. `mode` allows tools to refuse mutation in Plan mode.
    fn run(&self, args: serde_json::Value, mode: Mode) -> Result<ToolOutput>;
}

/// Run a single [`ToolCall`] against the registry.
pub async fn dispatch(reg: &ToolRegistry, call: &ToolCall, mode: Mode) -> ToolOutput {
    match reg.get(&call.name).await {
        Some(tool) => {
            // Configuration-level deny takes precedence over everything.
            if reg.is_denied(&call.name) {
                return ToolOutput::err(format!(
                    "`{}` is denied by your permission settings.",
                    call.name
                ));
            }
            if !tool.read_only() && !mode.permits_mutation() {
                return ToolOutput::err(format!(
                    "`{}` is a mutating tool and is blocked in Plan mode. \
                     Switch to Build to run it.",
                    tool.name()
                ));
            }
            match tool.run(call.arguments.clone(), mode) {
                Ok(out) => out,
                Err(e) => ToolOutput::err(format!("{}", e)),
            }
        }
        None => ToolOutput::err(format!("unknown tool `{}`", call.name)),
    }
}

pub type SharedTool = Arc<dyn Tool>;

/// A named collection of tools the agent may call.
#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<String, SharedTool>,
    denied: std::sync::Mutex<HashMap<String, ()>>,
}

impl ToolRegistry {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Deny a tool by name (from user permission settings).
    pub fn deny(&self, name: &str) {
        self.denied.lock().unwrap().insert(name.to_string(), ());
    }

    pub fn is_denied(&self, name: &str) -> bool {
        self.denied.lock().unwrap().contains_key(name)
    }

    /// Registry with the built-in tools, rooted at `root`.
    pub fn new(root: PathBuf) -> Self {
        let mut r = Self::empty();
        r.register(ReadFile::new(root.clone()));
        r.register(WriteFile::new(root.clone()));
        r.register(EditFile::new(root.clone()));
        r.register(ListDir::new(root.clone()));
        r.register(WorkspaceInfo::new(root.clone()));
        r.register(Grep::new(root.clone()));
        r.register(ListFiles::new(root.clone()));
        r.register(GlobFiles::new(root.clone()));
        r.register(RunCommand::new(root.clone()));
        r.register(Todo::new(Todo::fresh()));
        r
    }

    /// Like [`Self::new`] but shares the given session todo store with the UI.
    pub fn new_shared(root: PathBuf, todos: super::todo::Todos) -> Self {
        let mut r = Self::empty();
        r.register(ReadFile::new(root.clone()));
        r.register(WriteFile::new(root.clone()));
        r.register(EditFile::new(root.clone()));
        r.register(ListDir::new(root.clone()));
        r.register(WorkspaceInfo::new(root.clone()));
        r.register(Grep::new(root.clone()));
        r.register(ListFiles::new(root.clone()));
        r.register(GlobFiles::new(root.clone()));
        r.register(RunCommand::new(root.clone()));
        r.register(Todo::new(todos));
        r
    }

    /// Registry rooted at the current working directory.
    pub fn with_defaults() -> Self {
        Self::new(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }

    pub fn register<T: Tool + 'static>(&mut self, tool: T) {
        let name = tool.name().to_string();
        self.tools.insert(name, Arc::new(tool));
    }

    pub async fn get(&self, name: &str) -> Option<SharedTool> {
        self.tools.get(name).cloned()
    }

    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.tools.keys().cloned().collect();
        v.sort();
        v
    }

    /// All registered tools (for building tool definitions).
    pub fn shared(&self) -> Vec<SharedTool> {
        let mut v: Vec<SharedTool> = self.tools.values().cloned().collect();
        v.sort_by(|a, b| a.name().cmp(b.name()));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
async fn dispatch_unknown_tool() {
    let reg = ToolRegistry::empty();
        let call = ToolCall {
            id: "1".into(),
            name: "nope".into(),
            arguments: serde_json::json!({}),
        };
        let out = dispatch(&reg, &call, Mode::Build).await;
        assert!(!out.ok);
    }

    #[tokio::test]
    async fn plan_blocks_mutating() {
        let reg = ToolRegistry::with_defaults();
        let call = ToolCall {
            id: "2".into(),
            name: "write_file".into(),
            arguments: serde_json::json!({"path": "/tmp/x", "content": "y"}),
        };
        let out = dispatch(&reg, &call, Mode::Plan).await;
        assert!(!out.ok, "mutating tool must be blocked in Plan mode");
    }
}