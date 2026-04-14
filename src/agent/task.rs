//! Task delegation: parallel diagnostic tasks within the orchestrator.
//!
//! Phase 1 of multi-agent: `TaskRunner` executes isolated diagnostic tasks
//! with their own output budget. The orchestrator can fan out multiple tasks
//! in parallel (e.g. "check logs" + "check metrics") via `tokio::join!`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, warn};

use aiclaw_types::agent::EvidenceRecord;
use chrono::Utc;

use super::output_budget::{self, OutputBudget};
use crate::llm::summarizer::ToolOutput;
use crate::skills::SkillExecutor;

/// A discrete diagnostic task that can run in isolation.
#[derive(Debug, Clone)]
pub struct DiagnosticTask {
    pub name: String,
    pub description: String,
    pub commands: Vec<String>,
    pub timeout: Duration,
}

/// Result of a completed task.
#[derive(Debug)]
pub struct TaskResult {
    pub task_name: String,
    pub tool_outputs: Vec<ToolOutput>,
    pub evidence: Vec<EvidenceRecord>,
    pub success: bool,
    pub duration: Duration,
}

/// Runs isolated diagnostic tasks with output budget constraints.
pub struct TaskRunner {
    skill_executor: Arc<SkillExecutor>,
    output_budget: OutputBudget,
}

impl TaskRunner {
    pub fn new(skill_executor: Arc<SkillExecutor>, output_budget: OutputBudget) -> Self {
        Self {
            skill_executor,
            output_budget,
        }
    }

    /// Run a single diagnostic task, executing its commands sequentially.
    pub async fn run(
        &self,
        task: &DiagnosticTask,
        kubeconfig: Option<&Path>,
        kubectl_ctx: Option<&str>,
    ) -> TaskResult {
        let start = std::time::Instant::now();
        let mut tool_outputs = Vec::new();
        let mut evidence = Vec::new();
        let mut overall_success = true;

        for (i, command) in task.commands.iter().enumerate() {
            let cmd = crate::skills::apply_kubectl_context(command, kubectl_ctx);
            debug!(task = %task.name, step = i + 1, command = %cmd, "Executing task command");

            let tool = aiclaw_types::skill::SkillTool {
                name: format!("{}_{}", task.name, i + 1),
                description: format!("{} step {}", task.description, i + 1),
                kind: aiclaw_types::skill::ToolKind::Shell,
                command: cmd.clone(),
                args: HashMap::new(),
                env: HashMap::new(),
                timeout_secs: Some(task.timeout.as_secs()),
            };

            let result = self
                .skill_executor
                .execute_tool(&tool, &HashMap::new(), kubeconfig)
                .await;

            let (output_text, success) = match result {
                Ok(r) => {
                    let text = if r.success {
                        r.output.unwrap_or_default()
                    } else {
                        r.error.unwrap_or_default()
                    };
                    (text, r.success)
                }
                Err(e) => {
                    warn!(task = %task.name, step = i + 1, error = %e, "Task command failed");
                    (format!("Error: {}", e), false)
                }
            };

            let truncated = output_budget::truncate_tool_output(&output_text, &self.output_budget);

            tool_outputs.push(ToolOutput::new(
                format!("{}_{}", task.name, i + 1),
                truncated.content,
                success,
            ));

            evidence.push(EvidenceRecord {
                timestamp: Utc::now(),
                source: "task".to_string(),
                action: format!("{}_{}", task.name, i + 1),
                data: serde_json::json!({
                    "command": cmd,
                    "success": success,
                }),
            });

            if !success {
                overall_success = false;
            }
        }

        TaskResult {
            task_name: task.name.clone(),
            tool_outputs,
            evidence,
            success: overall_success,
            duration: start.elapsed(),
        }
    }

    /// Run multiple diagnostic tasks in parallel.
    pub async fn run_parallel(
        &self,
        tasks: &[DiagnosticTask],
        kubeconfig: Option<&Path>,
        kubectl_ctx: Option<&str>,
    ) -> Vec<TaskResult> {
        let futures: Vec<_> = tasks
            .iter()
            .map(|task| self.run(task, kubeconfig, kubectl_ctx))
            .collect();

        futures::future::join_all(futures).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_task_builds() {
        let task = DiagnosticTask {
            name: "check-pods".to_string(),
            description: "Check pod status".to_string(),
            commands: vec!["kubectl get pods".to_string()],
            timeout: Duration::from_secs(30),
        };
        assert_eq!(task.commands.len(), 1);
    }
}
