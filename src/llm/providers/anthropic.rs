//! Anthropic (Claude) Provider implementation

use async_trait::async_trait;
use reqwest::Client;
use std::time::Duration;

use super::parse_env_var;
use crate::llm::factory::ProviderConfig;
use crate::llm::types::{ChatMessage, ChatOptions, ChatResponse, MessageRole, Usage};
use crate::llm::LLMProvider;

/// Anthropic Claude API provider
pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
    max_tokens: u32,
}

impl AnthropicProvider {
    pub fn new(config: &ProviderConfig) -> anyhow::Result<Self> {
        let api_key = parse_env_var(&config.api_key);
        let base_url = config
            .base_url
            .as_ref()
            .map(|s| parse_env_var(s))
            .unwrap_or_else(|| "https://api.anthropic.com".to_string());

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

    async fn chat_internal(
        &self,
        messages: Vec<ChatMessage>,
        options: Option<ChatOptions>,
    ) -> anyhow::Result<ChatResponse> {
        let opts = options.unwrap_or_default();

        // Convert messages to Anthropic format
        let mut system_prompt = String::new();
        let mut chat_messages: Vec<serde_json::Value> = Vec::new();

        for msg in messages {
            match msg.role {
                MessageRole::System => {
                    if !system_prompt.is_empty() {
                        system_prompt.push('\n');
                    }
                    system_prompt.push_str(&msg.content);
                }
                MessageRole::User => {
                    chat_messages.push(serde_json::json!({
                        "role": "user",
                        "content": msg.content,
                    }));
                }
                MessageRole::Assistant => {
                    chat_messages.push(serde_json::json!({
                        "role": "assistant",
                        "content": msg.content,
                    }));
                }
            }
        }

        let max_tokens_to_sample = opts.max_tokens.unwrap_or(self.max_tokens);

        let mut payload = serde_json::json!({
            "model": self.model,
            "messages": chat_messages,
            "max_tokens": max_tokens_to_sample,
        });

        if !system_prompt.is_empty() {
            payload["system"] = serde_json::json!(system_prompt);
        }

        if let Some(temp) = opts.temperature {
            payload["temperature"] = serde_json::json!(temp);
        }

        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            anyhow::bail!("Anthropic API error: {}", error_text);
        }

        let raw: serde_json::Value = response.json().await?;

        let content = raw["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string();

        let usage = Usage {
            prompt_tokens: raw["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32,
            completion_tokens: raw["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32,
            total_tokens: raw["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32
                + raw["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32,
        };

        Ok(ChatResponse {
            content,
            model: raw["model"].as_str().unwrap_or(&self.model).to_string(),
            provider: "anthropic".to_string(),
            usage,
            raw_response: raw,
        })
    }
}

#[async_trait]
impl LLMProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn provider_type(&self) -> &str {
        "anthropic"
    }

    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        options: Option<ChatOptions>,
    ) -> anyhow::Result<ChatResponse> {
        self.chat_internal(messages, options).await
    }

    async fn health_check(&self) -> bool {
        match self.chat(vec![ChatMessage::user("Hi")], None).await {
            Ok(_) => true,
            Err(e) => {
                tracing::warn!("Anthropic health check failed: {}", e);
                false
            }
        }
    }
}
