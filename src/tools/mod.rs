pub mod builtin;
pub mod http_tool;
pub mod registry;

pub use registry::ToolRegistry;

use crate::error::ToolError;
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

/// A single tool available to the agent.
///
/// Implementations are stored as `Arc<dyn Tool>` in the registry and shared
/// across sessions (stateless per-call semantics).
#[async_trait]
pub trait Tool: Send + Sync + 'static {
    fn name(&self) -> &str;
    fn description(&self) -> &str;

    /// JSON Schema describing the tool's parameters.
    /// Example: `{ "type": "object", "properties": { "expr": { "type": "string" } }, "required": ["expr"] }`
    fn schema(&self) -> serde_json::Value;

    /// Execute the tool.  `cancel` is a child token — the implementation
    /// should respect it for long-running or I/O-bound operations.
    async fn execute(
        &self,
        params: serde_json::Value,
        cancel: CancellationToken,
    ) -> Result<String, ToolError>;
}
