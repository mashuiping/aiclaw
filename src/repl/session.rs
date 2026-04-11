//! Session persistence: save/load conversation history to JSONL files.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::llm::types::ChatMessage;

/// A serializable session record.
#[derive(Debug, Serialize, Deserialize)]
struct SessionEntry {
    role: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

/// Directory for session files.
fn sessions_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".aiclaw").join("sessions"))
}

/// Generate a new session ID.
pub fn new_session_id() -> String {
    let now = chrono::Utc::now();
    format!("session-{}", now.format("%Y%m%d-%H%M%S"))
}

/// Save conversation messages to a JSONL session file.
pub fn save_session(session_id: &str, messages: &[ChatMessage]) -> anyhow::Result<PathBuf> {
    let dir = sessions_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    std::fs::create_dir_all(&dir)?;

    let path = dir.join(format!("{session_id}.jsonl"));
    let mut file = std::fs::File::create(&path)?;

    for msg in messages {
        let entry = SessionEntry {
            role: msg.role.as_str().to_string(),
            content: msg.content.clone(),
            tool_calls: msg.tool_calls.as_ref().map(|tc| serde_json::to_value(tc).unwrap_or_default()),
            tool_call_id: msg.tool_call_id.clone(),
        };
        let line = serde_json::to_string(&entry)?;
        writeln!(file, "{line}")?;
    }

    Ok(path)
}

/// Load the most recent session file.
pub fn load_latest_session() -> anyhow::Result<(String, Vec<ChatMessage>)> {
    let dir = sessions_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    if !dir.exists() {
        anyhow::bail!("No sessions directory found");
    }

    let mut entries: Vec<_> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "jsonl")
                .unwrap_or(false)
        })
        .collect();

    entries.sort_by_key(|e| std::cmp::Reverse(e.metadata().ok().and_then(|m| m.modified().ok())));

    let entry = entries
        .first()
        .ok_or_else(|| anyhow::anyhow!("No session files found"))?;

    let session_id = entry
        .path()
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let messages = load_session_file(&entry.path())?;
    Ok((session_id, messages))
}

/// Load messages from a session file.
fn load_session_file(path: &Path) -> anyhow::Result<Vec<ChatMessage>> {
    use crate::llm::types::{MessageRole, ToolCall};

    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    let mut messages = Vec::new();

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let entry: SessionEntry = serde_json::from_str(&line)?;
        let role = match entry.role.as_str() {
            "system" => MessageRole::System,
            "user" => MessageRole::User,
            "assistant" => MessageRole::Assistant,
            "tool" => MessageRole::Tool,
            _ => continue,
        };

        let tool_calls = entry.tool_calls.and_then(|v| {
            serde_json::from_value::<Vec<ToolCall>>(v).ok()
        });

        messages.push(ChatMessage {
            role,
            content: entry.content,
            name: None,
            tool_calls,
            tool_call_id: entry.tool_call_id,
        });
    }

    Ok(messages)
}
