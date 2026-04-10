//! LLM-driven skill executor
//!
//! This module implements an LLM-driven approach to skill execution.
//! Instead of parsing SKILL.md into structured steps, we let the LLM
//! read the full SKILL.md markdown and execute commands iteratively.

use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::llm::traits::LLMProvider;
use crate::skills::SkillExecutor;

/// Maximum iterations per skill execution
const MAX_EXECUTION_STEPS: usize = 10;

/// Prompt for LLM to decide next action given skill and context
const SKILL_EXECUTION_PROMPT: &str = r#"你是运维诊断专家。请根据以下 Skill 的内容，帮助用户诊断问题。

## Skill 内容
{{skill_content}}

## 用户问题
{{user_query}}

## 已执行的命令和结果
{{executed_commands}}

## 当前状态
{{current_status}}

请分析以上信息，决定下一步：

1. 如果诊断尚未完成，选择一个合适的命令执行（从 Skill 中选择或根据实际情况构造）
2. 如果已收集到足够信息，给出最终诊断结论

直接返回 JSON 格式：
{
  "next_command": "kubectl describe pod xxx -n yyy",  // 或 null 表示诊断完成
  "reasoning": "因为结果显示...，需要进一步检查...",
  "diagnosis": null,  // 如果诊断完成，填写诊断结论
  "recommendations": ["建议1", "建议2"]  // 如果诊断完成，填写建议
}

重要：
- 命令必须从 Skill 中提供的命令中选择，或根据实际情况构造合理的 kubectl 命令
- 如果 Skill 包含条件判断，根据命令结果判断条件是否满足
- 最多执行 {{max_steps}} 步，如果仍未诊断清楚，给出当前最佳判断"#;

/// Result of skill execution
#[derive(Debug)]
pub struct SkillExecutionResult {
    /// Whether execution was successful
    pub success: bool,
    /// Final diagnosis (if completed)
    pub diagnosis: Option<String>,
    /// Recommendations
    pub recommendations: Vec<String>,
    /// All executed commands and their results
    pub execution_history: Vec<CommandRecord>,
    /// Final output to show user
    pub output: String,
}

/// Record of a single command execution
#[derive(Debug, Clone)]
pub struct CommandRecord {
    pub command: String,
    pub output: String,
    pub success: bool,
}

/// LLM-driven skill executor
pub struct LLMSkillExecutor {
    skill_executor: Arc<SkillExecutor>,
}

impl LLMSkillExecutor {
    pub fn new(skill_executor: Arc<SkillExecutor>) -> Self {
        Self { skill_executor }
    }

