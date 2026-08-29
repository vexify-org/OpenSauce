//! Conversation + persistence.
//!
//! [`Conversation`] is the ordered transcript the agent builds and the UI
//! renders. [`SessionStore`] persists conversations to disk as JSONL so work
//! survives restarts, mirroring how opencode keeps session history.

use super::message::{self, Message, Seq};
use crate::mode::Mode;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// A named conversation with its mode and transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub mode: Mode,
    pub system: String,
    pub messages: Vec<Message>,
    /// Next sequence number handed to appended messages.
    #[serde(default)]
    pub next_seq: Seq,
    pub created_at: u64,
    pub updated_at: u64,
}

impl Conversation {
    pub fn new(id: impl Into<String>, title: impl Into<String>, mode: Mode, system: String) -> Self {
        let now = message::now_millis();
        Conversation {
            id: id.into(),
            title: title.into(),
            mode,
            system,
            messages: Vec::new(),
            next_seq: 0,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn system_prefix(&self) -> String {
        self.system.clone()
    }

    /// Append a message, assigning an order-preserving sequence number.
    pub fn push(&mut self, mut m: Message) -> Seq {
        m.seq = self.next_seq;
        self.next_seq += 1;
        let seq = m.seq;
        self.messages.push(m);
        self.updated_at = message::now_millis();
        seq
    }

    pub fn last_assistant_text(&self) -> Option<&str> {
        self.messages
            .iter()
            .rev()
            .find(|m| m.role.is_assistant() && m.content.is_some())
            .and_then(|m| m.content.as_deref())
    }

    /// Messages excluding the system intro (the system string is kept
    /// separately in `system`).
    pub fn turns(&self) -> &[Message] {
        &self.messages
    }
}

/// Where conversations are stored on disk.
#[derive(Debug, Clone)]
pub struct SessionStore {
    pub dir: PathBuf,
    index: HashMap<String, PathBuf>,
}

impl SessionStore {
    /// `dir` defaults to `<config>/opensauce/sessions` resolved via [`dirs`].
    pub fn open() -> Result<Self> {
        let base = dirs::data_dir()
            .or_else(dirs::config_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("opensauce");
        Ok(Self::open_at(base))
    }

    pub fn open_at(dir: PathBuf) -> Self {
        let store = SessionStore {
            dir,
            index: HashMap::new(),
        };
        store
    }

    fn file_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    pub fn ensure_dir(&self) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        Ok(())
    }

    pub fn save(&mut self, conv: &Conversation) -> Result<()> {
        self.ensure_dir()?;
        let path = self.file_path(&conv.id);
        let json = serde_json::to_string_pretty(conv)?;
        std::fs::write(&path, json)?;
        self.index.insert(conv.id.clone(), path);
        Ok(())
    }

    pub fn load(&self, id: &str) -> Result<Conversation> {
        let path = self.file_path(id);
        let raw = std::fs::read_to_string(&path)?;
        let conv = serde_json::from_str(&raw)?;
        Ok(conv)
    }

    /// List stored conversation ids sorted by most-recently-updated.
    pub fn list(&self) -> Result<Vec<String>> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }
        let mut v: Vec<Conversation> = Vec::new();
        for e in std::fs::read_dir(&self.dir)? {
            let e = e?;
            if !e.file_type()?.is_file() {
                continue;
            }
            if e.path().extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            if let Ok(raw) = std::fs::read_to_string(e.path()) {
                if let Ok(c) = serde_json::from_str::<Conversation>(&raw) {
                    v.push(c);
                }
            }
        }
        v.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(v.into_iter().map(|c| c.id).collect())
    }
}

/// Build the base system prompt for a given mode.
pub fn default_system(mode: Mode) -> String {
    match mode {
        Mode::Build => concat!(
            "You are OpenSauce, a coding agent that lives in the terminal. ",
            "You are in **Build** mode: you execute. You plan minimally, ",
            "use the provided tools to inspect and edit the workspace, and ",
            "get the task done.",
        )
        .to_string(),
        Mode::Plan => concat!(
            "You are OpenSauce, a coding agent living in the terminal. ",
            "You are in **Plan** mode: you investigate and reason before ",
            "acting. Read files, run read-only commands, and produce a clear ",
            "plan. Do not modify files or run mutating commands — present ",
            "your plan and wait for the user to switch to Build.",
        )
        .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seq_monotonic() {
        let mut c = Conversation::new("a", "t", Mode::Build, default_system(Mode::Build));
        let s1 = c.push(Message::user("hi"));
        let s2 = c.push(Message::assistant("hey"));
        assert!(s1 < s2);
        assert_eq!(c.turns().len(), 2);
    }
}