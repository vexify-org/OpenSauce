//! `todo` tool — a session-scoped task list, shown in the UI (Plan mode esp.).

use super::{Tool, ToolOutput};
use crate::mode::Mode;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

/// One task in the session list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: u64,
    pub text: String,
    pub done: bool,
}

/// Thread-shared task store so the agent (calling the tool) and the TUI both
/// see the same list.
pub type Todos = Arc<Mutex<Vec<TodoItem>>>;

/// `todo` — read/add/complete tasks in the session list.
///
/// Actions: `list`, `add` (text), `update` (`id` + `done`), `clear`.
#[derive(Debug)]
pub struct Todo {
    pub todos: Todos,
}

impl Todo {
    pub fn new(todos: Todos) -> Self {
        Todo { todos }
    }
    pub fn fresh() -> Todos {
        Arc::new(Mutex::new(Vec::new()))
    }
    pub fn snapshot(todos: &Todos) -> Vec<TodoItem> {
        todos.lock().unwrap().clone()
    }
}

impl Tool for Todo {
    fn name(&self) -> &'static str {
        "todo"
    }
    fn description(&self) -> &'static str {
        "Read or update the session task list. Actions: list, add, update, clear."
    }
    fn read_only(&self) -> bool {
        true
    }
    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["list", "add", "update", "clear"], "description": "What to do."},
                "text": {"type": "string", "description": "Task text (add)."},
                "id": {"type": "integer", "description": "Task id (update)."},
                "done": {"type": "boolean", "description": "Completion state (update)."}
            },
            "required": ["action"],
            "additionalProperties": false,
        })
    }
    fn run(&self, args: serde_json::Value, _mode: Mode) -> Result<ToolOutput> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("list");
        let mut list = self.todos.lock().unwrap();
        match action {
            "list" => {
                if list.is_empty() {
                    return Ok(ToolOutput::ok("No tasks yet."));
                }
                let s = list
                    .iter()
                    .map(|t| format!("[{}] {} — {}", t.id, if t.done { "x" } else { " " }, t.text))
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok(ToolOutput::ok(s))
            }
            "add" => {
                let text = args
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if text.is_empty() {
                    return Ok(ToolOutput::err("add requires `text`"));
                }
                let id = list.iter().map(|t| t.id).max().unwrap_or(0) + 1;
                list.push(TodoItem { id, text: text.to_string(), done: false });
                Ok(ToolOutput::ok(format!("added task {id}: {text}")))
            }
            "update" => {
                let id = match args.get("id").and_then(|v| v.as_u64()) {
                    Some(i) => i,
                    None => return Ok(ToolOutput::err("update requires `id`")),
                };
                let done = if args.get("done").is_some() {
                    args.get("done").and_then(|v| v.as_bool()).unwrap_or(false)
                } else {
                    return Ok(ToolOutput::err("update requires `done`"));
                };
                match list.iter_mut().find(|t| t.id == id) {
                    Some(t) => {
                        t.done = done;
                        Ok(ToolOutput::ok(format!("task {id} → {}", if done { "done" } else { "pending" })))
                    }
                    None => Ok(ToolOutput::err(format!("no task with id {id}"))),
                }
            }
            "clear" => {
                list.clear();
                Ok(ToolOutput::ok("task list cleared"))
            }
            other => Ok(ToolOutput::err(format!("unknown todo action '{other}'"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_update_list() {
        let todos = Todo::fresh();
        let t = Todo::new(todos.clone());
        t.run(serde_json::json!({"action": "add", "text": "write tests"}), Mode::Build).unwrap();
        t.run(serde_json::json!({"action": "add", "text": "ship it"}), Mode::Build).unwrap();
        let out = t.run(serde_json::json!({"action": "list"}), Mode::Build).unwrap();
        assert!(out.ok);
        assert!(out.text.contains("write tests"));
        t.run(serde_json::json!({"action": "update", "id": 1, "done": true}), Mode::Build).unwrap();
        let snap = Todo::snapshot(&todos);
        assert_eq!(snap.len(), 2);
        assert!(snap[0].done);
        assert!(!snap[0].text.is_empty());
    }
}