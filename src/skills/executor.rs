//! Skill executor - executes skills and their tools

use aiclaw_types::skill::{SkillTool, ToolKind, ToolResult};
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, warn};

use crate::security::command_validator::CommandValidator;

/// Skill executor - executes tools defined in skills
pub struct SkillExecutor {
    validator: Arc<CommandValidator>,
    default_timeout: Duration,
    /// From `AICLAW_KUBECONFIG` at startup; merged with per-call override (session path wins upstream).
    kubeconfig: Option<std::path::PathBuf>,
}

impl SkillExecutor {
    pub fn new() -> Self {
        Self::with_validator(Arc::new(CommandValidator::default()))
    }

    pub fn with_validator(validator: Arc<CommandValidator>) -> Self {
        Self {
            validator,
            default_timeout: Duration::from_secs(30),
            kubeconfig: None,
        }
    }

    pub fn with_validator_and_kubeconfig(
        validator: Arc<CommandValidator>,
        kubeconfig: Option<std::path::PathBuf>,
    ) -> Self {
        Self {
            validator,
            default_timeout: Duration::from_secs(30),
            kubeconfig,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = timeout;
        self
    }

    /// Apply kubeconfig policy for skill subprocesses: drop inherited `KUBECONFIG`, then set from
    /// `kube_override` or this executor's `kubeconfig` (from `AICLAW_KUBECONFIG`).
    fn apply_skill_kube_env(&self, cmd: &mut tokio::process::Command, kube_override: Option<&Path>) {
        cmd.env_remove("KUBECONFIG");
        let effective = kube_override.or(self.kubeconfig.as_deref());
        if let Some(p) = effective {
            if !p.as_os_str().is_empty() {
                cmd.env("KUBECONFIG", p);
            }
        }
    }

    /// Execute a tool from a skill
    pub async fn execute_tool(
        &self,
        tool: &SkillTool,
        args: &HashMap<String, String>,
        kubeconfig: Option<&Path>,
    ) -> anyhow::Result<ToolResult> {
        let start = Instant::now();

        // Validate command before execution (for shell commands)
        if tool.kind == ToolKind::Shell {
            let command = self.interpolate(&tool.command, args);
            let validation = self.validator.validate(&command);

            if !validation.allowed {
                warn!(
                    "Command blocked by validator: {} - {}",
                    command,
                    validation.reason.as_deref().unwrap_or("Unknown reason")
                );
                return Ok(ToolResult {
                    tool_name: tool.name.clone(),
                    success: false,
                    output: None,
                    error: Some(format!(
                        "Command blocked: {}. Reason: {}",
                        command,
                        validation.reason.as_deref().unwrap_or("Not in whitelist")
                    )),
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    evidence: vec![],
                });
            }

            if validation.requires_confirmation {
                warn!("Command requires confirmation: {}", command);
                return Ok(ToolResult {
                    tool_name: tool.name.clone(),
                    success: false,
                    output: None,
                    error: Some(format!(
                        "Command requires confirmation: {}. Risk level: {:?}",
                        command, validation.risk_level
                    )),
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    evidence: vec![],
                });
            }
        }

        match tool.kind {
            ToolKind::Shell => self.execute_shell(tool, args, start, kubeconfig).await,
            ToolKind::Http => self.execute_http(tool, args, start).await,
            ToolKind::Script => self.execute_script(tool, args, start, kubeconfig).await,
        }
    }

    /// Execute a shell command with timeout
    async fn execute_shell(
        &self,
        tool: &SkillTool,
        args: &HashMap<String, String>,
        start: Instant,
        kubeconfig: Option<&Path>,
    ) -> anyhow::Result<ToolResult> {
        let command = self.interpolate(&tool.command, args);
        let mut cmd_parts = command.split_whitespace();
        let program = cmd_parts.next().unwrap_or(&tool.command);

        let mut cmd = tokio::process::Command::new(program);
        self.apply_skill_kube_env(&mut cmd, kubeconfig);
        cmd.envs(&tool.env);

        for arg in cmd_parts {
            cmd.arg(self.interpolate(arg, args));
        }

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.kill_on_drop(true);

        debug!("Executing shell with timeout {:?}: {}", self.default_timeout, command);

        let output = match tokio::time::timeout(self.default_timeout, cmd.output()).await {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                return Ok(ToolResult {
                    tool_name: tool.name.clone(),
                    success: false,
                    output: None,
                    error: Some(format!("Command execution error: {}", e)),
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    evidence: vec![],
                });
            }
            Err(_) => {
                // Timeout
                warn!("Command timed out after {:?}: {}", self.default_timeout, command);
                return Ok(ToolResult {
                    tool_name: tool.name.clone(),
                    success: false,
                    output: None,
                    error: Some(format!(
                        "Command timed out after {:?}",
                        self.default_timeout
                    )),
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    evidence: vec![],
                });
            }
        };

        let duration = start.elapsed();

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        let success = output.status.success();
        let output_combined = if stdout.is_empty() {
            stderr.clone()
        } else {
            stdout
        };

        Ok(ToolResult {
            tool_name: tool.name.clone(),
            success,
            output: if success {
                Some(output_combined)
            } else {
                None
            },
            error: if !success { Some(stderr) } else { None },
            execution_time_ms: duration.as_millis() as u64,
            evidence: vec![],
        })
    }

