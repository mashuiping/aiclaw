//! Intent parsing - combines LLM-based and rule-based parsing

use aiclaw_types::agent::{Intent, IntentEntities, IntentType};
use regex::Regex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, warn};

use crate::llm::intent::IntentClassifierImpl;
use crate::llm::traits::{IntentClassifier, LLMProvider};

/// Intent parser - parses user messages into structured intents
/// Uses LLM when available, falls back to regex-based rules
pub struct IntentParser {
    patterns: Vec<IntentPattern>,
    llm_classifier: Option<Arc<dyn IntentClassifier>>,
    llm_timeout: Duration,
}

struct IntentPattern {
    intent_type: IntentType,
    keywords: Vec<String>,
    regex: Regex,
    entity_extractor: Regex,
}

impl IntentParser {
    /// Create a new parser with optional LLM classifier
    pub fn new() -> Self {
        Self {
            patterns: Self::build_patterns(),
            llm_classifier: None,
            llm_timeout: Duration::from_secs(10),
        }
    }

    /// Create parser with LLM classifier
    pub fn with_llm(provider: Arc<dyn LLMProvider>) -> Self {
        let classifier = Arc::new(IntentClassifierImpl::new(provider));
        Self {
            patterns: Self::build_patterns(),
            llm_classifier: Some(classifier),
            llm_timeout: Duration::from_secs(10),
        }
    }

    fn build_patterns() -> Vec<IntentPattern> {
        vec![
            IntentPattern {
                intent_type: IntentType::Logs,
                keywords: vec![
                    "log".to_string(),
                    "日志".to_string(),
                    "查看日志".to_string(),
                    "tail".to_string(),
                ],
                regex: Regex::new(r"(?i)(?:get|show|view|tail|查看|日志|log)\s+(?:.*?)(?:log|日志)").unwrap(),
                entity_extractor: Regex::new(r"(?i)(?:pod|pods|namespace|ns)\s*[:=]?\s*(\S+)").unwrap(),
            },
            IntentPattern {
                intent_type: IntentType::Metrics,
                keywords: vec![
                    "metric".to_string(),
                    "指标".to_string(),
                    "监控".to_string(),
                    "promql".to_string(),
                ],
                regex: Regex::new(r"(?i)(?:get|show|view|metric|指标|监控)\s+").unwrap(),
                entity_extractor: Regex::new(r"(?i)(?:metric|prometheus|vm)\s*[:=]?\s*(\S+)").unwrap(),
            },
            IntentPattern {
                intent_type: IntentType::Health,
                keywords: vec![
                    "health".to_string(),
                    "状态".to_string(),
                    "健康".to_string(),
                    "检查".to_string(),
                    "status".to_string(),
                ],
                regex: Regex::new(r"(?i)(?:health|status|状态|健康|检查)\s*").unwrap(),
                entity_extractor: Regex::new(r"(?i)(?:cluster|k8s|kubernetes)\s*[:=]?\s*(\S+)").unwrap(),
            },
            IntentPattern {
                intent_type: IntentType::Debug,
                keywords: vec![
                    "debug".to_string(),
                    "troubleshoot".to_string(),
                    "排查".to_string(),
                    "问题".to_string(),
                    "调查".to_string(),
                    "为什么".to_string(),
                ],
                regex: Regex::new(r"(?i)(?:debug|troubleshoot|排查|问题|调查|为什么)\s*").unwrap(),
                entity_extractor: Regex::new(r"(?i)(?:pod|pods|svc|service|deploy)\s*[:=]?\s*(\S+)").unwrap(),
            },
            IntentPattern {
                intent_type: IntentType::Query,
                keywords: vec![
                    "query".to_string(),
                    "查询".to_string(),
                    "search".to_string(),
                    "搜索".to_string(),
                ],
                regex: Regex::new(r"(?i)(?:query|search|查询|搜索)\s*").unwrap(),
                entity_extractor: Regex::new(r"(?i)(?:query|q)\s*[:=]?\s*(.+)").unwrap(),
            },
            IntentPattern {
                intent_type: IntentType::Scale,
                keywords: vec![
                    "scale".to_string(),
                    "扩缩容".to_string(),
                    "replica".to_string(),
                    "扩容".to_string(),
                    "缩容".to_string(),
                ],
                regex: Regex::new(r"(?i)(?:scale|扩缩容|replica|扩容|缩容)\s*").unwrap(),
                entity_extractor: Regex::new(r"(?i)(?:deploy|deployment)\s*[:=]?\s*(\S+)\s*(?:replicas?|:|to)\s*(\d+)").unwrap(),
            },
        ]
    }

