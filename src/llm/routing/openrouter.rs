//! OpenRouter wrapper - routes through OpenRouter gateway

use async_trait::async_trait;
use reqwest::Client;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::llm::factory::RouterConfig;
use crate::llm::traits::{LLMProvider, LLMRouter};
use crate::llm::types::{ChatMessage, ChatOptions, ChatResponse, Usage};

/// OpenRouter wrapper - provides unified API through OpenRouter gateway
pub struct OpenRouterWrapper {
    client: Client,
    api_key: String,
    base_url: String,
    providers: HashMap<String, Arc<dyn LLMProvider>>,
    default_provider: String,
}

impl OpenRouterWrapper {
    pub fn new(
        config: &RouterConfig,
        providers: HashMap<String, Arc<dyn LLMProvider>>,
        default_provider: &str,
    ) -> anyhow::Result<Self> {
        let api_key = config
            .openrouter_api_key
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("OpenRouter API key not provided"))?
            .clone();

        let base_url = config
            .openrouter_base_url
            .clone()
            .unwrap_or_else(|| "https://openrouter.ai/api".to_string());

        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(120))
                .build()?,
            api_key,
            base_url,
            providers,
            default_provider: default_provider.to_string(),
        })
    }
}

#[async_trait]
impl LLMRouter for OpenRouterWrapper {
    async fn route(
        &self,
        messages: Vec<ChatMessage>,
        options: Option<ChatOptions>,
    ) -> anyhow::Result<ChatResponse> {
        // Get model from default provider for routing
        let model = self
            .providers
            .get(&self.default_provider)
            .map(|p| p.name().to_string())
            .unwrap_or_else(|| "anthropic/claude-3.5-sonnet".to_string());

        let opts = options.unwrap_or_default();

        let payload = serde_json::json!({
            "model": model,
            "messages": messages.iter().map(|m| {
                serde_json::json!({
                    "role": m.role.as_str(),
                    "content": m.content,
                })
            }).collect::<Vec<_>>(),
            "max_tokens": opts.max_tokens.unwrap_or(1024),
            "temperature": opts.temperature.unwrap_or(0.7),
        });

        let response = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .header("HTTP-Referer", "https://aiclaw.dev")
            .header("X-Title", "AIClaw")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            anyhow::bail!("OpenRouter API error: {}", error_text);
        }

        let raw: serde_json::Value = response.json().await?;

        let content = raw["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        let usage = Usage {
            prompt_tokens: raw["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32,
            completion_tokens: raw["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32,
            total_tokens: raw["usage"]["total_tokens"].as_u64().unwrap_or(0) as u32,
        };

        Ok(ChatResponse {
            content,
            model: raw["model"].as_str().unwrap_or(&model).to_string(),
            provider: "openrouter".to_string(),
            usage,
            raw_response: raw,
        })
    }

    fn name(&self) -> &str {
        "openrouter"
    }

    fn available_providers(&self) -> Vec<String> {
        self.providers.keys().cloned().collect()
    }
}
