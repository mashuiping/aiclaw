//! Kubernetes module

pub mod client;
pub mod kubeconfig;
pub mod kubectl;

pub use client::*;
pub use kubeconfig::*;
pub use kubectl::*;

use crate::config::K8sClusterConfig;
use std::collections::HashMap;

/// Factory for creating K8s clients
pub struct K8sClientFactory;

impl K8sClientFactory {
    pub fn create(
        name: &str,
        config: &K8sClusterConfig,
    ) -> anyhow::Result<Box<dyn K8sClient>> {
        let client = K8sClientImpl::new(name, config)?;
        Ok(Box::new(client))
    }

    pub fn create_all(
        configs: &HashMap<String, K8sClusterConfig>,
    ) -> anyhow::Result<HashMap<String, Box<dyn K8sClient>>> {
        let mut clients = HashMap::new();
        for (name, config) in configs {
            if config.enabled {
                let client = Self::create(name, config)?;
                clients.insert(name.to_string(), client);
            }
        }
        Ok(clients)
    }
}