    /// Execute an HTTP request with timeout
    async fn execute_http(
        &self,
        tool: &SkillTool,
        args: &HashMap<String, String>,
        start: Instant,
    ) -> anyhow::Result<ToolResult> {
        let url = self.interpolate(&tool.command, args);

        let client = reqwest::Client::builder()
            .timeout(self.default_timeout)
            .build()?;

        let mut request = client.get(&url);

        for (key, value) in &tool.env {
            request = request.header(key, self.interpolate(value, args));
        }

        let query_params: HashMap<String, String> = tool
            .args
            .iter()
            .map(|(k, v)| (k.clone(), self.interpolate(v, args)))
            .collect();

        if !query_params.is_empty() {
            request = request.query(&query_params);
        }

        debug!("Executing HTTP GET with timeout {:?}: {}", self.default_timeout, url);

        let response = match tokio::time::timeout(self.default_timeout, request.send()).await {
            Ok(Ok(response)) => response,
            Ok(Err(e)) => {
                return Ok(ToolResult {
                    tool_name: tool.name.clone(),
                    success: false,
                    output: None,
                    error: Some(format!("HTTP request error: {}", e)),
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    evidence: vec![],
                });
            }
            Err(_) => {
                warn!(
                    "HTTP request timed out after {:?}: {}",
                    self.default_timeout, url
                );
                return Ok(ToolResult {
                    tool_name: tool.name.clone(),
                    success: false,
                    output: None,
                    error: Some(format!(
                        "HTTP request timed out after {:?}",
                        self.default_timeout
                    )),
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    evidence: vec![],
                });
            }
        };

        let duration = start.elapsed();

        let status = response.status();
        let success = status.is_success();
        let body = response.text().await.unwrap_or_default();

        Ok(ToolResult {
            tool_name: tool.name.clone(),
            success,
            output: if success { Some(body.clone()) } else { None },
            error: if !success {
                Some(format!("HTTP {}: {}", status, body))
            } else {
                None
            },
            execution_time_ms: duration.as_millis() as u64,
            evidence: vec![],
        })
    }

    /// Execute a script file with timeout
    async fn execute_script(
        &self,
        tool: &SkillTool,
        args: &HashMap<String, String>,
        start: Instant,
        kubeconfig: Option<&Path>,
    ) -> anyhow::Result<ToolResult> {
        let script_path = self.interpolate(&tool.command, args);

        let mut cmd = tokio::process::Command::new(&script_path);
        self.apply_skill_kube_env(&mut cmd, kubeconfig);
        cmd.envs(&tool.env);

        for (_key, value) in &tool.args {
            cmd.arg(self.interpolate(value, args));
        }

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.kill_on_drop(true);

        debug!(
            "Executing script with timeout {:?}: {}",
            self.default_timeout, script_path
        );

        let output = match tokio::time::timeout(self.default_timeout, cmd.output()).await {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                return Ok(ToolResult {
                    tool_name: tool.name.clone(),
                    success: false,
                    output: None,
                    error: Some(format!("Script execution error: {}", e)),
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    evidence: vec![],
                });
            }
            Err(_) => {
                warn!(
                    "Script timed out after {:?}: {}",
                    self.default_timeout, script_path
                );
                return Ok(ToolResult {
                    tool_name: tool.name.clone(),
                    success: false,
                    output: None,
                    error: Some(format!("Script timed out after {:?}", self.default_timeout)),
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    evidence: vec![],
                });
            }
        };

        let duration = start.elapsed();

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        let success = output.status.success();

        Ok(ToolResult {
            tool_name: tool.name.clone(),
            success,
            output: if success { Some(stdout) } else { None },
            error: if !success { Some(stderr) } else { None },
            execution_time_ms: duration.as_millis() as u64,
            evidence: vec![],
        })
    }

    /// Interpolate template variables in a string
    fn interpolate(&self, template: &str, args: &HashMap<String, String>) -> String {
        let mut result = template.to_string();

        for (key, value) in args {
            let placeholder = format!("{{{{{}}}}}", key);
            result = result.replace(&placeholder, value);
        }

        result
    }
}

impl Default for SkillExecutor {
    fn default() -> Self {
        Self::new()
    }
}
