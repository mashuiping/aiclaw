//! MiniMax Provider implementation

use async_trait::async_trait;
use reqwest::Client;
use std::time::Duration;

use super::parse_env_var;
use crate::llm::factory::ProviderConfig;
use crate::llm::types::{ChatMessage, ChatOptions, ChatResponse, Usage};
use crate::llm::LLMProvider;

/// MiniMax API provider
pub struct MiniMaxProvider {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
    max_tokens: u32,
}

impl MiniMaxProvider {
    pub fn new(config: &ProviderConfig) -> anyhow::Result<Self> {
        let api_key = parse_env_var(&config.api_key);
        let base_url = config
            .base_url
            .as_ref()
            .map(|s| parse_env_var(s))
            .unwrap_or_else(|| "https://api.minimax.chat".to_string());

        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(config.timeout_secs))
                .build()?,
            api_key,
            base_url,
            model: config.model.clone(),
            max_tokens: config.max_tokens,
        })
    }
}

#[async_trait]
impl LLMProvider for MiniMaxProvider {
    fn name(&self) -> &str {
        "minimax"
    }

    fn provider_type(&self) -> &str {
        "minimax"
    }

    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        options: Option<ChatOptions>,
    ) -> anyhow::Result<ChatResponse> {
        let opts = options.unwrap_or_default();

        let payload = serde_json::json!({
            "model": self.model,
            "messages": messages.iter().map(|m| {
                serde_json::json!({
                    "role": m.role.as_str(),
                    "content": m.content,
                })
            }).collect::<Vec<_>>(),
            "max_tokens": opts.max_tokens.unwrap_or(self.max_tokens),
            "temperature": opts.temperature.unwrap_or(0.7),
        });

        let response = self
            .client
            .post(format!("{}/v1/text/chatcompletion_v2", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            anyhow::bail!("MiniMax API error: {}", error_text);
        }

        let raw: serde_json::Value = response.json().await?;

        let content = raw["choices"][0]["messages"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|m| m["text"].as_str())
            .unwrap_or("")
            .to_string();

        let usage = Usage {
            prompt_tokens: raw["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32,
            completion_tokens: raw["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32,
            total_tokens: raw["usage"]["total_tokens"].as_u64().unwrap_or(0) as u32,
        };

        Ok(ChatResponse {
            content,
            model: raw["model"].as_str().unwrap_or(&self.model).to_string(),
            provider: "minimax".to_string(),
            usage,
            raw_response: raw,
        })
    }

    async fn health_check(&self) -> bool {
        match self.chat(vec![ChatMessage::user("Hi")], None).await {
            Ok(_) => true,
            Err(e) => {
                tracing::warn!("MiniMax health check failed: {}", e);
                false
            }
        }
    }
}
