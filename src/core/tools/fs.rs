//! Filesystem tools: `read_file`, `write_file`, `list_dir`, `workspace_info`.

use super::{Tool, ToolOutput};
use crate::mode::Mode;
use anyhow::{bail, Result};
use std::fs;
use std::path::PathBuf;

fn schema(props: serde_json::Value, required: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": props,
        "required": required,
        "additionalProperties": false,
    })
}

/// `read_file` — read a UTF-8 text file.
#[derive(Debug)]
pub struct ReadFile {
    pub root: PathBuf,
}

impl ReadFile {
    pub fn new(root: PathBuf) -> Self {
        ReadFile { root }
    }
}

impl Tool for ReadFile {
    fn name(&self) -> &'static str {
        "read_file"
    }
    fn description(&self) -> &'static str {
        "Read the contents of a text file as UTF-8."
    }
    fn read_only(&self) -> bool {
        true
    }
    fn schema(&self) -> serde_json::Value {
        schema(
            serde_json::json!({
                "path": {"type": "string", "description": "Path to the file, relative to the workspace root."}
            }),
            &["path"],
        )
    }
    fn run(&self, args: serde_json::Value, _mode: Mode) -> Result<ToolOutput> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("`path` is required"))?;
        let full = self.root.join(path);
        if !full.exists() {
            bail!("no such file: {path}");
        }
        match fs::read(&full) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) => Ok(ToolOutput::ok(format!("```\n{text}\n```"))),
                Err(_) => bail!("{path} is not valid UTF-8 text"),
            },
            Err(e) => bail!("cannot read {path}: {e}"),
        }
    }
}

/// `write_file` — create or overwrite a UTF-8 text file.
#[derive(Debug)]
pub struct WriteFile {
    pub root: PathBuf,
}

impl WriteFile {
    pub fn new(root: PathBuf) -> Self {
        WriteFile { root }
    }
}

impl Tool for WriteFile {
    fn name(&self) -> &'static str {
        "write_file"
    }
    fn description(&self) -> &'static str {
        "Create or overwrite a text file with the given content (UTF-8)."
    }
    fn read_only(&self) -> bool {
        false
    }
    fn schema(&self) -> serde_json::Value {
        schema(
            serde_json::json!({
                "path": {"type": "string", "description": "Path to the file, relative to the workspace root."},
                "content": {"type": "string", "description": "Full content to write."}
            }),
            &["path", "content"],
        )
    }
    fn run(&self, args: serde_json::Value, _mode: Mode) -> Result<ToolOutput> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("`path` is required"))?;
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let full = self.root.join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&full, content)?;
        Ok(ToolOutput::ok(format!("wrote {path} ({} bytes)", fs::metadata(&full)?.len())))
    }
}

/// `list_dir` — list a directory (one line per entry).
#[derive(Debug)]
pub struct ListDir {
    pub root: PathBuf,
}

impl ListDir {
    pub fn new(root: PathBuf) -> Self {
        ListDir { root }
    }
}

impl Tool for ListDir {
    fn name(&self) -> &'static str {
        "list_dir"
    }
    fn description(&self) -> &'static str {
        "List entries in a directory, relative to the workspace root. Dir entries are suffixed with `/`."
    }
    fn read_only(&self) -> bool {
        true
    }
    fn schema(&self) -> serde_json::Value {
        schema(serde_json::json!({"path": {"type": "string"}}), &[])
    }
    fn run(&self, args: serde_json::Value, _mode: Mode) -> Result<ToolOutput> {
        let rel = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let full = self.root.join(rel);
        let mut entries = Vec::new();
        for e in fs::read_dir(&full)? {
            let e = e?;
            let name = e.file_name().to_string_lossy().to_string();
            let suffix = if e.file_type()?.is_dir() { "/" } else { "" };
            // Hide common noise.
            if name.starts_with('.') {
                continue;
            }
            entries.push(format!("{name}{suffix}"));
        }
        entries.sort();
        if entries.is_empty() {
            return Ok(ToolOutput::ok("(empty directory)"));
        }
        Ok(ToolOutput::ok(format!("{}\n{}", full.display(), entries.join("\n"))))
    }
}

/// `workspace_info` — current working directory context.
#[derive(Debug)]
pub struct WorkspaceInfo {
    pub root: PathBuf,
}

impl WorkspaceInfo {
    pub fn new(root: PathBuf) -> Self {
        WorkspaceInfo { root }
    }
}

impl Tool for WorkspaceInfo {
    fn name(&self) -> &'static str {
        "workspace_info"
    }
    fn description(&self) -> &'static str {
        "Return the absolute path and a short summary of the workspace."
    }
    fn read_only(&self) -> bool {
        true
    }
    fn schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {}})
    }
    fn run(&self, _args: serde_json::Value, _mode: Mode) -> Result<ToolOutput> {
        let file_count = count_files(&self.root);
        Ok(ToolOutput::ok(format!(
            "Workspace root: {}\n{} files/dirs",
            self.root.display(),
            file_count
        )))
    }
}

fn count_files(root: &std::path::Path) -> usize {
    let mut n = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if let Ok(rd) = fs::read_dir(&dir) {
            for e in rd.flatten() {
                let p = e.path();
                if let Ok(md) = e.metadata() {
                    if md.is_dir() {
                        if dir != root || !dir_is_target(&p) {
                            stack.push(p);
                        }
                    } else {
                        n += 1;
                    }
                }
            }
        }
    }
    n
}

fn dir_is_target(_p: &std::path::Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mode::Mode;
    use std::env;
    use std::fs;

    #[test]
    fn write_then_read_round_trip() {
        let root = env::temp_dir();
        let w = WriteFile::new(root.clone());
        let out = w
            .run(serde_json::json!({"path": "opensauce_test.txt", "content": "hi"}), Mode::Build)
            .unwrap();
        assert!(out.ok);
        let r = ReadFile::new(root.clone());
        let out = r
            .run(serde_json::json!({"path": "opensauce_test.txt"}), Mode::Build)
            .unwrap();
        assert!(out.text.contains("hi"));
        let _ = fs::remove_file(root.join("opensauce_test.txt"));
    }
}