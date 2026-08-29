//! OpenSauce configuration.
//!
//! Sources (later wins): defaults → `OpenSauce.toml` in the working directory
//! (or `$XDG_CONFIG_HOME/opensauce/opensauce.toml`) → environment variables.

use crate::mode::Mode;
use crate::permission::PermissionConfig;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Model name to use. Empty = provider default.
    pub model: String,
    /// Provider key. One of: auto | openai | mock. `auto` picks mock when no
    /// API key is present.
    pub provider: String,
    /// Default conversation mode.
    pub mode: Mode,
    /// Whether to attempt remote (real) models at all.
    pub model_hint: String,
    /// Permission rules (opencode `permission` shape: allow/ask/deny + optional
    /// per-tool granular patterns). Last matching rule wins.
    pub permission: PermissionConfig,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            model: String::new(),
            provider: "auto".into(),
            mode: Mode::Build,
            model_hint: String::new(),
            permission: PermissionConfig::default(),
        }
    }
}

impl Config {
    pub fn load() -> Result<Config> {
        let from_file = load_file().unwrap_or_default();
        Ok(from_file.apply_env())
    }

    fn apply_env(mut self) -> Config {
        if let Ok(v) = std::env::var("OPENSAUCE_MODEL") {
            if !v.is_empty() {
                self.model = v;
            }
        }
        if let Ok(v) = std::env::var("OPENSAUCE_PROVIDER") {
            if !v.is_empty() {
                self.provider = v;
            }
        }
        if let Ok(v) = std::env::var("OPENSAUCE_MODE") {
            if let Some(m) = Mode::from_name(&v) {
                self.mode = m;
            }
        }
        // `OPENCODE_PERMISSION` lets tools like opencode auto-approve everything
        // (e.g. `{"*":"allow"}`), matching the ecosystem convention.
        if let Ok(raw) = std::env::var("OPENCODE_PERMISSION") {
            if let Ok(cfg) = toml::from_str(&format!("permission = {raw}")) {
                self.permission = cfg;
            }
        }
        self
    }

    /// Decide whether a real provider is available.
    pub fn has_real_api(&self) -> bool {
        std::env::var("OPENSAUCE_API_KEY")
            .or_else(|_| std::env::var("OPENAI_API_KEY"))
            .map(|k| !k.is_empty())
            .unwrap_or(false)
    }
}

fn default_file_candidates() -> Vec<PathBuf> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let user = dirs::config_dir().map(|d| d.join("opensauce/opensauce.toml"));
    let mut v = vec![cwd.join("OpenSauce.toml")];
    if let Some(u) = user {
        v.push(u);
    }
    v
}

fn load_file() -> Result<Config> {
    for path in default_file_candidates() {
        if path.exists() {
            let raw = std::fs::read_to_string(&path)?;
            let cfg = toml::from_str(&raw).context(format!("bad config at {}", path.display()))?;
            return Ok(cfg);
        }
    }
    Ok(Config::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_loads() {
        let c = Config::load().unwrap();
        assert_eq!(c.provider, "auto");
    }
}