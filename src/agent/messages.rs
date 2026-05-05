use super::events::AgentEvent;
use crate::error::AgentError;
use tokio::sync::{mpsc, oneshot};

pub enum AgentMessage {
    /// Non-streaming: wait for final answer.
    Chat {
        content: String,
        reply: oneshot::Sender<Result<String, AgentError>>,
    },
    /// Streaming: events pushed on `events`; sender drop = EOF.
    StreamChat {
        content: String,
        events: mpsc::Sender<AgentEvent>,
    },
    Cancel,
}
