use serde::Serialize;

/// Events emitted by the ReAct loop during a single user turn.
/// Serialized with `type` tag so JS can switch on `event.type`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    IterationStart { iteration: usize },
    ThinkStart     { iteration: usize },
    ThinkDone      { iteration: usize, partial_text: Option<String> },
    ActStart       { iteration: usize, num_tools: usize },
    ToolCallStart  { call_id: String, name: String, args: serde_json::Value },
    ToolCallDone   { call_id: String, name: String, result: String, success: bool },
    ObserveDone    { iteration: usize },
    FinalAnswer    { content: String, iterations: usize },
    Cancelled,
    Error          { message: String },
}
