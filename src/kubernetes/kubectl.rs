//! Kubectl wrapper implementation

use async_trait::async_trait;
use aiclaw_types::kubernetes::{
    Container, ContainerState, Deployment, Event, EventType, InvolvedObject, Node, NodeStatus, Pod,
    PodStatus, Replicas, Service, ServicePort, ServiceType,
};
use chrono::Utc;
use std::collections::HashMap;
use k8s_openapi::api::apps::v1::Deployment as KubeDeployment;
use k8s_openapi::api::core::v1::{
    Event as KubeEvent, Node as KubeNode, Pod as KubePod, Service as KubeService,
};
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::api::{Api, ListParams, LogParams};
use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::{Client, Config as KubeClientConfig};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::info;

use super::client::K8sClient;
use super::kubeconfig::is_sensitive_path;
use crate::config::K8sClusterConfig;

/// K8s client implementation using kube-rs
pub struct K8sClientImpl {
    name: String,
    context: Option<String>,
    kubeconfig_path: std::path::PathBuf,
    _default_namespace: String,
    client: Arc<RwLock<Option<Client>>>,
}

impl K8sClientImpl {
    pub fn new(name: &str, config: &K8sClusterConfig) -> anyhow::Result<Self> {
        if is_sensitive_path(&config.kubeconfig_path) {
            info!("Loading potentially sensitive kubeconfig: {:?}", config.kubeconfig_path);
        }

        Ok(Self {
            name: name.to_string(),
            context: config.context.clone(),
            kubeconfig_path: config.kubeconfig_path.clone(),
            _default_namespace: config.default_namespace.clone(),
            client: Arc::new(RwLock::new(None)),
        })
    }

    pub async fn connect(&self) -> anyhow::Result<Client> {
        let mut client_guard = self.client.write().await;

        if let Some(ref existing) = *client_guard {
            return Ok(existing.clone());
        }

        let kubeconfig_content = std::fs::read_to_string(&self.kubeconfig_path)?;
        let kubeconfig = Kubeconfig::from_yaml(&kubeconfig_content)?;
        let options = KubeConfigOptions {
            context: self.context.clone(),
            cluster: None,
            user: None,
        };
        let config = KubeClientConfig::from_custom_kubeconfig(kubeconfig, &options).await?;

        let client = Client::try_from(config)?;

        *client_guard = Some(client.clone());

        Ok(client)
    }

    async fn ensure_connected(&self) -> anyhow::Result<Client> {
        let client = self.connect().await?;
        Ok(client)
    }
}

#[async_trait]
impl K8sClient for K8sClientImpl {
    fn name(&self) -> &str {
        &self.name
    }

    fn context(&self) -> &str {
        self.context.as_deref().unwrap_or("default")
    }

    async fn get_pods(&self, namespace: &str, label_selector: Option<&str>) -> anyhow::Result<Vec<Pod>> {
        let client = self.ensure_connected().await?;
        let ns = Api::<KubePod>::namespaced(client, namespace);

        let mut list = ns
            .list(&ListParams::default().labels(label_selector.unwrap_or("")))
            .await?;

        let pods: Vec<Pod> = list.items.drain(..).map(|p| {
            let status = p.status.as_ref();

            let phase = status.map(|s| s.phase.as_deref().unwrap_or("Unknown")).unwrap_or("Unknown");
            let pod_status = match phase {
                "Pending" => PodStatus::Pending,
                "Running" => PodStatus::Running,
                "Succeeded" => PodStatus::Succeeded,
                "Failed" => PodStatus::Failed,
                _ => PodStatus::Unknown,
            };

            let containers = p.spec.as_ref().map(|spec| {
                spec.containers.iter().map(|c| {
                    let cs_opt = status
                        .and_then(|s| s.container_statuses.as_ref())
                        .and_then(|statuses| statuses.iter().find(|cs| cs.name == c.name));
                    let (state, ready, restart_count) = if let Some(cs) = cs_opt {
                        let st = cs.state.as_ref();
                        let waiting_reason = st
                            .and_then(|s| s.waiting.as_ref())
                            .and_then(|w| w.reason.clone());
                        let terminated_reason = st
                            .and_then(|s| s.terminated.as_ref())
                            .and_then(|t| t.reason.clone());
                        let state = ContainerState {
                            running: st.and_then(|s| s.running.as_ref()).is_some(),
                            waiting: st.and_then(|s| s.waiting.as_ref()).is_some(),
                            terminated: st.and_then(|s| s.terminated.as_ref()).is_some(),
                            message: st
                                .and_then(|s| s.waiting.as_ref())
                                .and_then(|w| w.message.clone()),
                            reason: waiting_reason.or(terminated_reason),
                        };
                        (state, cs.ready, cs.restart_count.max(0) as u32)
                    } else {
                        (
                            ContainerState {
                                running: false,
                                waiting: false,
                                terminated: false,
                                message: None,
                                reason: None,
                            },
                            false,
                            0,
                        )
                    };
                    Container {
                        name: c.name.clone(),
                        image: c.image.clone().unwrap_or_default(),
                        ready,
                        restart_count,
                        state,
                    }
                }).collect()
            }).unwrap_or_default();

            Pod {
                name: p.metadata.name.clone().unwrap_or_default(),
                namespace: p.metadata.namespace.clone().unwrap_or_else(|| namespace.to_string()),
                status: pod_status,
                containers,
                labels: p
                    .metadata
                    .labels
                    .clone()
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
                created_at: p
                    .metadata
                    .creation_timestamp
                    .map(|t| t.0.with_timezone(&Utc)),
            }
        }).collect();

        Ok(pods)
    }

