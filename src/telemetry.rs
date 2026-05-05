//! OpenTelemetry initialisation and span-name / attribute constants.
//!
//! Span hierarchy:
//! ```text
//! react.session
//!   └── react.iteration
//!         ├── react.think
//!         ├── react.act
//!         │     ├── tool.call
//!         │     └── tool.call
//!         └── react.observe
//! ```
//!
//! For production use, enable the `otlp` Cargo feature and set
//! `OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4317`.

use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::trace::TracerProvider;
use tokio::sync::broadcast;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

// ── Span names ───────────────────────────────────────────────────────────────

pub mod spans {
    pub const SESSION: &str = "react.session";
    pub const ITERATION: &str = "react.iteration";
    pub const THINK: &str = "react.think";
    pub const ACT: &str = "react.act";
    pub const OBSERVE: &str = "react.observe";
    pub const TOOL_CALL: &str = "tool.call";
}

// ── Attribute keys (mirrors OTel GenAI semantic conventions) ─────────────────

pub mod attrs {
    // OpenTelemetry GenAI semantic conventions
    pub const GEN_AI_SYSTEM: &str = "gen_ai.system";
    pub const GEN_AI_REQUEST_MODEL: &str = "gen_ai.request.model";
    pub const GEN_AI_RESPONSE_MODEL: &str = "gen_ai.response.model";
    pub const GEN_AI_OPERATION_NAME: &str = "gen_ai.operation.name";
    pub const GEN_AI_USAGE_INPUT_TOKENS: &str = "gen_ai.usage.input_tokens";
    pub const GEN_AI_USAGE_OUTPUT_TOKENS: &str = "gen_ai.usage.output_tokens";

    // ReAct-specific attributes
    pub const REACT_SESSION_ID: &str = "react.session_id";
    pub const REACT_ITERATION: &str = "react.iteration";
    pub const REACT_FINISH_REASON: &str = "react.finish_reason";
    pub const REACT_HAS_TOOL_CALLS: &str = "react.has_tool_calls";
    pub const REACT_TOOL_CALL_COUNT: &str = "react.tool_call_count";
    pub const REACT_NUM_RESULTS: &str = "react.num_results";

    // Tool-specific attributes
    pub const TOOL_NAME: &str = "tool.name";
    pub const TOOL_CALL_ID: &str = "tool.call_id";
    pub const TOOL_SUCCESS: &str = "tool.success";
    pub const TOOL_ERROR: &str = "tool.error";
}

// ── TelemetryGuard — flushes all pending spans on drop ───────────────────────

pub struct TelemetryGuard;

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        opentelemetry::global::shutdown_tracer_provider();
    }
}

// ── Initialisation ────────────────────────────────────────────────────────────

/// Initialise tracing + OpenTelemetry with a stdout exporter.
///
/// Keep the returned `TelemetryGuard` alive for the program's lifetime.
pub fn init(service_name: &str) -> TelemetryGuard {
    build(service_name, None)
}

/// Initialise tracing + OpenTelemetry with **both** a stdout exporter and an
/// SSE broadcast exporter.  Spans are forwarded to the broadcast channel so
/// the web UI can display them in real time.
pub fn init_with_sse(service_name: &str, tx: broadcast::Sender<String>) -> TelemetryGuard {
    build(service_name, Some(tx))
}

fn build(service_name: &str, sse_tx: Option<broadcast::Sender<String>>) -> TelemetryGuard {
    use crate::web::exporter::SseBroadcastExporter;

    let stdout  = opentelemetry_stdout::SpanExporter::default();
    let mut builder = TracerProvider::builder().with_simple_exporter(stdout);

    if let Some(tx) = sse_tx {
        builder = builder.with_simple_exporter(SseBroadcastExporter::new(tx));
    }

    let provider = builder.build();
    opentelemetry::global::set_tracer_provider(provider.clone());
    let tracer = provider.tracer(service_name.to_string());

    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
    let fmt_layer = tracing_subscriber::fmt::layer().with_target(false);
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .with(otel_layer)
        .init();

    TelemetryGuard
}
