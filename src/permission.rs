//! Permission system — a faithful port of opencode's `permission` model.
//!
//! Every permission rule resolves to one of three actions:
//! - `allow` — run without approval
//! - `ask` — prompt for approval
//! - `deny` — block the action
//!
//! Rules are keyed by *opencode permission name* (`read`, `edit`, `glob`,
//! `grep`, `bash`, …) and support granular object syntax with simple glob
//! patterns (`*` zero-or-more, `?` one char). The **last matching rule wins**.
//! When no rule matches, the global default applies (opencode default: allow).

use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

/// The outcome of a permission check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Allow,
    Ask,
    Deny,
}

impl Level {
    pub fn parse(s: &str) -> Option<Level> {
        match s.to_ascii_lowercase().as_str() {
            "allow" | "always" => Some(Level::Allow),
            "ask" | "once" => Some(Level::Ask),
            "deny" | "disallow" | "reject" | "block" => Some(Level::Deny),
            _ => None,
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Level::Allow => "allow",
            Level::Ask => "ask",
            Level::Deny => "deny",
        }
    }
}

/// A single tool rule: when `tool` is invoked and `pattern` matches the input,
/// the action is `level`.
#[derive(Debug, Clone)]
pub struct Rule {
    pub tool: String,
    pub pattern: String,
    pub level: Level,
}

/// Parsed permission configuration.
#[derive(Debug, Clone)]
pub struct PermissionConfig {
    /// The `*` default (opencode: allow).
    pub global: Level,
    /// All rules from the `permission` config, in declaration order.
    pub rules: Vec<Rule>,
}

impl Default for PermissionConfig {
    fn default() -> Self {
        // opencode's default is permissive; sensitive classes default to ask.
        PermissionConfig {
            global: Level::Allow,
            rules: vec![
                Rule { tool: "bash".into(), pattern: "*".into(), level: Level::Allow },
                Rule { tool: "edit".into(), pattern: "*".into(), level: Level::Allow },
                Rule { tool: "read".into(), pattern: "*.env".into(), level: Level::Deny },
                Rule { tool: "read".into(), pattern: "*.env.*".into(), level: Level::Deny },
                Rule { tool: "read".into(), pattern: "*.env.example".into(), level: Level::Allow },
            ],
        }
    }
}

impl PermissionConfig {
    /// Resolve the action for a tool call whose permission key is `tool` and
    /// whose (normalized) input is `input`. Last matching rule wins.
    pub fn resolve(&self, tool: &str, input: &str) -> Level {
        let input = expand_home(input);
        let mut level = self.global;
        for r in &self.rules {
            if r.tool == tool && glob_match(&r.pattern, &input) {
                level = r.level;
            }
        }
        level
    }

    /// The permission rules for `tool` (sorted, deduped) — for display.
    pub fn rules_for(&self, tool: &str) -> Vec<&Rule> {
        self.rules.iter().filter(|r| r.tool == tool).collect()
    }
}

/// Deserialize from the flexible shapes opencode accepts:
///   permission = "allow"
///   permission = { "*" = "ask", bash = "allow", edit = { "*" = "deny" } }
impl<'de> Deserialize<'de> for PermissionConfig {
    fn deserialize<D>(d: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let v = toml::Value::deserialize(d)?;
        parse_permission(&v).ok_or_else(|| serde::de::Error::custom("malformed `permission` config"))
    }
}

fn parse_permission(v: &toml::Value) -> Option<PermissionConfig> {
    match v {
        toml::Value::String(s) => Some(PermissionConfig {
            global: Level::parse(s)?,
            rules: Vec::new(),
        }),
        toml::Value::Table(t) => {
            let mut global = Level::Allow;
            let mut rules = Vec::new();
            for (tool, node) in t {
                match node {
                    toml::Value::String(s) => {
                        let lvl = Level::parse(s)?;
                        if tool == "*" {
                            global = lvl;
                        } else {
                            // A lone level applies to the catch-all `*` input pattern,
                            // but sits after any prior specific rules for the tool.
                            rules.push(Rule { tool: tool.clone(), pattern: "*".into(), level: lvl });
                        }
                    }
                    toml::Value::Table(sub) => {
                        for (pattern, pv) in sub {
                            let lvl = Level::parse(pv.as_str()?)?;
                            rules.push(Rule { tool: tool.clone(), pattern: pattern.clone(), level: lvl });
                        }
                    }
                    _ => return None,
                }
            }
            Some(PermissionConfig { global, rules })
        }
        _ => None,
    }
}

