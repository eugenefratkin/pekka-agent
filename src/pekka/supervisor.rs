/// What the actor wants to happen after an error in `handle`.
pub enum SupervisionDirective {
    /// Ignore the error and continue processing messages.
    Resume,
    /// Stop the actor permanently.
    Stop,
    /// Re-call `on_start` and resume (note: actor state is **not** reset;
    /// the actor is responsible for resetting its own fields in `on_start`).
    Restart,
}

/// Simple configuration used by a parent actor that wants to supervise
/// children (not yet a full actor — kept minimal for now).
#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    pub max_restarts: usize,
    pub restart_window_secs: u64,
}

impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            max_restarts: 3,
            restart_window_secs: 60,
        }
    }
}

// Placeholder for a full SupervisorActor implementation.
pub struct Supervisor {
    pub config: SupervisorConfig,
}

impl Supervisor {
    pub fn new(config: SupervisorConfig) -> Self {
        Self { config }
    }
}
