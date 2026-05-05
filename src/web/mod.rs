pub mod exporter;

use crate::{
    agent::events::AgentEvent,
    error::AgentError,
    orchestrator::OrchestratorMessage,
    pekka::ActorRef,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        Html, IntoResponse, Response,
    },
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, sync::Arc};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_stream::{wrappers::{BroadcastStream, ReceiverStream}, StreamExt as _};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

// ── Shared state ──────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub orch: ActorRef<OrchestratorMessage>,
    /// Broadcast channel for OTel span events (from SseBroadcastExporter).
    pub span_tx: broadcast::Sender<String>,
    pub root_cancel: CancellationToken,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/",                            get(index))
        .route("/api/session",                 post(create_session))
        .route("/api/session/:id/chat",        post(chat))
        .route("/api/session/:id/stream",      post(stream_chat))
        .route("/api/session/:id/cancel",      delete(cancel_session))
        .route("/events/spans",                get(sse_spans))
        .with_state(state)
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn index() -> Html<&'static str> {
    Html(UI_HTML)
}

#[derive(Serialize)]
struct SessionResponse { session_id: Uuid }

async fn create_session(State(s): State<Arc<AppState>>) -> Result<Json<SessionResponse>, AppError> {
    let (tx, rx) = oneshot::channel();
    s.orch.tell(OrchestratorMessage::StartSession { reply: tx }).await.map_err(|_| AppError::gone())?;
    let session_id = rx.await.map_err(|_| AppError::gone())??;
    Ok(Json(SessionResponse { session_id }))
}

#[derive(Deserialize)]
struct ChatRequest { message: String }

#[derive(Serialize)]
struct ChatResponse { answer: String }

async fn chat(
    State(s): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
    Json(body): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, AppError> {
    let (tx, rx) = oneshot::channel();
    s.orch.tell(OrchestratorMessage::Chat { session_id, content: body.message, reply: tx })
        .await.map_err(|_| AppError::gone())?;
    let answer = rx.await.map_err(|_| AppError::gone())??;
    Ok(Json(ChatResponse { answer }))
}

/// Streaming endpoint: returns SSE of `AgentEvent` frames.
/// Uses fetch + ReadableStream on the client (not EventSource) because POST.
async fn stream_chat(
    State(s): State<Arc<AppState>>,
    Path(session_id): Path<Uuid>,
    Json(body): Json<ChatRequest>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let (event_tx, event_rx) = mpsc::channel::<AgentEvent>(128);

    // Detach the send so the SSE response headers go out immediately.
    let orch = s.orch.clone();
    tokio::spawn(async move {
        let _ = orch.tell(OrchestratorMessage::StreamChat {
            session_id,
            content: body.message,
            events: event_tx,
        }).await;
    });

    let stream = ReceiverStream::new(event_rx).map(|ev| {
        let json = serde_json::to_string(&ev)
            .unwrap_or_else(|_| r#"{"type":"error","message":"serialization error"}"#.into());
        Ok::<_, Infallible>(Event::default().event("agent").data(json))
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn cancel_session(State(s): State<Arc<AppState>>, Path(session_id): Path<Uuid>) -> StatusCode {
    let _ = s.orch.tell(OrchestratorMessage::CancelSession { session_id }).await;
    StatusCode::NO_CONTENT
}

/// SSE stream of OTel span events from the global broadcast channel.
async fn sse_spans(
    State(s): State<Arc<AppState>>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let rx = s.span_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|msg| {
        msg.ok().map(|data| Ok(Event::default().event("span").data(data)))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

// ── Error type ────────────────────────────────────────────────────────────────

struct AppError(StatusCode, String);
impl AppError {
    fn gone() -> Self { Self(StatusCode::SERVICE_UNAVAILABLE, "orchestrator unavailable".into()) }
}
impl From<AgentError> for AppError {
    fn from(e: AgentError) -> Self {
        match e {
            AgentError::SessionNotFound(_) => Self(StatusCode::NOT_FOUND, e.to_string()),
            AgentError::Cancelled          => Self(StatusCode::CONFLICT, "cancelled".into()),
            other                          => Self(StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
        }
    }
}
impl IntoResponse for AppError {
    fn into_response(self) -> Response { (self.0, self.1).into_response() }
}

// ── Embedded UI ───────────────────────────────────────────────────────────────

const UI_HTML: &str = include_str!("ui.html");
