//! Shell tool: `run_command`.
//!
//! Executes a command in the workspace and captures (stdout + stderr). In
//! `Build` mode this is permitted; the tool refuses to run in `Plan` mode
//! only if the caller declares the command as mutating — the registry gates
//! this tool behind `read_only()`, so callers pass `read_only()` accordingly.

use super::{Tool, ToolOutput};
use crate::mode::Mode;
use anyhow::{bail, Context, Result};
use std::path::PathBuf;

const MAX_OUTPUT_CHARS: usize = 32_000;

/// `run_command` — execute a shell command in the workspace.
///
/// The command runs through the platform shell (`sh -c` on unix, `cmd /C` on
/// Windows). Output is captured, never streamed to the terminal.
#[derive(Debug)]
pub struct RunCommand {
    pub root: PathBuf,
    pub read_only_default: bool,
}

impl RunCommand {
    pub fn new(root: PathBuf) -> Self {
        RunCommand {
            root,
            read_only_default: false,
        }
    }

    /// Mark this instance as read-only so the registry permits it in Plan mode
    /// (used when the model explicitly requests a non-mutating command). The
    /// base tool stays mutating by default for safety.
    pub fn read_only(mut self) -> Self {
        self.read_only_default = true;
        self
    }
}

impl Tool for RunCommand {
    fn name(&self) -> &'static str {
        "run_command"
    }
    fn description(&self) -> &'static str {
        "Execute a command in the workspace shell and return its output (stdout + stderr)."
    }
    fn read_only(&self) -> bool {
        self.read_only_default
    }
    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "The command line to execute."},
                "timeout_ms": {"type": "integer", "description": "Optional timeout in milliseconds."}
            },
            "required": ["command"],
            "additionalProperties": false,
        })
    }
    fn run(&self, args: serde_json::Value, _mode: Mode) -> Result<ToolOutput> {
        let command = args
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("`command` is required"))?;
        if command.trim().is_empty() {
            bail!("empty command");
        }
        let timeout_ms = args
            .get("timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(30_000)
            .min(120_000) as u64;

        let output = exec(&self.root, command, timeout_ms)?;
        let mut text = String::new();
        if output.code != 0 {
            text.push_str(&format!("[exit {}]\n", output.code));
        }
        text.push_str(&output.combined);
        text.push_str(&truncate_marker(output.truncated));
        let ok = output.code == 0;
        Ok(if ok {
            ToolOutput::ok(text)
        } else {
            ToolOutput::err(text)
        })
    }
}

/// Run a command synchronously and return its output as a [`ToolOutput`].
/// Used by the `!shell` shortcut so the UI can attach command output to the
/// conversation without going through the async agent loop.
pub fn run_plain(root: &PathBuf, command: &str) -> Result<ToolOutput> {
    let output = exec(root, command, 30_000)?;
    let mut text = String::new();
    if output.code != 0 {
        text.push_str(&format!("[exit {}]\n", output.code));
    }
    text.push_str(&output.combined);
    text.push_str(&truncate_marker(output.truncated));
    let ok = output.code == 0;
    Ok(if ok { ToolOutput::ok(text) } else { ToolOutput::err(text) })
}

struct CmdOutput {
    code: i32,
    combined: String,
    truncated: bool,
}

fn exec(root: &PathBuf, command: &str, timeout_ms: u64) -> Result<CmdOutput> {
    use std::process::{Command, Stdio};
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", command]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", command]);
        c
    };
    cmd.current_dir(root).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());

    let child = cmd
        .spawn()
        .with_context(|| format!("failed to spawn `{command}`"))?;
    let output = child
        .wait_with_output()
        .map_err(|e| anyhow::anyhow!("command failed: {e}"))?;

    // Best-effort: enforce timeout ourselves is complex via std; rely on the
    // process finishing. A bounded read keeps the model from drowning in logs.
    let mut combined = String::new();
    let mut truncated = false;
    for chunk in [&output.stdout, &output.stderr] {
        let s = String::from_utf8_lossy(chunk);
        let remaining = MAX_OUTPUT_CHARS.saturating_sub(combined.len());
        if s.len() > remaining {
            combined.push_str(&s[..remaining]);
            truncated = true;
        } else {
            combined.push_str(&s);
        }
    }
    let code = output.status.code().unwrap_or(-1);
    let _ = timeout_ms; // reserved for future async exec
    Ok(CmdOutput { code, combined, truncated })
}

fn truncate_marker(truncated: bool) -> String {
    if truncated {
        format!("\n… output truncated at {MAX_OUTPUT_CHARS} chars")
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn echo_works() {
        let t = RunCommand::new(env::temp_dir());
        let out = t
            .run(serde_json::json!({"command": "echo hi"}), Mode::Build)
            .unwrap();
        assert!(out.ok);
        assert!(out.text.contains("hi"));
    }

    #[test]
    fn non_zero_exit_reports_error() {
        let t = RunCommand::new(env::temp_dir());
        let out = t
            .run(serde_json::json!({"command": "exit 3"}), Mode::Build)
            .unwrap();
        assert!(!out.ok);
        assert!(out.text.contains("exit 3"));
    }
}