//! `pekka-llm` — interactive LLM chatbot framework built on Pekka actors
//! with OpenTelemetry tracing.
//!
//! # Quick start
//!
//! ```no_run
//! use pekka_llm::{
//!     config::AppConfig,
//!     llm::openai::OpenAiClient,
//!     orchestrator::{OrchestratorActor, OrchestratorMessage},
//!     pekka,
//!     telemetry,
//!     tools::{builtin::{CalculatorTool, EchoTool}, ToolRegistry},
//! };
//! use std::sync::Arc;
//! use tokio::sync::oneshot;
//! use tokio_util::sync::CancellationToken;
//!
//! #[tokio::main]
//! async fn main() {
//!     let _guard = telemetry::init("my-app");
//!
//!     let cfg = AppConfig::from_file("config/default.toml").unwrap();
//!
//!     let tools = ToolRegistry::new();
//!     tools.register(CalculatorTool);
//!     tools.register(EchoTool);
//!
//!     let llm = OpenAiClient::new(
//!         &cfg.llm.base_url,
//!         cfg.llm.api_key().unwrap(),
//!         &cfg.agent.model,
//!     );
//!
//!     let root_cancel = CancellationToken::new();
//!     let orchestrator = OrchestratorActor::new(
//!         Arc::new(cfg.agent),
//!         Arc::new(llm),
//!         Arc::new(tools),
//!         root_cancel.clone(),
//!     );
//!     let (orch_ref, _handle) = pekka::spawn(orchestrator, "orchestrator", 128, None);
//!
//!     // Start a session
//!     let (tx, rx) = oneshot::channel();
//!     orch_ref.tell(OrchestratorMessage::StartSession { reply: tx }).await.unwrap();
//!     let session_id = rx.await.unwrap().unwrap();
//!
//!     // Chat
//!     let (tx, rx) = oneshot::channel();
//!     orch_ref.tell(OrchestratorMessage::Chat {
//!         session_id,
//!         content: "What is (123 + 456) * 2?".into(),
//!         reply: tx,
//!     }).await.unwrap();
//!     let answer = rx.await.unwrap().unwrap();
//!     println!("{answer}");
//! }
//! ```

pub mod agent;
pub mod config;
pub mod error;
pub mod llm;
pub mod orchestrator;
pub mod pekka;
pub mod telemetry;
pub mod tools;
pub mod web;
