//! VictoriaMetrics provider implementation

use async_trait::async_trait;
use aiclaw_types::aiops::{LogEntry, LogsResult, MetricValue, MetricsResult, TimeRange};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::time::Duration;
use tracing::{debug, error};

use super::traits::{AIOpsProvider, QueryOptions};
use crate::config::AIOpsProviderConfig;

/// VictoriaMetrics provider
pub struct VictoriaMetricsProvider {
    name: String,
    endpoint: String,
    client: reqwest::Client,
    timeout: Duration,
}

impl VictoriaMetricsProvider {
    pub fn new(name: &str, config: &AIOpsProviderConfig) -> anyhow::Result<Self> {
        let timeout = Duration::from_secs(config.timeout_secs);

        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()?;

        Ok(Self {
            name: name.to_string(),
            endpoint: config.endpoint.trim_end_matches('/').to_string(),
            client,
            timeout,
        })
    }

    async fn query_prometheus_api(&self, query: &str, time: Option<DateTime<Utc>>) -> anyhow::Result<Vec<MetricValue>> {
        let mut url = format!("{}/api/v1/query", self.endpoint);

        if let Some(t) = time {
            url = format!("{}/api/v1/query_at", self.endpoint);
            url = format!("{}?time={}", url, t.timestamp());
        } else {
            url = format!("{}?query={}", url, urlencoding::encode(query));
        }

        debug!("Querying VictoriaMetrics: {}", url);

        let response = self.client.get(&url).send().await?;
        let data: serde_json::Value = response.json().await?;

        let results = data
            .get("data")
            .and_then(|d| d.get("result"))
            .and_then(|r| r.as_array())
            .ok_or_else(|| anyhow::anyhow!("Invalid response format"))?;

        let mut values = Vec::new();

        for result in results {
            let metric = result.get("metric").and_then(|m| m.as_object());
            let value_array = result.get("value").and_then(|v| v.as_array());

            if let (Some(_metric_obj), Some(value_arr)) = (metric, value_array) {
                if value_arr.len() >= 2 {
                    let timestamp = value_arr[0].as_f64().unwrap_or(0.0) as i64;
                    let value = value_arr[1].as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);

                    values.push(MetricValue {
                        timestamp: DateTime::from_timestamp(timestamp, 0).unwrap_or_else(Utc::now),
                        value,
                    });
                }
            }
        }

        Ok(values)
    }

    async fn query_range_api(&self, query: &str, time_range: &TimeRange) -> anyhow::Result<MetricsResult> {
        let start = time_range.start.timestamp();
        let end = time_range.end.timestamp();
        let step = ((end - start) / 100).max(1);

        let url = format!(
            "{}/api/v1/query_range?query={}&start={}&end={}&step={}",
            self.endpoint,
            urlencoding::encode(query),
            start,
            end,
            step
        );

        debug!("Querying VictoriaMetrics range: {}", url);

        let response = self.client.get(&url).send().await?;
        let data: serde_json::Value = response.json().await?;

        let results = data
            .get("data")
            .and_then(|d| d.get("result"))
            .and_then(|r| r.as_array())
            .ok_or_else(|| anyhow::anyhow!("Invalid response format"))?;

        let metric_name = data
            .get("data")
            .and_then(|d| d.get("metric"))
            .and_then(|m| m.get("__name__"))
            .and_then(|n| n.as_str())
            .unwrap_or(query)
            .to_string();

        let mut labels = HashMap::new();
        if let Some(first) = results.first() {
            if let Some(metric_obj) = first.get("metric").and_then(|m| m.as_object()) {
                for (k, v) in metric_obj {
                    if k != "__name__" {
                        if let Some(v_str) = v.as_str() {
                            labels.insert(k.clone(), v_str.to_string());
                        }
                    }
                }
            }
        }

        let mut values = Vec::new();
        for result in results {
            let values_array = result.get("values").and_then(|v| v.as_array());

            if let Some(arr) = values_array {
                for pair in arr {
                    if let [ts, val] = arr[..] {
                        let timestamp = ts.as_f64().unwrap_or(0.0) as i64;
                        let value = val.as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);

                        values.push(MetricValue {
                            timestamp: DateTime::from_timestamp(timestamp, 0).unwrap_or_else(Utc::now),
                            value,
                        });
                    }
                }
            }
        }

        Ok(MetricsResult {
            metric_name,
            values,
            labels,
        })
    }

    async fn query_logs_api(&self, query: &str, limit: usize) -> anyhow::Result<LogsResult> {
        let url = format!(
            "{}/select/logsql/query?query={}&limit={}",
            self.endpoint,
            urlencoding::encode(query),
            limit
        );

        debug!("Querying VictoriaMetrics logs: {}", url);

        let response = self.client.get(&url).send().await?;
        let data: serde_json::Value = response.json().await?;

        let logs_array = data.as_array().ok_or_else(|| anyhow::anyhow!("Invalid logs response"))?;

        let logs: Vec<LogEntry> = logs_array
            .iter()
            .map(|entry| {
                let timestamp = entry
                    .get("_time")
                    .and_then(|t| t.as_str())
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(Utc::now);

                let message = entry
                    .get("message")
                    .or_else(|| entry.get("_msg"))
                    .and_then(|m| m.as_str())
                    .unwrap_or("")
                    .to_string();

                let mut labels = HashMap::new();
                if let Some(obj) = entry.as_object() {
                    for (k, v) in obj {
                        if k != "_time" && k != "message" && k != "_msg" {
                            if let Some(v_str) = v.as_str() {
                                labels.insert(k.clone(), v_str.to_string());
                            }
                        }
                    }
                }

                LogEntry {
                    timestamp,
                    message,
                    labels,
                    stream: entry.get("_stream").and_then(|s| s.as_str()).unwrap_or("").to_string(),
                    filename: entry.get("filename").and_then(|f| f.as_str()).map(String::from),
                    line_number: entry.get("line_num").and_then(|n| n.as_u64()),
                }
            })
            .collect();

        let total_count = logs.len();

        Ok(LogsResult {
            logs,
            total_count,
            stream_stats: None,
        })
    }
}

#[async_trait]
impl AIOpsProvider for VictoriaMetricsProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn provider_type(&self) -> &str {
        "victoriametrics"
    }

    async fn query_metrics(&self, query: &str, time_range: TimeRange) -> anyhow::Result<MetricsResult> {
        self.query_range_api(query, &time_range).await
    }

    async fn query_instant(&self, query: &str) -> anyhow::Result<f64> {
        let values = self.query_prometheus_api(query, None).await?;
        values
            .last()
            .map(|v| v.value)
            .ok_or_else(|| anyhow::anyhow!("No data returned for query"))
    }

    async fn query_logs(&self, query: &str, limit: usize) -> anyhow::Result<LogsResult> {
        self.query_logs_api(query, limit).await
    }

    async fn health_check(&self) -> bool {
        let url = format!("{}/health", self.endpoint);
        match self.client.get(&url).send().await {
            Ok(response) => response.status().is_success(),
            Err(_) => false,
        }
    }
}

/// Simple URL encoder since we don't want to add another dependency
mod urlencoding {
    pub fn encode(input: &str) -> String {
        let mut encoded = String::new();
        for c in input.chars() {
            match c {
                'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => {
                    encoded.push(c);
                }
                _ => {
                    for b in c.to_string().as_bytes() {
                        encoded.push_str(&format!("%{:02X}", b));
                    }
                }
            }
        }
        encoded
    }
}
