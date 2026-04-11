//! Log observer implementation

use super::traits::*;
use async_trait::async_trait;
use chrono::Utc;

/// LogObserver - structured logging observer
pub struct LogObserver {
    name: String,
}

impl LogObserver {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl LogObserver {
    fn format_event(&self, event: &ObserverEvent) -> String {
        let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S%.3f");
        match event {
            ObserverEvent::AgentStart { provider, model } => {
                format!("[{}] INFO {} - Agent started with {} ({})", timestamp, self.name, provider, model)
            }
            ObserverEvent::LlmRequest { provider, model, messages_count } => {
                format!("[{}] DEBUG {} - LLM request to {} ({}) with {} messages", 
                    timestamp, self.name, provider, model, messages_count)
            }
            ObserverEvent::LlmResponse { provider, model, duration, success, .. } => {
                let status = if *success { "success" } else { "failure" };
                format!("[{}] INFO {} - LLM response from {} ({}) in {:?}: {}", 
                    timestamp, self.name, provider, model, duration, status)
            }
            ObserverEvent::AgentEnd { provider, model, duration, tokens_used, cost_usd } => {
                let tokens_str = tokens_used.map(|t| format!(", tokens: {}", t)).unwrap_or_default();
                let cost_str = cost_usd.map(|c| format!(", cost: ${:.4}", c)).unwrap_or_default();
                format!("[{}] INFO {} - Agent ended {} ({}) in {:?}{}{}", 
                    timestamp, self.name, provider, model, duration, tokens_str, cost_str)
            }
            ObserverEvent::ToolCallStart { tool } => {
                format!("[{}] DEBUG {} - Tool call started: {}", timestamp, self.name, tool)
            }
            ObserverEvent::ToolCall { tool, duration, success, error_message } => {
                let status = if *success { "success" } else { "failure" };
                let error_str = error_message.as_ref().map(|e| format!(", error: {}", e)).unwrap_or_default();
                format!("[{}] INFO {} - Tool call {} completed in {:?}: {}{}", 
                    timestamp, self.name, tool, duration, status, error_str)
            }
            ObserverEvent::TurnComplete => {
                format!("[{}] DEBUG {} - Turn completed", timestamp, self.name)
            }
            ObserverEvent::ChannelMessage { channel, direction } => {
                let dir = match direction {
                    MessageDirection::Inbound => "inbound",
                    MessageDirection::Outbound => "outbound",
                };
                format!("[{}] DEBUG {} - Channel message on {} ({})", timestamp, self.name, channel, dir)
            }
            ObserverEvent::WebhookAuthFailure { channel } => {
                format!("[{}] WARN {} - Webhook auth failure on {}", timestamp, self.name, channel)
            }
            ObserverEvent::HeartbeatTick => {
                format!("[{}] DEBUG {} - Heartbeat", timestamp, self.name)
            }
            ObserverEvent::Error { component, message } => {
                format!("[{}] ERROR {} - {} error: {}", timestamp, self.name, component, message)
            }
            ObserverEvent::SkillExecutionStart { skill } => {
                format!("[{}] DEBUG {} - Skill execution started: {}", timestamp, self.name, skill)
            }
            ObserverEvent::SkillExecutionEnd { skill, duration, success } => {
                let status = if *success { "success" } else { "failure" };
                format!("[{}] INFO {} - Skill {} completed in {:?}: {}",
                    timestamp, self.name, skill, duration, status)
            }
        }
    }

    fn format_metric(&self, metric: &ObserverMetric) -> String {
        let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S%.3f");
        match metric {
            ObserverMetric::RequestLatency(duration) => {
                format!("[{}] METRIC {} - request_latency_ms: {:?}", timestamp, self.name, duration.as_millis())
            }
            ObserverMetric::TokensUsed(count) => {
                format!("[{}] METRIC {} - tokens_used: {}", timestamp, self.name, count)
            }
            ObserverMetric::ActiveSessions(count) => {
                format!("[{}] METRIC {} - active_sessions: {}", timestamp, self.name, count)
            }
            ObserverMetric::QueueDepth(depth) => {
                format!("[{}] METRIC {} - queue_depth: {}", timestamp, self.name, depth)
            }
        }
    }
}

#[async_trait]
impl Observer for LogObserver {
    fn record_event(&self, event: ObserverEvent) {
        let msg = self.format_event(&event);
        match &event {
            ObserverEvent::Error { .. } | ObserverEvent::WebhookAuthFailure { .. } => {
                tracing::warn!(event = %msg, "observability event");
            }
            _ => {
                tracing::info!(event = %msg, "observability event");
            }
        }
    }

    fn record_metric(&self, metric: ObserverMetric) {
        let msg = self.format_metric(&metric);
        tracing::debug!(metric = %msg, "observability metric");
    }

    fn flush(&self) {
        // Log observer doesn't need flushing
    }

    fn name(&self) -> &str {
        &self.name
    }
}
