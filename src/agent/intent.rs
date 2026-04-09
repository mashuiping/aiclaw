//! Intent parsing

use aiclaw_types::agent::{Intent, IntentEntities, IntentType};
use regex::Regex;
use std::collections::HashMap;

/// Intent parser - parses user messages into structured intents
pub struct IntentParser {
    patterns: Vec<IntentPattern>,
}

struct IntentPattern {
    intent_type: IntentType,
    keywords: Vec<String>,
    regex: Regex,
    entity_extractor: Regex,
}

impl IntentParser {
    pub fn new() -> Self {
        let patterns = vec![
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
                ],
                regex: Regex::new(r"(?i)(?:debug|troubleshoot|排查|问题|调查)\s*").unwrap(),
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
                ],
                regex: Regex::new(r"(?i)(?:scale|扩缩容|replica)\s*").unwrap(),
                entity_extractor: Regex::new(r"(?i)(?:deploy|deployment)\s*[:=]?\s*(\S+)\s*(?:replicas?|:|to)\s*(\d+)").unwrap(),
            },
        ];

        Self { patterns }
    }

    /// Parse a user message into an intent
    pub fn parse(&self, message: &str) -> Intent {
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

        let svc_pattern = Regex::new(r"(?i)(?:service|svc)\s*[:=]?\s*(\S+)").unwrap();
        if let Some(caps) = svc_pattern.captures(message) {
            if let Some(svc) = caps.get(1) {
                entities.service_name = Some(svc.as_str().to_string());
            }
        }

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

        let intent = parser.parse("查看 pod nginx-123 的日志");
        assert_eq!(intent.intent_type, IntentType::Logs);
        assert!(intent.confidence > 0.5);
    }

    #[test]
    fn test_parse_health_intent() {
        let parser = IntentParser::new();

        let intent = parser.parse("检查集群健康状态");
        assert_eq!(intent.intent_type, IntentType::Health);
    }
}
