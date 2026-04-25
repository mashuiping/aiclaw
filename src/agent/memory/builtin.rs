use std::fs;
use std::path::PathBuf;
use aiclaw_types::memory::{MemoryProvider, ToolSchema};

const MEMORY_FILE: &str = "MEMORY.md";
const USER_FILE: &str = "USER.md";

pub struct BuiltinMemoryProvider {
    memory_content: std::sync::RwLock<String>,
}

impl BuiltinMemoryProvider {
    pub fn new() -> Self {
        Self {
            memory_content: std::sync::RwLock::new(String::new()),
        }
    }

    /// Call with the aiclaw_home path to load files.
    pub fn with_home(mut self, aiclaw_home: PathBuf) -> Self {
        let mut content = String::new();
        let mem_path = aiclaw_home.join(MEMORY_FILE);
        let user_path = aiclaw_home.join(USER_FILE);

        if mem_path.exists() {
            if let Ok(text) = fs::read_to_string(&mem_path) {
                content.push_str(&text);
            }
        }
        if user_path.exists() {
            if let Ok(text) = fs::read_to_string(&user_path) {
                if !content.is_empty() {
                    content.push_str("\n\n");
                }
                content.push_str(&text);
            }
        }

        *self.memory_content.write().unwrap() = content;
        self
    }
}

impl MemoryProvider for BuiltinMemoryProvider {
    fn name(&self) -> &str {
        "builtin"
    }

    fn is_available(&self) -> bool {
        true
    }

    fn system_prompt_block(&self) -> String {
        let content = self.memory_content.read().unwrap();
        if content.is_empty() {
            return String::new();
        }
        format!(
            "<memory>\n\
             {}\n\
             </memory>",
            content
        )
    }

    fn get_tool_schemas(&self) -> Vec<ToolSchema> {
        Vec::new()
    }
}