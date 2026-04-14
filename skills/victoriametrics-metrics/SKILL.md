---
name: victoriametrics-metrics
description: >
  Query VictoriaMetrics metrics via PromQL/MetricsQL. Use for: metric queries, PromQL rate calculations,
  label discovery, series exploration, cardinality analysis, alert status, recording rules,
  TSDB diagnostics, raw data export, and metric usage statistics.
  Triggers on: metrics, PromQL, MetricsQL, label discovery, series exploration, cardinality,
  alert status, top queries, unused metrics.
tags: ["victoriametrics", "metrics", "promql", "observability"]
---

# VictoriaMetrics Metrics Query

Query VictoriaMetrics HTTP API via curl. Covers instant/range queries, label/series discovery, alerts, rules, TSDB diagnostics, raw data export, and metric usage statistics.

## Environment Variables

These are **injected automatically** by AIClaw — do not hardcode values:

- **$VM_METRICS_URL** — base URL, e.g. `https://vmselect.example.com/select/0/prometheus` (cluster)
  or `http://localhost:8428` (single-node). No trailing slash.
- **$VM_AUTH_HEADER** — auth header value, e.g. `Bearer <token>`. Empty when no auth is configured.

## Auth Pattern

### curl (Bearer token or no auth)

```bash
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} "$VM_METRICS_URL/api/v1/query?query=up" | jq .
```

When `$VM_AUTH_HEADER` is empty, the `-H` flag is omitted automatically.

### Python script (AK/SK auth via aiops platform)

When querying VictoriaMetrics via the aiops platform REST API with AK/SK authentication, use the helper script.
It supports two endpoints: `queryHistoryPromql` (PromQL range queries) and `queryLatestMetrics` (latest values).

```bash
# PromQL range query
python skills/victoriametrics-metrics/vm_query.py query-promql \
  --query 'rate(http_requests_total{namespace="tai-develop"}[5m])' \
  --start "$(date -u -d '1 hour ago' +%Y-%m-%dT%H:%M:%SZ)" \
  --end "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --step 60

# Query latest metric values
python skills/victoriametrics-metrics/vm_query.py query-latest \
  --dimensions 'namespace:tai-develop;pod:my-pod-abc123' \
  --item-names 'cpu_usage,mem_usage'
```

Environment variables (injected by AIClaw from `[skills.exec.victoriametrics]` config):
- `$VM_METRICS_URL` — Aiops platform base URL, e.g. `https://aiops.example.com`
- `$VM_AK` — Access Key (from `vm_ak` config)
- `$VM_SK` — Secret Key (from `vm_sk` config)

The script replicates the HMAC-SHA256 double-signature scheme from `aiops-api-go-client`.
Use `--format prometheus` to get pretty-printed Prometheus-format output.

## Instant Query

```bash
# Query at current time
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  "$VM_METRICS_URL/api/v1/query?query=up" | jq .

# Query at specific time
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  "$VM_METRICS_URL/api/v1/query?query=up&time=2026-03-07T09:00:00Z" | jq .
```

Parameters: `query` (required), `time` (optional, RFC3339 or Unix seconds).

## Range Query

```bash
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  "$VM_METRICS_URL/api/v1/query_range?query=rate(http_requests_total[5m])&start=2026-03-07T00:00:00Z&end=2026-03-07T12:00:00Z&step=5m" | jq .
```

Parameters: `query` (required), `start` (required), `end` (optional, defaults to now), `step` (required).

## Labels Discovery

```bash
# All label names
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  "$VM_METRICS_URL/api/v1/labels" | jq '.data[]'

# Label values (namespace is a PATH parameter)
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  "$VM_METRICS_URL/api/v1/label/namespace/values" | jq '.data[]'

# Label values filtered by series matcher
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  --data-urlencode 'match[]={job="kubelet"}' \
  "$VM_METRICS_URL/api/v1/label/namespace/values" | jq '.data[]'
```

## Series Discovery

```bash
# Find series matching selector
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  --data-urlencode 'match[]={namespace="myapp"}' \
  "$VM_METRICS_URL/api/v1/series?limit=20" | jq '.data[].__name__'
```

## Metric Metadata

```bash
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  "$VM_METRICS_URL/api/v1/metadata?metric=http_request&limit=10" | jq .
```

## Alerts and Rules

```bash
# All firing/pending alerts
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  "$VM_METRICS_URL/api/v1/alerts" | jq '.data.alerts[]'

# All alerting and recording rules
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  "$VM_METRICS_URL/api/v1/rules" | jq '.data.groups[]'
```

## Instance Diagnostics

```bash
# TSDB cardinality stats
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  "$VM_METRICS_URL/api/v1/status/tsdb" | jq .

# Currently executing queries
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  "$VM_METRICS_URL/api/v1/status/active_queries" | jq .

# Most frequent/slowest queries
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  "$VM_METRICS_URL/api/v1/status/top_queries?topN=10" | jq .

# Metric usage statistics (find unused/rarely-used metrics)
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  "$VM_METRICS_URL/api/v1/status/metric_names_stats?limit=50&le=1" | jq .
```

## Export Raw Data

```bash
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  --data-urlencode 'match[]=http_requests_total' \
  -d 'start=2026-03-07T00:00:00Z' -d 'end=2026-03-07T12:00:00Z' \
  "$VM_METRICS_URL/api/v1/export" | head -5
```

## Common Patterns

```bash
# Get all namespaces with active pods
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  --data-urlencode 'match[]={__name__="kube_pod_info"}' \
  "$VM_METRICS_URL/api/v1/label/namespace/values" | jq '.data[]'

# Rate of errors over last hour
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  --data-urlencode 'query=sum(rate(http_requests_total{code=~"5.."}[5m])) by (namespace)' \
  "$VM_METRICS_URL/api/v1/query" | jq '.data.result[] | {ns: .metric.namespace, rate: .value[1]}'
```

## Important Notes

- `match[]` parameter requires the `[]` suffix
- All times accept RFC3339 or Unix seconds
- Export endpoint returns JSON Lines (one object per line), not wrapped JSON
- On cluster mode, `$VM_METRICS_URL` should include `/select/X/prometheus` suffix
