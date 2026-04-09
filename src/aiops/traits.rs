//! AI/OPS provider traits

use async_trait::async_trait;
use aiclaw_types::aiops::{LogsResult, MetricsResult, TimeRange};

/// AI/OPS provider trait - for observability data sources
#[async_trait]
pub trait AIOpsProvider: Send + Sync {
    /// Get provider name
    fn name(&self) -> &str;

    /// Get provider type
    fn provider_type(&self) -> &str;

    /// Query metrics with time range
    async fn query_metrics(
        &self,
        query: &str,
        time_range: TimeRange,
    ) -> anyhow::Result<MetricsResult>;

    /// Query instant metrics (single value)
    async fn query_instant(&self, query: &str) -> anyhow::Result<f64>;

    /// Query logs
    async fn query_logs(&self, query: &str, limit: usize) -> anyhow::Result<LogsResult>;

    /// Health check
    async fn health_check(&self) -> bool;
}

/// Query options for fine-tuning queries
#[derive(Debug, Clone, Default)]
pub struct QueryOptions {
    pub timeout_secs: Option<u64>,
    pub retry_attempts: Option<u32>,
    pub cache: bool,
}

impl QueryOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = Some(secs);
        self
    }

    pub fn with_retry(mut self, attempts: u32) -> Self {
        self.retry_attempts = Some(attempts);
        self
    }

    pub fn with_cache(mut self, enable: bool) -> Self {
        self.cache = enable;
        self
    }
}
