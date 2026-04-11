//! Tool executor for REPL mode.
//!
//! Provides bash, read_file, write_file, and list_files tools
//! that the LLM can invoke via native tool-use protocol.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tracing::debug;

use crate::llm::types::ToolSpec;

/// Tool definitions sent to the LLM.
pub fn tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "bash".to_string(),
            description: "Run a shell command and return its stdout/stderr. \
                          Use for kubectl, helm, grep, cat, jq, curl, and any system command."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to execute"
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "description": "Timeout in seconds (default 30)"
                    }
                },
                "required": ["command"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "read_file".to_string(),
            description: "Read the contents of a file at the given path.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute or relative path to the file"
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "list_files".to_string(),
            description: "List files and directories at a given path.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path to list (default: current directory)"
                    },
                    "pattern": {
                        "type": "string",
                        "description": "Optional glob pattern to filter results"
                    }
                },
                "additionalProperties": false
            }),
        },
    ]
}

/// Result of executing a tool.
pub struct ToolResult {
    pub output: String,
    pub is_error: bool,
}

/// Execute a tool by name with the given JSON arguments.
pub async fn execute_tool(
    name: &str,
    args_json: &str,
    kubeconfig: Option<&PathBuf>,
) -> ToolResult {
    let args: serde_json::Value = match serde_json::from_str(args_json) {
        Ok(v) => v,
        Err(e) => {
            let preview = if args_json.len() > 100 {
                format!("{}...", &args_json[..100])
            } else {
                args_json.to_string()
            };
            return ToolResult {
                output: format!(
                    "Failed to parse tool arguments: {e}\nRaw input: {preview}"
                ),
                is_error: true,
            };
        }
    };

    match name {
        "bash" => execute_bash(&args, kubeconfig).await,
        "read_file" => execute_read_file(&args).await,
        "list_files" => execute_list_files(&args).await,
        _ => ToolResult {
            output: format!("Unknown tool: {name}"),
            is_error: true,
        },
    }
}

async fn execute_bash(args: &serde_json::Value, kubeconfig: Option<&PathBuf>) -> ToolResult {
    let command = match args["command"].as_str() {
        Some(c) => c,
        None => {
            return ToolResult {
                output: "Missing 'command' argument".to_string(),
                is_error: true,
            };
        }
    };

    let timeout_secs = args["timeout_secs"].as_u64().unwrap_or(30);
    debug!("bash tool: {}", command);

    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c").arg(command);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);

    // Apply kubeconfig
    cmd.env_remove("KUBECONFIG");
    if let Some(kc) = kubeconfig {
        cmd.env("KUBECONFIG", kc);
    }

    match tokio::time::timeout(Duration::from_secs(timeout_secs), cmd.output()).await {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = if stdout.is_empty() {
                stderr.to_string()
            } else if stderr.is_empty() {
                stdout.to_string()
            } else {
                format!("{stdout}\n--- stderr ---\n{stderr}")
            };

            ToolResult {
                output: combined,
                is_error: !output.status.success(),
            }
        }
        Ok(Err(e)) => ToolResult {
            output: format!("Command execution error: {e}"),
            is_error: true,
        },
        Err(_) => ToolResult {
            output: format!("Command timed out after {timeout_secs}s"),
            is_error: true,
        },
    }
}

async fn execute_read_file(args: &serde_json::Value) -> ToolResult {
    let path = match args["path"].as_str() {
        Some(p) => p,
        None => {
            return ToolResult {
                output: "Missing 'path' argument".to_string(),
                is_error: true,
            };
        }
    };

    match tokio::fs::read_to_string(path).await {
        Ok(content) => ToolResult {
            output: content,
            is_error: false,
        },
        Err(e) => ToolResult {
            output: format!("Failed to read {path}: {e}"),
            is_error: true,
        },
    }
}

async fn execute_list_files(args: &serde_json::Value) -> ToolResult {
    let path = args["path"].as_str().unwrap_or(".");

    match tokio::fs::read_dir(path).await {
        Ok(mut entries) => {
            let mut names = Vec::new();
            while let Ok(Some(entry)) = entries.next_entry().await {
                let file_type = entry.file_type().await.ok();
                let suffix = if file_type.map(|t| t.is_dir()).unwrap_or(false) {
                    "/"
                } else {
                    ""
                };
                names.push(format!("{}{suffix}", entry.file_name().to_string_lossy()));
            }
            names.sort();
            ToolResult {
                output: names.join("\n"),
                is_error: false,
            }
        }
        Err(e) => ToolResult {
            output: format!("Failed to list {path}: {e}"),
            is_error: true,
        },
    }
}