/// Map a tool's concrete name onto the opencode permission key that governs it.
/// Unknown tools fall under the implicit `custom` class (like opencode), so the
/// returned key is always `'static`.
pub fn permission_key(name: &str) -> &'static str {
    match name {
        "read_file" | "list_dir" | "list_files" | "workspace_info" | "read" => "read",
        "write_file" | "edit_file" | "edit" => "edit",
        "grep" => "grep",
        "glob_files" | "glob" => "glob",
        "run_command" | "bash" => "bash",
        "todo" => "todo",
        _ => "custom",
    }
}

/// Produce the input string a granular rule matches against for a tool call.
/// For `bash` this is the command; for path tools it is the path; otherwise the
/// compact JSON of the arguments.
pub fn rule_input(tool: &str, args: &serde_json::Value) -> String {
    let key = permission_key(tool);
    if key == "bash" {
        if let Some(c) = args.get("command").and_then(|v| v.as_str()) {
            return c.to_string();
        }
    }
    for k in ["path", "file_path", "pattern"] {
        if let Some(p) = args.get(k).and_then(|v| v.as_str()) {
            return p.to_string();
        }
    }
    args.to_string()
}

/// Expand a leading `~` or `$HOME` in a path-like input.
pub fn expand_home(p: &str) -> String {
    let home = dirs::home_dir().map(|h| h.to_string_lossy().into_owned());
    if let Some(h) = &home {
        if p == "~" {
            return h.clone();
        }
        if let Some(rest) = p.strip_prefix("~/") {
            return format!("{}{}", h, std::path::MAIN_SEPARATOR).trim_end_matches('/').to_string() + "/" + rest;
        }
        if let Some(rest) = p.strip_prefix("$HOME/") {
            return format!("{}/{}", h, rest);
        }
    }
    p.to_string()
}

