//! Anthropic (Claude) Provider implementation (with streaming support)

use anyhow::Context;
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use std::time::Duration;
use tokio::sync::mpsc;

use super::parse_env_var;
use crate::llm::factory::ProviderConfig;
use crate::llm::sse;
use crate::llm::types::{ChatDelta, ChatMessage, ChatOptions, ChatResponse, MessageRole, Usage};
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

    fn build_messages(
        messages: &[ChatMessage],
    ) -> (String, Vec<serde_json::Value>) {
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
                    let mut assistant_msg = serde_json::json!({
                        "role": "assistant",
                    });
                    // Build Anthropic-style content blocks
                    let mut content_blocks: Vec<serde_json::Value> = Vec::new();
                    if !msg.content.is_empty() {
                        content_blocks.push(serde_json::json!({"type": "text", "text": msg.content}));
                    }
                    if let Some(ref tool_calls) = msg.tool_calls {
                        for tc in tool_calls {
                            let input: serde_json::Value =
                                serde_json::from_str(&tc.arguments)
                                    .unwrap_or_else(|_| serde_json::json!({}));
                            let input = if input.is_object() {
                                input
                            } else {
                                serde_json::json!({})
                            };
                            content_blocks.push(serde_json::json!({
                                "type": "tool_use",
                                "id": tc.id,
                                "name": tc.name,
                                "input": input,
                            }));
                        }
                    }
                    if content_blocks.is_empty() {
                        assistant_msg["content"] = serde_json::json!(msg.content);
                    } else {
                        assistant_msg["content"] = serde_json::json!(content_blocks);
                    }
                    chat_messages.push(assistant_msg);
                }
                MessageRole::Tool => {
                    // Anthropic expects tool results inside a "user" role message
                    chat_messages.push(serde_json::json!({
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": msg.tool_call_id,
                            "content": msg.content,
                        }],
                    }));
                }
            }
        }

        (system_prompt, chat_messages)
    }

    fn build_payload(
        &self,
        messages: &[ChatMessage],
        options: &ChatOptions,
        stream: bool,
    ) -> serde_json::Value {
        let (system_prompt, chat_messages) = Self::build_messages(messages);
        let max_tokens_to_sample = options.max_tokens.unwrap_or(self.max_tokens);

        let mut payload = serde_json::json!({
            "model": self.model,
            "messages": chat_messages,
            "max_tokens": max_tokens_to_sample,
            "stream": stream,
        });

        if !system_prompt.is_empty() {
            payload["system"] = serde_json::json!(system_prompt);
        }
        if let Some(temp) = options.temperature {
            payload["temperature"] = serde_json::json!(temp);
        }
        if let Some(ref tools) = options.tools {
            payload["tools"] = serde_json::json!(tools.iter().map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
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
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            let _ = tx.send(ChatDelta::Error(format!("Anthropic API error: {error_text}")));
            anyhow::bail!("Anthropic API error: {}", error_text);
        }

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("failed to read Anthropic response stream chunk")?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(pos) = buffer.find("\n\n") {
                let event_block = buffer[..pos].to_string();
                buffer = buffer[pos + 2..].to_string();

                for (event_type, data) in sse::iter_sse_lines(&event_block) {
                    if let Some(deltas) =
                        sse::parse_anthropic_sse_event(&event_type, &data)
                    {
                        for delta in deltas {
                            if tx.send(delta).is_err() {
                                return Ok(());
                            }
                        }
                    }
                }
            }
        }

        // Flush remaining buffer
        if !buffer.trim().is_empty() {
            for (event_type, data) in sse::iter_sse_lines(&buffer) {
                if let Some(deltas) = sse::parse_anthropic_sse_event(&event_type, &data) {
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
                tracing::warn!("Anthropic health check failed: {}", e);
                false
            }
        }
    }
}
