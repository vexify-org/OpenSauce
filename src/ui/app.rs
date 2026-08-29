//! The interactive TUI application: state + event loop + drawing.

use super::keybind::{self, Action, Binding};
use super::view::draw;
use crate::config::Config;
use crate::core::agent::{Agent, AgentEvent};
use crate::core::message::{Message, Role};
use crate::core::session::{Conversation, SessionStore, default_system};
use crate::core::tools::registry::ToolRegistry;
use crate::core::tools::todo::{Todo, Todos};
use crate::mode::Mode;
use crate::permission::PermissionHub;
use crate::provider::mock::MockProvider;
use crate::provider::openai::OpenAIClient;
use crate::provider::Provider;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::execute;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::io;

pub struct Shared {
    pub conv: RwLock<Conversation>,
    pub streaming: AtomicBool,
    pub pending: Mutex<String>,
    pub status: Mutex<String>,
    pub error: Mutex<Option<String>>,
}

impl Shared {
    pub fn new(conv: Conversation) -> Arc<Self> {
        Arc::new(Shared {
            conv: RwLock::new(conv),
            streaming: AtomicBool::new(false),
            pending: Mutex::new(String::new()),
            status: Mutex::new("ready".into()),
            error: Mutex::new(None),
        })
    }
    pub fn lock_conv(&self) -> std::sync::RwLockReadGuard<'_, Conversation> {
        self.conv.read().unwrap()
    }
    pub fn lock_conv_mut(&self) -> std::sync::RwLockWriteGuard<'_, Conversation> {
        self.conv.write().unwrap()
    }
    pub fn is_streaming(&self) -> bool {
        self.streaming.load(Ordering::Relaxed)
    }
    pub fn set_streaming(&self, v: bool) {
        self.streaming.store(v, Ordering::Relaxed);
    }
    pub fn pending(&self) -> String {
        self.pending.lock().unwrap().clone()
    }
    pub fn status_line(&self) -> String {
        self.status.lock().unwrap().clone()
    }
}

pub struct App {
    pub shared: Arc<Shared>,
    pub input: String,
    pub mode: Mode,
    tools: Arc<ToolRegistry>,
    provider: Arc<dyn Provider>,
    model: String,
    store: SessionStore,
    todos: Todos,
    root: PathBuf,
    // opencode-style state
    perm: Arc<PermissionHub>,
    thinking: bool,
    bindings: Vec<Binding>,
    leader_pending: bool,
    abort: Option<tokio::task::AbortHandle>,
    sidebar_open: bool,
    details: bool,
    palette_open: bool,
    palette_index: usize,
    undo_buf: Vec<Message>,
    redo_buf: Vec<Message>,
}

pub async fn run_tui(config: Config) -> Result<()> {
    let root = std::env::current_dir().unwrap_or_else(|_| ".".into());
    let todos = Todo::fresh();
    let tools = Arc::new(ToolRegistry::new_shared(root.clone(), todos.clone()));
    let perm = PermissionHub::new(config.permission.clone());
    let auto_from_env = std::env::var("OPENCODE_AUTO").map(|v| v == "1").unwrap_or(false)
        || std::env::var("OPENSAUCE_AUTO").map(|v| v == "1").unwrap_or(false);
    if auto_from_env {
        perm.set_auto(true);
    }

    // Provider selection
    let (provider, model): (Arc<dyn Provider>, String) = if config.has_real_api() {
        match OpenAIClient::from_config(&config) {
            Some(c) => {
                let m = c.resolved_model().to_string();
                (Arc::new(c), m)
            }
            None => (Arc::new(MockProvider::new()), "mock".into()),
        }
    } else {
        (Arc::new(MockProvider::new()), "mock".into())
    };

    let store = SessionStore::open()?;
    store.ensure_dir()?;
    let conv = Conversation::new(
        format!("s-{}", crate::core::message::now_millis()),
        "new session",
        config.mode,
        default_system(config.mode),
    );

    let shared = Shared::new(conv);
    shared.set_streaming(false);

    let mut app = App {
        shared,
        input: String::new(),
        mode: config.mode,
        tools,
        provider,
        model,
        store,
        todos,
        root,
        perm,
        thinking: false,
        bindings: keybind::default_bindings(),
        leader_pending: false,
        abort: None,
        sidebar_open: false,
        details: false,
        palette_open: false,
        palette_index: 0,
        undo_buf: Vec::new(),
        redo_buf: Vec::new(),
    };

    let mut terminal = init_terminal()?;
    let res = app_event_loop(&mut terminal, &mut app).await;
    restore_terminal(&mut terminal)?;
    res
}

