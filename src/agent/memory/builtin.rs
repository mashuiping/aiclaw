use aiclaw_types::memory::{MemoryProvider, ToolSchema};

pub struct BuiltinMemoryProvider;

impl BuiltinMemoryProvider {
    pub fn new() -> Self {
        Self
    }
}

impl MemoryProvider for BuiltinMemoryProvider {
    fn name(&self) -> &str {
        "builtin"
    }

    fn is_available(&self) -> bool {
        true
    }

    fn get_tool_schemas(&self) -> Vec<ToolSchema> {
        Vec::new() // builtin has no tools
    }
}