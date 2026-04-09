//! Skill and MCP router

use aiclaw_types::agent::{Intent, IntentType};
use aiclaw_types::skill::SkillMetadata;
use std::sync::Arc;

/// Routing result
#[derive(Debug)]
pub struct RouteResult {
    pub skill_name: Option<String>,
    pub mcp_server: Option<String>,
    pub tool_name: Option<String>,
    pub confidence: f32,
}

impl RouteResult {
    pub fn skill(name: &str, confidence: f32) -> Self {
        Self {
            skill_name: Some(name.to_string()),
            mcp_server: None,
            tool_name: None,
            confidence,
        }
    }

    pub fn mcp(server: &str, tool: &str, confidence: f32) -> Self {
        Self {
            skill_name: None,
            mcp_server: Some(server.to_string()),
            tool_name: Some(tool.to_string()),
            confidence,
        }
    }
}

/// Skill and MCP router - routes intents to appropriate skills or MCP tools
pub struct Router {
    skill_registry: Arc<dyn SkillRegistryAccess>,
}

pub trait SkillRegistryAccess: Send + Sync {
    fn search(&self, query: &str) -> Vec<Arc<SkillMetadata>>;
    fn get_by_tag(&self, tag: &str) -> Vec<Arc<SkillMetadata>>;
    fn get_always(&self) -> Vec<Arc<SkillMetadata>>;
}

impl Router {
    pub fn new(skill_registry: Arc<dyn SkillRegistryAccess>) -> Self {
        Self { skill_registry }
    }

    /// Route an intent to the best matching skill or MCP tool
    pub fn route(&self, intent: &Intent) -> Vec<RouteResult> {
        let mut results = Vec::new();

        let skill_results = self.route_to_skill(intent);
        results.extend(skill_results);

        let mcp_results = self.route_to_mcp(intent);
        results.extend(mcp_results);

        results.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
        results.truncate(3);

        results
    }

    /// Route to internal skills
    fn route_to_skill(&self, intent: &Intent) -> Vec<RouteResult> {
        let mut results = Vec::new();

        let query = format!("{:?} {}", intent.intent_type, intent.raw_query);
        let skills = self.skill_registry.search(&query);

        for skill in skills {
            let confidence = self.calculate_skill_confidence(&skill, intent);
            if confidence > 0.3 {
                results.push(RouteResult::skill(&skill.name, confidence));
            }
        }

        let tag = match intent.intent_type {
            IntentType::Logs => "logs",
            IntentType::Metrics => "metrics",
            IntentType::Health => "health",
            IntentType::Debug => "debug",
            IntentType::Query => "query",
            IntentType::Scale => "scale",
            IntentType::Deploy => "deploy",
            IntentType::Unknown => "",
        };

        if !tag.is_empty() {
            let tagged_skills = self.skill_registry.get_by_tag(tag);
            for skill in tagged_skills {
                if !results.iter().any(|r| r.skill_name.as_ref() == Some(&skill.name)) {
                    results.push(RouteResult::skill(&skill.name, 0.6));
                }
            }
        }

        let always_skills = self.skill_registry.get_always();
        for skill in always_skills {
            if !results.iter().any(|r| r.skill_name.as_ref() == Some(&skill.name)) {
                results.push(RouteResult::skill(&skill.name, 0.4));
            }
        }

        results
    }

    /// Route to MCP servers/tools
    fn route_to_mcp(&self, intent: &Intent) -> Vec<RouteResult> {
        let mut results = Vec::new();

        match intent.intent_type {
            IntentType::Logs => {
                results.push(RouteResult::mcp("victoriametrics", "victorialogs_query", 0.8));
            }
            IntentType::Metrics => {
                results.push(RouteResult::mcp("victoriametrics", "victoriametrics_query", 0.8));
            }
            IntentType::Health => {
                results.push(RouteResult::mcp("victoriametrics", "victoriametrics_health", 0.7));
            }
            _ => {}
        }

        results
    }

    /// Calculate confidence for a skill matching an intent
    fn calculate_skill_confidence(&self, skill: &SkillMetadata, intent: &Intent) -> f32 {
        let mut confidence = 0.5;

        let intent_type_str = format!("{:?}", intent.intent_type).to_lowercase();
        let skill_name_lower = skill.name.to_lowercase();
        let skill_desc_lower = skill.description.to_lowercase();

        if skill_name_lower.contains(&intent_type_str) {
            confidence += 0.2;
        }

        if skill_desc_lower.contains(&intent_type_str) {
            confidence += 0.1;
        }

        for tag in &skill.tags {
            let tag_lower = tag.to_lowercase();
            if tag_lower == intent_type_str {
                confidence += 0.15;
            }

            if intent.raw_query.to_lowercase().contains(&tag_lower) {
                confidence += 0.05;
            }
        }

        confidence.min(1.0)
    }
}

impl Default for Router {
    fn default() -> Self {
        Self {
            skill_registry: Arc::new(DefaultSkillRegistryAccess),
        }
    }
}

struct DefaultSkillRegistryAccess;

impl SkillRegistryAccess for DefaultSkillRegistryAccess {
    fn search(&self, _query: &str) -> Vec<Arc<SkillMetadata>> {
        Vec::new()
    }

    fn get_by_tag(&self, _tag: &str) -> Vec<Arc<SkillMetadata>> {
        Vec::new()
    }

    fn get_always(&self) -> Vec<Arc<SkillMetadata>> {
        Vec::new()
    }
}
