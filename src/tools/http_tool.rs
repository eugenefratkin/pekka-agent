//! An HTTP-backed tool that forwards `params` as a JSON POST body and returns
//! the response body as the tool result.

use crate::error::ToolError;
use crate::tools::Tool;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::json;
use tokio_util::sync::CancellationToken;

pub struct HttpTool {
    pub name: String,
    pub description: String,
    pub endpoint: String,
    pub schema: serde_json::Value,
    client: Client,
}

impl HttpTool {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        endpoint: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            endpoint: endpoint.into(),
            // Generic schema; override with `with_schema` for stricter validation.
            schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                }
            }),
            client: Client::new(),
        }
    }

    pub fn with_schema(mut self, schema: serde_json::Value) -> Self {
        self.schema = schema;
        self
    }
}

#[async_trait]
impl Tool for HttpTool {
    fn name(&self) -> &str { &self.name }
    fn description(&self) -> &str { &self.description }
    fn schema(&self) -> serde_json::Value { self.schema.clone() }

    async fn execute(
        &self,
        params: serde_json::Value,
        cancel: CancellationToken,
    ) -> Result<String, ToolError> {
        let req = self.client.post(&self.endpoint).json(&params).send();

        let response = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(ToolError::Cancelled),
            res = req => res?,
        };

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(ToolError::ExecutionFailed(format!("HTTP {status}: {body}")));
        }

        let text = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(ToolError::Cancelled),
            body = response.text() => body?,
        };

        Ok(text)
    }
}
