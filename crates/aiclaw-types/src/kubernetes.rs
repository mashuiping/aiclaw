//! Kubernetes types

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// Kubernetes resource identification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct K8sResourceId {
    pub namespace: Option<String>,
    pub name: String,
    pub kind: String,
}

/// Pod information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pod {
    pub name: String,
    pub namespace: String,
    pub status: PodStatus,
    pub containers: Vec<Container>,
    pub labels: std::collections::HashMap<String, String>,
    pub created_at: Option<DateTime<Utc>>,
}

/// Pod status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PodStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Unknown,
}

/// Container information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Container {
    pub name: String,
    pub image: String,
    pub ready: bool,
    pub restart_count: u32,
    pub state: ContainerState,
}

/// Container state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerState {
    pub running: bool,
    pub waiting: bool,
    pub terminated: bool,
    pub message: Option<String>,
    pub reason: Option<String>,
}

/// Node information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub name: String,
    pub status: NodeStatus,
    pub roles: Vec<String>,
    pub age: Option<DateTime<Utc>>,
    pub labels: std::collections::HashMap<String, String>,
}

/// Node status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum NodeStatus {
    Ready,
    NotReady,
    Unknown,
}

/// Kubernetes event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub name: String,
    pub namespace: Option<String>,
    pub event_type: EventType,
    pub reason: String,
    pub message: String,
    pub involved_object: InvolvedObject,
    pub first_timestamp: Option<DateTime<Utc>>,
    pub last_timestamp: Option<DateTime<Utc>>,
    pub count: u32,
}

/// Event type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EventType {
    Normal,
    Warning,
}

/// Involved object in an event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvolvedObject {
    pub kind: String,
    pub name: String,
    pub namespace: Option<String>,
}

/// K8s client configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct K8sClientConfig {
    pub name: String,
    pub context: Option<String>,
    pub kubeconfig_path: std::path::PathBuf,
    #[serde(default)]
    pub default_namespace: String,
    #[serde(default)]
    pub timeout_secs: u64,
}

/// Deployment information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deployment {
    pub name: String,
    pub namespace: String,
    pub replicas: Replicas,
    pub labels: std::collections::HashMap<String, String>,
    pub created_at: Option<DateTime<Utc>>,
}

/// Replica status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Replicas {
    pub desired: u32,
    pub ready: u32,
    pub available: u32,
    pub updated: u32,
}

/// Service information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Service {
    pub name: String,
    pub namespace: String,
    pub service_type: ServiceType,
    pub cluster_ip: String,
    pub ports: Vec<ServicePort>,
    pub selector: std::collections::HashMap<String, String>,
}

/// Service type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ServiceType {
    ClusterIP,
    NodePort,
    LoadBalancer,
    ExternalName,
}

/// Service port
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServicePort {
    pub name: Option<String>,
    pub port: u16,
    pub target_port: u16,
    pub protocol: String,
}
