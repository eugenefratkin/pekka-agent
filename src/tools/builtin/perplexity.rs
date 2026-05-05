//! Perplexity internet-search tool.
//!
//! Calls `https://api.perplexity.ai/chat/completions` (OpenAI-compatible) with
//! the `sonar` model and returns the answer text plus any cited URLs.
//!
//! Requires `PERPLEXITY_API_KEY` in the environment (or pass the key to `new()`).

use crate::{error::ToolError, tools::Tool};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

const BASE_URL: &str = "https://api.perplexity.ai/chat/completions";

pub struct PerplexitySearchTool {
    client:  Client,
    api_key: String,
    model:   String,
}

impl PerplexitySearchTool {
    /// Build the tool, reading the API key from `PERPLEXITY_API_KEY`.
    pub fn from_env() -> Result<Self, ToolError> {
        let key = std::env::var("PERPLEXITY_API_KEY")
            .map_err(|_| ToolError::InvalidParams("PERPLEXITY_API_KEY not set".into()))?;
        Ok(Self::new(key))
    }

    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client:  Client::new(),
            api_key: api_key.into(),
            model:   "sonar".into(),
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }
}

// ── Wire types ────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct Request {
    model:    String,
    messages: Vec<Msg>,
}

#[derive(Serialize)]
struct Msg {
    role:    &'static str,
    content: String,
}

#[derive(Deserialize)]
struct Response {
    choices:   Vec<Choice>,
    #[serde(default)]
    citations: Vec<String>,
}

#[derive(Deserialize)]
struct Choice {
    message: MsgContent,
}

#[derive(Deserialize)]
struct MsgContent {
    content: String,
}

// ── Tool impl ─────────────────────────────────────────────────────────────────

#[async_trait]
impl Tool for PerplexitySearchTool {
    fn name(&self) -> &str { "perplexity_search" }

    fn description(&self) -> &str {
        "Search the internet using Perplexity AI. Use this to find current information, \
         news, reviews, or anything that requires up-to-date web knowledge."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, params: Value, cancel: CancellationToken) -> Result<String, ToolError> {
        let query = params["query"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidParams("missing 'query' field".into()))?
            .to_string();

        let body = Request {
            model: self.model.clone(),
            messages: vec![
                Msg { role: "system", content: "Be precise and concise.".into() },
                Msg { role: "user",   content: query },
            ],
        };

        let req = self.client
            .post(BASE_URL)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send();

        let response = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(ToolError::Cancelled),
            res = req => res?,
        };

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let text   = response.text().await.unwrap_or_default();
            return Err(ToolError::ExecutionFailed(format!("Perplexity {status}: {text}")));
        }

        let parsed = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(ToolError::Cancelled),
            res = response.json::<Response>() => res?,
        };

        let content = parsed.choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .unwrap_or_else(|| "<empty response>".into());

        if parsed.citations.is_empty() {
            Ok(content)
        } else {
            let sources = parsed.citations
                .iter()
                .enumerate()
                .map(|(i, url)| format!("[{}] {}", i + 1, url))
                .collect::<Vec<_>>()
                .join("\n");
            Ok(format!("{content}\n\nSources:\n{sources}"))
        }
    }
}