async fn app_event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
) -> Result<()> {
    let mut last_tick = std::time::Instant::now();
    let tick_rate = std::time::Duration::from_millis(80);
    let mut leader_armed_at = std::time::Instant::now();

    loop {
        // Leader-key timeout: clear if the follow-up is never pressed.
        if app.leader_pending && last_tick.duration_since(leader_armed_at) > tick_rate * 5 {
            app.leader_pending = false;
        }

        terminal.draw(|f| draw(f, app))?;

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) => {
                    if handle_key(app, key).await? {
                        break;
                    }
                    if app.leader_pending {
                        leader_armed_at = std::time::Instant::now();
                    }
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
        last_tick = std::time::Instant::now();
    }
    Ok(())
}

async fn handle_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    // While a permission dialog is open, only its three choices matter.
    if app.perm.has_pending() {
        return app.handle_permission_key(key);
    }
    // While the command palette is open, navigation controls it.
    if app.palette_open {
        return app.handle_palette_key(key);
    }

    let Some(chord) = keybind::chord_of(key) else {
        return Ok(false);
    };

    // Start a leader sequence.
    if keybind::is_leader(&chord) && !app.leader_pending {
        app.leader_pending = true;
        return Ok(false);
    }

    let action = keybind::resolve(&app.bindings, chord, app.leader_pending);
    app.leader_pending = false;

    // While the agent is busy, only interrupt / quit are honoured.
    if app.shared.is_streaming() {
        match action {
            Some(Action::SessionInterrupt) => {
                app.abort();
            }
            Some(Action::AppQuit) => return Ok(true),
            _ => {}
        }
        return Ok(false);
    }

    match action {
        Some(Action::AppQuit) => return Ok(true),
        Some(Action::SessionNew) => app.new_session(),
        Some(Action::SessionList) => app.list_sessions(),
        Some(Action::SessionExport) => app.export_session()?,
        Some(Action::SessionCompact) => app.compact(),
        Some(Action::ModelList) => app.show_models(),
        Some(Action::AgentCycle) => app.toggle_mode(),
        Some(Action::EditorOpen) => app.editor_open()?,
        Some(Action::ToggleSidebar) => app.toggle_sidebar(),
        Some(Action::ToggleDetails) => app.toggle_details(),
        Some(Action::HelpShow) => {
            app.run_command("/help")?;
        }
        Some(Action::CommandList) => app.palette_toggle(),
        Some(Action::Share) => app.run_command("/share")?,
        Some(Action::Themes) => app.run_command("/themes")?,
        Some(Action::Init) => app.run_command("/init")?,
        Some(Action::Thinking) => app.toggle_thinking(),
        Some(Action::AutoApprove) => app.toggle_auto(),
        Some(Action::InputSubmit) => {
            app.submit().await?;
        }
        Some(Action::InputNewline) => app.input.push('\n'),
        Some(Action::InputClear) => app.input.clear(),
        Some(Action::Undo) => app.undo(),
        Some(Action::Redo) => app.redo(),
        _ => {
            default_text_input(app, key);
        }
    }
    Ok(false)
}

