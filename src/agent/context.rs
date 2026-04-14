//! Context window management with LLM-based compaction.
//!
//! Monitors estimated token usage and compacts conversation history when
//! approaching the context limit. Old messages are summarized into a single
//! `[Summary]` message while the most recent messages are kept verbatim.

use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::llm::traits::LLMProvider;
use crate::llm::types::{ChatMessage, ChatOptions, MessageRole};

const CHARS_PER_TOKEN_ESTIMATE: usize = 4;
const DEFAULT_MAX_CONTEXT_TOKENS: usize = 100_000;
const COMPACTION_THRESHOLD_RATIO: f64 = 0.75;
const KEEP_RECENT_MESSAGES: usize = 6;

/// Routes for preemptive context management.
#[derive(Debug, PartialEq)]
pub enum CompactionRoute {
    Fits,
    CompactOnly,
    CompactAndTruncateTools,
}

/// Manages the context window, compacting when necessary.
pub struct ContextManager {
    provider: Arc<dyn LLMProvider>,
    max_context_tokens: usize,
}

impl ContextManager {
    pub fn new(provider: Arc<dyn LLMProvider>) -> Self {
        Self {
            provider,
            max_context_tokens: DEFAULT_MAX_CONTEXT_TOKENS,
        }
    }

    pub fn with_max_tokens(mut self, max_tokens: usize) -> Self {
        self.max_context_tokens = max_tokens;
        self
    }

    /// Rough token count estimate: chars / 4.
    pub fn estimate_tokens(messages: &[ChatMessage]) -> usize {
        let total_chars: usize = messages.iter().map(|m| m.content.len()).sum();
        total_chars / CHARS_PER_TOKEN_ESTIMATE
    }

    /// Decide what action to take before the next LLM call.
    pub fn preemptive_check(&self, messages: &[ChatMessage]) -> CompactionRoute {
        let estimated = Self::estimate_tokens(messages);
        let threshold = (self.max_context_tokens as f64 * COMPACTION_THRESHOLD_RATIO) as usize;

        if estimated <= threshold {
            CompactionRoute::Fits
        } else if estimated <= self.max_context_tokens {
            CompactionRoute::CompactOnly
        } else {
            CompactionRoute::CompactAndTruncateTools
        }
    }

    /// Log context utilization for observability.
    pub fn log_utilization(&self, messages: &[ChatMessage]) {
        let estimated = Self::estimate_tokens(messages);
        let pct = (estimated as f64 / self.max_context_tokens as f64 * 100.0) as u32;
        debug!(
            estimated_tokens = estimated,
            max_tokens = self.max_context_tokens,
            utilization_pct = pct,
            message_count = messages.len(),
            "Context window utilization"
        );
    }

    /// Compact conversation history if over threshold.
    ///
    /// Summarizes older messages into a single summary message via LLM,
    /// keeping the system prompt and the most recent `KEEP_RECENT_MESSAGES`.
    pub async fn compact_if_needed(
        &self,
        messages: &mut Vec<ChatMessage>,
    ) -> anyhow::Result<()> {
        let route = self.preemptive_check(messages);
        self.log_utilization(messages);

        match route {
            CompactionRoute::Fits => return Ok(()),
            CompactionRoute::CompactOnly | CompactionRoute::CompactAndTruncateTools => {}
        }

        info!(
            route = ?route,
            message_count = messages.len(),
            "Compacting conversation context"
        );

        if messages.len() <= KEEP_RECENT_MESSAGES + 1 {
            // Not enough messages to compact (system + recent)
            return Ok(());
        }

        // Separate system prompt, compactable messages, and recent messages.
        let system_msg = if messages.first().map(|m| &m.role) == Some(&MessageRole::System) {
            Some(messages[0].clone())
        } else {
            None
        };

        let start_idx = if system_msg.is_some() { 1 } else { 0 };
        let keep_from = messages.len().saturating_sub(KEEP_RECENT_MESSAGES);

        if keep_from <= start_idx {
            return Ok(());
        }

        let to_summarize = &messages[start_idx..keep_from];
        if to_summarize.is_empty() {
            return Ok(());
        }

        // Build a summarization request
        let history_text: String = to_summarize
            .iter()
            .map(|m| format!("[{}]: {}", m.role.as_str(), truncate_for_summary(&m.content)))
            .collect::<Vec<_>>()
            .join("\n\n");

        let summary = self.summarize_history(&history_text).await?;

        // Rebuild messages: system + summary + recent
        let recent = messages[keep_from..].to_vec();
        messages.clear();

        if let Some(sys) = system_msg {
            messages.push(sys);
        }

        messages.push(ChatMessage::system(format!(
            "<context_summary>\n{}\n</context_summary>",
            summary
        )));

        messages.extend(recent);

        let new_estimated = Self::estimate_tokens(messages);
        info!(
            new_message_count = messages.len(),
            new_estimated_tokens = new_estimated,
            "Context compaction complete"
        );

        Ok(())
    }

