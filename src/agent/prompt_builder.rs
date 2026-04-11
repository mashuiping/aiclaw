//! Dynamic prompt builder: assembles system prompts from skills, tools, and context.
//!
//! Instead of hardcoding diagnostic steps, the prompt builder injects available
//! skill knowledge and tool descriptions so the LLM can determine the optimal
//! approach based on the user's query and available capabilities.

use aiclaw_types::skill::{SkillMetadata, SkillTool};

const MAX_SKILL_CONTENT_CHARS: usize = 2_000;
const MAX_TOTAL_SKILL_CHARS: usize = 12_000;

/// Dynamic prompt builder that assembles system prompts from available context.
pub struct PromptBuilder {
    identity: String,
    #[allow(dead_code)]
    capabilities: String,
}

impl PromptBuilder {
    pub fn new() -> Self {
        Self {
            identity: DEFAULT_IDENTITY.to_string(),
            capabilities: DEFAULT_CAPABILITIES.to_string(),
        }
    }

    pub fn with_identity(mut self, identity: impl Into<String>) -> Self {
        self.identity = identity.into();
        self
    }

    /// Build a system prompt for the planner, injecting matched skills and tools.
    pub fn build_planner_prompt(
        &self,
        skills: &[&SkillMetadata],
        tools: &[&SkillTool],
    ) -> String {
        let mut prompt = String::with_capacity(4096);
        prompt.push_str(&self.identity);
        prompt.push_str("\n\n");
        prompt.push_str(PLANNER_ROLE);

        if !skills.is_empty() {
            prompt.push_str("\n\n## Available Diagnostic Knowledge\n\n");
            let mut total_chars = 0;
            for skill in skills {
                if total_chars >= MAX_TOTAL_SKILL_CHARS {
                    prompt.push_str("\n(Additional skills omitted due to context budget)\n");
                    break;
                }
                prompt.push_str(&format!("### {}\n", skill.name));
                if !skill.description.is_empty() {
                    prompt.push_str(&format!("{}\n", skill.description));
                }
                if !skill.raw_content.is_empty() {
                    let content = truncate_skill_content(&skill.raw_content, MAX_SKILL_CONTENT_CHARS);
                    prompt.push_str(&content);
                    prompt.push('\n');
                    total_chars += content.len();
                }
                if !skill.tools.is_empty() {
                    prompt.push_str("Available commands:\n");
                    for tool in &skill.tools {
                        prompt.push_str(&format!("- `{}`: {}\n", tool.command, tool.description));
                    }
                }
                prompt.push('\n');
            }
        }

        if !tools.is_empty() {
            prompt.push_str("\n## Available Tools\n\n");
            for tool in tools {
                prompt.push_str(&format!("- **{}** ({}): {}\n", tool.name, format_tool_kind(&tool.kind), tool.description));
            }
        }

        prompt.push_str(PLANNER_INSTRUCTIONS);
        prompt
    }

    /// Build a system prompt for the summarizer, without hardcoded analysis steps.
    pub fn build_summarizer_prompt(
        &self,
        skills: &[&SkillMetadata],
    ) -> String {
        let mut prompt = String::with_capacity(2048);
        prompt.push_str(&self.identity);
        prompt.push_str("\n\n");
        prompt.push_str(SUMMARIZER_ROLE);

        if !skills.is_empty() {
            prompt.push_str("\n\n## Domain Knowledge\n\n");
            let mut total_chars = 0;
            for skill in skills {
                if total_chars >= MAX_TOTAL_SKILL_CHARS / 2 {
                    break;
                }
                if !skill.description.is_empty() {
                    let line = format!("- **{}**: {}\n", skill.name, skill.description);
                    prompt.push_str(&line);
                    total_chars += line.len();
                }
            }
        }

        prompt.push_str(SUMMARIZER_INSTRUCTIONS);
        prompt
    }

    /// Build system prompt for the REPL with skill context.
    pub fn build_repl_prompt(
        &self,
        skills: &[&SkillMetadata],
    ) -> String {
        let mut prompt = String::with_capacity(4096);
        prompt.push_str(&self.identity);
        prompt.push_str("\n\n");
        prompt.push_str(REPL_ROLE);

        if !skills.is_empty() {
            prompt.push_str("\n\n## Loaded Skills\n\n");
            prompt.push_str("You have knowledge from these diagnostic skills:\n");
            for skill in skills {
                prompt.push_str(&format!("- **{}**: {}\n", skill.name, skill.description));
            }
        }

        prompt
    }
}

