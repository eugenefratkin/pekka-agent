use crate::{agent::events::AgentEvent, error::AgentError};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

pub enum OrchestratorMessage {
    StartSession {
        reply: oneshot::Sender<Result<Uuid, AgentError>>,
    },
    Chat {
        session_id: Uuid,
        content: String,
        reply: oneshot::Sender<Result<String, AgentError>>,
    },
    /// Streaming: route to AgentMessage::StreamChat.
    StreamChat {
        session_id: Uuid,
        content: String,
        events: mpsc::Sender<AgentEvent>,
    },
    CancelSession { session_id: Uuid },
    RemoveSession  { session_id: Uuid },
}
