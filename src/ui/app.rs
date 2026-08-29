//! The interactive TUI application: state + event loop + drawing.

use super::view::draw;
use crate::config::Config;
use crate::core::agent::{Agent, AgentEvent};
use crate::core::message::{Message, Role};
use crate::core::session::{Conversation, SessionStore, default_system};
use crate::core::tools::registry::ToolRegistry;
use crate::mode::Mode;
use crate::provider::mock::MockProvider;
use crate::provider::openai::OpenAIClient;
use crate::provider::Provider;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::execute;
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
    cmd_mode: String, // "" normal, or temp for command display
}

pub async fn run_tui(config: Config) -> Result<()> {
    let root = std::env::current_dir().unwrap_or_else(|_| ".".into());
    let tools = Arc::new(ToolRegistry::new(root));

    // Provider selection
    let (provider, model): (Arc<dyn Provider>, String) = if config.has_real_api() {
        match OpenAIClient::from_env() {
            Some(c) => {
                let m = if config.model.is_empty() {
                    c.default_model().to_string()
                } else {
                    config.model.clone()
                };
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
        cmd_mode: String::new(),
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

    loop {
        terminal.draw(|f| draw(f, app))?;

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) => {
                    if handle_key(app, key).await? {
                        break;
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
    use KeyCode::*;
    // Global quit
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == Char('c') {
        return Ok(true);
    }
    if key.code == Esc {
        return Ok(true);
    }

    // Slash commands (enter to submit)
    if !app.input.is_empty() && app.input.starts_with('/') && key.code == Enter {
        app.submit().await?;
        return Ok(false);
    }

    match key.code {
        Char(c) if !app.shared.is_streaming()
            && !key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            app.input.push(c);
        }
        Backspace => {
            app.input.pop();
        }
        Delete => {
            app.input.pop();
        }
        Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.input.clear();
        }
        Enter => {
            app.submit().await?;
        }
        Tab | Char('M') => {
            app.toggle_mode();
        }
        Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => app.new_session(),
        _ => {}
    }
    Ok(false)
}

impl App {
    async fn submit(&mut self) -> Result<()> {
        {
            let text = self.input.trim().to_string();
            self.input.clear();
            if text.is_empty() {
                return Ok(());
            }
            if text.starts_with('/') {
                return self.run_command(&text);
            }
            // Commit the prompt and kick off the agent.
            let shared = self.shared.clone();
            {
                let mut conv = shared.lock_conv_mut();
                conv.push(Message::user(text));
            }
            self.spawn_agent(shared);
        }
        Ok(())
    }

    fn spawn_agent(&mut self, shared: Arc<Shared>) {
        let provider = self.provider.clone();
        let tools = self.tools.clone();
        let model = self.model.clone();
        let mut store = self.store.clone();

        shared.set_streaming(true);
        *shared.status.lock().unwrap() = "thinking…".into();
        *shared.error.lock().unwrap() = None;

        let mut mk_sink = make_sink(shared.clone());
        tokio::spawn(async move {
            let agent = Agent::new(provider, tools, model);

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
    }

    fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            Mode::Build => Mode::Plan,
            Mode::Plan => Mode::Build,
        };
        // Reflect mode in conversation's system prompt for future runs.
        {
            let mut conv = self.shared.lock_conv_mut();
            conv.mode = self.mode;
            conv.system = default_system(self.mode);
        }
    }

    fn new_session(&mut self) {
        {
            let mut conv = self.shared.lock_conv_mut();
            *conv = Conversation::new(
                format!("s-{}", crate::core::message::now_millis()),
                "new session",
                self.mode,
                default_system(self.mode),
            );
        }
        self.input.clear();
        *self.shared.status.lock().unwrap() = "new session".into();
    }

    fn run_command(&mut self, cmd: &str) -> Result<()> {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        if parts.is_empty() {
            return Ok(());
        }
        match parts[0] {
            "/help" | "/?" => {
                self.cmd_mode = "/help".into();
                let text = format!(
                    "commands:\n  /mode build|plan   switch conversation mode\n  /model <name>      set model\n  /new               start a fresh session\n  /exit               quit\nkeys:\n  Tab  toggle Build/Plan   Ctrl+C quit"
                );
                self.append_assistant(text);
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
            "/new" => self.new_session(),
            "/exit" | "/quit" => {
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

    /// Ready-made transcript copy for potential future use (e.g. /save).
    #[allow(dead_code)]
    fn transcript(&self) -> String {
        let conv = self.shared.lock_conv();
        let mut lines = Vec::new();
        for m in conv.turns() {
            let who = match m.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
                Role::System => "system",
            };
            lines.push(format!("[{who}] {}", m.display_text()));
        }
        lines.join("\n")
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