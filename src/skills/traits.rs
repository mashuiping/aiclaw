//! Skill traits

use async_trait::async_trait;
use aiclaw_types::skill::{SkillContext, SkillMetadata, SkillPrompts, ToolResult};
use serde_json::Value;

/// Skill trait - all skills must implement this
#[async_trait]
pub trait Skill: Send + Sync {
    /// Get skill metadata
    fn metadata(&self) -> &SkillMetadata;

    /// Get skill description
    fn description(&self) -> &str {
        &self.metadata().description
    }

    /// Get skill tags
    fn tags(&self) -> &[String] {
        &self.metadata().tags
    }

    /// Whether this skill should always be included
    fn always(&self) -> bool {
        self.metadata().always
    }

    /// Get skill prompts
    fn prompts(&self) -> SkillPrompts {
        SkillPrompts::default()
    }

    /// Execute the skill
    async fn execute(&self, context: &SkillContext) -> anyhow::Result<Vec<ToolResult>>;
}

/// Tool trait - tools used by skills
#[async_trait]
pub trait Tool: Send + Sync {
    /// Get tool name
    fn name(&self) -> &str;

    /// Get tool description
    fn description(&self) -> &str;

    /// Execute the tool with arguments
    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult>;

    /// Validate arguments before execution
    fn validate_args(&self, args: &Value) -> anyhow::Result<()> {
        let _ = args;
        Ok(())
    }
}
