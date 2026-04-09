//! Skill executor - executes skills and their tools

use aiclaw_types::skill::{SkillContext, SkillTool, ToolKind, ToolResult};
use chrono::Utc;
use serde_json::Value;
use std::collections::HashMap;
use std::process::Stdio;
use std::time::Instant;
use tracing::{debug, error, info};

/// Skill executor - executes tools defined in skills
pub struct SkillExecutor;

impl SkillExecutor {
    pub fn new() -> Self {
        Self
    }

    /// Execute a tool from a skill
    pub async fn execute_tool(
        &self,
        tool: &SkillTool,
        args: &HashMap<String, String>,
    ) -> anyhow::Result<ToolResult> {
        let start = Instant::now();
        let template_args = args;

        match tool.kind {
            ToolKind::Shell => self.execute_shell(tool, template_args, start).await,
            ToolKind::Http => self.execute_http(tool, template_args, start).await,
            ToolKind::Script => self.execute_script(tool, template_args, start).await,
        }
    }

    /// Execute a shell command
    async fn execute_shell(
        &self,
        tool: &SkillTool,
        args: &HashMap<String, String>,
        start: Instant,
    ) -> anyhow::Result<ToolResult> {
        let command = self.interpolate(&tool.command, args);
        let mut cmd_parts = command.split_whitespace();
        let program = cmd_parts.next().unwrap_or(&tool.command);

        let mut cmd = tokio::process::Command::new(program);
        cmd.envs(&tool.env);

        for arg in cmd_parts {
            cmd.arg(self.interpolate(arg, args));
        }

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        debug!("Executing shell: {}", command);

        let output = cmd.output().await?;
        let duration = start.elapsed();

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        let success = output.status.success();
        let output_combined = if stdout.is_empty() { stderr.clone() } else { stdout };

        Ok(ToolResult {
            tool_name: tool.name.clone(),
            success,
            output: if success { Some(output_combined) } else { None },
            error: if !success { Some(stderr) } else { None },
            execution_time_ms: duration.as_millis() as u64,
            evidence: vec![],
        })
    }

    /// Execute an HTTP request
    async fn execute_http(
        &self,
        tool: &SkillTool,
        args: &HashMap<String, String>,
        start: Instant,
    ) -> anyhow::Result<ToolResult> {
        let url = self.interpolate(&tool.command, args);
        let duration = start.elapsed();

        let client = reqwest::Client::new();
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

        debug!("Executing HTTP GET: {}", url);

        let response = request.send().await?;
        let duration = start.elapsed();

        let success = response.status().is_success();
        let body = response.text().await.unwrap_or_default();

        Ok(ToolResult {
            tool_name: tool.name.clone(),
            success,
            output: if success { Some(body.clone()) } else { None },
            error: if !success { Some(body) } else { None },
            execution_time_ms: duration.as_millis() as u64,
            evidence: vec![],
        })
    }

    /// Execute a script file
    async fn execute_script(
        &self,
        tool: &SkillTool,
        args: &HashMap<String, String>,
        start: Instant,
    ) -> anyhow::Result<ToolResult> {
        let script_path = self.interpolate(&tool.command, args);
        let duration = start.elapsed();

        let mut cmd = tokio::process::Command::new(&script_path);
        cmd.envs(&tool.env);

        for (key, value) in &tool.args {
            cmd.arg(self.interpolate(value, args));
        }

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        debug!("Executing script: {}", script_path);

        let output = cmd.output().await?;
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