    /// Execute a skill using LLM-driven approach
    pub async fn execute_skill(
        &self,
        skill_content: &str,
        user_query: &str,
        params: &HashMap<String, String>,
    ) -> anyhow::Result<SkillExecutionResult> {
        info!("Starting LLM-driven skill execution for query: {}", user_query);

        let mut execution_history: Vec<CommandRecord> = Vec::new();
        let mut current_status = "等待开始诊断".to_string();

        // Prompt is rebuilt each iteration so the LLM sees execution history and status.
        let mut prompt = Self::build_prompt(
            skill_content,
            user_query,
            &execution_history,
            &current_status,
        );

        // Get LLM provider from somewhere - this should be injected
        // For now, we'll use a placeholder that gets resolved later
        let provider = self.get_llm_provider()?;

        // Iteratively execute until diagnosis or max steps
        for step in 0..MAX_EXECUTION_STEPS {
            debug!("Execution step {} of {}", step + 1, MAX_EXECUTION_STEPS);

            // Call LLM to decide next action
            let llm_response = provider
                .chat(
                    vec![
                        crate::llm::types::ChatMessage::system(SYSTEM_PROMPT),
                        crate::llm::types::ChatMessage::user(&prompt),
                    ],
                    None,
                )
                .await?;

            // Parse LLM response
            let action: SkillAction = match serde_json::from_str(&llm_response.content) {
                Ok(a) => a,
                Err(e) => {
                    warn!("Failed to parse LLM response as JSON: {}", e);
                    // Try to extract JSON from response
                    if let Some(json_str) = Self::extract_json_from_response(&llm_response.content) {
                        serde_json::from_str(&json_str).unwrap_or(SkillAction {
                            next_command: None,
                            reasoning: "解析失败".to_string(),
                            diagnosis: Some(format!("LLM 响应格式错误: {}", llm_response.content)),
                            recommendations: vec![],
                        })
                    } else {
                        SkillAction {
                            next_command: None,
                            reasoning: "解析失败".to_string(),
                            diagnosis: Some(format!("无法理解 LLM 响应: {}", llm_response.content)),
                            recommendations: vec![],
                        }
                    }
                }
            };

            // If no next command, diagnosis is complete
            let next_command = action.next_command.as_ref().map(|c| {
                // Interpolate parameters into command
                Self::interpolate_command(c, params)
            });

            if next_command.is_none() || action.diagnosis.is_some() {
                // Diagnosis complete
                let diagnosis = action.diagnosis.unwrap_or_else(|| {
                    format!(
                        "诊断未能完全确定。当前状态：{}\n\n执行了 {} 步。",
                        current_status,
                        execution_history.len()
                    )
                });

                let output = Self::format_output(&diagnosis, &action.recommendations, &execution_history);

                return Ok(SkillExecutionResult {
                    success: !execution_history.is_empty(),
                    diagnosis: Some(diagnosis),
                    recommendations: action.recommendations,
                    execution_history,
                    output,
                });
            }

            // Execute the command
            let command = next_command.unwrap();
            info!("Executing command: {}", command);

            let (output, success) = self.execute_shell_command(&command).await;

            let record = CommandRecord {
                command: command.clone(),
                output: output.clone(),
                success,
            };
            execution_history.push(record);

            // Update status
            current_status = if success {
                format!("命令执行成功: {}", &output[..output.len().min(200)])
            } else {
                format!("命令执行失败: {}", &output[..output.len().min(200)])
            };

            prompt = Self::build_prompt(
                skill_content,
                user_query,
                &execution_history,
                &current_status,
            );
        }

        // Max steps reached
        let diagnosis = format!(
            "诊断步骤已达上限 ({} 步)。当前状态：{}\n\n请人工进一步排查。",
            MAX_EXECUTION_STEPS, current_status
        );

        let output = Self::format_output(&diagnosis, &vec![], &execution_history);

        Ok(SkillExecutionResult {
            success: false,
            diagnosis: Some(diagnosis),
            recommendations: vec!["请人工进一步排查".to_string()],
            execution_history,
            output,
        })
    }

