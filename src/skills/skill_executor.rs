//! LLM-driven skill executor
//!
//! This module implements an LLM-driven approach to skill execution.
//! Instead of parsing SKILL.md into structured steps, we let the LLM
//! read the full SKILL.md markdown and execute commands iteratively.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::config::VictoriametricsConfig;
use crate::llm::traits::LLMProvider;
use crate::llm::types::Usage;
use crate::skills::SkillExecutor;
use crate::utils::string::utf8_prefix_chars;

/// If `command` starts with `kubectl`, inject `--context` after the binary name.
pub fn apply_kubectl_context(command: &str, context: Option<&str>) -> String {
    let Some(ctx) = context.filter(|c| !c.is_empty()) else {
        return command.to_string();
    };
    let trimmed = command.trim_start();
    if let Some(rest) = trimmed.strip_prefix("kubectl ") {
        return format!("kubectl --context={ctx} {rest}");
    }
    if trimmed == "kubectl" {
        return format!("kubectl --context={ctx}");
    }
    command.to_string()
}

/// Prompt for LLM to decide next action given skill and context
const SKILL_EXECUTION_PROMPT: &str = r#"You are an operations diagnostics expert. Use the skill content below to help the user diagnose the problem.

## Skill Content
{{skill_content}}

## User Question
{{user_query}}

## Executed Commands and Results
{{executed_commands}}

## Current Status
{{current_status}}

Analyze the information above and decide the next step:

1. If diagnosis is not complete, choose an appropriate command to execute (from the Skill or construct one based on the situation)
2. If enough information has been collected, provide the final diagnosis

Return JSON directly:
{
  "next_command": "kubectl describe pod xxx -n yyy",  // or null when diagnosis is complete
  "reasoning": "Because the results show..., need to further check...",
  "diagnosis": null,  // fill in diagnosis conclusion when complete
  "recommendations": ["recommendation 1", "recommendation 2"]  // fill in when complete
}

Important:
- Commands should be chosen from those provided in the Skill, or construct reasonable kubectl commands based on the situation
- If the Skill contains conditional logic, evaluate conditions based on command results
- Execute at most {{max_steps}} steps; if still unclear, provide your best judgment
- If kubectl/helm commands fail repeatedly (likely missing cluster credentials), **stop retrying**; ask the user to start the program with **`AICLAW_KUBECONFIG=<absolute_path>`**. If the user already specified a kubeconfig path in their question, **do not ask again**."#;

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
    /// Cumulative LLM token usage for the skill loop (each `chat` call).
    pub llm_usage: Usage,
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
    provider: Arc<dyn LLMProvider>,
    skill_executor: Arc<SkillExecutor>,
    max_steps: usize,
    shell_timeout: Duration,
    /// VictoriaMetrics connection settings injected as env vars into shell commands.
    vm_config: VictoriametricsConfig,
}

impl LLMSkillExecutor {
    pub fn new(
        provider: Arc<dyn LLMProvider>,
        skill_executor: Arc<SkillExecutor>,
        max_steps: usize,
        shell_timeout: Duration,
    ) -> Self {
        Self {
            provider,
            skill_executor,
            max_steps: max_steps.max(1),
            shell_timeout,
            vm_config: VictoriametricsConfig::default(),
        }
    }

    /// Create with VictoriaMetrics config for env var injection.
    pub fn with_vm_config(
        provider: Arc<dyn LLMProvider>,
        skill_executor: Arc<SkillExecutor>,
        max_steps: usize,
        shell_timeout: Duration,
        vm_config: VictoriametricsConfig,
    ) -> Self {
        Self {
            provider,
            skill_executor,
            max_steps: max_steps.max(1),
            shell_timeout,
            vm_config,
        }
    }