/// Slash commands shown (and runnable) from the command palette (ctrl+p).
/// Each entry maps directly to a `/…` handler in `App::run_command`.
pub const PALETTE: &[&str] = &[
    "/help",
    "/new",
    "/compact",
    "/sessions",
    "/export",
    "/model",
    "/mode",
    "/undo",
    "/redo",
    "/init",
    "/share",
    "/permissions",
    "/thinking",
    "/themes",
    "/exit",
];

fn default_text_input(app: &mut App, key: KeyEvent) {
    use KeyCode::*;
    match key.code {
        Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => app.input.push(c),
        Backspace => {
            app.input.pop();
        }
        Delete => {
            app.input.pop();
        }
        Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            while let Some(c) = app.input.pop() {
                if c == ' ' || c == '\n' {
                    break;
                }
            }
        }
        _ => {}
    }
}

impl App {
    /// Keys active while a permission dialog is open: approve once / always, reject.
    fn handle_permission_key(&mut self, key: KeyEvent) -> Result<bool> {
        if let Some(chord) = keybind::chord_of(key) {
            if keybind::is_leader(&chord) {
                self.leader_pending = true;
                return Ok(false);
            }
            if let Some(Action::AppQuit) = keybind::resolve(&self.bindings, chord, false) {
                self.perm.reject();
                return Ok(true);
            }
        }
        match key.code {
            // approve once
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Char('o') | KeyCode::Char('1')
            | KeyCode::Enter => self.perm.reply(true, false),
            // approve always (rest of session)
            KeyCode::Char('a') | KeyCode::Char('A') | KeyCode::Char('2') | KeyCode::Tab => {
                self.perm.reply(true, true)
            }
            // reject
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Char('r') | KeyCode::Char('R')
            | KeyCode::Char('3') | KeyCode::Esc => self.perm.reply(false, false),
            _ => {}
        }
        Ok(false)
    }

