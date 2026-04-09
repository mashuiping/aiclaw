//! DeepSeek Provider implementation

use async_trait::async_trait;
use reqwest::Client;
use std::time::Duration;

use super::parse_env_var;
use crate::llm::factory::ProviderConfig;
use crate::llm::types::{ChatMessage, ChatOptions, ChatResponse, Usage};
use crate::llm::LLMProvider;

/// DeepSeek API provider
pub struct DeepSeekProvider {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
    max_tokens: u32,
}

impl DeepSeekProvider {
    pub fn new(config: &ProviderConfig) -> anyhow::Result<Self> {
        let api_key = parse_env_var(&config.api_key);
        let base_url = config
            .base_url
            .as_ref()
            .map(|s| parse_env_var(s))
            .unwrap_or_else(|| "https://api.deepseek.com".to_string());

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
impl LLMProvider for DeepSeekProvider {
    fn name(&self) -> &str {
        "deepseek"
    }

    fn provider_type(&self) -> &str {
        "deepseek"
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
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            anyhow::bail!("DeepSeek API error: {}", error_text);
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
            model: raw["model"].as_str().unwrap_or(&self.model).to_string(),
            provider: "deepseek".to_string(),
            usage,
            raw_response: raw,
        })
    }

    async fn health_check(&self) -> bool {
        match self.chat(vec![ChatMessage::user("Hi")], None).await {
            Ok(_) => true,
            Err(e) => {
                tracing::warn!("DeepSeek health check failed: {}", e);
                false
            }
        }
    }
}
