//! Pekka — a lightweight Tokio-based actor framework.
//!
//! Actors communicate exclusively through typed message channels.  Each actor
//! runs in its own Tokio task and processes messages sequentially.  For
//! request–reply patterns the message carries a `oneshot::Sender` for the
//! response.  Cancellation flows through a `CancellationToken` tree that is
//! created per-actor at spawn time and can be a child of a parent token.

mod runtime;
mod supervisor;

pub use runtime::{spawn, ActorHandle, ActorRef, SendError};
pub use supervisor::{Supervisor, SupervisorConfig, SupervisionDirective};

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

// ── ActorContext ────────────────────────────────────────────────────────────

/// Passed into every handler call; provides cancellation and identity.
#[derive(Clone)]
pub struct ActorContext {
    pub id: Uuid,
    pub name: String,
    /// Cancelling this token stops the actor loop after the current message
    /// finishes.  Create child tokens for sub-tasks so they inherit
    /// cancellation without cancelling the actor itself.
    pub cancellation: CancellationToken,
}

impl ActorContext {
    /// Returns a new `CancellationToken` that is a child of the actor's token.
    /// Cancelling the actor will also cancel any child tokens.
    pub fn child_token(&self) -> CancellationToken {
        self.cancellation.child_token()
    }
}

// ── Actor trait ─────────────────────────────────────────────────────────────

#[async_trait]
pub trait Actor: Send + 'static {
    type Message: Send + 'static;

    /// Called once before the message loop starts.
    async fn on_start(&mut self, _ctx: &ActorContext) -> Result<(), BoxError> {
        Ok(())
    }

    /// Called for each incoming message.
    async fn handle(
        &mut self,
        msg: Self::Message,
        ctx: &ActorContext,
    ) -> Result<(), BoxError>;

    /// Called when an error is returned from `handle`.  The default behaviour
    /// is to log the error and stop the actor.
    async fn on_error(
        &mut self,
        err: BoxError,
        ctx: &ActorContext,
    ) -> SupervisionDirective {
        tracing::error!(actor = %ctx.name, error = %err, "actor error");
        SupervisionDirective::Stop
    }

    /// Called once after the message loop exits (graceful stop or cancel).
    async fn on_stop(&mut self, _ctx: &ActorContext) {}
}