    /// Key navigation for the command palette (ctrl+p).
    fn handle_palette_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc => self.palette_close(),
            KeyCode::Down | KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.palette_index = (self.palette_index + 1).min(PALETTE.len().saturating_sub(1));
            }
            KeyCode::Down => {
                self.palette_index = (self.palette_index + 1).min(PALETTE.len().saturating_sub(1))
            }
            KeyCode::Up | KeyCode::Char('k') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.palette_index = self.palette_index.saturating_sub(1);
            }
            KeyCode::Up => self.palette_index = self.palette_index.saturating_sub(1),
            KeyCode::Enter => {
                let cmd = PALETTE[self.palette_index].to_string();
                self.palette_close();
                self.run_command(&cmd)?;
            }
            _ => {}
        }
        Ok(false)
    }

    /// The open permission request, if any — for the dialog overlay.
    pub fn permission_request(&self) -> Option<(String, String, String)> {
        self.perm.pending_request()
    }
    pub fn perm_pending(&self) -> bool {
        self.perm.has_pending()
    }
    pub fn is_auto(&self) -> bool {
        self.perm.is_auto()
    }
    /// Palette overlay state: `(selected index, entries)` when open.
    pub fn palette(&self) -> Option<(usize, &'static [&'static str])> {
        if self.palette_open {
            Some((self.palette_index, PALETTE))
        } else {
            None
        }
    }

    async fn submit(&mut self) -> Result<()> {
        let text = self.input.trim_end().to_string();
        self.input.clear();
        if text.trim().is_empty() {
            return Ok(());
        }
        // `/command`
        if text.trim().starts_with('/') {
            return self.run_command(text.trim());
        }
        // `!shell` shortcut — run a command and attach its output.
        if let Some(stripped) = text.trim_start().strip_prefix('!') {
            self.redo_buf.clear();
            self.run_bash(stripped.trim());
            return Ok(());
        }
        // `@file` references are expanded into the message.
        let expanded = self.expand_references(&text);
        let shared = self.shared.clone();
        self.redo_buf.clear();
        {
            let mut conv = shared.lock_conv_mut();
            conv.push(Message::user(expanded));
        }
        self.spawn_agent(shared);
        Ok(())
    }

    /// Resolve `@path` tokens to real files (opencode-style file references).
    fn expand_references(&self, text: &str) -> String {
        let mut out = String::new();
        let mut rest = text;
        while let Some(idx) = rest.find('@') {
            out.push_str(&rest[..idx]);
            rest = &rest[idx..];
            // Take the token until whitespace/comma/EOF.
            let token_end = rest[1..]
                .find(|c: char| c.is_whitespace() || c == ',' || c == ')' || c == ']' || c == '"')
                .map(|i| i + 1)
                .unwrap_or(rest.len());
            let tok = &rest[1..token_end];
            if !tok.is_empty() && !tok.contains('/') && tok.contains('.') {
                // heuristic bare ref like `src/main.rs`
            }
            let cand = tok.trim().trim_end_matches(['.', ',' , ')']);
            if !cand.is_empty() {
                let full = self.root.join(cand);
                if full.is_file() {
                    if let Ok(content) = std::fs::read_to_string(&full) {
                        out.push_str(&format!("\n@{} →\n```\n{}\n```\n", cand, content.truncate_safe(60_000)));
                        rest = &rest[token_end..];
                        continue;
                    }
                }
            }
            out.push('@');
            rest = &rest[1..];
            // continue scanning after the token to avoid re-matching
            if token_end > 1 {
                out.push_str(&rest);
                rest = "";
            }
        }
        out.push_str(rest);
        out
    }

    fn run_bash(&mut self, cmd: &str) {
        let shared = self.shared.clone();
        {
            let mut conv = shared.lock_conv_mut();
            conv.push(Message::user(format!("!{cmd}")));
        }
        self.spawn_bash(shared, cmd.to_string());
    }

    fn spawn_bash(&self, shared: Arc<Shared>, cmd: String) {
        let root = self.root.clone();
        shared.set_streaming(true);
        *shared.status.lock().unwrap() = format!("$ {cmd}");
        let mut mk_sink = make_sink(shared.clone());
        tokio::spawn(async move {
            let mut sink = mk_sink();
            let mut working = shared.lock_conv_mut().clone();
            match crate::core::tools::shell::run_plain(&root, &cmd) {
                Ok(out) => {
                    sink(crate::core::agent::AgentEvent::ToolCallFinished {
                        name: "bash".into(),
                        ok: out.ok,
                    });
                    let m = crate::core::message::ToolResult {
                        call_id: format!("bash-{}", crate::core::message::now_millis()),
                        name: "bash".into(),
                        success: out.ok,
                        content: out.text,
                    };
                    working.push(Message::from_tool_result(m));
                }
                Err(e) => {
                    *shared.error.lock().unwrap() = Some(e.to_string());
                }
            }
            *shared.lock_conv_mut() = working;
            shared.set_streaming(false);
            *shared.status.lock().unwrap() = "ready".into();
        });
    }

    fn spawn_agent(&mut self, shared: Arc<Shared>) {
        let provider = self.provider.clone();
        let tools = self.tools.clone();
        let hub = self.perm.clone();
        let model = self.model.clone();
        let mut store = self.store.clone();

        shared.set_streaming(true);
        *shared.status.lock().unwrap() = "thinking…".into();
        *shared.error.lock().unwrap() = None;

        let mut mk_sink = make_sink(shared.clone());
        let handle = tokio::spawn(async move {
            let agent = Agent::new(provider, tools, hub, model);

            // Clone the transcript out, run against the copy, then write back
            // so no lock is held across `await`.
            let mut working = shared.lock_conv_mut().clone();
            let mut sink = mk_sink();
            let res = agent.run(&mut working, &mut sink).await;
            if let Err(e) = store.save(&working) {
                eprintln!("failed to persist session: {e}");
            }
            *shared.lock_conv_mut() = working;

            shared.set_streaming(false);
            {
                let mut pending = shared.pending.lock().unwrap();
                pending.clear();
            }
            if let Err(e) = res {
                *shared.status.lock().unwrap() = format!("agent error: {e}");
            } else {
                *shared.status.lock().unwrap() = "ready".into();
            }
        });
        self.abort.replace(handle.abort_handle());
    }

    /// Interrupt the in-flight generation, keeping whatever streamed so far.
    fn abort(&mut self) {
        if let Some(h) = self.abort.take() {
            h.abort();
        }
        self.shared.set_streaming(false);
        {
            // Commit any pending streamed text as a partial assistant reply.
            let partial = self.shared.pending();
            if !partial.is_empty() {
                let mut conv = self.shared.lock_conv_mut();
                conv.push(Message::assistant(partial));
            }
            *self.shared.pending.lock().unwrap() = String::new();
        }
        *self.shared.status.lock().unwrap() = "interrupted".into();
    }

    fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            Mode::Build => Mode::Plan,
            Mode::Plan => Mode::Build,
        };
        {
            let mut conv = self.shared.lock_conv_mut();
            conv.mode = self.mode;
            conv.system = default_system(self.mode);
        }
    }

    fn toggle_thinking(&mut self) {
        self.thinking = !self.thinking;
        *self.shared.status.lock().unwrap() = if self.thinking { "thinking: on" } else { "thinking: off" }.into();
    }

    fn palette_toggle(&mut self) {
        if self.palette_open {
            self.palette_close();
        } else {
            self.palette_open = true;
            self.palette_index = 0;
        }
    }
    fn palette_close(&mut self) {
        self.palette_open = false;
    }
    fn toggle_auto(&mut self) {
        let on = self.perm.toggle_auto();
        self.append_assistant(if on {
            "auto-approve permissions: enabled".into()
        } else {
            "auto-approve permissions: disabled".into()
        });
    }

    fn new_session(&mut self) {
        let mut conv = self.shared.lock_conv_mut();
        *conv = Conversation::new(
            format!("s-{}", crate::core::message::now_millis()),
            "new session",
            self.mode,
            default_system(self.mode),
        );
        drop(conv);
        self.input.clear();
        self.undo_buf.clear();
        self.redo_buf.clear();
        {
            let mut t = self.todos.lock().unwrap();
            t.clear();
        }
        *self.shared.status.lock().unwrap() = "new session".into();
    }

    fn toggle_sidebar(&mut self) {
        self.sidebar_open = !self.sidebar_open;
    }

    fn toggle_details(&mut self) {
        self.details = !self.details;
    }

    fn undo(&mut self) {
        let mut conv = self.shared.lock_conv_mut();
        if conv.messages.len() > 1 {
            if let Some(m) = conv.messages.pop() {
                self.redo_buf.push(m);
            }
        }
        drop(conv);
        let mut pending = self.shared.pending.lock().unwrap();
        pending.clear();
    }

    fn redo(&mut self) {
        if let Some(m) = self.redo_buf.pop() {
            let mut conv = self.shared.lock_conv_mut();
            conv.push(m);
            drop(conv);
            *self.shared.status.lock().unwrap() = "redo".into();
        } else {
            self.append_assistant("nothing to redo".into());
        }
    }

    fn compact(&mut self) {
        // Summarize by trimming the oldest turns, keeping the tail.
        let mut conv = self.shared.lock_conv_mut();
        if conv.messages.len() > 12 {
            let keep_from = conv.messages.len() - 12;
            conv.messages.drain(..keep_from);
        }
        drop(conv);
        *self.shared.status.lock().unwrap() = "session compacted".into();
    }

    fn list_sessions(&mut self) {
        let ids = self.store.list().unwrap_or_default();
        if ids.is_empty() {
            self.append_assistant("no sessions yet".into());
            return;
        }
        let mut text = String::from("sessions:");
        for id in ids.iter().take(20) {
            let t = self
                .store
                .load(id)
                .map(|c| c.title)
                .unwrap_or_else(|_| id.clone());
            text.push_str(&format!("\n  • {t}  ({id})"));
        }
        self.append_assistant(text);
    }

    fn show_models(&mut self) {
        let names = self.tools.names();
        let mut text = format!("model: {}\nprovider tools:\n", self.model);
        for n in names {
            text.push_str(&format!("  • {n}\n"));
        }
        text.push_str("\nset a model with `/model <name>` or env OPENSAUCE_MODEL.");
        self.append_assistant(text.trim_end().to_string());
    }

    /// Edit the prompt in the user's `$EDITOR`.
    fn editor_open(&mut self) -> Result<()> {
        let editor = std::env::var("EDITOR")
            .or_else(|_| std::env::var("VISUAL"))
            .unwrap_or_else(|_| "vi".into());
        let tmp = std::env::temp_dir().join("opensauce-prompt.txt");
        std::fs::write(&tmp, &self.input)?;
        let status = std::process::Command::new(editor)
            .arg(&tmp)
            .status()
            .map_err(|e| anyhow::anyhow!("failed to open editor: {e}"))?;
        if status.success() {
            self.input = std::fs::read_to_string(&tmp).unwrap_or_default();
        }
        Ok(())
    }

    fn export_session(&mut self) -> Result<()> {
        let md = self.transcript();
        let path = std::env::temp_dir().join(format!("opensauce-{}.md", crate::core::message::now_millis()));
        std::fs::write(&path, md)?;
        self.append_assistant(format!("exported to {}", path.display()));
        Ok(())
    }

    fn run_command(&mut self, cmd: &str) -> Result<()> {
        let trimmed = cmd.trim();
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.is_empty() {
            return Ok(());
        }
        match parts[0] {
            "/help" | "/?" | "/commands" => {
                self.append_assistant(
                    "commands:\n  /mode build|plan   switch mode\n  /model <name>      set model\n  /models            list tools/help\n  /new /clear        new session\n  /compact           summarize\n  /sessions          list sessions\n  /undo              undo last turn\n  /redo              redo undone turn\n  /export            export chat as markdown\n  /editor            compose in $EDITOR\n  /init              rule/todo help\n  /exit /quit        quit\n\nleader (ctrl+x): n new · l sessions · m models · e editor · s status · x export · u undo · c compact · q quit\n  Tab: mode · Esc: interrupt · Ctrl+P: commands"
                        .into(),
                );
            }
            "/mode" => {
                if let Some(m) = parts.get(1).and_then(|m| Mode::from_name(m)) {
                    self.mode = m;
                    let mut conv = self.shared.lock_conv_mut();
                    conv.mode = m;
                    conv.system = default_system(m);
                } else {
                    self.append_assistant(format!("mode is [{}] — use /mode build|plan", self.mode));
                }
            }
            "/model" => {
                if let Some(m) = parts.get(1) {
                    self.model = m.to_string();
                    self.append_assistant(format!("model → {m}"));
                } else {
                    self.append_assistant(format!("model: {}", self.model));
                }
            }
            "/models" => self.show_models(),
            "/new" | "/clear" => self.new_session(),
            "/compact" => self.compact(),
            "/undo" => self.undo(),
            "/redo" => self.redo(),
            "/sessions" => self.list_sessions(),
            "/export" => self.export_session()?,
            "/editor" => self.editor_open()?,
            "/init" => {
                self.append_assistant(
                    "Opensauce follows AGENTS/rules from project files when present. Create a `AGENTS.md` at the repo root to steer the agent.\n\nTip: to plan work, switch to Plan (yellow) mode and let the agent propose tasks before Build."
                        .into(),
                );
            }
            "/share" => {
                self.append_assistant(
                    "share — export this conversation as a shareable link/markdown.\n(Coming soon; use /export meanwhile to dump the markdown.)"
                        .into(),
                );
            }
            "/permissions" => {
                self.append_permissions();
            }
            "/thinking" => {
                self.toggle_thinking();
            }
            "/themes" => {
                self.append_assistant(format!(
                    "theme: [{}] — build=blue · plan=yellow.\n`/mode plan` switches to the yellow planning theme.",
                    self.mode.label()
                ));
            }
            "/exit" | "/quit" | "/q" => {
                use std::process::exit;
                exit(0);
            }
            other => {
                self.append_assistant(format!("unknown command: {other} (try /help)"));
            }
        }
        Ok(())
    }

    fn append_assistant(&mut self, text: String) {
        let mut conv = self.shared.lock_conv_mut();
        conv.push(Message::assistant(text));
    }

    /// `/permissions` — show configured rules and session-approved inputs.
    fn append_permissions(&mut self) {
        let mut text = String::from("permissions:");
        for rule in &self.perm.cfg.rules {
            text.push_str(&format!("\n  {:<6} {} @ {}", rule.level.label(), rule.tool, rule.pattern));
        }
        let keys = ["bash", "edit", "read", "grep", "glob", "todo", "custom"];
        let mut has_approved = false;
        for k in keys {
            let approved = self.perm.approved(k);
            if !approved.is_empty() {
                has_approved = true;
                for a in approved {
                    text.push_str(&format!("\n  [allow {}] {} (session)", k, a));
                }
            }
        }
        if !has_approved {
            text.push_str("\n  (no session approvals yet)");
        }
        text.push_str("\n\nkeys: y/once · a/always · n/reject — `--auto` or OPENCODE_AUTO=1 to approve all");
        self.append_assistant(text);
    }

    fn transcript(&self) -> String {
        let conv = self.shared.lock_conv();
        let mut lines = Vec::new();
        for m in conv.turns() {
            let who = match m.role {
                Role::User => "you",
                Role::Assistant => "opencode",
                Role::Tool => "tool",
                Role::System => "system",
            };
            lines.push(format!("### {who}\n{}", m.display_text()));
        }
        lines.join("\n\n")
    }
}

