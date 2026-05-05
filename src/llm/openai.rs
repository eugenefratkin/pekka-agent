//! OpenAI-compatible HTTP client (works with OpenAI, Azure OpenAI, Ollama,
//! LiteLLM, vLLM, and any other `/v1/chat/completions`-compatible endpoint).

use super::{
    types::{FinishReason, LlmResponse, TokenUsage},
    ChatMessage, LlmClient, ToolDefinition,
};
use crate::error::LlmError;
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

// ── Public client ────────────────────────────────────────────────────────────

pub struct OpenAiClient {
    client: Client,
    base_url: String,
    api_key: String,
    model: String,
    cancel: Option<CancellationToken>,
}

impl OpenAiClient {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            model: model.into(),
            cancel: None,
        }
    }

    /// Attach a cancellation token; any in-flight `complete` call will be
    /// aborted if the token is cancelled.
    pub fn with_cancellation(mut self, token: CancellationToken) -> Self {
        self.cancel = Some(token);
        self
    }
}

#[async_trait]
impl LlmClient for OpenAiClient {
    async fn complete(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolDefinition]>,
    ) -> Result<LlmResponse, LlmError> {
        let body = ChatRequest {
            model: self.model.clone(),
            messages: messages.to_vec(),
            tools: tools.map(|t| t.to_vec()),
            tool_choice: tools.map(|_| "auto".to_string()),
        };

        let url = format!("{}/chat/completions", self.base_url);

        let req_future = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send();

        // Race the HTTP request against cancellation.
        let response = if let Some(token) = &self.cancel {
            tokio::select! {
                biased;
                _ = token.cancelled() => return Err(LlmError::Cancelled),
                res = req_future => res?,
            }
        } else {
            req_future.await?
        };

        let status = response.status().as_u16();
        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(LlmError::Api { status, body });
        }

        let raw: ChatCompletion = response.json().await?;
        parse_completion(raw)
    }
}

// ── Wire-format types ────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
}

#[derive(Deserialize)]
struct ChatCompletion {
    model: String,
    choices: Vec<Choice>,
    usage: Option<UsageRaw>,
}

#[derive(Deserialize)]
struct Choice {
    message: MessageRaw,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct MessageRaw {
    content: Option<String>,
    tool_calls: Option<Vec<crate::llm::types::ToolCallRequest>>,
}

#[derive(Deserialize)]
struct UsageRaw {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

// ── Parsing ──────────────────────────────────────────────────────────────────

fn parse_completion(raw: ChatCompletion) -> Result<LlmResponse, LlmError> {
    let choice = raw
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| LlmError::InvalidResponse("no choices in response".into()))?;

    let finish_reason = match choice.finish_reason.as_deref() {
        Some("stop") => FinishReason::Stop,
        Some("tool_calls") => FinishReason::ToolCalls,
        Some("length") => FinishReason::MaxTokens,
        Some(other) => FinishReason::Other(other.to_string()),
        None => FinishReason::Stop,
    };

    Ok(LlmResponse {
        content: choice.message.content,
        tool_calls: choice.message.tool_calls.unwrap_or_default(),
        finish_reason,
        model: raw.model,
        usage: raw.usage.map(|u| TokenUsage {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
            total_tokens: u.total_tokens,
        }),
    })
}
