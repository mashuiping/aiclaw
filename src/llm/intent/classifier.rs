//! LLM-based intent classifier

use std::sync::Arc;

use async_trait::async_trait;
use tracing::{debug, warn};

use crate::llm::traits::{
    IntentClassification, IntentClassifier, IntentEntities, IntentParseResult,
    INTENT_CLASSIFICATION_PROMPT,
};
use crate::llm::types::{ChatMessage, ChatOptions};
use crate::llm::LLMProvider;

/// LLM-based intent classifier implementation
pub struct IntentClassifierImpl {
    provider: Arc<dyn LLMProvider>,
    fallback_enabled: bool,
}

impl IntentClassifierImpl {
    pub fn new(provider: Arc<dyn LLMProvider>) -> Self {
        Self {
            provider,
            fallback_enabled: true,
        }
    }

    pub fn with_fallback(mut self, enabled: bool) -> Self {
        self.fallback_enabled = enabled;
        self
    }

    fn extract_json_from_response(content: &str) -> Option<String> {
        // Try to find JSON in the response
        // Sometimes LLM wraps JSON in markdown code blocks
        let content = content.trim();

        // Check for markdown code block
        if content.contains("```json") {
            let start = content.find("```json").unwrap() + 7;
            let end = content[start..].find("```").map(|i| start + i);
            return end.map(|e| content[start..e].trim().to_string());
        }

        if content.contains("```") {
            let start = content.find("```").unwrap() + 3;
            let end = content[start..].find("```").map(|i| start + i);
            return end.map(|e| content[start..e].trim().to_string());
        }

        // Try to find JSON directly
        if content.starts_with('{') {
            Some(content.to_string())
        } else if let Some(start) = content.find('{') {
            let remaining = &content[start..];
            // Find matching closing brace
            let mut depth = 0;
            for (i, c) in remaining.chars().enumerate() {
                if c == '{' {
                    depth += 1;
                } else if c == '}' {
                    depth -= 1;
                    if depth == 0 {
                        return Some(remaining[..=i].to_string());
                    }
                }
            }
        }

        None
    }

    fn fallback_classify(message: &str) -> IntentClassification {
        // Simple rule-based fallback
        let message_lower = message.to_lowercase();

        let (intent_type, confidence) = if message_lower.contains("日志")
            || message_lower.contains("log")
            || message_lower.contains("tail")
        {
            ("Logs", 0.6)
        } else if message_lower.contains("指标")
            || message_lower.contains("metric")
            || message_lower.contains("监控")
            || message_lower.contains("promql")
        {
            ("Metrics", 0.6)
        } else if message_lower.contains("健康")
            || message_lower.contains("health")
            || message_lower.contains("状态")
        {
            ("Health", 0.6)
        } else if message_lower.contains("排查")
            || message_lower.contains("debug")
            || message_lower.contains("问题")
            || message_lower.contains("为什么")
        {
            ("Debug", 0.6)
        } else if message_lower.contains("查询")
            || message_lower.contains("query")
            || message_lower.contains("搜索")
        {
            ("Query", 0.6)
        } else if message_lower.contains("扩缩")
            || message_lower.contains("scale")
        {
            ("Scale", 0.6)
        } else if message_lower.contains("部署")
            || message_lower.contains("deploy")
            || message_lower.contains("发布")
        {
            ("Deploy", 0.6)
        } else {
            ("Unknown", 0.3)
        };

        IntentClassification {
            intent_type: intent_type.to_string(),
            confidence,
            entities: IntentEntities::default(),
            reasoning: Some("Fallback classification based on keywords".to_string()),
        }
    }
}

#[async_trait]
impl IntentClassifier for IntentClassifierImpl {
    async fn classify(&self, message: &str) -> anyhow::Result<IntentClassification> {
        debug!("Classifying intent for: {}", message);

        let messages = vec![
            ChatMessage::system(INTENT_CLASSIFICATION_PROMPT),
            ChatMessage::user(message),
        ];

        let options = ChatOptions::new()
            .with_temperature(0.1) // Low temperature for structured output
            .with_max_tokens(512);

        let response = self.provider.chat(messages, Some(options)).await?;

        debug!("LLM response: {}", response.content);

        // Parse JSON response
        if let Some(json_str) = Self::extract_json_from_response(&response.content) {
            match serde_json::from_str::<IntentParseResult>(&json_str) {
                Ok(result) => {
                    debug!(
                        "Parsed intent: {} (confidence: {})",
                        result.intent, result.confidence
                    );
                    return Ok(result.into());
                }
                Err(e) => {
                    warn!("Failed to parse intent JSON: {}", e);
                }
            }
        }

        // Fallback to rule-based if parsing fails
        if self.fallback_enabled {
            warn!("Using fallback classification");
            Ok(Self::fallback_classify(message))
        } else {
            anyhow::bail!("Failed to parse LLM response as JSON and fallback is disabled")
        }
    }

    fn provider(&self) -> Arc<dyn LLMProvider> {
        self.provider.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_from_response() {
        // Plain JSON
        let json = r#"{"intent": "Debug", "confidence": 0.9}"#;
        assert!(IntentClassifierImpl::extract_json_from_response(json).is_some());

        // JSON in markdown code block
        let wrapped = r#"```json
{"intent": "Debug", "confidence": 0.9}
```"#;
        assert!(IntentClassifierImpl::extract_json_from_response(wrapped).is_some());

        // JSON in text
        let text = r#"Based on the query, the intent is:\n```json\n{"intent": "Debug", "confidence": 0.9}\n```"#;
        assert!(IntentClassifierImpl::extract_json_from_response(text).is_some());
    }

    #[test]
    fn test_fallback_classify() {
        let result = IntentClassifierImpl::fallback_classify("查看 pod nginx 日志");
        assert_eq!(result.intent_type, "Logs");
        assert_eq!(result.confidence, 0.6);

        let result = IntentClassifierImpl::fallback_classify("排查支付服务响应慢");
        assert_eq!(result.intent_type, "Debug");
        assert_eq!(result.confidence, 0.6);
    }
}
