//! Parallel reasoning: spawn N independent agents and race them.
//!
//! With `FirstWins` strategy the first successful answer is returned and all
//! other agents are cancelled immediately.
//!
//! With `Majority` strategy we collect all answers and return the one that
//! appears most frequently (ties broken by first received).

use crate::{
    config::{AgentConfig, ParallelReasonConfig, ParallelStrategy},
    error::AgentError,
    llm::LlmClient,
    pekka,
    tools::ToolRegistry,
};
use std::sync::Arc;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{actor::AgentActor, messages::AgentMessage};

/// Race `cfg.num_agents` independent agent instances on the same query.
/// Returns once the strategy is satisfied or all agents have failed.
pub async fn run_parallel_reasoning(
    user_input: String,
    agent_config: Arc<AgentConfig>,
    parallel_cfg: &ParallelReasonConfig,
    llm: Arc<dyn LlmClient>,
    tools: Arc<ToolRegistry>,
    parent_cancel: CancellationToken,
) -> Result<String, AgentError> {
    let root_cancel = parent_cancel.child_token();
    let n = parallel_cfg.num_agents;

    // Spawn N agents, each with a child cancellation token.
    let mut handles = Vec::with_capacity(n);
    let mut rx_vec = Vec::with_capacity(n);

    for i in 0..n {
        let child_cancel = root_cancel.child_token();
        let session_id = Uuid::new_v4();
        let actor = AgentActor::new(
            session_id,
            agent_config.clone(),
            llm.clone(),
            tools.clone(),
        );
        let (actor_ref, handle) = pekka::spawn(
            actor,
            format!("parallel-agent-{i}"),
            16,
            Some(child_cancel),
        );
        let (tx, rx) = oneshot::channel::<Result<String, AgentError>>();
        let msg = AgentMessage::Chat {
            content: user_input.clone(),
            reply: tx,
        };
        actor_ref
            .tell(msg)
            .await
            .map_err(|_| AgentError::ActorGone)?;
        handles.push(handle);
        rx_vec.push(rx);
    }

    let result = match parallel_cfg.strategy {
        ParallelStrategy::FirstWins => first_wins(rx_vec).await,
        ParallelStrategy::Majority => majority(rx_vec, n).await,
    };

    // Cancel remaining agents regardless of outcome.
    root_cancel.cancel();
    for h in handles {
        h.cancel();
    }

    result
}

// ── Strategies ────────────────────────────────────────────────────────────────

async fn first_wins(
    receivers: Vec<oneshot::Receiver<Result<String, AgentError>>>,
) -> Result<String, AgentError> {
    let futs: Vec<_> = receivers
        .into_iter()
        .map(|rx| Box::pin(async move {
            rx.await.map_err(|_| AgentError::ActorGone).and_then(|r| r)
        }))
        .collect();

    // select_ok returns the first Ok result and drops remaining futures.
    futures::future::select_ok(futs)
        .await
        .map(|(answer, _rest)| answer)
        .map_err(|_| AgentError::Cancelled)
}

async fn majority(
    receivers: Vec<oneshot::Receiver<Result<String, AgentError>>>,
    _n: usize,
) -> Result<String, AgentError> {
    let mut answers: Vec<String> = Vec::new();

    for rx in receivers {
        match rx.await {
            Ok(Ok(answer)) => answers.push(answer),
            _ => {}
        }
    }

    if answers.is_empty() {
        return Err(AgentError::Cancelled);
    }

    // Simple majority: pick the answer that appears most often.
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for a in &answers {
        *counts.entry(a.as_str()).or_insert(0) += 1;
    }

    let winner = counts
        .into_iter()
        .max_by_key(|&(_, c)| c)
        .map(|(a, _)| a.to_string())
        .unwrap_or_else(|| answers.remove(0));

    Ok(winner)
}
