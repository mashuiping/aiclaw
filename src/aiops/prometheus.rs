//! Prometheus provider implementation

use async_trait::async_trait;
use aiclaw_types::aiops::{LogsResult, MetricValue, MetricsResult, TimeRange};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::time::Duration;
use tracing::debug;

use super::traits::AIOpsProvider;
use crate::config::AIOpsProviderConfig;

/// Prometheus provider
pub struct PrometheusProvider {
    name: String,
    endpoint: String,
    client: reqwest::Client,
    _timeout: Duration,
}

impl PrometheusProvider {
    pub fn new(name: &str, config: &AIOpsProviderConfig) -> anyhow::Result<Self> {
        let timeout = Duration::from_secs(config.timeout_secs);

        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()?;

        Ok(Self {
            name: name.to_string(),
            endpoint: config.endpoint.trim_end_matches('/').to_string(),
            client,
            _timeout: timeout,
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

        debug!("Querying Prometheus: {}", url);

        let response = self.client.get(&url).send().await?;
        let data: serde_json::Value = response.json().await?;

        let results = data
            .get("data")
            .and_then(|d| d.get("result"))
            .and_then(|r| r.as_array())
            .ok_or_else(|| anyhow::anyhow!("Invalid response format"))?;

        let mut values = Vec::new();

        for result in results {
            let value_array = result.get("value").and_then(|v| v.as_array());

            if let Some(value_arr) = value_array {
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

        debug!("Querying Prometheus range: {}", url);

        let response = self.client.get(&url).send().await?;
        let data: serde_json::Value = response.json().await?;

        let results = data
            .get("data")
            .and_then(|d| d.get("result"))
            .and_then(|r| r.as_array())
            .ok_or_else(|| anyhow::anyhow!("Invalid response format"))?;

        let metric_name = query.to_string();

        let mut labels = HashMap::new();
        if let Some(first) = results.first() {
            if let Some(metric_obj) = first.get("metric").and_then(|m| m.as_object()) {
                for (k, v) in metric_obj {
                    if let Some(v_str) = v.as_str() {
                        labels.insert(k.clone(), v_str.to_string());
                    }
                }
            }
        }

        let mut values = Vec::new();
        for result in results {
            let values_array = result.get("values").and_then(|v| v.as_array());

            if let Some(arr) = values_array {
                for pair in arr {
                    if let Some(values_slice) = pair.as_array().filter(|a| a.len() == 2).map(|a| a.as_slice()) {
                        let timestamp = values_slice[0].as_f64().unwrap_or(0.0) as i64;
                        let value = values_slice[1].as_str().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);

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
}

#[async_trait]
impl AIOpsProvider for PrometheusProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn provider_type(&self) -> &str {
        "prometheus"
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

    async fn query_logs(&self, _query: &str, _limit: usize) -> anyhow::Result<LogsResult> {
        Err(anyhow::anyhow!("Prometheus provider does not support log queries. Use VictoriaMetrics or Loki."))
    }

    async fn health_check(&self) -> bool {
        let url = format!("{}/-/healthy", self.endpoint);
        match self.client.get(&url).send().await {
            Ok(response) => response.status().is_success(),
            Err(_) => false,
        }
    }
}

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