    /// Parse a user message into an intent
    pub async fn parse(&self, message: &str) -> Intent {
        // Try LLM first if available
        if let Some(ref classifier) = self.llm_classifier {
            match tokio::time::timeout(self.llm_timeout, classifier.classify(message)).await {
                Ok(Ok(classification)) => {
                    debug!(
                        "LLM classified intent: {} (confidence: {:.2})",
                        classification.intent_type, classification.confidence
                    );

                    let intent_type = match classification.intent_type.to_lowercase().as_str() {
                        "logs" | "log" => IntentType::Logs,
                        "metrics" | "metric" => IntentType::Metrics,
                        "health" => IntentType::Health,
                        "debug" => IntentType::Debug,
                        "query" => IntentType::Query,
                        "scale" => IntentType::Scale,
                        "deploy" => IntentType::Deploy,
                        _ => IntentType::Unknown,
                    };

                    // Merge LLM entities with rule-based extraction
                    let mut entities = self.extract_entities_by_rules(message);
                    if let Some(pod) = classification.entities.pod_name {
                        entities.pod_name = Some(pod);
                    }
                    if let Some(ns) = classification.entities.namespace {
                        entities.namespace = Some(ns);
                    }
                    if let Some(cluster) = classification.entities.cluster {
                        entities.cluster = Some(cluster);
                    }
                    if let Some(svc) = classification.entities.service_name {
                        entities.service_name = Some(svc);
                    }

                    return Intent {
                        intent_type,
                        confidence: classification.confidence,
                        entities,
                        raw_query: message.to_string(),
                    };
                }
                Ok(Err(e)) => {
                    warn!("LLM classification failed: {}", e);
                }
                Err(_) => {
                    warn!("LLM classification timed out after {:?}", self.llm_timeout);
                }
            }
        }

        // Fall back to rule-based parsing
        self.parse_by_rules(message)
    }

    /// Synchronous parse - uses rules only (for backwards compatibility)
    pub fn parse_sync(&self, message: &str) -> Intent {
        self.parse_by_rules(message)
    }

    fn parse_by_rules(&self, message: &str) -> Intent {
        let message_lower = message.to_lowercase();

        for pattern in &self.patterns {
            if pattern.regex.is_match(&message_lower) {
                let confidence = self.calculate_confidence(&message_lower, &pattern);

                if confidence > 0.5 {
                    let entities = self.extract_entities(message, &pattern);

                    return Intent {
                        intent_type: pattern.intent_type.clone(),
                        confidence,
                        entities,
                        raw_query: message.to_string(),
                    };
                }
            }
        }

        Intent {
            intent_type: IntentType::Unknown,
            confidence: 0.0,
            entities: IntentEntities::default(),
            raw_query: message.to_string(),
        }
    }

    /// Calculate confidence score
    fn calculate_confidence(&self, message: &str, pattern: &IntentPattern) -> f32 {
        let mut confidence = 0.5;

        for keyword in &pattern.keywords {
            if message.contains(&keyword.to_lowercase()) {
                confidence += 0.1;
            }
        }

        confidence.min(1.0)
    }

