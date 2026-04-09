//! AI/OPS Provider types

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// Time range for queries
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

/// Metrics query result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsResult {
    pub metric_name: String,
    pub values: Vec<MetricValue>,
    pub labels: HashMap<String, String>,
}

/// A single metric value point
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricValue {
    pub timestamp: DateTime<Utc>,
    pub value: f64,
}

/// Instant query result (single value)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstantQueryResult {
    pub metric_name: String,
    pub value: f64,
    pub labels: HashMap<String, String>,
}

/// Logs query result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogsResult {
    pub logs: Vec<LogEntry>,
    pub total_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_stats: Option<StreamStats>,
}

/// A single log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub message: String,
    pub labels: HashMap<String, String>,
    pub stream: String,
    pub filename: Option<String>,
    pub line_number: Option<u64>,
}

/// Stream statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamStats {
    pub lines_total: u64,
    pub bytes_total: u64,
    pub stream_type: String,
}

/// AIOps provider configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AIOpsProviderConfig {
    pub name: String,
    pub provider_type: String,
    pub endpoint: String,
    #[serde(default)]
    pub timeout_secs: u64,
    #[serde(default)]
    pub retry_attempts: u32,
    #[serde(default)]
    pub headers: HashMap<String, String>,
}