    /// Build prompt for LLM
    fn build_prompt(
        skill_content: &str,
        user_query: &str,
        execution_history: &[CommandRecord],
        current_status: &str,
    ) -> String {
        let executed_commands = if execution_history.is_empty() {
            "（尚无已执行的命令）".to_string()
        } else {
            execution_history
                .iter()
                .map(|r| format!("命令: {}\n结果: {}\n状态: {}\n", r.command, r.output, if r.success { "成功" } else { "失败" }))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let prompt = SKILL_EXECUTION_PROMPT
            .replace("{{skill_content}}", skill_content)
            .replace("{{user_query}}", user_query)
            .replace("{{executed_commands}}", &executed_commands)
            .replace("{{current_status}}", current_status)
            .replace("{{max_steps}}", &MAX_EXECUTION_STEPS.to_string());

        prompt
    }

    /// Interpolate parameters into command
    fn interpolate_command(command: &str, params: &HashMap<String, String>) -> String {
        let mut result = command.to_string();
        for (key, value) in params {
            let placeholder = format!("{{{{{}}}}}", key);
            result = result.replace(&placeholder, value);
        }
        result
    }

    /// Execute a shell command
    async fn execute_shell_command(&self, command: &str) -> (String, bool) {
        // Use the skill executor to run the command
        let tool = aiclaw_types::skill::SkillTool {
            name: "shell_command".to_string(),
            description: "Shell command".to_string(),
            kind: aiclaw_types::skill::ToolKind::Shell,
            command: command.to_string(),
            args: HashMap::new(),
            env: HashMap::new(),
            timeout_secs: Some(60),
        };

        match self.skill_executor.execute_tool(&tool, &HashMap::new()).await {
            Ok(result) => {
                if result.success {
                    (result.output.unwrap_or_default(), true)
                } else {
                    (result.error.unwrap_or_default(), false)
                }
            }
            Err(e) => (e.to_string(), false),
        }
    }

    /// Extract JSON from LLM response
    fn extract_json_from_response(content: &str) -> Option<String> {
        let content = content.trim();

        // Check for markdown code block
        if content.contains("```json") {
            let start = content.find("```json").unwrap() + 7;
            let end = content[start..].find("```").map(|i| start + i);
            return end.map(|e| content[start..e].trim().to_string());
        }

        if content.contains("```") {
            let start = content.find("```").unwrap() + 3;
            let end = content[start..].find("```").map(|i| start + i);
            return end.map(|e| content[start..e].trim().to_string());
        }

        // Try to find JSON directly
        if content.starts_with('{') {
            let mut depth = 0;
            for (i, c) in content.chars().enumerate() {
                if c == '{' {
                    depth += 1;
                } else if c == '}' {
                    depth -= 1;
                    if depth == 0 {
                        return Some(content[..=i].to_string());
                    }
                }
            }
        }

        None
    }

    /// Format final output
    fn format_output(
        diagnosis: &str,
        recommendations: &[String],
        history: &[CommandRecord],
    ) -> String {
        let mut output = String::from("## 诊断结果\n\n");
        output += &format!("{}\n\n", diagnosis);

        if !recommendations.is_empty() {
            output += "## 建议\n\n";
            for (i, rec) in recommendations.iter().enumerate() {
                output += &format!("{}. {}\n", i + 1, rec);
            }
            output += "\n";
        }

        if !history.is_empty() {
            output += "## 执行详情\n\n";
            output += "| 命令 | 状态 | 结果摘要 |\n";
            output += "|------|------|----------|\n";
            for record in history {
                let status = if record.success { "✅" } else { "❌" };
                let summary = if record.output.len() > 50 {
                    format!("{}...", &record.output[..50])
                } else {
                    record.output.clone()
                }.replace("\n", " ");
                output += &format!("| `{}` | {} | {} |\n", record.command, status, summary);
            }
        }

        output
    }

    /// Get LLM provider (placeholder - should be injected)
    fn get_llm_provider(&self) -> anyhow::Result<Arc<dyn LLMProvider>> {
        // This is a temporary solution - in practice, the provider should be injected
        // For now, we return an error indicating this needs to be implemented
        anyhow::bail!("LLM provider not configured for skill executor. Please inject LLM provider.")
    }
}

/// System prompt for skill execution
const SYSTEM_PROMPT: &str = r#"你是一个运维诊断专家。你会收到一个 SKILL.md 格式的诊断指南和用户问题。

你需要：
1. 仔细阅读 SKILL.md 中的诊断流程
2. 根据用户问题，执行相应的诊断命令
3. 分析命令结果，决定下一步
4. 直到有足够信息给出诊断结论

对于每个步骤，返回一个 JSON 对象，包含：
- next_command: 要执行的命令（从 SKILL 中选择），如果诊断完成则为 null
- reasoning: 为什么执行这个命令
- diagnosis: 诊断结论（如果诊断完成）
- recommendations: 建议（如果诊断完成）

重要：
- 只返回 JSON，不要有其他内容
- 命令必须是从 SKILL 中选择的，或合理构造的 kubectl 命令
- 如果 SKILL 中有条件判断，根据结果判断条件是否满足"#;

/// Action to take from LLM
#[derive(Debug, serde::Deserialize)]
struct SkillAction {
    #[serde(default)]
    pub next_command: Option<String>,
    #[allow(dead_code)]
    pub reasoning: String,
    #[serde(default)]
    pub diagnosis: Option<String>,
    #[serde(default)]
    pub recommendations: Vec<String>,
}
