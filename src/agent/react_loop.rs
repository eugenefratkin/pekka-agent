use crate::{
    agent::events::AgentEvent,
    config::AgentConfig,
    error::{AgentError, ToolError},
    llm::{ChatMessage, LlmClient, ToolCallRequest},
    tools::ToolRegistry,
};
use std::{future::Future, pin::Pin, sync::Arc};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{field, info_span, Instrument as _};
use uuid::Uuid;

pub struct ReactResult {
    pub answer: String,
    pub iterations: usize,
}

// Fire-and-forget event emit; does nothing when `tx` is None.
macro_rules! emit {
    ($tx:expr, $ev:expr) => {
        if let Some(tx) = &$tx {
            let _ = tx.try_send($ev);
        }
    };
}

pub async fn run(
    session_id: Uuid,
    history: &mut Vec<ChatMessage>,
    user_input: String,
    config: &AgentConfig,
    llm: Arc<dyn LlmClient>,
    tools: Arc<ToolRegistry>,
    cancel: CancellationToken,
    events: Option<mpsc::Sender<AgentEvent>>,
) -> Result<ReactResult, AgentError> {
    history.push(ChatMessage::user(user_input));

    let tool_defs = if tools.is_empty() { None } else { Some(tools.all_definitions()) };

    let _session_span = info_span!(
        "react.session",
        "react.session_id" = %session_id,
        "gen_ai.system" = "openai",
        "gen_ai.request.model" = %config.model,
    )
    .entered();

    for iteration in 0..config.max_iterations {
        if cancel.is_cancelled() {
            emit!(events, AgentEvent::Cancelled);
            return Err(AgentError::Cancelled);
        }

        let _iter_span = info_span!(
            "react.iteration",
            "react.session_id" = %session_id,
            "react.iteration" = iteration,
        )
        .entered();

        emit!(events, AgentEvent::IterationStart { iteration });

        // ── THINK ────────────────────────────────────────────────────────────
        emit!(events, AgentEvent::ThinkStart { iteration });

        let llm_response = {
            let _span = info_span!(
                "react.think",
                "gen_ai.operation.name" = "chat",
                "gen_ai.request.model" = %config.model,
                "react.iteration" = iteration,
            )
            .entered();

            tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    emit!(events, AgentEvent::Cancelled);
                    return Err(AgentError::Cancelled);
                }
                res = llm.complete(history, tool_defs.as_deref()) => res?,
            }
        };

        emit!(events, AgentEvent::ThinkDone {
            iteration,
            partial_text: if llm_response.has_tool_calls() {
                llm_response.content.clone()
            } else {
                None  // full text arrives in FinalAnswer
            },
        });

        // ── Final answer ──────────────────────────────────────────────────────
        if !llm_response.has_tool_calls() {
            let answer = llm_response.content.clone().unwrap_or_else(|| "<empty>".into());
            history.push(ChatMessage::assistant(answer.clone()));

            emit!(events, AgentEvent::FinalAnswer {
                content: answer.clone(),
                iterations: iteration + 1,
            });

            tracing::info!(session_id = %session_id, iteration, "react loop complete");
            return Ok(ReactResult { answer, iterations: iteration + 1 });
        }

        // ── ACT ───────────────────────────────────────────────────────────────
        history.push(ChatMessage::assistant_tool_calls(llm_response.tool_calls.clone()));

        let tool_calls = llm_response.tool_calls.clone();
        emit!(events, AgentEvent::ActStart { iteration, num_tools: tool_calls.len() });

        let tool_results = {
            let _span = info_span!(
                "react.act",
                "react.iteration" = iteration,
                "react.tool_call_count" = tool_calls.len(),
            )
            .entered();

            execute_tools_parallel(tool_calls, &tools, &cancel, events.clone()).await?
        };

        // ── OBSERVE ───────────────────────────────────────────────────────────
        {
            let _span = info_span!(
                "react.observe",
                "react.iteration" = iteration,
                "react.num_results" = tool_results.len(),
            )
            .entered();

            for r in &tool_results {
                history.push(r.to_chat_message());
            }
        }

        emit!(events, AgentEvent::ObserveDone { iteration });
    }

    Err(AgentError::MaxIterationsReached(config.max_iterations))
}

// ── Tool execution ─────────────────────────────────────────────────────────────

struct ToolResult { call_id: String, tool_name: String, output: String }

impl ToolResult {
    fn to_chat_message(&self) -> ChatMessage {
        ChatMessage::tool_result(&self.call_id, &self.tool_name, &self.output)
    }
}

type ToolFut = Pin<Box<dyn Future<Output = Result<ToolResult, AgentError>> + Send>>;

async fn execute_tools_parallel(
    calls: Vec<ToolCallRequest>,
    registry: &ToolRegistry,
    cancel: &CancellationToken,
    events: Option<mpsc::Sender<AgentEvent>>,
) -> Result<Vec<ToolResult>, AgentError> {
    let futs: Vec<ToolFut> = calls.into_iter().map(|call| -> ToolFut {
        let tool = match registry.get(&call.function.name) {
            Some(t) => t,
            None => {
                let name = call.function.name.clone();
                return Box::pin(async move {
                    Err(AgentError::Tool(ToolError::NotFound(name)))
                });
            }
        };

        let child_cancel = cancel.child_token();
        let call_id   = call.id.clone();
        let tool_name = call.function.name.clone();
        let ev        = events.clone();

        Box::pin(async move {
            let params = call.parsed_args().unwrap_or(serde_json::Value::Null);

            emit!(ev, AgentEvent::ToolCallStart {
                call_id: call_id.clone(),
                name: tool_name.clone(),
                args: params.clone(),
            });

            let span = info_span!(
                "tool.call",
                "tool.name"    = %tool_name,
                "tool.call_id" = %call_id,
                "tool.success" = field::Empty,
                "tool.error"   = field::Empty,
            );

            let result = tool.execute(params, child_cancel).instrument(span.clone()).await;

            match result {
                Ok(output) => {
                    span.record("tool.success", true);
                    emit!(ev, AgentEvent::ToolCallDone {
                        call_id: call_id.clone(), name: tool_name.clone(),
                        result: output.clone(), success: true,
                    });
                    Ok(ToolResult { call_id, tool_name, output })
                }
                Err(ToolError::Cancelled) => Err(AgentError::Cancelled),
                Err(e) => {
                    span.record("tool.success", false);
                    span.record("tool.error", e.to_string().as_str());
                    emit!(ev, AgentEvent::ToolCallDone {
                        call_id: call_id.clone(), name: tool_name.clone(),
                        result: e.to_string(), success: false,
                    });
                    Ok(ToolResult { call_id, tool_name, output: format!("Error: {e}") })
                }
            }
        })
    }).collect();

    futures::future::try_join_all(futs).await
}
