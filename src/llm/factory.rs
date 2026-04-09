//! LLM Provider and Router Factory
//!
//! Note: This module defines its own config structs to avoid circular dependencies
//! with the config module. The main application should convert from config::LLMProviderConfig
//! to provider_factory::ProviderConfig.

use std::collections::HashMap;
use std::sync::Arc;

use tracing::info;

use super::providers::*;
use super::routing::*;
use super::traits::*;
use super::types::*;

/// Provider config for factory creation
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub provider_type: String,
    pub api_key: String,
    pub base_url: Option<String>,
    pub model: String,
    pub max_tokens: u32,
    pub timeout_secs: u64,
}

impl ProviderConfig {
    pub fn new(provider_type: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            provider_type: provider_type.into(),
            api_key: api_key.into(),
            base_url: None,
            model: "gpt-4o".to_string(),
            max_tokens: 1024,
            timeout_secs: 60,
        }
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    pub fn with_max_tokens(mut self, tokens: u32) -> Self {
        self.max_tokens = tokens;
        self
    }

    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }
}

/// Router config for routing creation
#[derive(Debug, Clone, Default)]
pub struct RouterConfig {
    pub mode: String,
    pub openrouter_api_key: Option<String>,
    pub openrouter_base_url: Option<String>,
    pub ollama_base_url: Option<String>,
    pub ollama_model: Option<String>,
}

impl RouterConfig {
    pub fn new(mode: impl Into<String>) -> Self {
        Self {
            mode: mode.into(),
            ..Default::default()
        }
    }

    pub fn with_openrouter(mut self, api_key: impl Into<String>, base_url: Option<String>) -> Self {
        self.openrouter_api_key = Some(api_key.into());
        self.openrouter_base_url = base_url;
        self
    }

    pub fn with_ollama(mut self, base_url: impl Into<String>, model: Option<String>) -> Self {
        self.ollama_base_url = Some(base_url.into());
        self.ollama_model = model;
        self
    }
}

/// Factory for creating LLM providers
pub struct LLMProviderFactory;

impl LLMProviderFactory {
    /// Create a provider from config
    pub fn create(name: &str, config: &ProviderConfig) -> anyhow::Result<Arc<dyn LLMProvider>> {
        let provider: Arc<dyn LLMProvider> = match config.provider_type.as_str() {
            "openai" => Arc::new(OpenAIProvider::new(config)?),
            "anthropic" => Arc::new(AnthropicProvider::new(config)?),
            "deepseek" => Arc::new(DeepSeekProvider::new(config)?),
            "zhipu" => Arc::new(ZhipuProvider::new(config)?),
            "minimax" => Arc::new(MiniMaxProvider::new(config)?),
            "qwen" => Arc::new(QwenProvider::new(config)?),
            _ => anyhow::bail!("Unknown provider type: {}", config.provider_type),
        };

        info!("Created LLM provider: {} ({})", name, config.provider_type);
        Ok(provider)
    }

    /// Create multiple providers from config map
    pub fn create_all(
        configs: &HashMap<String, ProviderConfig>,
    ) -> anyhow::Result<HashMap<String, Arc<dyn LLMProvider>>> {
        let mut providers = HashMap::new();
        for (name, config) in configs {
            match Self::create(name, config) {
                Ok(provider) => {
                    providers.insert(name.clone(), provider);
                }
                Err(e) => {
                    tracing::warn!("Failed to create provider {}: {}", name, e);
                }
            }
        }
        Ok(providers)
    }
}

/// Factory for creating LLM routers
pub struct LLM RouterFactory;

impl LLM RouterFactory {
    /// Create a router based on routing config and providers
    pub fn create(
        router_config: &RouterConfig,
        providers: HashMap<String, Arc<dyn LLMProvider>>,
        default_provider: &str,
    ) -> anyhow::Result<Arc<dyn LLMRouter>> {
        let router: Arc<dyn LLMRouter> = match router_config.mode.as_str() {
            "direct" => Arc::new(DirectRouter::new(providers, default_provider)),
            "openrouter" => Arc::new(OpenRouterWrapper::new(router_config, providers, default_provider)?),
            "ollama" => Arc::new(OllamaRouter::new(router_config)?),
            _ => anyhow::bail!("Unknown routing mode: {}", router_config.mode),
        };

        info!(
            "Created LLM router: {} (mode={})",
            router.name(),
            router_config.mode
        );
        Ok(router)
    }
}