    async fn get_pod_logs(&self, namespace: &str, pod: &str, tail: usize) -> anyhow::Result<String> {
        let client = self.ensure_connected().await?;
        let ns = Api::<KubePod>::namespaced(client, namespace);

        let logs = ns
            .logs(
                pod,
                &LogParams {
                    tail_lines: Some(tail as i64),
                    ..LogParams::default()
                },
            )
            .await?;

        Ok(logs)
    }

    async fn describe_pod(&self, namespace: &str, pod: &str) -> anyhow::Result<String> {
        let pods = self.get_pods(namespace, None).await?;
        let pod_info = pods.iter().find(|p| p.name == pod);

        if let Some(pod) = pod_info {
            let mut output = format!("Name: {}\n", pod.name);
            output += &format!("Namespace: {}\n", pod.namespace);
            output += &format!("Status: {:?}\n", pod.status);
            output += "\nConditions:\n";

            for container in &pod.containers {
                output += &format!("  Container: {} (Image: {})\n", container.name, container.image);
                output += &format!("    Ready: {}\n", container.ready);
                output += &format!("    Restart Count: {}\n", container.restart_count);
                if let Some(reason) = &container.state.reason {
                    output += &format!("    Reason: {}\n", reason);
                }
            }

            output += "\nLabels:\n";
            for (k, v) in &pod.labels {
                output += &format!("  {}: {}\n", k, v);
            }

            Ok(output)
        } else {
            Err(anyhow::anyhow!("Pod not found: {}/{}", namespace, pod))
        }
    }

    async fn get_events(&self, namespace: Option<&str>) -> anyhow::Result<Vec<Event>> {
        let client = self.ensure_connected().await?;

        let events: Api<KubeEvent>;
        let list_params = ListParams::default();

        if let Some(ns) = namespace {
            events = Api::<KubeEvent>::namespaced(client, ns);
        } else {
            events = Api::<KubeEvent>::all(client);
        }

        let mut list = events.list(&list_params).await?;

        let result: Vec<Event> = list.items.drain(..).map(|e| {
            Event {
                name: e.metadata.name.clone().unwrap_or_default(),
                namespace: e.metadata.namespace,
                event_type: if e.type_.as_deref() == Some("Normal") {
                    EventType::Normal
                } else {
                    EventType::Warning
                },
                reason: e.reason.clone().unwrap_or_default(),
                message: e.message.clone().unwrap_or_default(),
                involved_object: InvolvedObject {
                    kind: e.involved_object.kind.clone().unwrap_or_default(),
                    name: e.involved_object.name.clone().unwrap_or_default(),
                    namespace: e.involved_object.namespace.clone(),
                },
                first_timestamp: e.first_timestamp.map(|t| t.0.with_timezone(&Utc)),
                last_timestamp: e.last_timestamp.map(|t| t.0.with_timezone(&Utc)),
                count: e.count.unwrap_or(1) as u32,
            }
        }).collect();

        Ok(result)
    }

