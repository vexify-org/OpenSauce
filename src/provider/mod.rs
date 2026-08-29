//! LLM provider layer.
//!
//! [`Provider`] is the seam between the agent and a model backend. Two
//! implementations ship: [`openai::OpenAIClient`] (OpenAI-compatible chat
//! completions, streamed) and [`mock::MockProvider`] so the whole pipeline
//! runs without network or API keys.

pub mod mock;
pub mod openai;

use crate::core::message::Message;
use crate::core::tools::registry::SharedTool;
use anyhow::Result;
use std::pin::Pin;
use std::task::{Context, Poll};
use futures_util::{Stream, StreamExt};

/// A light tool descriptor handed to the model.
#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub schema: serde_json::Value,
}

impl From<SharedTool> for ToolDef {
    fn from(t: SharedTool) -> Self {
        ToolDef {
            name: t.name().to_string(),
            description: t.description().to_string(),
            schema: t.schema(),
        }
    }
}

/// Token usage, when the backend reports it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

impl Usage {
    pub fn total(&self) -> u64 {
        self.prompt_tokens + self.completion_tokens
    }
}

/// A request to the model.
#[derive(Debug, Clone)]
pub struct Request {
    pub model: String,
    pub system: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDef>,
}

/// Events emitted while streaming a model response.
#[derive(Debug, Clone)]
pub enum ProviderEvent {
    /// Incremental text delta.
    Text(String),
    /// A completed tool call is ready to be executed.
    ToolCallPrepared(crate::core::message::ToolCall),
    /// The assistant turn finished (may still carry tool_calls).
    MessageDone { message: Message, usage: Usage },
    /// Streamable error.
    Error(String),
}

/// A streaming provider.
#[async_trait::async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &'static str;
    fn default_model(&self) -> &'static str;
    /// Stream a single model turn. The returned stream must terminate.
    async fn stream(&self, req: &Request) -> Result<ProviderStream>;
}

pub type ProviderStream = Pin<Box<dyn Stream<Item = ProviderEvent> + Send>>;

/// Collect a provider stream into a fully-materialized [`Message`] with usage.
pub async fn collect(req_stream: &mut ProviderStream) -> (Message, Usage) {
    let mut text = String::new();
    let mut tool_calls: Vec<crate::core::message::ToolCall> = Vec::new();
    let mut usage = Usage::default();
    while let Some(ev) = req_stream.next().await {
        match ev {
            ProviderEvent::Text(t) => text.push_str(&t),
            ProviderEvent::ToolCallPrepared(tc) => tool_calls.push(tc),
            ProviderEvent::MessageDone { usage: u, .. } => usage = u,
            ProviderEvent::Error(_) => {}
        }
    }
    let content = if text.is_empty() { None } else { Some(text) };
    let mut msg = Message::new(crate::core::message::Role::Assistant);
    msg.content = content;
    msg.tool_calls = tool_calls;
    (msg, usage)
}

/// A stream that always yields a single item then finishes (for tests).
pub struct OnceStream(pub Option<ProviderEvent>);

impl Stream for OnceStream {
    type Item = ProviderEvent;
    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.0.take())
    }
}