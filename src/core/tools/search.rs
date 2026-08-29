//! Search tools: `grep` across the workspace.

use super::{Tool, ToolOutput};
use crate::mode::Mode;
use anyhow::{bail, Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::fs;
use std::path::{Path, PathBuf};

/// `grep` — regex search over text files in the workspace.
#[derive(Debug)]
pub struct Grep {
    pub root: PathBuf,
}

impl Grep {
    pub fn new(root: PathBuf) -> Self {
        Grep { root }
    }
}

impl Tool for Grep {
    fn name(&self) -> &'static str {
        "grep"
    }
    fn description(&self) -> &'static str {
        "Search file contents with a regex (unicode). Returns up to 50 matches with file:line."
    }
    fn read_only(&self) -> bool {
        true
    }
    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Regex pattern to match."},
                "path": {"type": "string", "description": "Optional directory to search (default: workspace root)."},
                "include": {"type": "array", "items": {"type": "string"}, "description": "Glob filters, e.g. [\"*.rs\"]."}
            },
            "required": ["pattern"],
            "additionalProperties": false,
        })
    }
    fn run(&self, args: serde_json::Value, _mode: Mode) -> Result<ToolOutput> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("`pattern` is required"))?;
        let re = regex::Regex::new(pattern).context("invalid regex")?;
        let rel_dir = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let dir = self.root.join(rel_dir);

        let include: Vec<String> = args
            .get("include")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        let matcher = build_include_matcher(&include);

        let mut results: Vec<String> = Vec::new();
        let mut stack = vec![dir];
        while let Some(d) = stack.pop() {
            let rd = match fs::read_dir(&d) {
                Ok(r) => r,
                Err(_) => continue,
            };
            for e in rd.flatten() {
                let p = e.path();
                let ft = match e.file_type() {
                    Ok(ft) => ft,
                    Err(_) => continue,
                };
                if ft.is_dir() {
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.starts_with('.') || name == "target" || name == "node_modules" {
                        continue;
                    }
                    stack.push(p);
                } else if ft.is_file() && matcher.is_match(rel_path(&p, &self.root)) {
                    if let Ok(text) = fs::read_to_string(&p) {
                        for (i, line) in text.lines().enumerate() {
                            if re.is_match(line) {
                                results.push(format!("{}:{}: {}", rel_path(&p, &self.root), i + 1, line.trim()));
                            }
                            if results.len() >= 50 {
                                break;
                            }
                        }
                    }
                }
                if results.len() >= 50 {
                    break;
                }
            }
            if results.len() >= 50 {
                break;
            }
        }

        if results.is_empty() {
            return Ok(ToolOutput::ok("no matches"));
        }
        Ok(ToolOutput::ok(results.join("\n")))
    }
}

fn rel_path(p: &Path, root: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .to_string()
}

fn build_include_matcher(globs: &[String]) -> globset::GlobSet {
    let mut b = GlobSetBuilder::new();
    for g in globs.iter().filter(|s| !s.is_empty()) {
        if let Ok(gl) = Glob::new(g) {
            b.add(gl);
        }
    }
    b.build().unwrap_or_else(|_| GlobSet::empty())
}

/// `list_files` — enumerate files (optionally filtered by glob).
#[derive(Debug)]
pub struct ListFiles {
    pub root: PathBuf,
}

impl ListFiles {
    pub fn new(root: PathBuf) -> Self {
        ListFiles { root }
    }
}

impl Tool for ListFiles {
    fn name(&self) -> &'static str {
        "list_files"
    }
    fn description(&self) -> &'static str {
        "Enumerate files in the workspace, with a gitignore/target/self filtering, up to 200."
    }
    fn read_only(&self) -> bool {
        true
    }
    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "include": {"type": "array", "items": {"type": "string"}}
            },
            "additionalProperties": false,
        })
    }
    fn run(&self, args: serde_json::Value, _mode: Mode) -> Result<ToolOutput> {
        let rel_dir = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let dir = self.root.join(rel_dir);
        let include: Vec<String> = args
            .get("include")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();
        let matcher = build_include_matcher(&include);

        let mut out = Vec::new();
        let mut stack = vec![dir.clone()];
        while let Some(d) = stack.pop() {
            if let Ok(rd) = fs::read_dir(&d) {
                for e in rd.flatten() {
                    let p = e.path();
                    if let Ok(ft) = e.file_type() {
                        if ft.is_dir() {
                            let name = e.file_name().to_string_lossy().to_string();
                            if name.starts_with('.') || name == "target" || name == ".git" || name == "node_modules" {
                                continue;
                            }
                            stack.push(p);
                        } else if ft.is_file() && (include.is_empty() || matcher.is_match(rel_path(&p, &self.root))) {
                            out.push(rel_path(&p, &self.root));
                        }
                    }
                    if out.len() >= 200 {
                        break;
                    }
                }
            }
            if out.len() >= 200 {
                break;
            }
        }
        out.sort();
        if out.is_empty() {
            bail!("no files matched in {}", rel_dir);
        }
        Ok(ToolOutput::ok(out.join("\n")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mode::Mode;

    #[test]
    fn grep_finds_itself() {
        let t = Grep::new(PathBuf::from(env!("CARGO_MANIFEST_DIR")));
        let out = t
            .run(serde_json::json!({"pattern": "fn build_include_matcher", "include": ["src/core/tools/search.rs"]}), Mode::Build)
            .unwrap();
        assert!(out.ok);
        assert!(out.text.contains("fn build_include_matcher"));
    }
}