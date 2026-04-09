//! Ollama local router

use async_trait::async_trait;
use reqwest::Client;
use std::time::Duration;

use crate::llm::factory::RouterConfig;
use crate::llm::traits::LLMRouter;
use crate::llm::types::{ChatMessage, ChatOptions, ChatResponse, Usage};

/// Ollama router for local LLM inference
pub struct OllamaRouter {
    client: Client,
    base_url: String,
    model: String,
}

impl OllamaRouter {
    pub fn new(config: &RouterConfig) -> anyhow::Result<Self> {
        let base_url = config
            .ollama_base_url
            .clone()
            .unwrap_or_else(|| "http://localhost:11434".to_string());

        let model = config
            .ollama_model
            .clone()
            .unwrap_or_else(|| "llama3".to_string());

        Ok(Self {
            client: Client::builder()
                .timeout(Duration::from_secs(120))
                .build()?,
            base_url,
            model,
        })
    }
}

#[async_trait]
impl LLMRouter for OllamaRouter {
    async fn route(
        &self,
        messages: Vec<ChatMessage>,
        options: Option<ChatOptions>,
    ) -> anyhow::Result<ChatResponse> {
        let opts = options.unwrap_or_default();

        let ollama_messages: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| {
                serde_json::json!({
                    "role": m.role.as_str(),
                    "content": m.content,
                })
            })
            .collect();

        let mut payload = serde_json::json!({
            "model": self.model,
            "messages": ollama_messages,
            "stream": false,
        });

        if let Some(temp) = opts.temperature {
            payload["temperature"] = serde_json::json!(temp);
        }

        if let Some(max_tokens) = opts.max_tokens {
            payload["options"]["num_predict"] = serde_json::json!(max_tokens);
        }

        let response = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            anyhow::bail!("Ollama API error: {}", error_text);
        }

        let raw: serde_json::Value = response.json().await?;

        let content = raw["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        let usage = Usage {
            prompt_tokens: raw["prompt_eval_count"].as_u64().unwrap_or(0) as u32,
            completion_tokens: raw["eval_count"].as_u64().unwrap_or(0) as u32,
            total_tokens: raw["prompt_eval_count"].as_u64().unwrap_or(0) as u32
                + raw["eval_count"].as_u64().unwrap_or(0) as u32,
        };

        Ok(ChatResponse {
            content,
            model: raw["model"].as_str().unwrap_or(&self.model).to_string(),
            provider: "ollama".to_string(),
            usage,
            raw_response: raw,
        })
    }

    fn name(&self) -> &str {
        "ollama"
    }

    fn available_providers(&self) -> Vec<String> {
        vec!["ollama".to_string()]
    }
}