trait Truncate {
    fn truncate_safe(self, n: usize) -> String;
}
impl Truncate for String {
    fn truncate_safe(mut self, n: usize) -> String {
        if self.len() > n {
            self.truncate(n);
            self.push_str("\n… [truncated]");
        }
        self
    }
}

/// Build a sink closure that applies agent events to shared state.
fn make_sink(shared: Arc<Shared>) -> impl FnMut() -> Box<dyn FnMut(AgentEvent) + Send> {
    move || -> Box<dyn FnMut(AgentEvent) + Send> {
        let s = shared.clone();
        Box::new(move |event: AgentEvent| match event {
            AgentEvent::TextDelta(t) => {
                s.pending.lock().unwrap().push_str(&t);
            }
            AgentEvent::ToolCallQueued { name, .. } => {
                *s.status.lock().unwrap() = format!("call {name}…");
            }
            AgentEvent::PermissionRequest { tool, .. } => {
                // The dialog state lives in the shared hub; just reflect it.
                *s.status.lock().unwrap() = format!("permission: {tool}");
            }
            AgentEvent::ToolCallFinished { name, ok } => {
                *s.status.lock().unwrap() = if ok {
                    format!("{name} ✓")
                } else {
                    format!("{name} ✗")
                };
            }
            AgentEvent::Done { usage_prompt, usage_completion } => {
                *s.status.lock().unwrap() =
                    format!("ready · {usage_prompt} ↥ / {usage_completion} ↧ tokens");
            }
        })
    }
}

fn init_terminal() -> Result<ratatui::DefaultTerminal> {
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut t = ratatui::Terminal::new(backend)?;
    t.hide_cursor()?;
    Ok(t)
}

fn restore_terminal(terminal: &mut ratatui::DefaultTerminal) -> Result<()> {
    terminal::disable_raw_mode()?;
    terminal.show_cursor()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}