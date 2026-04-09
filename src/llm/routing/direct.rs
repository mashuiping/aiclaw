//! Direct routing - routes to configured providers directly

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

use crate::llm::traits::{LLMProvider, LLMRouter};
use crate::llm::types::{ChatMessage, ChatOptions, ChatResponse};

/// Direct router - routes to configured providers without additional gateway
pub struct DirectRouter {
    providers: HashMap<String, Arc<dyn LLMProvider>>,
    default_provider: String,
}

impl DirectRouter {
    pub fn new(
        providers: HashMap<String, Arc<dyn LLMProvider>>,
        default_provider: &str,
    ) -> Self {
        Self {
            providers,
            default_provider: default_provider.to_string(),
        }
    }
}

#[async_trait]
impl LLMRouter for DirectRouter {
    async fn route(
        &self,
        messages: Vec<ChatMessage>,
        options: Option<ChatOptions>,
    ) -> anyhow::Result<ChatResponse> {
        let provider = self
            .providers
            .get(&self.default_provider)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Default provider '{}' not found. Available: {:?}",
                    self.default_provider,
                    self.providers.keys().collect::<Vec<_>>()
                )
            })?;

        provider.chat(messages, options).await
    }

    fn name(&self) -> &str {
        "direct"
    }

    fn available_providers(&self) -> Vec<String> {
        self.providers.keys().cloned().collect()
    }
}