    async fn get_nodes(&self) -> anyhow::Result<Vec<Node>> {
        let client = self.ensure_connected().await?;
        let nodes: Api<KubeNode> = Api::all(client);

        let mut list = nodes.list(&ListParams::default()).await?;

        let result: Vec<Node> = list.items.drain(..).map(|n| {
            let conditions = n.status.as_ref().and_then(|s| s.conditions.as_ref());
            let ready = conditions
                .and_then(|c| c.iter().find(|cond| cond.type_ == "Ready"))
                .map(|cond| cond.status == "True")
                .unwrap_or(false);

            let roles: Vec<String> = n
                .metadata
                .labels
                .as_ref()
                .map(|l| {
                    l.iter()
                        .filter(|(k, _)| k.starts_with("node-role.kubernetes.io/"))
                        .map(|(k, _)| {
                            k.trim_start_matches("node-role.kubernetes.io/")
                                .to_string()
                        })
                        .collect()
                })
                .unwrap_or_default();

            Node {
                name: n.metadata.name.clone().unwrap_or_default(),
                status: if ready { NodeStatus::Ready } else { NodeStatus::NotReady },
                roles,
                age: n
                    .metadata
                    .creation_timestamp
                    .map(|t| t.0.with_timezone(&Utc)),
                labels: n
                    .metadata
                    .labels
                    .clone()
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
            }
        }).collect();

        Ok(result)
    }

    async fn get_deployments(&self, namespace: &str) -> anyhow::Result<Vec<Deployment>> {
        let client = self.ensure_connected().await?;
        let ns = Api::<KubeDeployment>::namespaced(client, namespace);

        let mut list = ns.list(&ListParams::default()).await?;

        let result: Vec<Deployment> = list.items.drain(..).map(|d| {
            let replicas = d
                .spec
                .as_ref()
                .and_then(|s| s.replicas)
                .unwrap_or(0) as u32;
            let status = d.status.as_ref();

            Deployment {
                name: d.metadata.name.clone().unwrap_or_default(),
                namespace: d.metadata.namespace.clone().unwrap_or_else(|| namespace.to_string()),
                replicas: Replicas {
                    desired: replicas,
                    ready: status.and_then(|s| s.ready_replicas).unwrap_or(0) as u32,
                    available: status.and_then(|s| s.available_replicas).unwrap_or(0) as u32,
                    updated: status.and_then(|s| s.updated_replicas).unwrap_or(0) as u32,
                },
                labels: d
                    .metadata
                    .labels
                    .clone()
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
                created_at: d
                    .metadata
                    .creation_timestamp
                    .map(|t| t.0.with_timezone(&Utc)),
            }
        }).collect();

        Ok(result)
    }

    async fn get_services(&self, namespace: &str) -> anyhow::Result<Vec<Service>> {
        let client = self.ensure_connected().await?;
        let ns = Api::<KubeService>::namespaced(client, namespace);

        let mut list = ns.list(&ListParams::default()).await?;

        let result: Vec<Service> = list.items.drain(..).map(|s| {
            let service_type = match s.spec.as_ref().map(|spec| spec.type_.as_deref()).flatten() {
                Some("ClusterIP") => ServiceType::ClusterIP,
                Some("NodePort") => ServiceType::NodePort,
                Some("LoadBalancer") => ServiceType::LoadBalancer,
                Some("ExternalName") => ServiceType::ExternalName,
                _ => ServiceType::ClusterIP,
            };

            let ports: Vec<ServicePort> = s
                .spec
                .as_ref()
                .and_then(|spec| spec.ports.as_ref())
                .map(|ports| {
                    ports
                        .iter()
                        .map(|port| {
                            let target = port
                                .target_port
                                .as_ref()
                                .map(|tp| match tp {
                                    IntOrString::Int(i) => *i,
                                    IntOrString::String(_) => port.port,
                                })
                                .unwrap_or(port.port);
                            let port_u16 = port.port.clamp(0, u16::MAX as i32) as u16;
                            let target_u16 = target.clamp(0, u16::MAX as i32) as u16;
                            ServicePort {
                                name: port.name.clone(),
                                port: port_u16,
                                target_port: target_u16,
                                protocol: port
                                    .protocol
                                    .clone()
                                    .unwrap_or_else(|| "TCP".to_string()),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();

            Service {
                name: s.metadata.name.clone().unwrap_or_default(),
                namespace: s.metadata.namespace.clone().unwrap_or_else(|| namespace.to_string()),
                service_type,
                cluster_ip: s
                    .spec
                    .as_ref()
                    .and_then(|spec| spec.cluster_ip.clone())
                    .unwrap_or_default(),
                ports,
                selector: s
                    .spec
                    .as_ref()
                    .and_then(|spec| spec.selector.clone())
                    .unwrap_or_default()
                    .into_iter()
                    .collect::<HashMap<_, _>>(),
            }
        }).collect();

        Ok(result)
    }

    async fn health_check(&self) -> bool {
        match self.ensure_connected().await {
            Ok(client) => {
                let nodes: Api<KubeNode> = Api::all(client);
                match nodes.list(&ListParams::default().limit(1)).await {
                    Ok(_) => true,
                    Err(_) => false,
                }
            }
            Err(_) => false,
        }
    }
}
