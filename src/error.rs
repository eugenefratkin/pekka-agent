use thiserror::Error;

#[derive(Debug, Error)]
pub enum PekkaError {
    #[error("actor mailbox closed")]
    MailboxClosed,
    #[error("actor start failed: {0}")]
    StartFailed(String),
}

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("LLM error: {0}")]
    Llm(#[from] LlmError),
    #[error("tool error: {0}")]
    Tool(#[from] ToolError),
    #[error("max iterations ({0}) reached without final answer")]
    MaxIterationsReached(usize),
    #[error("cancelled")]
    Cancelled,
    #[error("session not found: {0}")]
    SessionNotFound(String),
    #[error("actor gone")]
    ActorGone,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API error (status {status}): {body}")]
    Api { status: u16, body: String },
    #[error("invalid response: {0}")]
    InvalidResponse(String),
    #[error("cancelled")]
    Cancelled,
}

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("tool '{0}' not found in registry")]
    NotFound(String),
    #[error("invalid parameters: {0}")]
    InvalidParams(String),
    #[error("execution failed: {0}")]
    ExecutionFailed(String),
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("cancelled")]
    Cancelled,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("missing environment variable '{0}'")]
    MissingEnv(String),
}
