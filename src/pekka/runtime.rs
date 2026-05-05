use super::{Actor, ActorContext, SupervisionDirective};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

// ── ActorRef ────────────────────────────────────────────────────────────────

/// A cheap-to-clone handle for sending messages to an actor.
///
/// Dropping the last `ActorRef` closes the mailbox channel; the actor will
/// drain any remaining messages and then stop.
#[derive(Clone)]
pub struct ActorRef<M: Send + 'static> {
    tx: mpsc::Sender<M>,
    cancellation: CancellationToken,
    /// Actor name — useful for logging.
    pub name: Arc<String>,
}

impl<M: Send + 'static> ActorRef<M> {
    /// Send a message asynchronously.  Waits for mailbox capacity.
    pub async fn tell(&self, msg: M) -> Result<(), SendError> {
        self.tx.send(msg).await.map_err(|_| SendError::MailboxClosed)
    }

    /// Send a message without waiting (best-effort).
    pub fn try_tell(&self, msg: M) -> Result<(), SendError> {
        self.tx.try_send(msg).map_err(|_| SendError::MailboxClosed)
    }

    /// Cancel the actor's `CancellationToken` — the actor will stop after its
    /// current message handler returns.
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }

    pub fn is_alive(&self) -> bool {
        !self.tx.is_closed()
    }
}

// ── ActorHandle ─────────────────────────────────────────────────────────────

/// Owned handle returned by [`spawn`].  Dropping it does NOT stop the actor
/// (the actor runs until all `ActorRef`s are dropped or it is cancelled).
pub struct ActorHandle {
    pub cancellation: CancellationToken,
    join: tokio::task::JoinHandle<()>,
}

impl ActorHandle {
    /// Cancel the actor and wait for its task to complete.
    pub async fn stop(self) {
        self.cancellation.cancel();
        let _ = self.join.await;
    }

    /// Cancel without waiting.
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }
}

// ── SendError ───────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum SendError {
    #[error("actor mailbox is closed")]
    MailboxClosed,
}

// ── spawn ────────────────────────────────────────────────────────────────────

/// Spawn an actor on the Tokio runtime.
///
/// * `name`          – human-readable name for logging / tracing.
/// * `mailbox_cap`   – mailbox channel capacity (`0` → unbounded).
/// * `parent_token`  – optional parent `CancellationToken`; cancelling the
///                     parent will also cancel this actor.
pub fn spawn<A>(
    mut actor: A,
    name: impl Into<String>,
    mailbox_cap: usize,
    parent_token: Option<CancellationToken>,
) -> (ActorRef<A::Message>, ActorHandle)
where
    A: Actor,
{
    let name = Arc::new(name.into());
    let cancellation = match parent_token {
        Some(parent) => parent.child_token(),
        None => CancellationToken::new(),
    };

    let (tx, rx) = if mailbox_cap == 0 {
        // Simulate unbounded by using a very large bounded channel.
        mpsc::channel(1 << 20)
    } else {
        mpsc::channel(mailbox_cap)
    };

    let ctx = ActorContext {
        id: Uuid::new_v4(),
        name: (*name).clone(),
        cancellation: cancellation.clone(),
    };

    let actor_ref = ActorRef {
        tx,
        cancellation: cancellation.clone(),
        name: name.clone(),
    };

    let join = tokio::spawn(actor_loop(actor, rx, ctx));
    let handle = ActorHandle { cancellation, join };

    (actor_ref, handle)
}

// ── internal actor loop ──────────────────────────────────────────────────────

async fn actor_loop<A: Actor>(
    mut actor: A,
    mut rx: mpsc::Receiver<A::Message>,
    ctx: ActorContext,
) {
    let name = ctx.name.clone();

    if let Err(e) = actor.on_start(&ctx).await {
        tracing::error!(actor = %name, error = %e, "on_start failed — actor stopping");
        return;
    }

    tracing::debug!(actor = %name, id = %ctx.id, "actor started");

    loop {
        tokio::select! {
            biased;

            // Cancellation takes priority over incoming messages.
            _ = ctx.cancellation.cancelled() => {
                tracing::debug!(actor = %name, "actor cancelled");
                break;
            }

            msg = rx.recv() => {
                match msg {
                    None => {
                        // All ActorRefs dropped — graceful shutdown.
                        tracing::debug!(actor = %name, "all senders dropped — stopping");
                        break;
                    }
                    Some(msg) => {
                        match actor.handle(msg, &ctx).await {
                            Ok(()) => {}
                            Err(e) => {
                                match actor.on_error(e, &ctx).await {
                                    SupervisionDirective::Resume => {}
                                    SupervisionDirective::Stop => {
                                        tracing::info!(actor = %name, "stopping after error");
                                        break;
                                    }
                                    SupervisionDirective::Restart => {
                                        // Simple restart: call on_start again.
                                        // A full supervisor would re-create the actor state.
                                        tracing::info!(actor = %name, "restarting after error");
                                        if let Err(e) = actor.on_start(&ctx).await {
                                            tracing::error!(actor = %name, error = %e, "restart failed");
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    actor.on_stop(&ctx).await;
    tracing::debug!(actor = %name, "actor stopped");
}
