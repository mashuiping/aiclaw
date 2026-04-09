//! Observability traits

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::time::Duration;

/// Observer event types
#[derive(Debug, Clone)]
pub enum ObserverEvent {
    AgentStart {
        provider: String,
        model: String,
    },
    LlmRequest {
        provider: String,
        model: String,
        messages_count: usize,
    },
    LlmResponse {
        provider: String,
        model: String,
        duration: Duration,
        success: bool,
        error_message: Option<String>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    },
    AgentEnd {
        provider: String,
        model: String,
        duration: Duration,
        tokens_used: Option<u64>,
        cost_usd: Option<f64>,
    },
    ToolCallStart {
        tool: String,
    },
    ToolCall {
        tool: String,
        duration: Duration,
        success: bool,
        error_message: Option<String>,
    },
    TurnComplete,
    ChannelMessage {
        channel: String,
        direction: MessageDirection,
    },
    WebhookAuthFailure {
        channel: String,
    },
    HeartbeatTick,
    Error {
        component: String,
        message: String,
    },
    SkillExecutionStart {
        skill: String,
    },
    SkillExecutionEnd {
        skill: String,
        duration: Duration,
        success: bool,
    },
    McpCall {
        server: String,
        tool: String,
        duration: Duration,
        success: bool,
    },
}

/// Message direction
#[derive(Debug, Clone, PartialEq)]
pub enum MessageDirection {
    Inbound,
    Outbound,
}

/// Observer metric types
#[derive(Debug, Clone)]
pub enum ObserverMetric {
    RequestLatency(Duration),
    TokensUsed(u64),
    ActiveSessions(u64),
    QueueDepth(u64),
}

/// Observer trait for collecting telemetry
#[async_trait]
pub trait Observer: Send + Sync {
    fn record_event(&self, event: ObserverEvent);
    fn record_metric(&self, metric: ObserverMetric);
    fn flush(&self);
    fn name(&self) -> &str;
}

/// No-op observer for when observability is disabled
pub struct NoopObserver;

impl NoopObserver {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NoopObserver {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Observer for NoopObserver {
    fn record_event(&self, _event: ObserverEvent) {}
    fn record_metric(&self, _metric: ObserverMetric) {}
    fn flush(&self) {}
    fn name(&self) -> &str {
        "noop"
    }
}

/// MultiObserver - combines multiple observers
pub struct MultiObserver {
    observers: Vec<Box<dyn Observer>>,
}

impl MultiObserver {
    pub fn new() -> Self {
        Self {
            observers: Vec::new(),
        }
    }

    pub fn add_observer(&mut self, observer: Box<dyn Observer>) {
        self.observers.push(observer);
    }
}

impl Default for MultiObserver {
    fn default() -> Self {
        Self::new()
    }
}

impl MultiObserver {
    fn for_each<F>(&self, f: F)
    where
        F: Fn(&dyn Observer),
    {
        for observer in &self.observers {
            f(observer.as_ref());
        }
    }
}

#[async_trait]
impl Observer for MultiObserver {
    fn record_event(&self, event: ObserverEvent) {
        self.for_each(|obs| obs.record_event(event.clone()));
    }

    fn record_metric(&self, metric: ObserverMetric) {
        self.for_each(|obs| obs.record_metric(metric.clone()));
    }

    fn flush(&self) {
        self.for_each(|obs| obs.flush());
    }

    fn name(&self) -> &str {
        "multi"
    }
}
