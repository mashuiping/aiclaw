//! Skill loader - loads skills from filesystem

use aiclaw_types::skill::SkillMetadata;
use std::path::{Path, PathBuf};
use tracing::{debug, error, info, warn};

use super::kubeconfig_hint::expand_path;

/// Skill loader - scans directories and loads skill manifests
pub struct SkillLoader {
    skills_dir: PathBuf,
}

impl SkillLoader {
    pub fn new(skills_dir: impl Into<PathBuf>) -> Self {
        let raw: PathBuf = skills_dir.into();
        Self {
            skills_dir: expand_path(raw.to_string_lossy().as_ref()),
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

    /// Load a single skill from a directory (`SKILL.md` with YAML frontmatter only).
    pub fn load_skill_from_dir(&self, dir: &Path) -> anyhow::Result<SkillMetadata> {
        let md_path = dir.join("SKILL.md");

        if md_path.exists() {
            self.load_from_md(&md_path)
        } else {
            anyhow::bail!("No SKILL.md found in {:?}", dir);
        }
    }

    /// Load skill from Markdown with frontmatter
    fn load_from_md(&self, path: &Path) -> anyhow::Result<SkillMetadata> {
        let content = std::fs::read_to_string(path)?;

        // Parse frontmatter
        let (frontmatter, markdown_body) = extract_frontmatter(&content)
            .ok_or_else(|| anyhow::anyhow!("No frontmatter found in {:?}", path))?;

        let fm: SkillFrontmatter = serde_yaml::from_str(&frontmatter)
            .map_err(|e| anyhow::anyhow!("Failed to parse frontmatter: {}", e))?;

        // Extract applicability scenarios from markdown body
        let applicability = extract_applicability(&markdown_body);

        // Extract domain tags from description and markdown
        let domain_tags = extract_domain_tags(&fm.description, &markdown_body);

        Ok(SkillMetadata {
            name: fm.name,
            description: fm.description,
            version: fm.version,
            author: fm.author,
            tags: fm.tags,
            always: false,
            raw_content: content,
            applicability,
            domain_tags,
            tools: Vec::new(),
        })
    }
}

/// Extract frontmatter and body from markdown
fn extract_frontmatter(content: &str) -> Option<(String, String)> {
    let lines = content.lines();
    let mut frontmatter_lines = Vec::new();
    let mut body_lines = Vec::new();
    let mut in_frontmatter = false;
    let mut found_frontmatter = false;
    let mut past_frontmatter = false;

    for line in lines {
        if line.trim() == "---" {
            if in_frontmatter {
                found_frontmatter = true;
                in_frontmatter = false;
                past_frontmatter = true;
                continue;
            } else if !past_frontmatter {
                in_frontmatter = true;
                continue;
            }
        }

        if in_frontmatter {
            frontmatter_lines.push(line);
        } else if past_frontmatter {
            body_lines.push(line);
        }
    }

    if found_frontmatter {
        Some((frontmatter_lines.join("\n"), body_lines.join("\n")))
    } else {
        None
    }
}

/// Extract applicability scenarios from markdown body
/// Looks for "## 适用场景" or "## Applicable Scenarios" section
fn extract_applicability(body: &str) -> Vec<String> {
    let mut scenarios = Vec::new();
    let mut in_section = false;

    for line in body.lines() {
        let trimmed = line.trim();

        // Check for section header
        if trimmed == "## 适用场景" || trimmed.contains("适用场景") {
            in_section = true;
            continue;
        }

        // End of section (next ## heading)
        if in_section && trimmed.starts_with("##") {
            break;
        }

        // Extract bullet points
        if in_section && (trimmed.starts_with("- ") || trimmed.starts_with("* ")) {
            // Remove the bullet and trim
            let scenario = trimmed[2..].trim().to_string();
            if !scenario.is_empty() {
                scenarios.push(scenario);
            }
        }
    }

    scenarios
}

/// Extract domain-specific tags from description and markdown
/// Recognizes: hami, gpu, vgpu, apisix, coredns, ingress, prometheus, etc.
fn extract_domain_tags(description: &str, body: &str) -> Vec<String> {
    let mut tags = Vec::new();

    let domain_keywords = [
        "hami", "gpu", "vgpu", "nvidia", "cuda",
        "apisix", "apigw", "gateway", "ingress", "nginx",
        "coredns", "dns", "kubelet", "kubernetes", "k8s",
        "prometheus", "victoriametrics", "vm", "metrics",
        "storage", "pvc", "ceph", "nfs",
        "network", "cilium", "istio", "linkerd",
        "apisix", "openresty",
        "oom", "memory", "crashloop",
    ];

    let text = format!("{} {}", description.to_lowercase(), body.to_lowercase());

    for keyword in &domain_keywords {
        if text.contains(keyword) && !tags.contains(&keyword.to_string()) {
            tags.push(keyword.to_string());
        }
    }

    tags
}

/// Frontmatter structure for SKILL.md
#[derive(Debug, serde::Deserialize)]
struct SkillFrontmatter {
    name: String,
    description: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
}
