//! Memory provider trait — pluggable cross-session memory backends.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Tool schema for a memory provider tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

pub trait MemoryProvider: Send + Sync {
    /// Short identifier, e.g. "builtin", "holographic", "byterover".
    fn name(&self) -> &str;

    /// True if this provider is configured and ready. No network calls.
    fn is_available(&self) -> bool;

    /// Initialize for a session. Called once at agent startup.
    fn initialize(&self, _session_id: &str, _aiclaw_home: &Path) {}

    /// Static text injected into the system prompt.
    fn system_prompt_block(&self) -> String {
        String::new()
    }

    /// Called before each LLM call — return context to inject.
    fn prefetch(&self, _query: &str) -> String {
        String::new()
    }

    /// Called after each turn — persist the exchange.
    fn sync_turn(&self, _user_content: &str, _assistant_content: &str) {}

    /// Called after each turn — precompute next turn's context (non-blocking).
    fn queue_prefetch(&self, _query: &str) {}

    /// Called at turn start — for cadence counters, turn counting.
    fn on_turn_start(&self, _turn_number: usize, _message: &str) {}

    /// Called when a session ends — flush pending writes.
    fn on_session_end(&self, _messages: &[(String, String)]) {}

    /// Called before context compression — extract insights from messages
    /// about to be dropped. Return text to include in the summary.
    fn on_pre_compress(&self, _messages: &[(String, String)]) -> String {
        String::new()
    }

    /// Tool schemas exposed by this provider.
    fn get_tool_schemas(&self) -> Vec<ToolSchema> {
        Vec::new()
    }

    /// Handle a tool call. args is the JSON deserialized argument map.
    fn handle_tool_call(&self, name: &str, args: &serde_json::Value) -> String {
        serde_json::json!({ "error": format!("{} does not implement tool calls", self.name()) })
            .to_string()
    }

    fn shutdown(&self) {}
}