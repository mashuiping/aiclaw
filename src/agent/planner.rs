//! Execution planner - plans what queries to execute based on user intent

use std::sync::Arc;
use tracing::{debug, warn};

use crate::llm::traits::LLMProvider;
use crate::llm::types::{ChatMessage, ChatOptions};

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
    pub query_type: QueryType,
    pub parameters: Vec<QueryParameter>,
}

#[derive(Debug, Clone)]
pub enum QueryType {
    PodLogs,
    PodDescribe,
    PodEvents,
    NodeStatus,
    DeploymentStatus,
    ServiceStatus,
    MetricsQuery,
    ClusterHealth,
    CustomQuery,
}

#[derive(Debug, Clone)]
pub struct QueryParameter {
    pub name: String,
    pub value: String,
}

/// Planner - creates execution plans for complex queries
pub struct Planner {
    provider: Arc<dyn LLMProvider>,
}

impl Planner {
    pub fn new(provider: Arc<dyn LLMProvider>) -> Self {
        Self { provider }
    }

    /// Create an execution plan from user intent
    pub async fn plan(&self, user_query: &str, intent_type: &str) -> anyhow::Result<ExecutionPlan> {
        debug!("Creating execution plan for: {}", user_query);

        let prompt = format!(
            r#"用户请求：{}
意图类型：{}

你是一个运维排查专家。用户描述了一个问题，你需要规划需要执行哪些查询来帮助诊断。

可用的查询类型：
- pod_logs: 查询 Pod 日志（需要 pod_name, namespace）
- pod_describe: 获取 Pod 详细信息（需要 pod_name, namespace）
- pod_events: 查看 Pod 相关事件（需要 pod_name, namespace）
- node_status: 查看 Node 状态（需要 node_name）
- deployment_status: 查看 Deployment 状态（需要 deployment_name, namespace）
- service_status: 查看 Service 状态（需要 service_name, namespace）
- metrics_query: 查询指标数据（需要 query 表达式）
- cluster_health: 检查集群健康状态
- custom_query: 自定义查询

请分析问题并规划查询步骤。

直接返回 JSON 格式，不要有其他内容：
{{
    "reasoning": "你的分析思路",
    "steps": [
        {{
            "description": "步骤描述",
            "query_type": "查询类型",
            "parameters": [
                {{"name": "参数名", "value": "参数值"}}
            ]
        }}
    ]
}}

注意：
- 步骤数量根据问题复杂度决定，通常 2-5 个步骤
- 参数值如果不确定，使用占位符如 {{pod_name}}
- 如果问题简单，可以只有 1 个步骤
- 先查基础状态，再查详细日志/指标"#,
            user_query, intent_type
        );

        let messages = vec![
            ChatMessage::system(PLANNER_SYSTEM_PROMPT),
            ChatMessage::user(&prompt),
        ];

        let options = ChatOptions::new()
            .with_temperature(0.1)
            .with_max_tokens(1024);

        let response = self.provider.chat(messages, Some(options)).await?;

        self.parse_plan(&response.content).await
    }

    async fn parse_plan(&self, response: &str) -> anyhow::Result<ExecutionPlan> {
        // Try to extract JSON from response
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
            query_type: String,
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
                let query_type = match s.query_type.as_str() {
                    "pod_logs" => QueryType::PodLogs,
                    "pod_describe" => QueryType::PodDescribe,
                    "pod_events" => QueryType::PodEvents,
                    "node_status" => QueryType::NodeStatus,
                    "deployment_status" => QueryType::DeploymentStatus,
                    "service_status" => QueryType::ServiceStatus,
                    "metrics_query" => QueryType::MetricsQuery,
                    "cluster_health" => QueryType::ClusterHealth,
                    _ => QueryType::CustomQuery,
                };

                let parameters = s
                    .parameters
                    .into_iter()
                    .map(|p| QueryParameter { name: p.name, value: p.value })
                    .collect();

                PlanStep {
                    step_id: i + 1,
                    description: s.description,
                    query_type,
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
        for (i, c) in remaining.chars().enumerate() {
            if c == '{' {
                depth += 1;
            } else if c == '}' {
                depth -= 1;
                if depth == 0 {
                    return Some(remaining[..=i].to_string());
                }
            }
        }
    }

    None
}

const PLANNER_SYSTEM_PROMPT: &str = r#"你是一个运维排查规划专家。

你的职责是分析用户问题，制定合理的查询计划，帮助快速定位根因。

规划原则：
1. 先查基础状态，再查详细信息（如先看 Pod 状态，再看日志）
2. 相关性原则：选择与问题最相关的查询
3. 最小化原则：用最少的查询解决问题
4. 证据链原则：查询结果应能形成完整的证据链

分析思路：
1. 问题是什么？（如：Pod 无法启动、响应慢、502错误）
2. 可能的原因有哪些？
3. 需要查什么数据来验证或排除这些原因？
4. 按什么顺序查最高效？"#;

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
