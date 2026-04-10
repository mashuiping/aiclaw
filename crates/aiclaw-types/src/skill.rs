//! Skill system types

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Skill metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMetadata {
    pub name: String,
    pub description: String,
    pub version: String,
    pub author: Option<String>,
    pub tags: Vec<String>,
    pub always: bool,
    /// Full markdown content for LLM to understand and execute
    #[serde(default)]
    pub raw_content: String,
    /// Scenarios/conditions where this skill applies
    #[serde(default)]
    pub applicability: Vec<String>,
    /// Domain-specific tags for routing (gpu, hami, apisix, coredns, etc.)
    #[serde(default)]
    pub domain_tags: Vec<String>,
}

/// Skill tool definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillTool {
    pub name: String,
    pub description: String,
    pub kind: ToolKind,
    pub command: String,
    pub args: HashMap<String, String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

/// Tool execution kind
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ToolKind {
    Shell,
    Http,
    Script,
}

/// Skill prompts
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillPrompts {
    #[serde(rename = "system")]
    pub system_prompt: Option<String>,
    #[serde(rename = "compact")]
    pub compact_prompt: Option<String>,
}

/// Tool execution result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_name: String,
    pub success: bool,
    pub output: Option<String>,
    pub error: Option<String>,
    pub execution_time_ms: u64,
    pub evidence: Vec<Evidence>,
}

/// Evidence for audit trail
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub evidence_type: EvidenceType,
    pub data: serde_json::Value,
}

/// Type of evidence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EvidenceType {
    Query,
    Response,
    Command,
    Result,
    Error,
}

/// Skill execution context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillContext {
    pub skill_name: String,
    pub user_id: String,
    pub channel: String,
    pub thread_id: Option<String>,
    pub parameters: HashMap<String, String>,
    pub session_id: Option<String>,
}

/// Skill manifest (from SKILL.toml or parsed from SKILL.md)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillManifest {
    pub name: String,
    pub description: String,
    pub version: String,
    pub author: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub always: bool,
    #[serde(default)]
    pub tools: Vec<SkillTool>,
    #[serde(default)]
    pub prompts: SkillPrompts,
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// Full markdown content (for SKILL.md)
    #[serde(default)]
    pub raw_content: String,
    /// Applicability scenarios (for SKILL.md)
    #[serde(default)]
    pub applicability: Vec<String>,
    /// Domain tags (for SKILL.md)
    #[serde(default)]
    pub domain_tags: Vec<String>,
}

impl SkillManifest {
    pub fn into_metadata(self) -> SkillMetadata {
        SkillMetadata {
            name: self.name,
            description: self.description,
            version: self.version,
            author: self.author,
            tags: self.tags,
            always: self.always,
            raw_content: self.raw_content,
            applicability: self.applicability,
            domain_tags: self.domain_tags,
        }
    }
}
