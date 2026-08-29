//! OpenAI-compatible provider via the chat-completions API with streaming.

use super::{Provider, ProviderEvent, ProviderStream, Request, ToolDef, Usage};
use crate::core::message::{Message, Role, ToolCall};
use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use serde::Deserialize;

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("http client")
}

/// An OpenAI-compatible streaming client.
pub struct OpenAIClient {
    base_url: String,
    api_key: Option<String>,
    model: String,
    http: Option<reqwest::Client>,
}

impl OpenAIClient {
    pub fn from_env() -> Option<Self> {
        let model = std::env::var("OPENSAUCE_MODEL").ok();
        let base = std::env::var("OPENSAUCE_BASE_URL")
            .ok()
            .or_else(|| std::env::var("OPENAI_BASE_URL").ok())
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
        let key = std::env::var("OPENSAUCE_API_KEY")
            .ok()
            .or_else(|| std::env::var("OPENAI_API_KEY").ok());
        let model = model.unwrap_or_else(|| "gpt-4o-mini".to_string());
        Some(OpenAIClient {
            base_url: base.trim_end_matches('/').to_string(),
            api_key: key,
            model,
            http: Some(client()),
        })
    }

    /// Build the client from the saved connection profile (created by
    /// `opensauce connect`), falling back to environment variables. Returns
    /// `None` when no API key is configured anywhere.
    pub fn from_config(config: &crate::config::Config) -> Option<Self> {
        let conn = crate::connect::load();
        let base = conn
            .as_ref()
            .map(|c| c.base_url.clone())
            .filter(|b| !b.trim().is_empty())
            .or_else(|| std::env::var("OPENSAUCE_BASE_URL").ok())
            .or_else(|| std::env::var("OPENAI_BASE_URL").ok())
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
        let key = conn
            .as_ref()
            .map(|c| c.api_key.clone())
            .filter(|k| !k.trim().is_empty())
            .or_else(|| std::env::var("OPENSAUCE_API_KEY").ok())
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .filter(|k| !k.trim().is_empty())?;
        let model = if !config.model.is_empty() {
            config.model.clone()
        } else {
            conn.as_ref()
                .map(|c| c.model.clone())
                .filter(|m| !m.trim().is_empty())
                .or_else(|| std::env::var("OPENSAUCE_MODEL").ok())
                .unwrap_or_else(|| "gpt-4o-mini".to_string())
        };
        Some(OpenAIClient {
            base_url: base.trim_end_matches('/').to_string(),
            api_key: Some(key),
            model,
            http: Some(client()),
        })
    }

    /// The resolved model name (config → connection → env → default).
    pub fn resolved_model(&self) -> &str {
        &self.model
    }

    pub fn with_http(mut self, http: reqwest::Client) -> Self {
        self.http = Some(http);
        self
    }

    fn ensure_creds(&self) -> Result<(String, &reqwest::Client)> {
        let key = self.api_key.as_deref().context("OPENSAUCE_API_KEY or OPENAI_API_KEY is not set")?;
        let http = self.http.as_ref().expect("http client");
        Ok((key.to_string(), http))
    }
}

#[async_trait::async_trait]
impl Provider for OpenAIClient {
    fn name(&self) -> &'static str {
        "openai-compatible"
    }
    fn default_model(&self) -> &'static str {
        "gpt-4o-mini"
    }
    async fn stream(&self, req: &Request) -> Result<ProviderStream> {
        let (key, http) = self.ensure_creds()?;
        // The model-specific model is a per-request field; env override already
        // set `self.model`, but the agent may choose a different one.
        let model = if req.model.is_empty() {
            self.model.clone()
        } else {
            req.model.clone()
        };

        let wire = build_wire_messages(&req.system, &req.messages);
        let payload = serde_json::json!({
            "model": model,
            "messages": wire,
            "stream": true,
            "tools": build_tools(&req.tools),
        });

        let url = format!("{}/chat/completions", self.base_url);
        let resp = http
            .post(&url)
            .bearer_auth(&key)
            .json(&payload)
            .send()
            .await
            .with_context(|| format!("request to {url} failed"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("model returned {status}: {body}");
        }

        let mut bytes = resp.bytes_stream();
        let mut buf = Vec::new();
        // Stream the SSE body line by line.
        let (tx, rx) = tokio::sync::mpsc::channel::<ProviderEvent>(64);

        tokio::spawn(async move {
            while let Some(chunk) = bytes.next().await {
                match chunk {
                    Ok(b) => buf.extend_from_slice(&b),
                    Err(e) => {
                        let _ = tx.send(ProviderEvent::Error(e.to_string())).await;
                        return;
                    }
                }
                // process complete lines
                while let Some(pos) = buf.iter().position(|&c| c == b'\n') {
                    let line = buf.drain(..=pos).collect::<Vec<_>>();
                    let line = String::from_utf8_lossy(&line);
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    match parse_sse_line(line) {
                        Some((delta, tool_delta, usage, done)) => {
                            if let Some(t) = delta {
                                let _ = tx.send(ProviderEvent::Text(t)).await;
                            }
                            if let Some(tc) = tool_delta {
                                let _ = tx.send(ProviderEvent::ToolCallPrepared(tc)).await;
                            }
                            if let Some(u) = usage {
                                let _ = tx.send(
                                    ProviderEvent::MessageDone {
                                        message: Message::new(Role::Assistant),
                                        usage: u,
                                    }
                                ).await;
                            }
                            if done {
                                return;
                            }
                        }
                        None => {}
                    }
                }
            }
        });

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Box::pin(stream) as ProviderStream)
    }
}

