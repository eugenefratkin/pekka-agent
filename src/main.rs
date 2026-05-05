//! Interactive CLI chatbot.
//!
//! Usage:
//!   OPENAI_API_KEY=sk-... cargo run --bin chat
//!   OPENAI_API_KEY=sk-... CONFIG_PATH=config/default.toml cargo run --bin chat
//!
//! Commands during the chat:
//!   /cancel  — cancel the current request
//!   /new     — start a new session
//!   /quit    — exit

use pekka_llm::{
    config::{AppConfig, ToolType},
    llm::openai::OpenAiClient,
    orchestrator::{OrchestratorActor, OrchestratorMessage},
    pekka,
    telemetry,
    tools::{
        builtin::{CalculatorTool, EchoTool},
        http_tool::HttpTool,
        ToolRegistry,
    },
};
use std::{io::{self, BufRead, Write}, sync::Arc};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── Telemetry ────────────────────────────────────────────────────────────
    let _telemetry_guard = telemetry::init("pekka-llm");

    // ── Config ───────────────────────────────────────────────────────────────
    let config_path = std::env::var("CONFIG_PATH")
        .unwrap_or_else(|_| "config/default.toml".into());

    let app_config = AppConfig::from_file(&config_path).unwrap_or_else(|e| {
        tracing::warn!(path = %config_path, error = %e, "failed to load config, using defaults");
        default_config()
    });

    // ── Tool registry ────────────────────────────────────────────────────────
    let tools = ToolRegistry::new();

    for tool_cfg in &app_config.tools {
        match tool_cfg.tool_type {
            ToolType::Builtin => match tool_cfg.name.as_str() {
                "calculator" => tools.register(CalculatorTool),
                "echo" => tools.register(EchoTool),
                other => tracing::warn!(name = other, "unknown builtin tool — skipping"),
            },
            ToolType::Http => {
                if let Some(endpoint) = &tool_cfg.endpoint {
                    tools.register(HttpTool::new(
                        &tool_cfg.name,
                        &tool_cfg.description,
                        endpoint,
                    ));
                } else {
                    tracing::warn!(name = %tool_cfg.name, "http tool missing endpoint — skipping");
                }
            }
        }
    }

    tracing::info!(tools = tools.len(), "tool registry ready");

    // ── LLM client ───────────────────────────────────────────────────────────
    let api_key = app_config.llm.api_key().unwrap_or_else(|_| {
        tracing::warn!("API key env var not set — LLM calls will fail");
        String::new()
    });
    let llm = OpenAiClient::new(&app_config.llm.base_url, api_key, &app_config.agent.model);

    // ── Orchestrator ─────────────────────────────────────────────────────────
    let root_cancel = CancellationToken::new();
    let orchestrator = OrchestratorActor::new(
        Arc::new(app_config.agent.clone()),
        Arc::new(llm),
        Arc::new(tools),
        root_cancel.clone(),
    );
    let (orch_ref, _orch_handle) = pekka::spawn(orchestrator, "orchestrator", 256, None);

    // ── Interactive chat loop ─────────────────────────────────────────────────
    println!("pekka-llm chat  (model: {})", app_config.agent.model);
    println!("Commands: /cancel  /new  /quit");
    println!("{}", "─".repeat(60));

    // Start an initial session.
    let mut session_id = start_session(&orch_ref).await?;
    println!("Session: {session_id}\n");

    let stdin = io::stdin();
    let mut current_cancel: Option<CancellationToken> = None;

    for line in stdin.lock().lines() {
        let input = line?.trim().to_string();
        if input.is_empty() {
            continue;
        }

        match input.as_str() {
            "/quit" | "/exit" => break,

            "/cancel" => {
                if let Some(token) = current_cancel.take() {
                    token.cancel();
                    println!("[cancelled]");
                } else {
                    println!("[nothing to cancel]");
                }
                continue;
            }

            "/new" => {
                session_id = start_session(&orch_ref).await?;
                println!("New session: {session_id}\n");
                continue;
            }

            _ => {}
        }

        // Send user message.
        let (tx, rx) = oneshot::channel();
        let turn_cancel = root_cancel.child_token();
        current_cancel = Some(turn_cancel.clone());

        orch_ref
            .tell(OrchestratorMessage::Chat {
                session_id,
                content: input,
                reply: tx,
            })
            .await?;

        print!("Assistant: ");
        io::stdout().flush()?;

        // Wait for response (or cancellation from /cancel command).
        let result = tokio::select! {
            res = rx => res.unwrap_or(Err(pekka_llm::error::AgentError::Cancelled)),
            _ = turn_cancel.cancelled() => Err(pekka_llm::error::AgentError::Cancelled),
        };
        current_cancel = None;

        match result {
            Ok(answer) => println!("{answer}\n"),
            Err(pekka_llm::error::AgentError::Cancelled) => println!("[request cancelled]\n"),
            Err(e) => println!("[error: {e}]\n"),
        }
    }

    println!("Shutting down…");
    root_cancel.cancel();
    Ok(())
}

// ── Helpers ──────────────────────────────────────────────────────────────────

async fn start_session(
    orch_ref: &pekka::ActorRef<OrchestratorMessage>,
) -> anyhow::Result<uuid::Uuid> {
    let (tx, rx) = oneshot::channel();
    orch_ref
        .tell(OrchestratorMessage::StartSession { reply: tx })
        .await
        .map_err(|e| anyhow::anyhow!("orchestrator gone: {e}"))?;
    let id = rx.await??;
    Ok(id)
}

fn default_config() -> AppConfig {
    AppConfig::from_str(
        r#"
[llm]
base_url    = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"
model       = "gpt-4o-mini"

[agent]
model          = "gpt-4o-mini"
max_iterations = 10
system_prompt  = "You are a helpful assistant."

[[tools]]
name        = "calculator"
type        = "builtin"
description = "Evaluates arithmetic expressions"

[[tools]]
name        = "echo"
type        = "builtin"
description = "Returns its input unchanged"
"#,
    )
    .expect("default config is valid")
}
