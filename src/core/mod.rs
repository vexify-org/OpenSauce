//! Core domain models shared by the agent, provider and UI.

pub mod agent;
pub mod message;
pub mod session;
pub mod tools;

pub mod prelude {
    pub use super::message::{Message, Role, ToolCall, ToolResult};
    pub use super::session::{Conversation, SessionStore};
}