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

/// Extracted entities from user message (domain-aware)
#[derive(Debug, Clone, Default)]
pub struct IntentEntities {
    // Standard K8s entities
    pub pod_name: Option<String>,
    pub namespace: Option<String>,
    pub cluster: Option<String>,
    pub service_name: Option<String>,
    pub deployment_name: Option<String>,
    pub node_name: Option<String>,
    pub query: Option<String>,
    // Domain-specific entities
    pub domain: Option<String>,              // gpu, storage, network, database
    pub virtualization: Option<String>,       // hami, vgpu, time-slicing
    pub kubernetes_resource: Option<String>, // pod, deployment, statefulset, daemonset
    pub resource_state: Option<String>,       // pending, crashloop, error, oom
    pub error_keyword: Option<String>,        // 502, 500, 404, oom
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

/// Prompt templates for domain-aware intent classification
pub const INTENT_CLASSIFICATION_PROMPT: &str = r#"你是一个运维助手，擅长Kubernetes和AI运维。用户会描述一个问题，你需要识别其意图类型并提取相关实体。

## 意图类型
- Logs: 查看日志（如：查看日志、tail logs、查看 pod 日志）
- Metrics: 查询指标/监控（如：查询 CPU、指标、监控、Prometheus）
- Health: 健康检查（如：检查状态、健康检查、集群状态）
- Debug: 故障排查（如：排查问题、debug、为什么挂了、诊断）
- Query: 数据查询（如：查询、search、搜索）
- Scale: 扩缩容（如：扩容、缩容、scale）
- Deploy: 部署（如：部署、发布、deploy）

## 实体提取（要识别所有相关实体）

### 标准K8s实体
- pod_name: Pod 名称
- namespace: 命名空间
- cluster: 集群名称
- service_name: 服务名
- deployment_name: Deployment 名称
- node_name: 节点名称

### 领域特定实体（重要！）
- domain: 领域，如 gpu, storage, network, database, apigw
- virtualization: 虚拟化技术，如 hami, vgpu, time-slicing, gpushare
- kubernetes_resource: K8s资源类型，如 pod, deployment, statefulset, daemonset, job
- resource_state: 资源状态，如 pending, crashloop, error, oom, running, terminated
- error_keyword: 错误关键词，如 502, 500, 404, oom, crashloop, timeout

## 领域关键词识别
请注意识别以下常见领域：
- **GPU虚拟化**: hami, gpu, vgpu, nvidia, cuda, gpumem, device-plugin
- **网关/APISIX**: apisix, apigw, gateway, ingress, nginx, openresty
- **DNS/CoreDNS**: coredns, dns, kubelet, clusterDNS, resolv.conf
- **存储**: storage, pvc, ceph, nfs, persistentvolume
- **监控**: prometheus, victoriametrics, metrics, alert
- **OOM/内存**: oom, memory, crashloop, oomkill

## 示例
输入: "gpu 虚拟化的集群里面有个 pod 一直 pending 看下是什么问题"
输出: {"intent": "Debug", "confidence": 0.95, "entities": {"pod_name": null, "namespace": null, "domain": "gpu", "virtualization": "hami", "kubernetes_resource": "pod", "resource_state": "pending", "reasoning": "用户遇到GPU虚拟化集群中Pod调度失败问题，需要按HAMi诊断流程排查"}}

直接返回 JSON 格式，不要有其他内容"#;

/// Result parser for intent classification
#[derive(Debug, serde::Deserialize)]
pub struct IntentParseResult {
    pub intent: String,
    pub confidence: f32,
    pub entities: ParsedEntities,
    #[serde(default)]
    pub reasoning: Option<String>,
}

#[derive(Debug, serde::Deserialize, Default)]
pub struct ParsedEntities {
    // Standard K8s entities
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
    pub node_name: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    // Domain-specific entities
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub virtualization: Option<String>,
    #[serde(default)]
    pub kubernetes_resource: Option<String>,
    #[serde(default)]
    pub resource_state: Option<String>,
    #[serde(default)]
    pub error_keyword: Option<String>,
}

impl From<ParsedEntities> for IntentEntities {
    fn from(p: ParsedEntities) -> Self {
        Self {
            pod_name: p.pod_name,
            namespace: p.namespace,
            cluster: p.cluster,
            service_name: p.service_name,
            deployment_name: p.deployment_name,
            node_name: p.node_name,
            query: p.query,
            domain: p.domain,
            virtualization: p.virtualization,
            kubernetes_resource: p.kubernetes_resource,
            resource_state: p.resource_state,
            error_keyword: p.error_keyword,
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
