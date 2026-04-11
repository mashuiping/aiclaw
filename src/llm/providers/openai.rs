//! OpenAI Provider implementation (with streaming support)

use anyhow::Context;
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use std::time::Duration;
use tokio::sync::mpsc;

use super::parse_env_var;
use crate::llm::factory::ProviderConfig;
use crate::llm::sse;
use crate::llm::types::{ChatDelta, ChatMessage, ChatOptions, ChatResponse, Usage};
use crate::llm::LLMProvider;

/// OpenAI API compatible provider (OpenAI, Azure OpenAI, DeepSeek, etc.)
pub struct OpenAIProvider {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
    max_tokens: u32,
}

impl OpenAIProvider {
    pub fn new(config: &ProviderConfig) -> anyhow::Result<Self> {
        let api_key = parse_env_var(&config.api_key);
        let base_url = config
            .base_url
            .as_ref()
            .map(|s| parse_env_var(s))
            .unwrap_or_else(|| "https://api.openai.com".to_string());

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

    fn build_payload(
        &self,
        messages: &[ChatMessage],
        options: &ChatOptions,
        stream: bool,
    ) -> serde_json::Value {
        let wire_messages: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| {
                let mut msg = serde_json::json!({
                    "role": m.role.as_str(),
                    "content": m.content,
                });
                if let Some(ref tc) = m.tool_calls {
                    msg["tool_calls"] = serde_json::json!(tc.iter().map(|t| {
                        let args = if serde_json::from_str::<serde_json::Value>(&t.arguments).is_ok() {
                            t.arguments.clone()
                        } else {
                            "{}".to_string()
                        };
                        serde_json::json!({
                            "id": t.id,
                            "type": "function",
                            "function": {
                                "name": t.name,
                                "arguments": args,
                            }
                        })
                    }).collect::<Vec<_>>());
                }
                if let Some(ref id) = m.tool_call_id {
                    msg["tool_call_id"] = serde_json::json!(id);
                }
                msg
            })
            .collect();

        let mut payload = serde_json::json!({
            "model": self.model,
            "messages": wire_messages,
            "max_tokens": options.max_tokens.unwrap_or(self.max_tokens),
            "temperature": options.temperature.unwrap_or(0.7),
            "stream": stream,
        });
        if stream {
            payload["stream_options"] = serde_json::json!({"include_usage": true});
        }
        if let Some(ref tools) = options.tools {
            payload["tools"] = serde_json::json!(tools.iter().map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }
                })
            }).collect::<Vec<_>>());
        }
        payload
    }

    async fn chat_internal(
        &self,
        messages: Vec<ChatMessage>,
        options: Option<ChatOptions>,
    ) -> anyhow::Result<ChatResponse> {
        let opts = options.unwrap_or_default();
        let payload = self.build_payload(&messages, &opts, false);

        let mut request = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json");

        if let Some(user) = opts.user {
            request = request.header("User", user);
        }

        let response = request.json(&payload).send().await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            anyhow::bail!("OpenAI API error: {}", error_text);
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
            provider: "openai".to_string(),
            usage,
            raw_response: raw,
        })
    }
}

#[async_trait]
impl LLMProvider for OpenAIProvider {
    fn name(&self) -> &str {
        "openai"
    }

    fn provider_type(&self) -> &str {
        "openai"
    }

    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        options: Option<ChatOptions>,
    ) -> anyhow::Result<ChatResponse> {
        self.chat_internal(messages, options).await
    }

    async fn stream_chat(
        &self,
        messages: Vec<ChatMessage>,
        options: Option<ChatOptions>,
        tx: mpsc::UnboundedSender<ChatDelta>,
    ) -> anyhow::Result<()> {
        let opts = options.unwrap_or_default();
        let payload = self.build_payload(&messages, &opts, true);

        let response = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            let _ = tx.send(ChatDelta::Error(format!("OpenAI API error: {error_text}")));
            anyhow::bail!("OpenAI API error: {}", error_text);
        }

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("failed to read OpenAI response stream chunk")?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            // Process complete SSE events from the buffer
            while let Some(pos) = buffer.find("\n\n") {
                let event_block = buffer[..pos].to_string();
                buffer = buffer[pos + 2..].to_string();

                for (_event_type, data) in sse::iter_sse_lines(&event_block) {
                    match sse::parse_openai_sse_data(&data) {
                        Some(deltas) => {
                            for delta in deltas {
                                if tx.send(delta).is_err() {
                                    return Ok(());
                                }
                            }
                        }
                        None => {
                            // [DONE] — stream finished
                            return Ok(());
                        }
                    }
                }
            }
        }

        // Process remaining buffer
        if !buffer.trim().is_empty() {
            for (_event_type, data) in sse::iter_sse_lines(&buffer) {
                if let Some(deltas) = sse::parse_openai_sse_data(&data) {
                    for delta in deltas {
                        let _ = tx.send(delta);
                    }
                }
            }
        }

        Ok(())
    }

    async fn health_check(&self) -> bool {
        match self.chat(vec![ChatMessage::user("Hi")], None).await {
            Ok(_) => true,
            Err(e) => {
                tracing::warn!("OpenAI health check failed: {}", e);
                false
            }
        }
    }
}