    /// Extract entities using only rules
    fn extract_entities_by_rules(&self, message: &str) -> IntentEntities {
        let mut entities = IntentEntities::default();

        let pod_pattern = Regex::new(r"(?i)(?:pod|pods)\s*[:=]?\s*(\S+)").unwrap();
        if let Some(caps) = pod_pattern.captures(message) {
            if let Some(pod) = caps.get(1) {
                entities.pod_name = Some(pod.as_str().to_string());
            }
        }

        let ns_pattern = Regex::new(r"(?i)(?:namespace|ns)\s*[:=]?\s*(\S+)").unwrap();
        if let Some(caps) = ns_pattern.captures(message) {
            if let Some(ns) = caps.get(1) {
                entities.namespace = Some(ns.as_str().to_string());
            }
        }

        let cluster_pattern = Regex::new(r"(?i)(?:cluster)\s*[:=]?\s*(\S+)").unwrap();
        if let Some(caps) = cluster_pattern.captures(message) {
            if let Some(cluster) = caps.get(1) {
                entities.cluster = Some(cluster.as_str().to_string());
            }
        }

        // Also detect common cluster naming patterns like "prod", "test", "staging"
        let cluster_keywords = [
            ("prod", "prod"),
            ("production", "prod"),
            ("pre-prod", "pre-prod"),
            ("preprod", "pre-prod"),
            ("staging", "staging"),
            ("test", "test"),
            ("dev", "dev"),
            ("development", "dev"),
        ];

        let message_lower = message.to_lowercase();
        for (keyword, cluster_name) in &cluster_keywords {
            // Match "prod cluster" or "cluster=prod" or "在 prod 集群"
            let pattern = format!(r"(?i)(?:{}集群|{}集群|集群{}\s|cluster\s*{}|{}$)", keyword, keyword, keyword, keyword, keyword);
            if let Some(re) = Regex::new(&pattern).ok().filter(|r| r.is_match(&message_lower)) {
                let _ = re; // Silence unused warning
                entities.cluster = Some(cluster_name.to_string());
                break;
            }
        }

        let svc_pattern = Regex::new(r"(?i)(?:service|svc)\s*[:=]?\s*(\S+)").unwrap();
        if let Some(caps) = svc_pattern.captures(message) {
            if let Some(svc) = caps.get(1) {
                entities.service_name = Some(svc.as_str().to_string());
            }
        }

        entities
    }

    /// Extract entities from message
    fn extract_entities(&self, message: &str, pattern: &IntentPattern) -> IntentEntities {
        let mut entities = IntentEntities::default();

        if let Some(caps) = pattern.entity_extractor.captures(message) {
            for (name, value) in caps.iter().enumerate() {
                if let Some(m) = value {
                    let val = m.as_str().to_string();
                    match name {
                        1 => entities.deployment_name = Some(val),
                        2 => entities.namespace = Some(val),
                        _ => {}
                    }
                }
            }
        }

        // Also run common entity patterns
        let common_entities = self.extract_entities_by_rules(message);
        entities.pod_name = entities.pod_name.or(common_entities.pod_name);
        entities.namespace = entities.namespace.or(common_entities.namespace);
        entities.cluster = entities.cluster.or(common_entities.cluster);
        entities.service_name = entities.service_name.or(common_entities.service_name);

        entities
    }
}

impl Default for IntentParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_log_intent() {
        let parser = IntentParser::new();

        let intent = parser.parse_by_rules("查看 pod nginx-123 的日志");
        assert_eq!(intent.intent_type, IntentType::Logs);
        assert!(intent.confidence > 0.5);
    }

    #[test]
    fn test_parse_health_intent() {
        let parser = IntentParser::new();

        let intent = parser.parse_by_rules("检查集群健康状态");
        assert_eq!(intent.intent_type, IntentType::Health);
    }

    #[test]
    fn test_parse_debug_intent() {
        let parser = IntentParser::new();

        let intent = parser.parse_by_rules("为什么我的 pod 启动失败了");
        assert_eq!(intent.intent_type, IntentType::Debug);
    }
}
