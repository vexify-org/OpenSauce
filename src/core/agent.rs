//! The agent — orchestrates the conversation ↔ provider ↔ tools loop.

use super::message::{Message, Role, ToolCall};
use super::session::Conversation;
use super::tools::registry::ToolRegistry;
use super::tools::{dispatch, ToolOutput};
use crate::provider::{Provider, ProviderEvent, ToolDef};
use anyhow::Result;
use futures_util::StreamExt;
use std::sync::Arc;

/// Events the agent emits while it works; the UI subscribes to these.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// A text delta of the assistant reply (as it streams).
    TextDelta(String),
    /// A tool call was requested by the model and will run.
    ToolCallQueued {
        name: String,
        arguments: serde_json::Value,
    },
    /// A tool call finished.
    ToolCallFinished {
        name: String,
        ok: bool,
    },
    /// The whole agent run finished.
    Done { usage_prompt: u64, usage_completion: u64 },
}

pub type EventSink = Box<dyn FnMut(AgentEvent) + Send>;

/// Configuration for a single agent run.
pub struct Agent {
    pub provider: Arc<dyn Provider>,
    pub tools: Arc<ToolRegistry>,
    pub model: String,
    pub max_turns: usize,
}

impl Agent {
    pub fn new(provider: Arc<dyn Provider>, tools: Arc<ToolRegistry>, model: String) -> Self {
        Agent {
            provider,
            tools,
            model,
            max_turns: 12,
        }
    }

    fn budget(&self) -> usize {
        self.max_turns
    }

    /// Run the loop on `conv`, emitting progress via `sink`, mutating `conv`
    /// as the transcript grows. `mode` is read from `conv`.
    pub async fn run(&self, conv: &mut Conversation, sink: &mut EventSink) -> Result<()> {
        let mode = conv.mode;
        let tool_defs: Vec<ToolDef> = self.tools.shared().into_iter().map(ToolDef::from).collect();
        let mut usage_prompt = 0u64;
        let mut usage_completion = 0u64;

        for turn in 0..self.budget() {
            let req = crate::provider::Request {
                model: if self.model.is_empty() {
                    self.provider.default_model().to_string()
                } else {
                    self.model.clone()
                },
                system: conv.system_prefix(),
                messages: conv.messages.clone(),
                tools: tool_defs.clone(),
            };

            let mut stream = match self.provider.stream(&req).await {
                Ok(s) => s,
                Err(e) => {
                    conv.push(Message::assistant(format!("⚠ {e}")));
                    sink(AgentEvent::ToolCallFinished {
                        name: "__error__".into(),
                        ok: false,
                    });
                    break;
                }
            };

            let mut reply = Message::new(Role::Assistant);
            let mut text = String::new();
            let mut has_tool_result_now = false;

            while let Some(ev) = stream.next().await {
                match ev {
                    ProviderEvent::Text(t) => {
                        text.push_str(&t);
                        sink(AgentEvent::TextDelta(t));
                    }
                    ProviderEvent::ToolCallPrepared(tc) => reply.tool_calls.push(tc),
                    ProviderEvent::MessageDone { usage, .. } => {
                        usage_prompt += usage.prompt_tokens;
                        usage_completion += usage.completion_tokens;
                    }
                    ProviderEvent::Error(e) => {
                        sink(AgentEvent::ToolCallFinished {
                            name: "__stream_error__".into(),
                            ok: false,
                        });
                        return Err(anyhow::anyhow!("{e}"));
                    }
                }
            }

            if !text.is_empty() {
                reply.content = Some(text);
            }
            conv.push(reply.clone());

            // Did the model ask for tools?
            if reply.tool_calls.is_empty() {
                break;
            }
            for tc in &reply.tool_calls {
                sink(AgentEvent::ToolCallQueued {
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                });
                let out = dispatch(&self.tools, tc, mode).await;
                sink(AgentEvent::ToolCallFinished {
                    name: tc.name.clone(),
                    ok: out.ok,
                });
                conv.push(mk_result(tc, out));
                has_tool_result_now = true;
            }
            let _ = has_tool_result_now;
            let _ = turn;
        }

        sink(AgentEvent::Done {
            usage_prompt,
            usage_completion,
        });
        Ok(())
    }
}

fn mk_result(tc: &ToolCall, out: ToolOutput) -> Message {
    Message::from_tool_result(super::message::ToolResult {
        call_id: tc.id.clone(),
        name: tc.name.clone(),
        success: out.ok,
        content: out.text,
    })
}

/// Convenience: run the agent headlessly and return the full assistant text.
pub async fn run_headless(agent: &Agent, conv: &mut Conversation) -> Result<String> {
    use std::sync::Mutex;
    let buffer = Arc::new(Mutex::new(String::new()));
    let buf = buffer.clone();
    let mut sink: EventSink = Box::new(move |event| {
        if let AgentEvent::TextDelta(t) = event {
            buf.lock().unwrap().push_str(&t);
        }
    });
    agent.run(conv, &mut sink).await?;
    let text = buffer.lock().unwrap().clone();
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::session::{default_system, Conversation};
    use crate::mode::Mode;
    use crate::provider::mock::MockProvider;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn mock_agent_completes_with_tool_loop() {
        let provider = Arc::new(MockProvider::new());
        let tools = Arc::new(ToolRegistry::with_defaults());
        let agent = Agent::new(provider, tools, "mock".into());
        let mut conv = Conversation::new("t1", "test", Mode::Build, default_system(Mode::Build));
        conv.push(Message::user("tell me about this workspace"));

        let events: Arc<Mutex<Vec<AgentEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let ev = events.clone();
        let mut sink: EventSink = Box::new(move |e| ev.lock().unwrap().push(e));
        agent.run(&mut conv, &mut sink).await.unwrap();

        // The mock should have requested a tool, then answered.
        let queued = events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, AgentEvent::ToolCallQueued { .. }))
            .count();
        assert!(queued >= 1, "mock should request tools");
        assert!(conv.turns().len() >= 3, "user + assistant(tools) + tool result + assistant text");
        assert!(conv.last_assistant_text().is_some());
    }
}