    async fn summarize_history(&self, history_text: &str) -> anyhow::Result<String> {
        let prompt = format!(
            "Summarize the following conversation history into a concise summary. \
             Preserve key facts, decisions, diagnostic findings, and entity names \
             (pods, namespaces, clusters, services). Remove redundant details.\n\n\
             Conversation:\n{}\n\nProvide a structured summary.",
            history_text
        );

        let messages = vec![
            ChatMessage::system(
                "You are a conversation summarizer. Produce concise, factual summaries \
                 that preserve all important context for an ongoing diagnostic session."
            ),
            ChatMessage::user(&prompt),
        ];

        let options = ChatOptions::new()
            .with_temperature(0.1)
            .with_max_tokens(1024);

        match self.provider.chat(messages, Some(options)).await {
            Ok(response) => Ok(response.content),
            Err(e) => {
                warn!("LLM-based compaction failed, using simple truncation: {}", e);
                Ok(simple_truncate_summary(history_text))
            }
        }
    }
}

fn truncate_for_summary(content: &str) -> String {
    const MAX_PER_MSG: usize = 500;
    if content.len() <= MAX_PER_MSG * CHARS_PER_TOKEN_ESTIMATE {
        return content.to_string();
    }
    let boundary = content
        .char_indices()
        .nth(MAX_PER_MSG)
        .map(|(i, _)| i)
        .unwrap_or(content.len());
    format!("{}... [truncated]", &content[..boundary])
}

fn simple_truncate_summary(history: &str) -> String {
    const MAX_SUMMARY_CHARS: usize = 2000;
    if history.len() <= MAX_SUMMARY_CHARS {
        return history.to_string();
    }
    let boundary = history
        .char_indices()
        .nth(MAX_SUMMARY_CHARS)
        .map(|(i, _)| i)
        .unwrap_or(history.len());
    format!(
        "{}...\n[Earlier conversation truncated due to context limits]",
        &history[..boundary]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_messages(count: usize) -> Vec<ChatMessage> {
        let mut msgs = vec![ChatMessage::system("You are a helpful assistant.")];
        for i in 0..count {
            if i % 2 == 0 {
                msgs.push(ChatMessage::user(format!("Question {}", i)));
            } else {
                msgs.push(ChatMessage::assistant(format!("Answer {}", i)));
            }
        }
        msgs
    }

    #[test]
    fn estimate_tokens_roughly_correct() {
        let msgs = vec![ChatMessage::user("hello world")]; // 11 chars
        let est = ContextManager::estimate_tokens(&msgs);
        assert_eq!(est, 2); // 11 / 4 = 2
    }

    #[test]
    fn small_context_fits() {
        let msgs = make_messages(4);
        let cm = ContextManager::new(Arc::new(MockProvider));
        assert_eq!(cm.preemptive_check(&msgs), CompactionRoute::Fits);
    }

    #[test]
    fn large_context_triggers_compaction() {
        let mut msgs = vec![ChatMessage::system("sys")];
        for _ in 0..200 {
            msgs.push(ChatMessage::user("x".repeat(3000)));
        }
        let cm = ContextManager::new(Arc::new(MockProvider))
            .with_max_tokens(1000);
        assert_ne!(cm.preemptive_check(&msgs), CompactionRoute::Fits);
    }

    struct MockProvider;

    #[async_trait::async_trait]
    impl LLMProvider for MockProvider {
        fn name(&self) -> &str { "mock" }
        fn provider_type(&self) -> &str { "mock" }
        async fn chat(
            &self,
            _messages: Vec<ChatMessage>,
            _options: Option<ChatOptions>,
        ) -> anyhow::Result<crate::llm::types::ChatResponse> {
            Ok(crate::llm::types::ChatResponse {
                content: "Summary of conversation.".to_string(),
                model: "mock".to_string(),
                provider: "mock".to_string(),
                usage: crate::llm::types::Usage::zero(),
                raw_response: serde_json::Value::Null,
            })
        }
        async fn health_check(&self) -> bool { true }
    }
}