/// Build `messages` for chat-completions from our core messages + system.
fn build_wire_messages(system: &str, msgs: &[Message]) -> Vec<serde_json::Value> {
    let mut out: Vec<serde_json::Value> = Vec::new();
    if !system.is_empty() {
        out.push(serde_json::json!({"role": "system", "content": system}));
    }
    for m in msgs {
        match m.role {
            Role::System => out.push(serde_json::json!({"role": "system", "content": m.display_text()})),
            Role::User => {
                let mut v = serde_json::json!({"role": "user"});
                if let Some(c) = &m.content {
                    v["content"] = serde_json::Value::String(c.clone());
                } else {
                    v["content"] = serde_json::json!(m.display_text());
                }
                out.push(v);
            }
            Role::Assistant => {
                let mut v = serde_json::Map::new();
                v.insert("role".into(), "assistant".into());
                if let Some(c) = &m.content {
                    v.insert("content".into(), serde_json::Value::String(c.clone()));
                } else {
                    v.insert("content".into(), serde_json::Value::Null);
                }
                if !m.tool_calls.is_empty() {
                    let calls: Vec<serde_json::Value> = m
                        .tool_calls
                        .iter()
                        .map(|tc| {
                            serde_json::json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {"name": tc.name, "arguments": tc.arguments.to_string()}
                            })
                        })
                        .collect();
                    v.insert("tool_calls".into(), serde_json::Value::Array(calls));
                }
                out.push(serde_json::Value::Object(v));
            }
            Role::Tool => {
                if let Some(tr) = &m.tool_result {
                    out.push(serde_json::json!({
                        "role": "tool",
                        "tool_call_id": tr.call_id,
                        "content": tr.content,
                    }));
                }
            }
        }
    }
    out
}

#[derive(Deserialize)]
struct SseChunk {
    choices: Option<Vec<Choice>>,
    usage: Option<UsageRaw>,
}
#[derive(Deserialize)]
struct Choice {
    delta: Option<Delta>,
    finish_reason: Option<String>,
}
#[derive(Deserialize, Default)]
struct Delta {
    content: Option<String>,
    tool_calls: Option<Vec<DeltaToolCall>>,
    role: Option<String>,
}
#[derive(Deserialize)]
struct DeltaToolCall {
    #[allow(dead_code)] // present in the wire format; identity is per-delta here
    index: Option<usize>,
    id: Option<String>,
    function: Option<DeltaFunction>,
}
#[derive(Deserialize)]
struct DeltaFunction {
    name: Option<String>,
    arguments: Option<String>,
}
#[derive(Deserialize, Default)]
struct UsageRaw {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
}

/// Parse one `data: {...}` SSE line into events.
fn parse_sse_line(line: &str) -> Option<(Option<String>, Option<ToolCall>, Option<Usage>, bool)> {
    let data = line.strip_prefix("data:")?.trim();
    if data.is_empty() || data == "[DONE]" {
        return Some((None, None, None, data == "[DONE]"));
    }
    let chunk: SseChunk = serde_json::from_str(data).ok()?;
    let mut text: Option<String> = None;
    let mut tool: Option<ToolCall> = None;
    let mut done = false;
    if let Some(usage) = chunk.usage {
        let u = Usage {
            prompt_tokens: usage.prompt_tokens.unwrap_or(0),
            completion_tokens: usage.completion_tokens.unwrap_or(0),
        };
        return Some((None, None, Some(u), false));
    }
    if let Some(choices) = chunk.choices {
        for ch in choices {
            if ch.finish_reason.is_some() {
                done = true;
            }
            if let Some(d) = ch.delta {
                if let Some(c) = d.content {
                    let _ = d.role;
                    text = Some(c);
                }
                let tc = d.tool_calls.and_then(|vc| vc.into_iter().next()).and_then(|t| {
                    let name = t.function.as_ref().and_then(|f| f.name.clone())?;
                    let args = t
                        .function
                        .as_ref()
                        .and_then(|f| f.arguments.clone())
                        .unwrap_or_default();
                    let args_v = serde_json::from_str(&args).unwrap_or(serde_json::json!({}));
                    Some(ToolCall {
                        id: t.id.clone().unwrap_or_else(|| format!("call_{}", rand_suffix())),
                        name,
                        arguments: args_v,
                    })
                });
                if tc.is_some() {
                    tool = tc;
                }
            }
        }
    }
    // Tool-call deltas normally arrive fragmented; for simplicity this transport
    // emits a completed call only when arguments parse as JSON. See agent.rs for
    // the buffering path.
    Some((text, tool, None, done))
}

use std::sync::atomic::{AtomicU64, Ordering};
static COUNTER: AtomicU64 = AtomicU64::new(0);
fn rand_suffix() -> String {
    COUNTER.fetch_add(1, Ordering::Relaxed).to_string()
}

fn build_tools(tools: &[ToolDef]) -> Vec<serde_json::Value> {
    tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "type": "function",
                "function": {"name": t.name, "description": t.description, "parameters": t.schema}
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_messages_shapes() {
        let msgs = vec![
            Message::user("hi"),
            Message::assistant("yo"),
            Message::from_tool_result(crate::core::message::ToolResult {
                call_id: "c1".into(),
                name: "echo".into(),
                success: true,
                content: "out".into(),
            }),
        ];
        let wire = build_wire_messages("sys", &msgs);
        assert_eq!(wire.len(), 4);
        assert_eq!(wire[0]["role"], "system");
        assert_eq!(wire[3]["role"], "tool");
        assert_eq!(wire[3]["tool_call_id"], "c1");
    }

    #[test]
    fn parses_done() {
        let (t, tc, u, done) = parse_sse_line("data: [DONE]").unwrap();
        assert!(done && t.is_none() && tc.is_none() && u.is_none());
    }
}