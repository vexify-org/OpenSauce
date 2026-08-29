//! Editing tools: `edit_file` (exact-string replacement with a rendered diff).

use super::{Tool, ToolOutput};
use crate::mode::Mode;
use anyhow::{bail, Result};
use std::fs;
use std::path::PathBuf;

/// `edit_file` — apply an exact-string replacement to a file and report a diff.
///
/// Mirrors opencode's `edit` tool: a precise `old_string` must match uniquely
/// (unless `replace_all` is set), then the replacement is written back.
#[derive(Debug)]
pub struct EditFile {
    pub root: PathBuf,
}

impl EditFile {
    pub fn new(root: PathBuf) -> Self {
        EditFile { root }
    }
}

impl Tool for EditFile {
    fn name(&self) -> &'static str {
        "edit_file"
    }
    fn description(&self) -> &'static str {
        "Apply an exact old_string → new_string replacement in a file and report the generated diff."
    }
    fn read_only(&self) -> bool {
        false
    }
    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {"type": "string", "description": "Path relative to the workspace root."},
                "old_string": {"type": "string", "description": "The exact text to find. Must match uniquely unless replace_all is set."},
                "new_string": {"type": "string", "description": "The replacement text."},
                "replace_all": {"type": "boolean", "description": "Replace every occurrence (default false)."}
            },
            "required": ["file_path", "old_string", "new_string"],
            "additionalProperties": false,
        })
    }
    fn run(&self, args: serde_json::Value, _mode: Mode) -> Result<ToolOutput> {
        let path = args
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("`file_path` is required"))?;
        let old_string = args
            .get("old_string")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("`old_string` is required"))?;
        let new_string = args
            .get("new_string")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let replace_all = args
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let full = self.root.join(path);
        if !full.exists() {
            bail!("no such file: {path}");
        }
        let old = fs::read_to_string(&full)?;

        let count = old.matches(old_string).count();
        if count == 0 {
            bail!("`old_string` was not found in {path}");
        }
        if count > 1 && !replace_all {
            return Ok(ToolOutput::err(format!(
                "`old_string` matched {count} times; set `replace_all: true` or include more surrounding context"
            )));
        }
        let new = if replace_all {
            old.replace(old_string, &new_string)
        } else {
            old.replacen(old_string, &new_string, 1)
        };

        // Diff the single region narrow: full-diff lines are fine here.
        let diff = unified_diff(path, &old, &new);
        fs::write(&full, &new)?;
        let delta = new.len() as i64 - old.len() as i64;
        Ok(ToolOutput::ok(format!("edited {path} ({count} occurrence{s}, {delta:+} bytes)\n{diff}",
            s = if count == 1 { "" } else { "s" })))
    }
}

/// Render a compact unified diff string with `-`/`+`/space-prefixed lines and a
/// `@@` hunk header, colorized later by the UI.
pub fn unified_diff(path: &str, old: &str, new: &str) -> String {
    let o: Vec<&str> = old.lines().collect();
    let n: Vec<&str> = new.lines().collect();

    // Backtracking edit script from a simple LCS DP (capped for safety).
    let (ops, _a, b) = diff_lines(&o, &n);

    let mut out = String::new();
    out.push_str(&format!("--- a/{path}\n+++ b/{path}\n"));
    if b > 0 {
        out.push_str(&format!("@@ -1,{a} +1,{b} @@\n", a = o.len()));
    }
    for op in &ops {
        match op {
            Op::Keep(s) => out.push_str(&format!(" {s}\n")),
            Op::Del(s) => out.push_str(&format!("-{s}\n")),
            Op::Add(s) => out.push_str(&format!("+{s}\n")),
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op<'a> {
    Keep(&'a str),
    Del(&'a str),
    Add(&'a str),
}

/// Line-wise diff. Returns ordered ops plus the two input lengths (for the hunk
/// header). Uses a capped LCS so edits on huge files degrade, not blow up.
fn diff_lines<'a>(a: &[&'a str], b: &[&'a str]) -> (Vec<Op<'a>>, usize, usize) {
    if a.len() * b.len() > 600_000 {
        // Too large for DP: emit "all removed, all added" to stay bounded.
        let mut ops = Vec::with_capacity(a.len() + b.len());
        ops.extend(a.iter().map(|s| Op::Del(s)));
        ops.extend(b.iter().map(|s| Op::Add(s)));
        return (ops, a.len(), b.len());
    }
    let rows = a.len() + 1;
    let cols = b.len() + 1;
    let mut dp = vec![0u32; rows * cols];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            dp[i * cols + j] = if a[i] == b[j] {
                dp[(i + 1) * cols + (j + 1)] + 1
            } else {
                dp[(i + 1) * cols + j].max(dp[i * cols + (j + 1)])
            };
        }
    }
    let mut ops = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            ops.push(Op::Keep(a[i]));
            i += 1;
            j += 1;
        } else if dp[(i + 1) * cols + j] >= dp[i * cols + (j + 1)] {
            ops.push(Op::Del(a[i]));
            i += 1;
        } else {
            ops.push(Op::Add(b[j]));
            j += 1;
        }
    }
    while i < a.len() {
        ops.push(Op::Del(a[i]));
        i += 1;
    }
    while j < b.len() {
        ops.push(Op::Add(b[j]));
        j += 1;
    }
    (ops, a.len(), b.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;

    #[test]
    fn edit_replaces_and_diffs() {
        let root = env::temp_dir();
        let p = "opensauce_edit_test.txt";
        fs::write(root.join(p), "alpha beta gamma\n").unwrap();
        let t = EditFile::new(root.clone());
        let out = t
            .run(
                serde_json::json!({"file_path": p, "old_string": "beta", "new_string": "BETA"}),
                Mode::Build,
            )
            .unwrap();
        assert!(out.ok);
        assert!(out.text.contains("-alpha beta gamma"));
        assert!(out.text.contains("+alpha BETA gamma"));
        let _ = fs::remove_file(root.join(p));
    }

    #[test]
    fn ambiguous_old_string_errors() {
        let root = env::temp_dir();
        let p = "opensauce_edit_ambig.txt";
        fs::write(root.join(p), "x x x\n").unwrap();
        let t = EditFile::new(root.clone());
        let out = t
            .run(
                serde_json::json!({"file_path": p, "old_string": "x", "new_string": "y"}),
                Mode::Build,
            )
            .unwrap();
        assert!(!out.ok);
        assert!(out.text.contains("matched 3 times"));
        let _ = fs::remove_file(root.join(p));
    }
}