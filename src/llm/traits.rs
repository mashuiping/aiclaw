//! LLM trait definitions

use async_trait::async_trait;
use std::sync::Arc;

use super::types::{ChatMessage, ChatOptions, ChatResponse};

/// LLM Provider trait - implemented by each provider (OpenAI, Anthropic, etc.)
#[async_trait]
pub trait LLMProvider: Send + Sync {
    /// Provider name (e.g., "openai", "anthropic")
    fn name(&self) -> &str;

    /// Provider type (e.g., "openai", "anthropic", "deepseek")
    fn provider_type(&self) -> &str;

    /// Send a chat request
    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        options: Option<ChatOptions>,
    ) -> anyhow::Result<ChatResponse>;

    /// Simple completion (single prompt)
    async fn complete(&self, prompt: &str) -> anyhow::Result<String> {
        let response = self
            .chat(vec![ChatMessage::user(prompt)], None)
            .await?;
        Ok(response.content)
    }

    /// Health check
    async fn health_check(&self) -> bool;
}

/// LLM Router trait - handles routing to different providers
#[async_trait]
pub trait LLMRouter: Send + Sync {
    /// Route an LLM request
    async fn route(
        &self,
        messages: Vec<ChatMessage>,
        options: Option<ChatOptions>,
    ) -> anyhow::Result<ChatResponse>;

    /// Get router name
    fn name(&self) -> &str;

    /// List available providers
    fn available_providers(&self) -> Vec<String>;
}

/// Intent classifier trait
#[async_trait]
pub trait IntentClassifier: Send + Sync {
    /// Classify user intent from a message
    async fn classify(&self, message: &str) -> anyhow::Result<IntentClassification>;

    /// Get the underlying provider
    fn provider(&self) -> Arc<dyn LLMProvider>;
}

/// Intent classification result
#[derive(Debug, Clone)]
pub struct IntentClassification {
    pub intent_type: String,
    pub confidence: f32,
    pub entities: IntentEntities,
    pub reasoning: Option<String>,
}

/// Extracted entities from user message
#[derive(Debug, Clone, Default)]
pub struct IntentEntities {
    pub pod_name: Option<String>,
    pub namespace: Option<String>,
    pub cluster: Option<String>,
    pub service_name: Option<String>,
    pub deployment_name: Option<String>,
    pub query: Option<String>,
}

impl IntentEntities {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_pod(mut self, pod: impl Into<String>) -> Self {
        self.pod_name = Some(pod.into());
        self
    }

    pub fn with_namespace(mut self, ns: impl Into<String>) -> Self {
        self.namespace = Some(ns.into());
        self
    }

    pub fn with_cluster(mut self, cluster: impl Into<String>) -> Self {
        self.cluster = Some(cluster.into());
        self
    }
}

/// Prompt templates for intent classification
pub const INTENT_CLASSIFICATION_PROMPT: &str = r#"你是一个运维助手。用户会描述一个问题，你需要识别其意图类型并提取相关实体。

意图类型：
- Logs: 查看日志（如：查看日志、tail logs、查看 pod 日志）
- Metrics: 查询指标/监控（如：查询 CPU、指标、监控）
- Health: 健康检查（如：检查状态、健康检查、集群状态）
- Debug: 故障排查（如：排查问题、debug、为什么挂了）
- Query: 数据查询（如：查询、search、搜索）
- Scale: 扩缩容（如：扩容、缩容、scale）
- Deploy: 部署（如：部署、发布、deploy）

实体提取：
- pod_name: Pod 名称
- namespace: 命名空间（通常用 ns= 或 namespace= 指定）
- cluster: 集群名称
- service_name: 服务名
- deployment_name: Deployment 名称
- query: 原始查询内容

直接返回 JSON 格式，不要有其他内容：
{"intent": "Debug", "confidence": 0.95, "entities": {"pod_name": "nginx-123", "namespace": "default"}, "reasoning": "用户询问为什么 pod 启动失败，属于故障排查"}"#;

/// Result parser for intent classification
#[derive(Debug, serde::Deserialize)]
pub struct IntentParseResult {
    pub intent: String,
    pub confidence: f32,
    pub entities: ParsedEntities,
    #[serde(default)]
    pub reasoning: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct ParsedEntities {
    #[serde(default)]
    pub pod_name: Option<String>,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub cluster: Option<String>,
    #[serde(default)]
    pub service_name: Option<String>,
    #[serde(default)]
    pub deployment_name: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
}

impl From<ParsedEntities> for IntentEntities {
    fn from(p: ParsedEntities) -> Self {
        Self {
            pod_name: p.pod_name,
            namespace: p.namespace,
            cluster: p.cluster,
            service_name: p.service_name,
            deployment_name: p.deployment_name,
            query: p.query,
        }
    }
}

impl From<IntentParseResult> for IntentClassification {
    fn from(r: IntentParseResult) -> Self {
        Self {
            intent_type: r.intent,
            confidence: r.confidence,
            entities: r.entities.into(),
            reasoning: r.reasoning,
        }
    }
}
