//! K8s client trait and implementation

use async_trait::async_trait;
use aiclaw_types::kubernetes::{
    Deployment, Event, EventType, InvolvedObject, K8sClientConfig, Node, NodeStatus, Pod,
    PodStatus, Replicas, Service, ServicePort, ServiceType,
};
use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// K8s client trait - for Kubernetes operations
#[async_trait]
pub trait K8sClient: Send + Sync {
    /// Get client name
    fn name(&self) -> &str;

    /// Get context/cluster name
    fn context(&self) -> &str;

    /// Get pods in namespace
    async fn get_pods(
        &self,
        namespace: &str,
        label_selector: Option<&str>,
    ) -> anyhow::Result<Vec<Pod>>;

    /// Get pod logs
    async fn get_pod_logs(
        &self,
        namespace: &str,
        pod: &str,
        tail: usize,
    ) -> anyhow::Result<String>;

    /// Describe a pod
    async fn describe_pod(
        &self,
        namespace: &str,
        pod: &str,
    ) -> anyhow::Result<String>;

    /// Get events
    async fn get_events(
        &self,
        namespace: Option<&str>,
    ) -> anyhow::Result<Vec<Event>>;

    /// Get nodes
    async fn get_nodes(&self) -> anyhow::Result<Vec<Node>>;

    /// Get deployments
    async fn get_deployments(
        &self,
        namespace: &str,
    ) -> anyhow::Result<Vec<Deployment>>;

    /// Get services
    async fn get_services(
        &self,
        namespace: &str,
    ) -> anyhow::Result<Vec<Service>>;

    /// Health check
    async fn health_check(&self) -> bool;
}
