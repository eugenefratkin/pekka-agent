pub mod openai;
pub mod types;

pub use types::*;

use crate::error::LlmError;
use async_trait::async_trait;

/// Core LLM abstraction.  Implementations must be `Send + Sync` so they can
/// be wrapped in `Arc` and shared across actors.
#[async_trait]
pub trait LlmClient: Send + Sync + 'static {
    async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolDefinition]>,
    ) -> Result<LlmResponse, LlmError>;
}
