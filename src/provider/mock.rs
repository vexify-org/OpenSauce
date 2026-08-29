//! A deterministic fake provider that exercises the whole agent pipeline
//! without network or API keys. Useful for demos, tests and CI.

use super::{Provider, ProviderEvent, ProviderStream, Request, Usage};
use crate::core::message::{Message, Role, ToolCall};
use anyhow::Result;

/// Deterministic mock: inspects the conversation and either requests a
/// tool call (first user turn) or produces a final text answer.
pub struct MockProvider {}

impl MockProvider {
    pub const MODEL: &'static str = "mock-sauce-0";

    pub fn new() -> Self {
        MockProvider {}
    }
}

impl Default for MockProvider {
    fn default() -> Self {
        MockProvider::new()
    }
}

#[async_trait::async_trait]
impl Provider for MockProvider {
    fn name(&self) -> &'static str {
        "mock"
    }
    fn default_model(&self) -> &'static str {
        Self::MODEL
    }
    async fn stream(&self, req: &Request) -> Result<ProviderStream> {
        let has_tool_result = req.messages.iter().any(|m| m.role == Role::Tool);

        if has_tool_result {
            // We already gathered context via a tool; stream a final answer.
            let tools = tool_results_summary(req);
            let answer = format!(
                "[mock] Here's what I did.\n{}\n(open a real model — set OPENSAUCE_API_KEY — for real answers)",
                tools
            );
            let (tx, rx) = tokio::sync::mpsc::channel(4);
            tokio::spawn(async move {
                for line in answer.split_inclusive('\n') {
                    let _ = tx.send(ProviderEvent::Text(line.to_string())).await;
                }
                let _ = tx.send(ProviderEvent::MessageDone {
                    message: Message::new(Role::Assistant),
                    usage: Usage { prompt_tokens: 10, completion_tokens: 20 },
                }).await;
            });
            let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
            return Ok(Box::pin(stream) as ProviderStream);
        }

        // Ask for a workspace_info tool call to prove the loop works.
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        tokio::spawn(async move {
            let _ = tx.send(ProviderEvent::Text("I'll inspect the workspace first.".to_string())).await;
            let tc = ToolCall {
                id: "call_mock_1".into(),
                name: "workspace_info".into(),
                arguments: serde_json::json!({}),
            };
            let _ = tx.send(ProviderEvent::ToolCallPrepared(tc)).await;
            let _ = tx.send(ProviderEvent::Text("Now I have context.".to_string())).await;
            let _ = tx.send(ProviderEvent::MessageDone { message: Message::new(Role::Assistant), usage: Usage::default() }).await;
        });
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Box::pin(stream) as ProviderStream)
    }
}

fn tool_results_summary(req: &Request) -> String {
    let mut out = Vec::new();
    for m in &req.messages {
        if let Some(tr) = &m.tool_result {
            out.push(format!(
                "  - [{}] {}: {}",
                if tr.success { "ok" } else { "err" },
                tr.name,
                first_line(&tr.content)
            ));
        }
    }
    if out.is_empty() {
        "  (no tool output)".to_string()
    } else {
        out.join("\n")
    }
}

fn first_line(s: &str) -> String {
    let t = s.trim();
    if t.is_empty() {
        return "".to_string();
    }
    if let Some(idx) = t.find('\n') {
        t[..idx].to_string()
    } else {
        t.to_string()
    }
}