//! Skill loader - loads skills from filesystem

use aiclaw_types::skill::{SkillManifest, SkillMetadata, SkillTool, SkillPrompts};
use std::path::{Path, PathBuf};
use tracing::{debug, error, info, warn};

/// Skill loader - scans directories and loads skill manifests
pub struct SkillLoader {
    skills_dir: PathBuf,
}

impl SkillLoader {
    pub fn new(skills_dir: impl Into<PathBuf>) -> Self {
        Self {
            skills_dir: skills_dir.into(),
        }
    }

    /// Load all skills from the skills directory
    pub fn load_skills(&self) -> anyhow::Result<Vec<SkillMetadata>> {
        let mut skills = Vec::new();

        if !self.skills_dir.exists() {
            warn!("Skills directory does not exist: {:?}", self.skills_dir);
            return Ok(skills);
        }

        info!("Loading skills from {:?}", self.skills_dir);

        for entry in std::fs::read_dir(&self.skills_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                match self.load_skill_from_dir(&path) {
                    Ok(skill) => {
                        debug!("Loaded skill: {}", skill.name);
                        skills.push(skill);
                    }
                    Err(e) => {
                        error!("Failed to load skill from {:?}: {}", path, e);
                    }
                }
            }
        }

        info!("Loaded {} skills", skills.len());
        Ok(skills)
    }

    /// Load a single skill from a directory
    pub fn load_skill_from_dir(&self, dir: &Path) -> anyhow::Result<SkillMetadata> {
        let toml_path = dir.join("SKILL.toml");
        let md_path = dir.join("SKILL.md");

        if toml_path.exists() {
            self.load_from_toml(&toml_path)
        } else if md_path.exists() {
            self.load_from_md(&md_path)
        } else {
            anyhow::bail!("No SKILL.toml or SKILL.md found in {:?}", dir);
        }
    }

    /// Load skill from TOML manifest
    fn load_from_toml(&self, path: &Path) -> anyhow::Result<SkillMetadata> {
        let content = std::fs::read_to_string(path)?;
        let manifest: SkillManifest = toml::from_str(&content)?;
        Ok(manifest.into_metadata())
    }

    /// Load skill from Markdown with frontmatter
    fn load_from_md(&self, path: &Path) -> anyhow::Result<SkillMetadata> {
        let content = std::fs::read_to_string(path)?;

        let mut lines = content.lines();
        let mut frontmatter_lines = Vec::new();

        let mut in_frontmatter = false;
        let mut found_frontmatter = false;

        for line in lines.by_ref() {
            if line.trim() == "---" {
                if in_frontmatter {
                    found_frontmatter = true;
                    break;
                } else {
                    in_frontmatter = true;
                }
            } else if in_frontmatter {
                frontmatter_lines.push(line);
            }
        }

        if !found_frontmatter {
            anyhow::bail!("No frontmatter found in {:?}", path);
        }

        let frontmatter = frontmatter_lines.join("\n");
        let manifest: SkillManifest = serde_frontmatter::parse(&frontmatter)
            .map_err(|e| anyhow::anyhow!("Failed to parse frontmatter: {}", e))?;

        Ok(manifest.into_metadata())
    }

    /// Load skill tools from manifest
    pub fn load_skill_tools(&self, dir: &Path) -> anyhow::Result<Vec<SkillTool>> {
        let toml_path = dir.join("SKILL.toml");

        if !toml_path.exists() {
            return Ok(Vec::new());
        }

        let content = std::fs::read_to_string(&toml_path)?;
        let manifest: SkillManifest = toml::from_str(&content)?;

        Ok(manifest.tools)
    }
}

/// Simple frontmatter parser for SKILL.md files
mod serde_frontmatter {
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;

    #[derive(Debug, Deserialize, Serialize)]
    pub struct SkillFrontmatter {
        pub name: String,
        pub description: String,
        #[serde(default)]
        pub version: String,
        #[serde(default)]
        pub author: Option<String>,
        #[serde(default)]
        pub tags: Vec<String>,
    }

    pub fn parse(frontmatter: &str) -> anyhow::Result<super::SkillManifest> {
        let fm: SkillFrontmatter = serde_yaml::from_str(frontmatter)?;

        Ok(super::SkillManifest {
            name: fm.name,
            description: fm.description,
            version: fm.version,
            author: fm.author,
            tags: fm.tags,
            always: false,
            tools: Vec::new(),
            prompts: super::SkillPrompts::default(),
            dependencies: Vec::new(),
        })
    }
}