    /// Execute a skill using LLM-driven approach
    pub async fn execute_skill(
        &self,
        skill_content: &str,
        user_query: &str,
        params: &HashMap<String, String>,
        kubectl_context: Option<&str>,
        kubeconfig: Option<&Path>,
    ) -> anyhow::Result<SkillExecutionResult> {
        info!("Starting LLM-driven skill execution for query: {}", user_query);

        let mut execution_history: Vec<CommandRecord> = Vec::new();
        let mut current_status = "等待开始诊断".to_string();
        let mut llm_usage = Usage::zero();

        // Prompt is rebuilt each iteration so the LLM sees execution history and status.
        let mut prompt = Self::build_prompt(
            skill_content,
            user_query,
            &execution_history,
            &current_status,
            self.max_steps,
        );

        // Iteratively execute until diagnosis or max steps
        for step in 0..self.max_steps {
            debug!(
                "Execution step {} of {}",
                step + 1,
                self.max_steps
            );

            // Call LLM to decide next action
            let llm_response = self
                .provider
                .chat(
                    vec![
                        crate::llm::types::ChatMessage::system(SYSTEM_PROMPT),
                        crate::llm::types::ChatMessage::user(&prompt),
                    ],
                    None,
                )
                .await?;

            llm_usage.merge_assign(&llm_response.usage);

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
                    llm_usage,
                });
            }

            // Execute the command
            let command = next_command.unwrap();
            let command = apply_kubectl_context(&command, kubectl_context);
            info!(event = "skill_exec_command", command = %command, "executing shell from LLM skill loop");

            let (output, success) = self.execute_shell_command(&command, kubeconfig).await;

            let record = CommandRecord {
                command: command.clone(),
                output: output.clone(),
                success,
            };
            execution_history.push(record);

            // Update status
            let out_preview = utf8_prefix_chars(&output, 200);
            current_status = if success {
                format!("命令执行成功: {}", out_preview)
            } else {
                format!("命令执行失败: {}", out_preview)
            };

            prompt = Self::build_prompt(
                skill_content,
                user_query,
                &execution_history,
                &current_status,
                self.max_steps,
            );
        }

        // Max steps reached
        let diagnosis = format!(
            "诊断步骤已达上限 ({} 步)。当前状态：{}\n\n请人工进一步排查。",
            self.max_steps, current_status
        );

        let output = Self::format_output(&diagnosis, &vec![], &execution_history);

        Ok(SkillExecutionResult {
            success: false,
            diagnosis: Some(diagnosis),
            recommendations: vec!["请人工进一步排查".to_string()],
            execution_history,
            output,
            llm_usage,
        })
    }

    /// Build prompt for LLM
    fn build_prompt(
        skill_content: &str,
        user_query: &str,
        execution_history: &[CommandRecord],
        current_status: &str,
        max_steps: usize,
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

        SKILL_EXECUTION_PROMPT
            .replace("{{skill_content}}", skill_content)
            .replace("{{user_query}}", user_query)
            .replace("{{executed_commands}}", &executed_commands)
            .replace("{{current_status}}", current_status)
            .replace("{{max_steps}}", &max_steps.to_string())
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
    async fn execute_shell_command(&self, command: &str, kubeconfig: Option<&Path>) -> (String, bool) {
        // Build env vars: VM connection settings for VictoriaMetrics skill curls.
        let mut tool_env: HashMap<String, String> = HashMap::new();
        if let Some(ref url) = self.vm_config.vm_metrics_url {
            tool_env.insert("VM_METRICS_URL".to_string(), url.clone());
        }
        if let Some(ref url) = self.vm_config.vm_logs_url {
            tool_env.insert("VM_LOGS_URL".to_string(), url.clone());
        }
        if let Some(ref header) = self.vm_config.vm_auth_header {
            tool_env.insert("VM_AUTH_HEADER".to_string(), header.clone());
        }
        if let Some(ref ak) = self.vm_config.vm_ak {
            tool_env.insert("VM_AK".to_string(), ak.clone());
        }
        if let Some(ref sk) = self.vm_config.vm_sk {
            tool_env.insert("VM_SK".to_string(), sk.clone());
        }

        // Use the skill executor to run the command
        let tool = aiclaw_types::skill::SkillTool {
            name: "shell_command".to_string(),
            description: "Shell command".to_string(),
            kind: aiclaw_types::skill::ToolKind::Shell,
            command: command.to_string(),
            args: HashMap::new(),
            env: tool_env,
            timeout_secs: Some(self.shell_timeout.as_secs()),
        };

        match self
            .skill_executor
            .execute_tool(&tool, &HashMap::new(), kubeconfig)
            .await
        {
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
            let end = content[start..].find("```");
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
            for (byte_idx, c) in content.char_indices() {
                if c == '{' {
                    depth += 1;
                } else if c == '}' {
                    depth -= 1;
                    if depth == 0 {
                        return Some(content[..=byte_idx].to_string());
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
                let summary = {
                    let s = utf8_prefix_chars(&record.output, 50);
                    if record.output.chars().count() > 50 {
                        format!("{}...", s)
                    } else {
                        s.to_string()
                    }
                }
                .replace("\n", " ");
                output += &format!("| `{}` | {} | {} |\n", record.command, status, summary);
            }
        }

        output
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
- 如果 SKILL 中有条件判断，根据结果判断条件是否满足
- 集群访问依赖进程环境变量 **`AICLAW_KUBECONFIG`**（本机 kubeconfig 绝对路径）；**不要**引导用户去设置通用环境变量 **`KUBECONFIG`**。若用户已在问题里给出 kubeconfig 路径，视为已提供，**不要重复索要**；若命令输出显示无法连接集群且当前没有可用 kubeconfig，**停止盲跑 kubectl**，请用户用 **`AICLAW_KUBECONFIG=<绝对路径>`** 重启本进程（或在本对话中再次发送该路径以便会话记录）。"#;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::traits::LLMProvider;
    use crate::llm::types::{ChatMessage, ChatOptions, ChatResponse, Usage};
    use crate::security::command_validator::CommandValidator;
    use async_trait::async_trait;
    use std::sync::Mutex;

    #[test]
    fn apply_kubectl_context_inserts_flag() {
        assert_eq!(
            apply_kubectl_context("kubectl get pods", Some("prod")),
            "kubectl --context=prod get pods"
        );
        assert_eq!(
            apply_kubectl_context("  kubectl get ns", Some("c1")),
            "kubectl --context=c1 get ns"
        );
    }

    struct QueuedLlm {
        queue: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl LLMProvider for QueuedLlm {
        fn name(&self) -> &str {
            "queued-mock"
        }

        fn provider_type(&self) -> &str {
            "mock"
        }

        async fn chat(
            &self,
            _messages: Vec<ChatMessage>,
            _options: Option<ChatOptions>,
        ) -> anyhow::Result<ChatResponse> {
            let s = self
                .queue
                .lock()
                .map_err(|e| anyhow::anyhow!("lock: {}", e))?
                .remove(0);
            Ok(ChatResponse {
                content: s,
                model: "mock".to_string(),
                provider: "mock".to_string(),
                usage: Usage::zero(),
                raw_response: serde_json::Value::Null,
            })
        }

        async fn health_check(&self) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn llm_skill_loop_respects_deny_all_validator() {
        let responses = vec![
            r#"{"next_command":"kubectl get pods","reasoning":"probe","diagnosis":null,"recommendations":[]}"#
                .to_string(),
            r#"{"next_command":null,"reasoning":"done","diagnosis":"finished","recommendations":["check RBAC"]}"#
                .to_string(),
        ];
        let provider = Arc::new(QueuedLlm {
            queue: Mutex::new(responses),
        });
        let v = Arc::new(CommandValidator::deny_all());
        let se = Arc::new(
            SkillExecutor::with_validator(v).with_timeout(std::time::Duration::from_secs(5)),
        );
        let ex = LLMSkillExecutor::new(
            provider,
            se,
            5,
            std::time::Duration::from_secs(5),
        );
        let res = ex
            .execute_skill("# guide", "hi", &HashMap::new(), None, None)
            .await
            .expect("skill exec");
        assert_eq!(res.execution_history.len(), 1);
        assert!(!res.execution_history[0].success);
        assert_eq!(res.diagnosis.as_deref(), Some("finished"));
    }
}
