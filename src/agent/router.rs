//! Skill and MCP router

use aiclaw_types::agent::{Intent, IntentType};
use aiclaw_types::skill::SkillMetadata;
use std::sync::Arc;

/// Routing result
#[derive(Debug, Clone)]
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
    fn get_by_domain_tag(&self, domain_tag: &str) -> Vec<Arc<SkillMetadata>>;
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

        // 1. First, route by domain tags (highest priority for domain-specific issues)
        let domain_results = self.route_by_domain(intent);
        results.extend(domain_results);

        // 2. Search by query
        let query = format!("{:?} {}", intent.intent_type, intent.raw_query);
        let skills = self.skill_registry.search(&query);

        for skill in skills {
            let confidence = self.calculate_skill_confidence(&skill, intent);
            if confidence > 0.3 {
                results.push(RouteResult::skill(&skill.name, confidence));
            }
        }

        // 3. Route by IntentType tag
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

        // 4. Always-on skills
        let always_skills = self.skill_registry.get_always();
        for skill in always_skills {
            if !results.iter().any(|r| r.skill_name.as_ref() == Some(&skill.name)) {
                results.push(RouteResult::skill(&skill.name, 0.4));
            }
        }

        // Deduplicate by skill name, keeping highest confidence
        let mut seen: std::collections::HashMap<String, f32> = std::collections::HashMap::new();
        results.retain(|r| {
            if let Some(ref name) = r.skill_name {
                let prev = seen.get(name).copied().unwrap_or(0.0);
                if r.confidence > prev {
                    seen.insert(name.clone(), r.confidence);
                    true
                } else {
                    false
                }
            } else {
                true
            }
        });

        results
    }

    /// Route by domain tags (GPU/HAMi, APISIX, CoreDNS, etc.)
    fn route_by_domain(&self, intent: &Intent) -> Vec<RouteResult> {
        let mut results = Vec::new();

        // Get domain tags from intent entities
        let domain_tags = self.get_intent_domain_tags(intent);

        for tag in &domain_tags {
            let skills = self.skill_registry.get_by_domain_tag(tag);
            for skill in skills {
                // Domain-matched skills get high confidence
                let confidence = if skill.domain_tags.contains(tag) {
                    0.9 // Direct domain match
                } else {
                    0.75
                };
                results.push(RouteResult::skill(&skill.name, confidence));
            }
        }

        results
    }

    /// Extract domain tags from intent entities
    fn get_intent_domain_tags(&self, intent: &Intent) -> Vec<String> {
        let mut tags = Vec::new();

        // Domain
        if let Some(ref domain) = intent.entities.domain {
            tags.push(domain.clone());
        }

        // Virtualization
        if let Some(ref virt) = intent.entities.virtualization {
            tags.push(virt.clone());
            // Also add common variations
            match virt.as_str() {
                "hami" => {
                    tags.push("gpu".to_string());
                    tags.push("vgpu".to_string());
                }
                "vgpu" => {
                    tags.push("gpu".to_string());
                }
                _ => {}
            }
        }

        // Resource state - may indicate specific skill
        if let Some(ref state) = intent.entities.resource_state {
            match state.as_str() {
                "pending" => {
                    // Could be scheduling issue
                    tags.push("pending".to_string());
                    tags.push("scheduling".to_string());
                }
                "crashloop" => {
                    tags.push("crashloop".to_string());
                    tags.push("oom".to_string());
                }
                "oom" => {
                    tags.push("oom".to_string());
                    tags.push("memory".to_string());
                }
                _ => {}
            }
        }

        // Error keyword
        if let Some(ref error) = intent.entities.error_keyword {
            tags.push(error.clone());
        }

        tags
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
        let mut confidence: f32 = 0.4; // Base confidence

        let intent_type_str = format!("{:?}", intent.intent_type).to_lowercase();
        let skill_name_lower = skill.name.to_lowercase();
        let skill_desc_lower = skill.description.to_lowercase();
        let query_lower = intent.raw_query.to_lowercase();

        // Intent type match
        if skill_name_lower.contains(&intent_type_str) {
            confidence += 0.15;
        }

        if skill_desc_lower.contains(&intent_type_str) {
            confidence += 0.1;
        }

        // Tag match
        for tag in &skill.tags {
            let tag_lower = tag.to_lowercase();
            if tag_lower == intent_type_str {
                confidence += 0.15;
            }
            if query_lower.contains(&tag_lower) {
                confidence += 0.05;
            }
        }

        // Domain tag match (from skill's domain_tags)
        for domain_tag in &skill.domain_tags {
            let tag_lower = domain_tag.to_lowercase();
            if query_lower.contains(&tag_lower) {
                confidence += 0.2;
            }
        }

        confidence.min(1.0) as f32
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

    fn get_by_domain_tag(&self, _domain_tag: &str) -> Vec<Arc<SkillMetadata>> {
        Vec::new()
    }

    fn get_always(&self) -> Vec<Arc<SkillMetadata>> {
        Vec::new()
    }
}
