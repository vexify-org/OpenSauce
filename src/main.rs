use clap::{Parser, Subcommand};
use opensauce::config::Config;
use opensauce::core::message::Message;
use opensauce::core::session::{Conversation, default_system};
use opensauce::core::tools::registry::ToolRegistry;
use opensauce::mode::Mode;
use opensauce::permission::PermissionHub;
use opensauce::provider::mock::MockProvider;
use opensauce::provider::openai::OpenAIClient;
use opensauce::provider::Provider;

#[derive(Parser)]
#[command(
    name = "opensauce",
    version,
    about = "OpenSauce — a modern Rust coding agent in your terminal. Powered By Vexify."
)]
struct Cli {
    /// Auto-approve all tool permissions (`permission=ask` still prompts with a dialog; this skips it).
    #[arg(long, global = true)]
    auto: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Start the interactive TUI at a given mode.
    Start {
        #[arg(long, default_value = "build")]
        mode: String,
    },
    /// Run a single prompt headlessly and print the reply.
    Run {
        prompt: String,
        #[arg(long, default_value = "build")]
        mode: String,
    },
    /// List stored sessions.
    Sessions,
    /// Interactively configure a model provider (name / API key / base URL /
    /// model), saved without editing any config file by hand.
    Connect,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let config = Config::load().expect("failed to load config");

    // `--auto` mirrors opencode's auto-approve mode; the hub (TUI + headless)
    // already honours `OPENCODE_AUTO=1`.
    if cli.auto {
        std::env::set_var("OPENCODE_AUTO", "1");
    }

    let exit_code = match cli.command {
        Some(Command::Start { mode }) => {
            let cfg = Config {
                mode: Mode::from_name(&mode).unwrap_or(config.mode),
                ..config
            };
            opensauce::ui::run_tui(cfg).await.map(|_| 0)
        }
        Some(Command::Run { prompt, mode }) => {
            let cfg = Config {
                mode: Mode::from_name(&mode).unwrap_or(config.mode),
                ..config
            };
            run_headless(&cfg, &prompt).await.map(|reply| {
                println!("{reply}");
                0
            })
        }
        Some(Command::Sessions) => list_sessions().map(|_| 0),
        Some(Command::Connect) => connect().await.map(|_| 0),
        None => opensauce::ui::run_tui(config).await.map(|_| 0),
    };

    std::process::exit(exit_code.unwrap_or(1));
}

async fn run_headless(config: &Config, prompt: &str) -> anyhow::Result<String> {
    let root = std::env::current_dir().unwrap_or_else(|_| ".".into());
    let tools = ToolRegistry::new(root);

    let (provider, model): (std::sync::Arc<dyn Provider>, String) = if config.has_real_api() {
        let c = OpenAIClient::from_config(config).ok_or_else(|| anyhow::anyhow!("api key set but client failed to init"))?;
        let m = c.resolved_model().to_string();
        (std::sync::Arc::new(c), m)
    } else {
        (std::sync::Arc::new(MockProvider::new()), "mock".into())
    };

    let mut conv = Conversation::new("headless", "headless", config.mode, default_system(config.mode));
    conv.push(Message::user(prompt));
    // Headless runs gate tools through the same permission hub as the TUI.
    let hub = PermissionHub::new(config.permission.clone());
    if std::env::var("OPENCODE_AUTO").map(|v| v == "1").unwrap_or(false)
        || std::env::var("OPENSAUCE_AUTO").map(|v| v == "1").unwrap_or(false)
    {
        hub.set_auto(true);
    }
    let agent = opensauce::core::agent::Agent::new(provider, std::sync::Arc::new(tools), hub, model);
    opensauce::core::agent::run_headless(&agent, &mut conv).await
}

fn list_sessions() -> anyhow::Result<()> {
    use opensauce::core::session::SessionStore;
    let store = SessionStore::open()?;
    let ids = store.list()?;
    if ids.is_empty() {
        println!("no sessions yet");
        return Ok(());
    }
    for id in &ids {
        println!("{id}");
    }
    Ok(())
}

async fn connect() -> anyhow::Result<()> {
    let conn = opensauce::connect::run_wizard().await?;
    opensauce::connect::save(&conn)?;
    let path = opensauce::connect::connection_path();
    println!(
        "✓ 已保存连接：{} @ {}（模型 {}）\n配置文件：{}",
        conn.name,
        conn.base_url,
        if conn.model.is_empty() { "(自动)" } else { &conn.model },
        path.display()
    );
    println!("现在直接运行 `opensauce` 即可使用该连接。");
    Ok(())
}