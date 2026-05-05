//! A custom OpenTelemetry span exporter that broadcasts finished spans as
//! JSON strings over a Tokio broadcast channel.  The web server subscribes
//! to this channel and forwards events to browsers via Server-Sent Events.

use futures::future::BoxFuture;
use opentelemetry_sdk::export::trace::{ExportResult, SpanData, SpanExporter};
use serde::Serialize;
use std::{collections::HashMap, fmt, time::UNIX_EPOCH};
use tokio::sync::broadcast;

// ── Wire format sent to the browser ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct SpanEvent {
    pub trace_id: String,
    pub span_id: String,
    /// Empty string for root spans.
    pub parent_span_id: String,
    pub name: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub duration_ms: u64,
    pub status: String,
    pub attributes: HashMap<String, serde_json::Value>,
}

// ── Exporter ──────────────────────────────────────────────────────────────────

pub struct SseBroadcastExporter {
    tx: broadcast::Sender<String>,
}

impl fmt::Debug for SseBroadcastExporter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SseBroadcastExporter").finish()
    }
}

impl SseBroadcastExporter {
    pub fn new(tx: broadcast::Sender<String>) -> Self {
        Self { tx }
    }
}

impl SpanExporter for SseBroadcastExporter {
    fn export(&mut self, batch: Vec<SpanData>) -> BoxFuture<'static, ExportResult> {
        let tx = self.tx.clone();
        Box::pin(async move {
            for span in batch {
                if let Ok(json) = serde_json::to_string(&span_to_event(&span)) {
                    // Ignore send errors — no SSE clients connected is fine.
                    let _ = tx.send(json);
                }
            }
            Ok(())
        })
    }
}

// ── Conversion ────────────────────────────────────────────────────────────────

fn span_to_event(span: &SpanData) -> SpanEvent {
    let ctx = &span.span_context;

    let start_ms = span
        .start_time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let end_ms = span
        .end_time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let parent_span_id = if span.parent_span_id
        == opentelemetry::trace::SpanId::INVALID
    {
        String::new()
    } else {
        span.parent_span_id.to_string()
    };

    let status = match &span.status {
        opentelemetry::trace::Status::Ok => "ok".into(),
        opentelemetry::trace::Status::Error { description } => {
            format!("error: {description}")
        }
        opentelemetry::trace::Status::Unset => "unset".into(),
    };

    let mut attributes = HashMap::new();
    for kv in &span.attributes {
        let val = match &kv.value {
            opentelemetry::Value::Bool(b) => serde_json::Value::Bool(*b),
            opentelemetry::Value::I64(i) => serde_json::Value::Number((*i).into()),
            opentelemetry::Value::F64(f) => {
                serde_json::json!(f)
            }
            opentelemetry::Value::String(s) => serde_json::Value::String(s.to_string()),
            opentelemetry::Value::Array(_) => serde_json::Value::String("[array]".into()),
        };
        attributes.insert(kv.key.to_string(), val);
    }

    SpanEvent {
        trace_id: ctx.trace_id().to_string(),
        span_id: ctx.span_id().to_string(),
        parent_span_id,
        name: span.name.to_string(),
        start_ms,
        end_ms,
        duration_ms: end_ms.saturating_sub(start_ms),
        status,
        attributes,
    }
}
