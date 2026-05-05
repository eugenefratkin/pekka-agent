use super::{events::AgentEvent, messages::AgentMessage, react_loop};
use crate::{
    config::AgentConfig,
    error::AgentError,
    llm::{ChatMessage, LlmClient},
    pekka::{Actor, ActorContext, BoxError, SupervisionDirective},
    tools::ToolRegistry,
};
use async_trait::async_trait;
use std::sync::Arc;
use uuid::Uuid;

pub struct AgentActor {
    pub session_id: Uuid,
    config: Arc<AgentConfig>,
    llm: Arc<dyn LlmClient>,
    tools: Arc<ToolRegistry>,
    history: Vec<ChatMessage>,
}

impl AgentActor {
    pub fn new(
        session_id: Uuid,
        config: Arc<AgentConfig>,
        llm: Arc<dyn LlmClient>,
        tools: Arc<ToolRegistry>,
    ) -> Self {
        Self { session_id, config, llm, tools, history: Vec::new() }
    }
}

#[async_trait]
impl Actor for AgentActor {
    type Message = AgentMessage;

    async fn on_start(&mut self, _ctx: &ActorContext) -> Result<(), BoxError> {
        if !self.config.system_prompt.trim().is_empty() {
            self.history.push(ChatMessage::system(&self.config.system_prompt));
        }
        tracing::info!(session_id = %self.session_id, "agent session started");
        Ok(())
    }

    async fn handle(&mut self, msg: AgentMessage, ctx: &ActorContext) -> Result<(), BoxError> {
        match msg {
            AgentMessage::Chat { content, reply } => {
                let result = react_loop::run(
                    self.session_id, &mut self.history, content,
                    &self.config, self.llm.clone(), self.tools.clone(),
                    ctx.child_token(), None,
                )
                .await
                .map(|r| r.answer);
                let _ = reply.send(result);
            }

            AgentMessage::StreamChat { content, events } => {
                let result = react_loop::run(
                    self.session_id, &mut self.history, content,
                    &self.config, self.llm.clone(), self.tools.clone(),
                    ctx.child_token(), Some(events.clone()),
                )
                .await;

                // Emit terminal error events; FinalAnswer/Cancelled already
                // emitted inside run().
                match result {
                    Ok(_) => {}
                    Err(AgentError::Cancelled) => {
                        let _ = events.try_send(AgentEvent::Cancelled);
                    }
                    Err(e) => {
                        let _ = events.try_send(AgentEvent::Error { message: e.to_string() });
                    }
                }
                // Dropping `events` closes the channel → SSE stream ends.
            }

            AgentMessage::Cancel => {
                ctx.cancellation.cancel();
                tracing::info!(session_id = %self.session_id, "session cancelled");
            }
        }
        Ok(())
    }

    async fn on_error(&mut self, err: BoxError, _ctx: &ActorContext) -> SupervisionDirective {
        tracing::error!(session_id = %self.session_id, error = %err, "agent error");
        SupervisionDirective::Resume
    }

    async fn on_stop(&mut self, _ctx: &ActorContext) {
        tracing::info!(session_id = %self.session_id, "agent session stopped");
    }
}
