//! Execution planner - plans what queries to execute based on user intent

use std::sync::Arc;
use tracing::debug;

use aiclaw_types::skill::SkillMetadata;
use crate::llm::traits::LLMProvider;
use crate::llm::types::{ChatMessage, ChatOptions, Usage};
use super::prompt_builder::PromptBuilder;

/// Execution plan - defines what queries to run
#[derive(Debug, Clone)]
pub struct ExecutionPlan {
    pub steps: Vec<PlanStep>,
    pub reasoning: String,
}

#[derive(Debug, Clone)]
pub struct PlanStep {
    pub step_id: usize,
    pub description: String,
    /// The actual shell command the LLM decided to run (e.g. `kubectl get pod ...`).
    pub command: String,
    pub parameters: Vec<QueryParameter>,
}

#[derive(Debug, Clone)]
pub struct QueryParameter {
    pub name: String,
    pub value: String,
}

/// Planner - creates execution plans for complex queries
pub struct Planner {
    provider: Arc<dyn LLMProvider>,
    prompt_builder: PromptBuilder,
}

impl Planner {
    pub fn new(provider: Arc<dyn LLMProvider>) -> Self {
        Self {
            provider,
            prompt_builder: PromptBuilder::new(),
        }
    }

    /// Create an execution plan from user intent, dynamically incorporating
    /// relevant skills and tools into the prompt.
    pub async fn plan(
        &self,
        user_query: &str,
        intent_type: &str,
        matched_skills: &[&SkillMetadata],
    ) -> anyhow::Result<(ExecutionPlan, Usage)> {
        debug!("Creating execution plan for: {}", user_query);

        // Collect tools from matched skills
        let tools: Vec<&aiclaw_types::skill::SkillTool> = matched_skills
            .iter()
            .flat_map(|s| s.tools.iter())
            .collect();

        let system_prompt = self.prompt_builder.build_planner_prompt(matched_skills, &tools);

        let user_prompt = format!(
            "User request: {}\nIntent type: {}\n\nAnalyze the problem and plan diagnostic steps. Return JSON directly.",
            user_query, intent_type
        );

        let messages = vec![
            ChatMessage::system(&system_prompt),
            ChatMessage::user(&user_prompt),
        ];

        let options = ChatOptions::new()
            .with_temperature(0.1)
            .with_max_tokens(1024);

        let response = self.provider.chat(messages, Some(options)).await?;
        let usage = response.usage.clone();
        let plan = self.parse_plan(&response.content).await?;
        Ok((plan, usage))
    }

    async fn parse_plan(&self, response: &str) -> anyhow::Result<ExecutionPlan> {
        let json_str = extract_json(response)
            .ok_or_else(|| anyhow::anyhow!("Failed to extract JSON from planner response"))?;

        #[derive(serde::Deserialize)]
        struct RawPlan {
            reasoning: String,
            steps: Vec<RawStep>,
        }

        #[derive(serde::Deserialize)]
        struct RawStep {
            description: String,
            command: String,
            #[serde(default)]
            parameters: Vec<RawParam>,
        }

        #[derive(serde::Deserialize)]
        struct RawParam {
            name: String,
            value: String,
        }

        let raw: RawPlan = serde_json::from_str(&json_str)
            .map_err(|e| anyhow::anyhow!("Failed to parse plan JSON: {} - response: {}", e, json_str))?;

        let steps: Vec<PlanStep> = raw
            .steps
            .into_iter()
            .enumerate()
            .map(|(i, s)| {
                let parameters = s
                    .parameters
                    .into_iter()
                    .map(|p| QueryParameter { name: p.name, value: p.value })
                    .collect();

                PlanStep {
                    step_id: i + 1,
                    description: s.description,
                    command: s.command,
                    parameters,
                }
            })
            .collect();

        Ok(ExecutionPlan {
            steps,
            reasoning: raw.reasoning,
        })
    }
}

/// Extract JSON from a response that might have markdown formatting
fn extract_json(response: &str) -> Option<String> {
    let response = response.trim();

    // Check for markdown code block
    if response.contains("```json") {
        let start = response.find("```json").unwrap() + 7;
        let end = response[start..].find("```").map(|i| start + i);
        return end.map(|e| response[start..e].trim().to_string());
    }

    if response.contains("```") {
        let start = response.find("```").unwrap() + 3;
        let end = response[start..].find("```").map(|i| start + i);
        return end.map(|e| response[start..e].trim().to_string());
    }

    // Try to find JSON directly
    if let Some(start) = response.find('{') {
        let remaining = &response[start..];
        let mut depth = 0;
        for (byte_idx, c) in remaining.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(remaining[..=byte_idx].to_string());
                    }
                }
                _ => {}
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json() {
        // Plain JSON
        let json = r#"{"reasoning": "test", "steps": []}"#;
        assert!(extract_json(json).is_some());

        // JSON in code block
        let wrapped = "```json\n{\"reasoning\": \"test\", \"steps\": []}\n```";
        assert!(extract_json(wrapped).is_some());
    }
}
