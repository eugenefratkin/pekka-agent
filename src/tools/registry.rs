use super::Tool;
use crate::llm::ToolDefinition;
use dashmap::DashMap;
use std::sync::Arc;

/// Thread-safe tool registry shared across all agent sessions.
#[derive(Clone, Default)]
pub struct ToolRegistry {
    tools: Arc<DashMap<String, Arc<dyn Tool>>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a tool.  Overwrites any tool with the same name.
    pub fn register(&self, tool: impl Tool) {
        let name = tool.name().to_string();
        self.tools.insert(name, Arc::new(tool));
    }

    /// Register a pre-boxed tool.
    pub fn register_arc(&self, tool: Arc<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).map(|r| r.value().clone())
    }

    /// Returns `ToolDefinition`s suitable for sending to the LLM.
    pub fn all_definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .iter()
            .map(|entry| {
                let tool = entry.value();
                ToolDefinition::new(tool.name(), tool.description(), tool.schema())
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }
}