/// Simple glob match over UTF-8 char lists, supporting `*` and `?`.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut si) = (0usize, 0usize);
    let mut star: Option<usize> = None;
    let mut star_si = 0usize;
    while si < t.len() {
        if pi < p.len() && (p[pi] == t[si] || p[pi] == '?') {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            star_si = si;
            pi += 1;
        } else if let Some(sp) = star {
            pi = sp + 1;
            star_si += 1;
            si = star_si;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Resolve a workspace-relative path string to an absolute path, for display.
pub fn abs_display(p: &str) -> String {
    let pb = PathBuf::from(expand_home(p));
    pb.to_string_lossy().into_owned()
}

/// A permission request waiting on the user. Created by the agent when a tool
/// resolves to `ask`, parked here so the TUI can render a dialog and reply.
#[derive(Debug)]
pub struct PendingRequest {
    pub call_id: String,
    pub tool: String,
    pub key: String,
    pub input: String,
    reply: oneshot::Sender<(bool, bool)>, // (approved, always)
}

/// Thread/async-shared permission core. The agent calls [`PermissionHub::request`]
/// before running a tool; the TUI answers pending requests with [`PermissionHub::reply`].
#[derive(Debug)]
pub struct PermissionHub {
    pub cfg: PermissionConfig,
    pub auto: AtomicBool,
    always: Mutex<Vec<(String, String)>>, // (permission key, input literal) approved for the session
    pending: Mutex<Option<PendingRequest>>,
    counts: Mutex<HashMap<(String, String), u32>>, // doom-loop detection
}

impl PermissionHub {
    pub fn new(cfg: PermissionConfig) -> Arc<Self> {
        Arc::new(PermissionHub {
            cfg,
            auto: AtomicBool::new(false),
            always: Mutex::new(Vec::new()),
            pending: Mutex::new(None),
            counts: Mutex::new(HashMap::new()),
        })
    }

    /// True when a permission dialog is currently open/unanswered.
    pub fn has_pending(&self) -> bool {
        self.pending.lock().unwrap().is_some()
    }

    /// Snapshot of the open request, if any (for drawing).
    pub fn pending_request(&self) -> Option<(String, String, String)> {
        self.pending
            .lock()
            .unwrap()
            .as_ref()
            .map(|p| (p.tool.clone(), p.key.clone(), p.input.clone()))
    }

    /// Toggle auto-approve mode; returns the new state.
    pub fn toggle_auto(&self) -> bool {
        let next = !self.auto.load(Ordering::Relaxed);
        self.auto.store(next, Ordering::Relaxed);
        next
    }
    pub fn set_auto(&self, v: bool) {
        self.auto.store(v, Ordering::Relaxed);
    }
    pub fn is_auto(&self) -> bool {
        self.auto.load(Ordering::Relaxed)
    }

    /// Answer the open request. `approved` runs the tool; `always` also records
    /// the input as session-approved for the future. No-op if nothing is open.
    pub fn reply(&self, approved: bool, always: bool) {
        if let Some(pr) = self.pending.lock().unwrap().take() {
            if approved && always {
                self.always.lock().unwrap().push((pr.key.clone(), pr.input.clone()));
                let mut counts = self.counts.lock().unwrap();
                counts.clear();
            }
            let _ = pr.reply.send((approved, always));
        }
    }

    /// Cancel the open request (reject), e.g. on interrupt/quit.
    pub fn reject(&self) {
        self.reply(false, false);
    }

    /// Evaluate and (if needed) wait for the user to authorise a tool call with
    /// permission key `key` and normalized `input`. Returns `true` to proceed.
    pub async fn request(&self, call_id: &str, key: &str, input: &str) -> bool {
        // doom-loop: the same tool+input repeated 3 times forces a prompt.
        // The guard is scoped so it is dropped before the potential await.
        let repeat = {
            let mut counts = self.counts.lock().unwrap();
            let n = counts.entry((key.to_string(), input.to_string())).or_insert(0);
            *n += 1;
            *n
        };
        if repeat >= 3 {
            return self.prompt(call_id, key, input).await;
        }
        match self.cfg.resolve(key, input) {
            Level::Deny => false,
            Level::Allow => true,
            Level::Ask => {
                if self.auto.load(Ordering::Relaxed) {
                    true
                } else if self.always_match(key, input) {
                    true
                } else {
                    self.prompt(call_id, key, input).await
                }
            }
        }
    }

    async fn prompt(&self, call_id: &str, key: &str, input: &str) -> bool {
        let (tx, rx) = oneshot::channel();
        let tool = key;
        *self.pending.lock().unwrap() = Some(PendingRequest {
            call_id: call_id.to_string(),
            tool: tool.to_string(),
            key: key.to_string(),
            input: input.to_string(),
            reply: tx,
        });
        match rx.await {
            Ok((approved, always)) => {
                if approved && always {
                    // skip: already recorded in reply()
                }
                approved
            }
            Err(_) => false,
        }
    }

    fn always_match(&self, key: &str, input: &str) -> bool {
        let a = self.always.lock().unwrap();
        a.iter().any(|(k, pat)| k == key && glob_match(pat, input))
    }

    /// Session-approved inputs for `key`, for display in `/permissions`.
    pub fn approved(&self, key: &str) -> Vec<String> {
        self.always
            .lock()
            .unwrap()
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, i)| i.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_basic() {
        assert!(glob_match("*.env*", ".env"));
        assert!(glob_match("scripts/*", "scripts/build.sh"));
        assert!(!glob_match("scripts/*", "other/build.sh"));
        assert!(glob_match("a?c", "abc"));
        assert!(!glob_match("a?c", "ac"));
        assert!(glob_match("*", "anything"));
    }

    #[test]
    fn resolve_last_matching_wins() {
        let cfg = PermissionConfig {
            global: Level::Ask,
            rules: vec![
                Rule { tool: "bash".into(), pattern: "*".into(), level: Level::Ask },
                Rule { tool: "bash".into(), pattern: "git *".into(), level: Level::Allow },
                Rule { tool: "bash".into(), pattern: "rm *".into(), level: Level::Deny },
            ],
        };
        assert_eq!(cfg.resolve("bash", "git status"), Level::Allow);
        assert_eq!(cfg.resolve("bash", "rm -rf x"), Level::Deny);
        assert_eq!(cfg.resolve("bash", "echo hi"), Level::Ask);
        assert_eq!(cfg.resolve("read", "main.rs"), Level::Ask); // fallback to global
    }

    #[test]
    fn parses_shapes() {
        #[derive(Deserialize)]
        struct Wrap {
            pub permission: PermissionConfig,
        }
        // string
        let w: Wrap = toml::from_str("permission = \"ask\"").unwrap();
        assert_eq!(w.permission.global, Level::Ask);
        // table with tool-level string + granular sub-table
        let w: Wrap = toml::from_str(
            r#"
            [permission]
            "*" = "ask"
            [permission.bash]
            "*" = "ask"
            "git *" = "allow"
            "rm *" = "deny"
            "#,
        )
        .unwrap();
        assert_eq!(w.permission.global, Level::Ask);
        assert_eq!(w.permission.resolve("bash", "git status"), Level::Allow);
        assert_eq!(w.permission.resolve("bash", "rm x"), Level::Deny);
    }

    #[test]
    fn key_mapping() {
        assert_eq!(permission_key("run_command"), "bash");
        assert_eq!(permission_key("edit_file"), "edit");
        assert_eq!(permission_key("read_file"), "read");
        assert_eq!(permission_key("anything_else"), "custom");
    }

    #[tokio::test]
    async fn ask_prompts_and_approve_once_resolves() {
        let hub = PermissionHub::new(PermissionConfig {
            global: Level::Ask,
            rules: Vec::new(),
        });
        let h = hub.clone();
        let handle = tokio::spawn(async move { h.request("c1", "bash", "rm x").await });
        tokio::task::yield_now().await;
        assert!(hub.has_pending(), "an `ask` tool must open a dialog");
        let (tool, key, input) = hub.pending_request().unwrap();
        assert_eq!(key, "bash");
        assert_eq!(input, "rm x");
        assert_eq!(tool, key);
        hub.reply(true, false); // approve once
        assert!(handle.await.unwrap());
    }

    #[tokio::test]
    async fn reject_returns_false() {
        let hub = PermissionHub::new(PermissionConfig {
            global: Level::Ask,
            rules: Vec::new(),
        });
        let h = hub.clone();
        let handle = tokio::spawn(async move { h.request("c1", "bash", "pkill -9 x").await });
        tokio::task::yield_now().await;
        hub.reply(false, false); // deny
        assert!(!handle.await.unwrap());
    }

    #[tokio::test]
    async fn always_approve_skips_future_prompt() {
        let hub = PermissionHub::new(PermissionConfig {
            global: Level::Ask,
            rules: Vec::new(),
        });
        // First call: user picks "always".
        let h1 = hub.clone();
        let t1 = tokio::spawn(async move { h1.request("c1", "edit", "src/main.rs").await });
        tokio::task::yield_now().await;
        hub.reply(true, true);
        assert!(t1.await.unwrap());
        // Second identical call should sail through without a dialog.
        let h2 = hub.clone();
        let t2 = tokio::spawn(async move { h2.request("c2", "edit", "src/main.rs").await });
        tokio::task::yield_now().await;
        assert!(!hub.has_pending(), "always-approve must not re-prompt");
        assert!(t2.await.unwrap());
    }

    #[tokio::test]
    async fn doom_loop_forces_prompt() {
        // Deny normally blocks; but the 3rd identical request still forces a
        // dialog so the user can override a runaway loop.
        let hub = PermissionHub::new(PermissionConfig {
            global: Level::Deny,
            rules: Vec::new(),
        });
        assert!(!hub.request("c1", "bash", "git push -f").await);
        assert!(!hub.request("c2", "bash", "git push -f").await);
        let h = hub.clone();
        let handle = tokio::spawn(async move { h.request("c3", "bash", "git push -f").await });
        tokio::task::yield_now().await;
        assert!(hub.has_pending(), "doom-loop must surface a dialog");
        hub.reply(true, false);
        assert!(handle.await.unwrap());
    }
}