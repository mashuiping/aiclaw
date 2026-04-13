---
name: victoriametrics-logs
description: >
  Query VictoriaLogs via LogsQL. Use for: log search, log stats queries, field/stream discovery,
  log hit patterns, log facets, and log volume analysis.
  Triggers on: logs, log search, LogsQL, log stats, field discovery, stream discovery, log facets.
tags: ["victoriametrics", "logs", "logsql", "observability"]
---

# VictoriaLogs Query

Query VictoriaLogs HTTP API via curl. Covers log search, stats queries, field/stream discovery, hits analysis, and facets.

## Environment Variables

These are **injected automatically** by AIClaw — do not hardcode values:

- **$VM_LOGS_URL** — base URL, e.g. `https://vlselect.example.com`. No trailing slash.
- **$VM_AUTH_HEADER** — auth header value, e.g. `Bearer <token>`. Empty when no auth is configured.

## Critical Rules

- **ALWAYS pass `start`** on ALL endpoints — omitting it scans ALL stored data (extremely expensive)
- `stats_query` uses `time` parameter (always pass explicitly)
- `stats_query_range` uses `start`/`end`/`step`
- `/select/logsql/query` returns JSON Lines (one JSON object per line), NOT a JSON array
- LogsQL queries with special characters MUST be URL-encoded (use `--data-urlencode`)

## Auth Pattern

### curl (Bearer token or no auth)

```bash
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  "$VM_LOGS_URL/select/logsql/query?query=*&start=2026-03-07T00:00:00Z&limit=10"
```

When `$VM_AUTH_HEADER` is empty, the `-H` flag is omitted automatically.

### Python script (AK/SK auth)

When VictoriaLogs is behind vmauth with AK/SK authentication, use the helper script:

```bash
python skills/victoriametrics-logs/vl_query.py \
  --query '{kubernetes_namespace="tai-develop"} kubernetes_pod_instance="infra-platform"' \
  --start "$(date -u -d '1 hour ago' +%Y-%m-%dT%H:%M:%SZ)" \
  --limit 100
```

Environment variables (injected by AIClaw from `[skills.exec.victoriametrics]` config):
- `$VM_LOGS_URL` — VictoriaLogs base URL, e.g. `https://vlselect.example.com`
- `$VM_AK` — Access Key (from `vm_ak` config)
- `$VM_SK` — Secret Key (from `vm_sk` config)

The script replicates the HMAC-SHA256 double-signature scheme from `aiops-api-go-client`.

## Log Query (Primary)

```bash
# Basic query (last hour, limit 100)
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  --data-urlencode 'query={namespace="myapp"} error' \
  "$VM_LOGS_URL/select/logsql/query?start=2026-03-07T00:00:00Z&limit=100"

# With time range and field selection
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  --data-urlencode 'query={namespace="myapp"} error' \
  "$VM_LOGS_URL/select/logsql/query?start=2026-03-07T00:00:00Z&end=2026-03-07T12:00:00Z&limit=50&fields=_time,_msg,level"
```

Parameters: `query` (required), `start` (required, RFC3339), `end`, `limit`, `fields`.

## Stats Query (Instant)

```bash
# Count errors by level at a point in time
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  --data-urlencode 'query={namespace="myapp"} | stats by (level) count() as total' \
  "$VM_LOGS_URL/select/logsql/stats_query?time=2026-03-07T09:00:00Z" | jq .
```

Parameters: `query` (must contain `| stats`), `time` (required, RFC3339).

## Stats Query Range

```bash
# Error count over time with 1h steps
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  --data-urlencode 'query={namespace="myapp"} error | stats count() as total' \
  "$VM_LOGS_URL/select/logsql/stats_query_range?start=2026-03-07T00:00:00Z&end=2026-03-07T12:00:00Z&step=1h" | jq .
```

Parameters: `query` (must contain `| stats`), `start`, `end`, `step`.

## Hits (Log Volume)

```bash
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  --data-urlencode 'query={namespace="myapp"}' \
  "$VM_LOGS_URL/select/logsql/hits?start=2026-03-07T00:00:00Z&end=2026-03-07T12:00:00Z&step=1h" | jq .
```

## Facets (Best Discovery Tool)

```bash
# Discover field value distributions in one call
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  --data-urlencode 'query={namespace="myapp"}' \
  "$VM_LOGS_URL/select/logsql/facets?start=2026-03-07T00:00:00Z&end=2026-03-07T12:00:00Z" | jq .
```

## Field Discovery

```bash
# Discover non-stream field names
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  --data-urlencode 'query={namespace="myapp"}' \
  "$VM_LOGS_URL/select/logsql/field_names?start=2026-03-07T00:00:00Z" | jq .

# Get values for a specific field
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  --data-urlencode 'query={namespace="myapp"}' \
  "$VM_LOGS_URL/select/logsql/field_values?start=2026-03-07T00:00:00Z&field=level&limit=20" | jq .
```

## Stream Discovery

```bash
# Discover stream field names
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  --data-urlencode 'query=*' \
  "$VM_LOGS_URL/select/logsql/stream_field_names?start=2026-03-07T00:00:00Z" | jq .

# Get values for a stream field (namespace, pod, etc.)
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  --data-urlencode 'query=*' \
  "$VM_LOGS_URL/select/logsql/stream_field_values?start=2026-03-07T00:00:00Z&field=namespace" | jq .
```

## LogsQL Quick Reference

```logsql
{namespace="myapp"}              # Stream filter
{namespace="myapp"} error         # Word filter
{namespace="myapp"} error timeout # Multiple words (AND)
{namespace="myapp"} (error OR warning)  # OR filter
{namespace="myapp"} ~"err|warn"  # Regex on _msg
{namespace="myapp"} level:error  # Field filter
{namespace="myapp"} _time:1h     # Time filter (last 1 hour)
{namespace="myapp"} error -"expected"  # Negation
{namespace="myapp"} | stats by (level) count() as total  # Stats
```

**Common mistakes**:
- `| grep` does NOT exist — use word filters or regex
- `| filter` is ONLY valid after `| stats`
- Time ranges go in API params OR `_time:` filter, NOT both
- Searching "error" without filtering vmselect: add `-"vm_slow_query_stats"`

## Common Patterns

```bash
# Quick error check for a namespace (last hour)
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  --data-urlencode 'query={namespace="myapp"} error' \
  "$VM_LOGS_URL/select/logsql/query?start=$(date -u -d '1 hour ago' +%Y-%m-%dT%H:%M:%SZ)&limit=20"

# Error rate over time
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  --data-urlencode 'query={namespace="myapp"} error | stats count() as errors' \
  "$VM_LOGS_URL/select/logsql/stats_query_range?start=2026-03-07T00:00:00Z&step=1h" | jq .

# Discover all namespaces with logs
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  --data-urlencode 'query=*' \
  "$VM_LOGS_URL/select/logsql/stream_field_values?start=2026-03-07T00:00:00Z&field=namespace" | jq .
```

## Important Notes

- All times use RFC3339 format: `2026-03-07T09:00:00Z`. Unix timestamps NOT supported.
- Collect JSON Lines: `| jq -s .`
- `facets` is the best single-call discovery tool
- `stats_query` uses `time`, `stats_query_range` uses `start`/`end`/`step`
