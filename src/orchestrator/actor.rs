//! `OrchestratorActor` manages the lifecycle of all chat sessions.
//!
//! It is the single entry point for the outside world:
//!   - `StartSession`  → spawn an `AgentActor`, return its `Uuid`.
//!   - `Chat`          → route a message to the right `AgentActor`.
//!   - `CancelSession` → cancel + remove the session.
//!   - `RemoveSession` → clean up a finished session.

use super::messages::OrchestratorMessage;
use crate::{
    agent::{actor::AgentActor, messages::AgentMessage},
    config::AgentConfig,
    error::AgentError,
    llm::LlmClient,
    pekka::{self, Actor, ActorContext, ActorHandle, ActorRef, BoxError, SupervisionDirective},
    tools::ToolRegistry,
};
use async_trait::async_trait;
use std::{collections::HashMap, sync::Arc};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

// Sessions are only accessed from the orchestrator's own actor task, so a
// plain HashMap is sufficient — no need for DashMap or Arc.
type Session = (ActorRef<AgentMessage>, ActorHandle);

pub struct OrchestratorActor {
    config: Arc<AgentConfig>,
    llm: Arc<dyn LlmClient>,
    tools: Arc<ToolRegistry>,
    sessions: HashMap<Uuid, Session>,
    /// Root token — cancelling it stops ALL sessions.
    root_cancel: CancellationToken,
}

impl OrchestratorActor {
    pub fn new(
        config: Arc<AgentConfig>,
        llm: Arc<dyn LlmClient>,
        tools: Arc<ToolRegistry>,
        root_cancel: CancellationToken,
    ) -> Self {
        Self {
            config,
            llm,
            tools,
            sessions: HashMap::new(),
            root_cancel,
        }
    }

    fn spawn_session(&self, session_id: Uuid) -> Session {
        let actor = AgentActor::new(
            session_id,
            self.config.clone(),
            self.llm.clone(),
            self.tools.clone(),
        );
        // Session token is a child of root — cancelling root stops everything.
        let session_token = self.root_cancel.child_token();
        pekka::spawn(
            actor,
            format!("agent-session-{}", &session_id.to_string()[..8]),
            64,
            Some(session_token),
        )
    }
}

#[async_trait]
impl Actor for OrchestratorActor {
    type Message = OrchestratorMessage;

    async fn handle(
        &mut self,
        msg: OrchestratorMessage,
        _ctx: &ActorContext,
    ) -> Result<(), BoxError> {
        match msg {
            OrchestratorMessage::StartSession { reply } => {
                let id = Uuid::new_v4();
                let session = self.spawn_session(id);
                self.sessions.insert(id, session);
                tracing::info!(session_id = %id, "session started");
                let _ = reply.send(Ok(id));
            }

            OrchestratorMessage::Chat {
                session_id,
                content,
                reply,
            } => {
                let actor_ref = self
                    .sessions
                    .get(&session_id)
                    .map(|(r, _)| r.clone());

                match actor_ref {
                    None => {
                        let _ = reply.send(Err(AgentError::SessionNotFound(
                            session_id.to_string(),
                        )));
                    }
                    Some(r) => {
                        if let Err(_) = r
                            .tell(AgentMessage::Chat { content, reply })
                            .await
                        {
                            // The actor died between the lookup and the send.
                            self.sessions.remove(&session_id);
                        }
                    }
                }
            }

            OrchestratorMessage::StreamChat { session_id, content, events } => {
                match self.sessions.get(&session_id).map(|(r, _)| r.clone()) {
                    None => {
                        let _ = events.try_send(
                            crate::agent::events::AgentEvent::Error {
                                message: format!("session not found: {session_id}"),
                            }
                        );
                    }
                    Some(r) => {
                        if r.tell(AgentMessage::StreamChat { content, events }).await.is_err() {
                            self.sessions.remove(&session_id);
                        }
                    }
                }
            }

            OrchestratorMessage::CancelSession { session_id } => {
                if let Some((actor_ref, handle)) = self.sessions.remove(&session_id) {
                    actor_ref.cancel();
                    handle.cancel();
                    tracing::info!(session_id = %session_id, "session cancelled");
                }
            }

            OrchestratorMessage::RemoveSession { session_id } => {
                self.sessions.remove(&session_id);
                tracing::debug!(session_id = %session_id, "session removed");
            }
        }
        Ok(())
    }

    async fn on_error(&mut self, err: BoxError, _ctx: &ActorContext) -> SupervisionDirective {
        tracing::error!(error = %err, "orchestrator error");
        SupervisionDirective::Resume
    }

    async fn on_stop(&mut self, _ctx: &ActorContext) {
        // Cancel all sessions on shutdown.
        let ids: Vec<Uuid> = self.sessions.keys().copied().collect();
        for id in ids {
            if let Some((actor_ref, handle)) = self.sessions.remove(&id) {
                actor_ref.cancel();
                handle.cancel();
            }
        }
        tracing::info!("orchestrator stopped — all sessions cancelled");
    }
}
