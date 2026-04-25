//! ByteRover memory provider — brv CLI integration.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use aiclaw_types::memory::{MemoryProvider, ToolSchema};
use serde_json::{json, Value};
use tracing::{debug, warn};

const QUERY_TIMEOUT: u64 = 10;
const CURATE_TIMEOUT: u64 = 120;

#[derive(Clone)]
pub struct ByteRoverMemoryProvider {
    session_strategy: SessionStrategy,
    brv_path: Option<PathBuf>,
    session_id: std::sync::Arc<std::sync::RwLock<String>>,
}

#[derive(Clone, Debug)]
enum SessionStrategy {
    PerSession,
    PerDirectory,
    Global,
}

impl ByteRoverMemoryProvider {
    pub fn new(session_strategy: &str, brv_path: Option<&str>) -> Self {
        let strategy = match session_strategy {
            "per-directory" => SessionStrategy::PerDirectory,
            "global" => SessionStrategy::Global,
            _ => SessionStrategy::PerSession,
        };
        let brv = brv_path.map(PathBuf::from).or_else(|| Self::find_brv());
        Self {
            session_strategy: strategy,
            brv_path: brv,
            session_id: Arc::new(std::sync::RwLock::new(String::new())),
        }
    }

    fn find_brv() -> Option<PathBuf> {
        let candidates = [
            PathBuf::from("brv"),
            dirs::home_dir()?.join(".brv-cli/bin/brv"),
            dirs::home_dir()?.join(".npm-global/bin/brv"),
            PathBuf::from("/usr/local/bin/brv"),
        ];
        candidates.into_iter().find(|p| {
            if p.to_string_lossy() == "brv" {
                std::process::Command::new("which").arg("brv").output().map(|o| o.status.success()).unwrap_or(false)
            } else {
                p.exists()
            }
        })
    }

    fn resolve_context(&self, session_id: &str) -> String {
        match &self.session_strategy {
            SessionStrategy::PerSession => session_id.to_string(),
            SessionStrategy::PerDirectory => {
                std::env::current_dir()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|_| session_id.to_string())
            }
            SessionStrategy::Global => "global".to_string(),
        }
    }

    fn run_brv(&self, args: &[&str], _timeout: u64, context: &str) -> Result<String, String> {
        let brv = self.brv_path.as_ref()
            .ok_or_else(|| "brv CLI not found. Install: npm install -g byterover-cli".to_string())?;

        let cwd = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".aiclaw/byterover")
            .join(context);

        std::fs::create_dir_all(&cwd).map_err(|e| e.to_string())?;

        let output = Command::new(brv)
            .args(args)
            .current_dir(&cwd)
            .output()
            .map_err(|e| format!("brv exec failed: {e}"))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).to_string())
        }
    }
}

impl MemoryProvider for ByteRoverMemoryProvider {
    fn name(&self) -> &str {
        "byterover"
    }

    fn is_available(&self) -> bool {
        self.brv_path.is_some()
    }

    fn initialize(&self, session_id: &str, _aiclaw_home: &Path) {
        *self.session_id.write().unwrap() = session_id.to_string();
    }

    fn system_prompt_block(&self) -> String {
        String::from(
            "# ByteRover Memory\n\
             Active. Use brv_query to search knowledge, brv_curate to store facts,\n\
             brv_status to check the knowledge tree.",
        )
    }

    fn get_tool_schemas(&self) -> Vec<ToolSchema> {
        vec![
            ToolSchema {
                name: "brv_query".to_string(),
                description: "Query the ByteRover knowledge tree. Returns relevant facts.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "What to search for"}
                    },
                    "required": ["query"]
                }),
            },
            ToolSchema {
                name: "brv_curate".to_string(),
                description: "Store a fact, decision, or pattern in the knowledge tree.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "content": {"type": "string", "description": "What to store"},
                        "type": {"type": "string", "enum": ["fact", "decision", "pattern", "conversation"], "default": "fact"}
                    },
                    "required": ["content"]
                }),
            },
            ToolSchema {
                name: "brv_status".to_string(),
                description: "Check ByteRover CLI version and knowledge tree statistics.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {}
                }),
            },
        ]
    }

    fn sync_turn(&self, user_content: &str, assistant_content: &str) {
        let sid = self.session_id.read().unwrap().clone();
        let ctx = self.resolve_context(&sid);
        let text = format!("user: {} | assistant: {}", user_content, assistant_content);
        if let Err(e) = self.run_brv(&["curate", "--type", "conversation", &text], CURATE_TIMEOUT, &ctx) {
            debug!("brv sync failed: {}", e);
        }
    }

    fn handle_tool_call(&self, name: &str, args: &Value) -> String {
        let sid = self.session_id.read().unwrap().clone();
        let ctx = self.resolve_context(&sid);

        match name {
            "brv_query" => {
                let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
                match self.run_brv(&["query", query], QUERY_TIMEOUT, &ctx) {
                    Ok(output) => json!({ "result": output }).to_string(),
                    Err(e) => json!({ "error": e }).to_string(),
                }
            }
            "brv_curate" => {
                let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
                let fact_type = args.get("type").and_then(|v| v.as_str()).unwrap_or("fact");
                match self.run_brv(&["curate", "--type", fact_type, content], CURATE_TIMEOUT, &ctx) {
                    Ok(output) => json!({ "result": output }).to_string(),
                    Err(e) => json!({ "error": e }).to_string(),
                }
            }
            "brv_status" => {
                match self.run_brv(&["status"], QUERY_TIMEOUT, &ctx) {
                    Ok(output) => json!({ "result": output }).to_string(),
                    Err(e) => json!({ "error": e }).to_string(),
                }
            }
            _ => json!({ "error": format!("unknown tool: {name}") }).to_string(),
        }
    }
}