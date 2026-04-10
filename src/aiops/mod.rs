//! AI/OPS module - Observability data providers

pub mod traits;
pub mod victoria;
pub mod prometheus;

pub use traits::*;
pub use victoria::*;
pub use prometheus::*;

use crate::config::AIOpsProviderConfig;
use std::collections::HashMap;

/// Factory for creating AI/OPS providers
pub struct AIOpsProviderFactory;

impl AIOpsProviderFactory {
    pub fn create(
        name: &str,
        config: &AIOpsProviderConfig,
    ) -> anyhow::Result<Box<dyn AIOpsProvider>> {
        match config.provider_type.as_str() {
            "victoria" | "victoriametrics" => {
                let provider = VictoriaMetricsProvider::new(name, config)?;
                Ok(Box::new(provider))
            }
            "prometheus" => {
                let provider = PrometheusProvider::new(name, config)?;
                Ok(Box::new(provider))
            }
            _ => {
                anyhow::bail!("Unknown AI/OPS provider type: {}", config.provider_type)
            }
        }
    }

    pub fn create_all(
        configs: &HashMap<String, AIOpsProviderConfig>,
    ) -> anyhow::Result<HashMap<String, Box<dyn AIOpsProvider>>> {
        let mut providers = HashMap::new();
        for (name, config) in configs {
            if config.enabled {
                let provider = Self::create(name, &config)?;
                providers.insert(name.to_string(), provider);
            }
        }
        Ok(providers)
    }
}
