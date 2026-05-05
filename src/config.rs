use crate::error::ConfigError;
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub llm: LlmConfig,
    pub agent: AgentConfig,
    #[serde(default)]
    pub tools: Vec<ToolConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LlmConfig {
    pub base_url: String,
    /// Name of the environment variable that holds the API key.
    pub api_key_env: String,
    pub model: String,
}

impl LlmConfig {
    pub fn api_key(&self) -> Result<String, ConfigError> {
        std::env::var(&self.api_key_env)
            .map_err(|_| ConfigError::MissingEnv(self.api_key_env.clone()))
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct AgentConfig {
    pub model: String,
    pub max_iterations: usize,
    pub system_prompt: String,
    pub parallel_reasoning: Option<ParallelReasonConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ParallelReasonConfig {
    pub num_agents: usize,
    pub strategy: ParallelStrategy,
}

#[derive(Debug, Deserialize, Clone, Default)]
#[serde(rename_all = "snake_case")]
pub enum ParallelStrategy {
    #[default]
    FirstWins,
    Majority,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ToolConfig {
    pub name: String,
    #[serde(rename = "type")]
    pub tool_type: ToolType,
    pub description: String,
    /// HTTP endpoint URL — only required when `type = "http"`.
    pub endpoint: Option<String>,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ToolType {
    Builtin,
    Http,
}

impl AppConfig {
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let contents = std::fs::read_to_string(path)?;
        let config = toml::from_str(&contents)?;
        Ok(config)
    }

    pub fn from_str(s: &str) -> Result<Self, ConfigError> {
        Ok(toml::from_str(s)?)
    }
}
