//! Skill registry - manages loaded skills

use aiclaw_types::skill::{SkillMetadata, SkillTool};
use dashmap::DashMap;
use std::sync::Arc;
use tracing::debug;

/// Skill registry - thread-safe registry for managing skills
pub struct SkillRegistry {
    skills: DashMap<String, Arc<SkillMetadata>>,
    tags_index: DashMap<String, Vec<String>>,
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self {
            skills: DashMap::new(),
            tags_index: DashMap::new(),
        }
    }

    /// Register a skill
    pub fn register(&self, skill: SkillMetadata) {
        let name = skill.name.clone();
        self.skills.insert(name.clone(), Arc::new(skill.clone()));

        for tag in &skill.tags {
            let tag_lower = tag.to_lowercase();
            self.tags_index
                .entry(tag_lower)
                .or_insert_with(Vec::new)
                .push(name.clone());
        }

        debug!("Registered skill: {}", skill.name);
    }

    /// Get a skill by name
    pub fn get(&self, name: &str) -> Option<Arc<SkillMetadata>> {
        self.skills.get(name).map(|r| r.value().clone())
    }

    /// Get all skills
    pub fn get_all(&self) -> Vec<Arc<SkillMetadata>> {
        self.skills.iter().map(|r| r.value().clone()).collect()
    }

    /// Search skills by query (matches name, description, tags)
    pub fn search(&self, query: &str) -> Vec<Arc<SkillMetadata>> {
        let query_lower = query.to_lowercase();
        let query_parts: Vec<&str> = query_lower.split_whitespace().collect();

        let mut scored: Vec<(&str, i32)> = Vec::new();

        for entry in self.skills.iter() {
            let skill = entry.value();
            let mut score = 0;

            let name_lower = skill.name.to_lowercase();
            let desc_lower = skill.description.to_lowercase();

            for part in &query_parts {
                if name_lower.contains(part) {
                    score += 10;
                }
                if desc_lower.contains(part) {
                    score += 5;
                }
                for tag in &skill.tags {
                    if tag.to_lowercase().contains(part) {
                        score += 3;
                    }
                }
            }

            if score > 0 {
                scored.push((skill.name.as_str(), score));
            }
        }

        scored.sort_by(|a, b| b.1.cmp(&a.1));

        scored
            .into_iter()
            .filter_map(|(name, _)| self.get(name))
            .collect()
    }

    /// Get skills by tag
    pub fn get_by_tag(&self, tag: &str) -> Vec<Arc<SkillMetadata>> {
        let tag_lower = tag.to_lowercase();
        if let Some(names) = self.tags_index.get(&tag_lower) {
            names
                .iter()
                .filter_map(|name| self.get(name))
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get skills that should always be included
    pub fn get_always(&self) -> Vec<Arc<SkillMetadata>> {
        self.skills
            .iter()
            .filter(|r| r.value().always)
            .map(|r| r.value().clone())
            .collect()
    }

    /// Get tools for a skill (placeholder - actual implementation would load from files)
    pub fn get_tools(&self, _skill_name: &str) -> Option<Vec<SkillTool>> {
        None
    }

    /// Get the count of registered skills
    pub fn len(&self) -> usize {
        self.skills.len()
    }

    /// Check if registry is empty
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for SkillRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkillRegistry")
            .field("skill_count", &self.skills.len())
            .field("tag_count", &self.tags_index.len())
            .finish()
    }
}
