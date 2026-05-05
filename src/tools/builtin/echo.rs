use crate::error::ToolError;
use crate::tools::Tool;
use async_trait::async_trait;
use serde_json::json;
use tokio_util::sync::CancellationToken;

pub struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str { "echo" }
    fn description(&self) -> &str { "Returns its input text unchanged" }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "The text to echo back" }
            },
            "required": ["text"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _cancel: CancellationToken,
    ) -> Result<String, ToolError> {
        let text = params
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("missing 'text' field".into()))?;
        Ok(text.to_string())
    }
}
