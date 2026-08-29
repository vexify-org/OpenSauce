//! Connection wizard — configure a model provider without touching config files.
//!
//! `opensauce connect` walks the user through picking a provider preset (or a
//! custom "Other" entry), entering a name / API key / base URL, and deciding
//! between automatically fetching the model list or typing a model by hand.
//! The result is persisted to `$XDG_CONFIG_HOME/opensauce/connection.toml`
//! (mode 0600) and picked up by the provider layer at runtime.

use anyhow::{Context, Result};
use inquire::{Confirm, Select, Text};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// The connection profile users create once via the wizard.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Connection {
    /// Human-friendly provider name (e.g. "DeepSeek").
    pub name: String,
    /// API key / secret (sk-…).
    pub api_key: String,
    /// Base URL without a trailing slash (e.g. `https://api.deepseek.com/v1`).
    pub base_url: String,
    /// Model or endpoint name to use.
    pub model: String,
}

impl Default for Connection {
    fn default() -> Self {
        Connection {
            name: String::new(),
            api_key: String::new(),
            base_url: String::new(),
            model: String::new(),
        }
    }
}

/// A selectable provider template. The list is shown with a trailing "Other"
/// option so users can define a fully custom provider.
#[derive(Debug, Clone, Copy)]
struct Preset {
    label: &'static str,
    base_url: &'static str,
    default_model: &'static str,
}

const PRESETS: &[Preset] = &[
    Preset { label: "OpenAI", base_url: "https://api.openai.com/v1", default_model: "gpt-4o-mini" },
    Preset { label: "OpenRouter", base_url: "https://openrouter.ai/api/v1", default_model: "openrouter/auto" },
    Preset { label: "DeepSeek", base_url: "https://api.deepseek.com/v1", default_model: "deepseek-chat" },
    Preset { label: "Moonshot", base_url: "https://api.moonshot.cn/v1", default_model: "moonshot-v1-8k" },
    Preset { label: "智谱 GLM", base_url: "https://open.bigmodel.cn/api/paas/v4", default_model: "glm-4-flash" },
    Preset { label: "通义千问 Qwen", base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1", default_model: "qwen-plus" },
    Preset { label: "Other（自定义）", base_url: "https://api.example.com/v1", default_model: "" },
];

/// Default path to the connection file.
pub fn connection_path() -> PathBuf {
    let dir = dirs::config_dir().map(|d| d.join("opensauce")).unwrap_or_else(|| PathBuf::from(".opensauce"));
    dir.join("connection.toml")
}

/// Load the saved connection, if any.
pub fn load() -> Option<Connection> {
    let path = connection_path();
    let raw = std::fs::read_to_string(&path).ok()?;
    toml::from_str(&raw).ok()
}

/// Whether a non-empty api key is configured anywhere (connection or env).
pub fn has_api_key() -> bool {
    if let Some(c) = load() {
        if !c.api_key.trim().is_empty() {
            return true;
        }
    }
    std::env::var("OPENSAUCE_API_KEY")
        .or_else(|_| std::env::var("OPENAI_API_KEY"))
        .map(|k| !k.is_empty())
        .unwrap_or(false)
}

/// Persist a connection to disk with restrictive permissions.
pub fn save(conn: &Connection) -> Result<()> {
    let path = connection_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let body = toml::to_string(conn)?;
    std::fs::write(&path, &body)?;
    // The file holds an API key; lock it down to the owner.
    let _ = std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600));
    Ok(())
}

/// Interactive wizard. Returns the fully-configured connection.
pub async fn run_wizard() -> Result<Connection> {
    let existing = load();
    let default_key = existing.as_ref().map(|c| c.api_key.clone()).unwrap_or_default();
    let default_model = existing.as_ref().map(|c| c.model.clone()).unwrap_or_default();

    // 1) Pick a provider preset, "Other" sits at the bottom.
    let labels: Vec<&str> = PRESETS.iter().map(|p| p.label).collect();
    let choice_label = Select::new("选择大模型服务商 / Provider:", labels)
        .with_starting_cursor(1)
        .prompt()
        .context("provider selection cancelled")?;
    let preset = PRESETS.iter().find(|p| p.label == choice_label).unwrap();

    let is_other = preset.label.starts_with("Other");
    let default_base = if is_other {
        existing.as_ref().map(|c| c.base_url.clone()).unwrap_or_default()
    } else {
        preset.base_url.to_string()
    };
    let suggested_model = if is_other {
        if default_model.is_empty() {
            preset.default_model.to_string()
        } else {
            default_model.clone()
        }
    } else {
        preset.default_model.to_string()
    };

    // 2) Provider name (display only).
    let name = Text::new("名称 / Name:")
        .with_default(&preset.label.to_string())
        .prompt()
        .context("name cancelled")?;

    // 3) API key (sk-…).
    let api_key = Text::new("API Key (sk-…):")
        .with_default(&default_key)
        .prompt()
        .context("api key cancelled")?;

    // 4) Base URL / address.
    let base_url = Text::new("服务地址 / Base URL（例如 https://api.deepseek.com/v1）:")
        .with_default(&default_base)
        .prompt()
        .context("base url cancelled")?
        .trim_end_matches('/')
        .to_string();

    // 5) Model: auto-fetch the list, or set it manually.
    let auto_fetch = Confirm::new("自动获取模型列表？（否则手动输入模型名）")
        .with_default(true)
        .prompt()
        .context("model choice cancelled")?;

    let model = if auto_fetch {
        match fetch_models(&base_url, &api_key).await {
            Ok(mut models) if !models.is_empty() => {
                models.sort();
                models.dedup();
                if models.len() == 1 {
                    models[0].clone()
                } else {
                    let picked = Select::new("选择模型 / Model:", models)
                        .with_starting_cursor(0)
                        .prompt()
                        .context("model selection cancelled")?;
                    picked
                }
            }
            _ => {
                // List unavailable — fall back to manual entry.
                eprintln!("⚠ 无法自动获取模型列表，请手动输入模型名。");
                Text::new("模型名 / Model:").with_default(&suggested_model).prompt().context("model cancelled")?
            }
        }
    } else {
        Text::new("模型名 / Model:")
            .with_default(&suggested_model)
            .prompt()
            .context("model cancelled")?
    };

    Ok(Connection { name, api_key, base_url, model })
}

/// Fetch `GET {base_url}/models` and return the model ids (OpenAI-compatible).
async fn fetch_models(base_url: &str, api_key: &str) -> Result<Vec<String>> {
    #[derive(Deserialize)]
    struct List {
        data: Vec<Item>,
    }
    #[derive(Deserialize)]
    struct Item {
        id: String,
    }

    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let resp = reqwest::Client::new().get(&url).bearer_auth(api_key).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("GET {url} -> {}", resp.status());
    }
    let list: List = resp.json().await?;
    Ok(list.data.into_iter().map(|i| i.id).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_end_with_other() {
        assert!(PRESETS.last().unwrap().label.starts_with("Other"));
    }

    #[test]
    fn connection_round_trip() {
        let conn = Connection {
            name: "DeepSeek".into(),
            api_key: "sk-test".into(),
            base_url: "https://api.deepseek.com/v1".into(),
            model: "deepseek-chat".into(),
        };
        let toml = toml::to_string(&conn).unwrap();
        let back: Connection = toml::from_str(&toml).unwrap();
        assert_eq!(back, conn);
    }

    #[test]
    fn path_is_under_config_dir() {
        let p = connection_path();
        assert!(p.file_name().unwrap().to_string_lossy().contains("connection"));
    }
}