//! Message model — the currency exchanged between the agent, providers and
//! the conversation store.
//!
//! A single `Message` is either:
//! - a plain text turn (`role` + `content`), or
//! - an assistant turn that carries `tool_calls`, or
//! - a `tool` turn that carries the result of a single `ToolCall`.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Who produced a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

impl Role {
    pub fn is_tool(&self) -> bool {
        matches!(self, Role::Tool)
    }
    pub fn is_assistant(&self) -> bool {
        matches!(self, Role::Assistant)
    }
}

/// A single tool invocation requested by the assistant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    /// Stable identifier echoed back when the result is delivered.
    pub id: String,
    /// Tool name, e.g. `read_file`, `run_command`.
    pub name: String,
    /// JSON arguments as parsed from the model.
    pub arguments: serde_json::Value,
}

/// The result attached to a `tool`-role message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResult {
    /// The id of the `ToolCall` this result answers.
    pub call_id: String,
    pub name: String,
    pub success: bool,
    /// Human/Markdown-rendered payload.
    pub content: String,
}

/// A message in a conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    /// Text payload. `None` when the *(assistant)* message only carries tools.
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default)]
    pub tool_result: Option<ToolResult>,
    /// Monotonic ordering key; not an absolute timestamp.
    #[serde(default)]
    pub seq: u64,
}

impl Message {
    pub fn new(role: Role) -> Self {
        Message {
            role,
            content: None,
            tool_calls: Vec::new(),
            tool_result: None,
            seq: 0,
        }
    }

    pub fn user(text: impl Into<String>) -> Self {
        Message {
            role: Role::User,
            content: Some(text.into()),
            ..Message::new(Role::User)
        }
    }

    pub fn system(text: impl Into<String>) -> Self {
        Message {
            role: Role::System,
            content: Some(text.into()),
            ..Message::new(Role::System)
        }
    }

    /// Assistant text turn (no tool calls).
    pub fn assistant(text: impl Into<String>) -> Self {
        Message {
            role: Role::Assistant,
            content: Some(text.into()),
            ..Message::new(Role::Assistant)
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, name: &str, success: bool, content: impl Into<String>) -> Self {
        Message::from_tool_result(ToolResult {
            call_id: tool_call_id.into(),
            name: name.to_string(),
            success,
            content: content.into(),
        })
    }

    pub fn from_tool_result(result: ToolResult) -> Self {
        Message {
            role: Role::Tool,
            tool_result: Some(result),
            ..Message::new(Role::Tool)
        }
    }

    pub fn display_text(&self) -> String {
        if let Some(ref c) = self.content {
            c.clone()
        } else if let Some(ref tr) = self.tool_result {
            tr.content.clone()
        } else if !self.tool_calls.is_empty() {
            let names: Vec<&str> = self.tool_calls.iter().map(|t| t.name.as_str()).collect();
            format!("calling {}…", names.join(", "))
        } else {
            String::new()
        }
    }
}

/// A monotonically increasing sequence for message ordering in a session.
pub type Seq = u64;

pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_kinds() {
        let m = Message::user("hello");
        assert_eq!(m.role, Role::User);
        assert_eq!(m.content.as_deref(), Some("hello"));

        let t = Message::from_tool_result(ToolResult {
            call_id: "1".into(),
            name: "ls".into(),
            success: true,
            content: "ok".into(),
        });
        assert!(t.role.is_tool());
        assert_eq!(t.display_text(), "ok");
    }
}