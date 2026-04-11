//! Construct a default chat [`LLMProvider`](crate::llm::traits::LLMProvider) from application config.

use std::sync::Arc;

use crate::config::Config;
use crate::llm::factory::{LLMProviderFactory, ProviderConfig};
use crate::llm::providers::parse_env_var;
use crate::llm::traits::LLMProvider;

/// Returns the configured default provider when `llm.enabled` and that provider entry are usable.
pub fn default_chat_provider(config: &Config) -> anyhow::Result<Option<Arc<dyn LLMProvider>>> {
    if !config.llm.enabled {
        return Ok(None);
    }
    let name = config.llm.default_provider.as_str();
    let Some(p) = config.llm.providers.get(name) else {
        anyhow::bail!(
            "LLM default_provider '{}' is not defined under [llm.providers]",
            name
        );
    };
    if !p.enabled {
        return Ok(None);
    }
    let api_key = p
        .api_key
        .as_deref()
        .map(parse_env_var)
        .unwrap_or_default();
    let factory_cfg = ProviderConfig {
        provider_type: p.provider_type.clone(),
        api_key,
        base_url: p.base_url.clone(),
        model: p.model.clone(),
        max_tokens: p.max_tokens,
        timeout_secs: p.timeout_secs,
    };
    Ok(Some(LLMProviderFactory::create(name, &factory_cfg)?))
}
