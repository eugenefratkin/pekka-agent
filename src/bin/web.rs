//! Web demo: chat UI + live OpenTelemetry trace viewer.
//!
//! Usage:
//!   OPENAI_API_KEY=sk-... cargo run --bin web
//!   Then open http://localhost:3000

use pekka_llm::{
    config::{AppConfig, ToolType},
    llm::openai::OpenAiClient,
    orchestrator::OrchestratorActor,
    pekka,
    telemetry,
    tools::{
        builtin::{CalculatorTool, EchoTool},
        http_tool::HttpTool,
        ToolRegistry,
    },
    web::{AppState, router},
};
use std::sync::Arc;
use tokio::{net::TcpListener, sync::broadcast};
use tokio_util::sync::CancellationToken;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── SSE broadcast channel for span events ────────────────────────────────
    let (span_tx, _) = broadcast::channel::<String>(2048);

    // ── Telemetry: stdout + SSE ───────────────────────────────────────────────
    let _telemetry = telemetry::init_with_sse("pekka-llm", span_tx.clone());

    // ── Config ────────────────────────────────────────────────────────────────
    let config_path = std::env::var("CONFIG_PATH")
        .unwrap_or_else(|_| "config/default.toml".to_string());
    let app_config = AppConfig::from_file(&config_path).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "using default config");
        default_config()
    });

    // ── Tools ─────────────────────────────────────────────────────────────────
    let tools = ToolRegistry::new();
    for tool_cfg in &app_config.tools {
        match tool_cfg.tool_type {
            ToolType::Builtin => match tool_cfg.name.as_str() {
                "calculator" => tools.register(CalculatorTool),
                "echo"       => tools.register(EchoTool),
                n => tracing::warn!(name = n, "unknown builtin — skipping"),
            },
            ToolType::Http => {
                if let Some(ep) = &tool_cfg.endpoint {
                    tools.register(HttpTool::new(&tool_cfg.name, &tool_cfg.description, ep));
                }
            }
        }
    }

    // ── LLM client ────────────────────────────────────────────────────────────
    let api_key = app_config.llm.api_key().unwrap_or_default();
    let llm = OpenAiClient::new(&app_config.llm.base_url, api_key, &app_config.agent.model);

    // ── Orchestrator actor ────────────────────────────────────────────────────
    let root_cancel = CancellationToken::new();
    let orchestrator = OrchestratorActor::new(
        Arc::new(app_config.agent.clone()),
        Arc::new(llm),
        Arc::new(tools),
        root_cancel.clone(),
    );
    let (orch_ref, _orch_handle) = pekka::spawn(orchestrator, "orchestrator", 256, None);

    // ── Axum server ───────────────────────────────────────────────────────────
    let state = Arc::new(AppState {
        orch: orch_ref,
        span_tx,
        root_cancel: root_cancel.clone(),
    });

    let app = router(state);
    let addr = std::env::var("LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".into());
    let listener = TcpListener::bind(&addr).await?;
    tracing::info!(addr = %addr, "web server started — open http://localhost:3000");

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::signal::ctrl_c().await.ok();
            tracing::info!("shutting down");
            root_cancel.cancel();
        })
        .await?;

    Ok(())
}

fn default_config() -> AppConfig {
    AppConfig::from_str(r#"
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
"#).expect("valid default config")
}
