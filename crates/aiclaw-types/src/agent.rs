//! Agent core types

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// Agent session for tracking conversation context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub user_id: String,
    pub channel: String,
    pub thread_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    pub state: SessionState,
    pub context: SessionContext,
}

/// Session state
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SessionState {
    Active,
    Waiting,
    Completed,
    Expired,
}

/// Session context data
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionContext {
    #[serde(default)]
    pub last_skill: Option<String>,
    #[serde(default)]
    pub last_parameters: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub history: Vec<InteractionRecord>,
}

/// Record of a single interaction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionRecord {
    pub timestamp: DateTime<Utc>,
    pub intent: String,
    pub skill: Option<String>,
    pub result: Option<String>,
    pub success: bool,
}

/// Parsed user intent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Intent {
    pub intent_type: IntentType,
    pub confidence: f32,
    pub entities: IntentEntities,
    pub raw_query: String,
}

/// Intent type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum IntentType {
    Query,
    Debug,
    Health,
    Logs,
    Metrics,
    Deploy,
    Scale,
    Unknown,
}

/// Intent entities extracted from user message
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IntentEntities {
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub pod_name: Option<String>,
    #[serde(default)]
    pub deployment_name: Option<String>,
    #[serde(default)]
    pub service_name: Option<String>,
    #[serde(default)]
    pub cluster: Option<String>,
    #[serde(default)]
    pub time_range: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub labels: std::collections::HashMap<String, String>,
}

/// Agent response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponse {
    pub session_id: String,
    pub message: OutgoingMessage,
    pub success: bool,
    pub evidence: Vec<EvidenceRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Outgoing message content
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutgoingMessage {
    pub content: String,
    pub format: OutputFormat,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_block: Option<CodeBlockInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table: Option<TableInfo>,
}

/// Output format
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum OutputFormat {
    Plain,
    Markdown,
    Json,
}

/// Code block info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeBlockInfo {
    pub language: Option<String>,
    pub content: String,
}

/// Table info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableInfo {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// Evidence record for audit trail
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub timestamp: DateTime<Utc>,
    pub source: String,
    pub action: String,
    pub data: serde_json::Value,
}