impl Default for PromptBuilder {
    fn default() -> Self {
        Self::new()
    }
}

fn truncate_skill_content(content: &str, max_chars: usize) -> String {
    let char_count = content.chars().count();
    if char_count <= max_chars {
        return content.to_string();
    }
    let boundary = content
        .char_indices()
        .nth(max_chars)
        .map(|(i, _)| i)
        .unwrap_or(content.len());
    format!("{}...\n(content truncated)", &content[..boundary])
}

fn format_tool_kind(kind: &aiclaw_types::skill::ToolKind) -> &'static str {
    match kind {
        aiclaw_types::skill::ToolKind::Shell => "shell",
        aiclaw_types::skill::ToolKind::Http => "http",
        aiclaw_types::skill::ToolKind::Script => "script",
    }
}

const DEFAULT_IDENTITY: &str = "\
You are AIClaw, an AI operations assistant specializing in Kubernetes diagnostics, \
infrastructure troubleshooting, and AIOps.";

const DEFAULT_CAPABILITIES: &str = "\
You can execute shell commands, query metrics and logs, inspect cluster resources, \
and provide root cause analysis with actionable remediation steps.";

const PLANNER_ROLE: &str = "\
You are a diagnostic planning expert. Your job is to analyze the user's problem \
and create an efficient investigation plan using available skills and tools.

Planning principles:
1. Start with broad status checks, then narrow to specific resources
2. Relevance: choose queries most likely to reveal the root cause
3. Efficiency: minimize the number of queries needed
4. Evidence chain: each step should build on previous findings

You should determine the investigation approach based on the user's query and \
the available skills/tools listed below, NOT from a fixed template.";

const PLANNER_INSTRUCTIONS: &str = r#"

## Output Format

Return a JSON object:
```json
{
    "reasoning": "Your analysis of the problem and investigation strategy",
    "steps": [
        {
            "description": "What this step investigates",
            "command": "kubectl get pod xxx -n yyy -o wide",
            "parameters": [
                {"name": "param_name", "value": "param_value_or_{{placeholder}}"}
            ]
        }
    ]
}
```

Guidelines:
- 1-5 steps depending on problem complexity
- The `command` field must be the actual shell command to run (kubectl, helm, curl, etc.)
- Use {{pod_name}}, {{namespace}} etc. as placeholders for unknown values
- Prefer commands from the available skills when applicable
- Order steps from broad investigation to specific diagnosis
"#;

const SUMMARIZER_ROLE: &str = "\
You are an operations data analyst. You transform raw tool outputs \
(logs, metrics, kubectl output, API responses) into clear, actionable summaries.";

const SUMMARIZER_INSTRUCTIONS: &str = r#"

## Output Requirements

- Use Chinese (中文) for the response
- Structure with Markdown headers, tables, and bullet points
- Highlight anomalies, errors, and warnings prominently
- Provide specific, actionable next steps
- When data permits, include before/after comparisons or trends
- Do NOT follow a rigid template; adapt your analysis structure to the data
"#;

const REPL_ROLE: &str = "\
You help users analyze cluster issues, debug pods, inspect GPU scheduling \
(HAMi/vGPU), check logs, and provide actionable remediation steps.

You have access to tools for running shell commands and reading files. \
Use them proactively to gather diagnostic data.

When diagnosing issues:
- Be systematic: gather data before concluding
- Show your reasoning at each step
- Run multiple commands in parallel when possible
- Format output clearly with Markdown tables and code blocks
- Provide actionable remediation steps";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_planner_prompt_without_skills() {
        let builder = PromptBuilder::new();
        let prompt = builder.build_planner_prompt(&[], &[]);
        assert!(prompt.contains("AIClaw"));
        assert!(prompt.contains("diagnostic planning"));
        assert!(prompt.contains("command"));
    }

    #[test]
    fn builds_planner_prompt_with_skills() {
        let skill = SkillMetadata {
            name: "k8s-health".to_string(),
            description: "Check K8s cluster health".to_string(),
            raw_content: "# Steps\n1. kubectl get nodes\n2. kubectl get pods".to_string(),
            ..Default::default()
        };
        let builder = PromptBuilder::new();
        let prompt = builder.build_planner_prompt(&[&skill], &[]);
        assert!(prompt.contains("k8s-health"));
        assert!(prompt.contains("kubectl get nodes"));
    }

    #[test]
    fn truncates_large_skill_content() {
        let content = "x".repeat(5000);
        let truncated = truncate_skill_content(&content, 100);
        assert!(truncated.len() < 200);
        assert!(truncated.contains("truncated"));
    }
}
