---
name: victoriametrics-logs
description: >
  Query logs with LogsQL: either direct VictoriaLogs HTTP API (curl, JSON Lines) or AIOps OpenAPI
  LogService.QueryLog via vl_query.py (AK/SK HMAC, POST JSON). Use for log search, stats, facets,
  field/stream discovery, and vmauth-protected gateways.
  Triggers on: VictoriaLogs, LogsQL, log search, logsql, vmauth, AK/SK, aiops logs, queryLog.
tags: ["victoriametrics", "logs", "logsql", "observability"]
---

# VictoriaLogs Query

Two ways to run LogsQL — pick the one that matches the environment:

| Mode | Auth | Transport | Response |
|------|------|-----------|----------|
| **Direct VictoriaLogs** | `$VM_AUTH_HEADER` (Bearer) or none | `GET` `/select/logsql/*` | JSON Lines (one object per line) |
| **AIOps OpenAPI** | AK/SK (HMAC, `vl_query.py`) | `POST` `/v1/openapi/log/queryLog` | JSON object; log rows in `data` |

The sections below use **curl** for direct VictoriaLogs. For gateways that only accept signed OpenAPI calls, use **`vl_query.py`** (see [AK/SK OpenAPI](#aksk-openapi-vl_querypy)).

## Environment Variables

These are **injected automatically** by AIClaw — do not hardcode values:

- **$VM_LOGS_URL** — base URL, e.g. `https://vlselect.example.com`. No trailing slash.
- **$VM_AUTH_HEADER** — auth header value, e.g. `Bearer <token>`. Empty when no auth is configured.

## Critical Rules

- **ALWAYS pass `start`** on direct `/select/logsql/*` calls — omitting it scans ALL stored data (extremely expensive)
- `stats_query` uses `time` (always pass explicitly); `stats_query_range` uses `start`/`end`/`step`
- **Direct** `/select/logsql/query` returns **JSON Lines** (one JSON object per line), not a JSON array
- LogsQL queries with special characters MUST be URL-encoded (`--data-urlencode` on curl)
- **OpenAPI** `queryLog` uses a **POST JSON body** (`page_size`, `start`/`end` as Unix seconds in the API). Do not confuse with GET query-string style.

## Auth Pattern

### curl (Bearer token or no auth)

```bash
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  "$VM_LOGS_URL/select/logsql/query?query=*&start=2026-03-07T00:00:00Z&limit=10"
```

When `$VM_AUTH_HEADER` is empty, the `-H` flag is omitted automatically.

### AK/SK OpenAPI (`vl_query.py`)

Use when the platform exposes **LogService.QueryLog** (same contract as `aiops-api-go-client` `LogServiceQueryLog`): **POST** `…/v1/openapi/log/queryLog` with JSON body, **not** GET to `/select/logsql/query`.

- **Auth**: HMAC-SHA256 double signature + headers (`x-alogic-*`, `x-original-url`), matching `middleware/auth` in `aiops-api-go-client`.
- **Body** (snake_case): `query` (LogsQL string), `start` / `end` (epoch seconds; script accepts RFC3339, Unix digits, or naive `YYYY-MM-DD HH:MM:SS` as **UTC**), `page_size` (from `--limit`), optional `region_id`, `data_source_xid`, `page_index`, `reverse`.
- **Response**: single JSON; script prints the **`data`** array (or stderr + exit on HTTP/API errors).

```bash
python skills/victoriametrics-logs/vl_query.py \
  --url "$VM_LOGS_URL" \
  --ak "$VM_AK" \
  --sk "$VM_SK" \
  --query '_msg: "infra_platform"' \
  --start "2026-04-19 10:00:00" \
  --end "2026-04-19 19:00:00" \
  --limit 2
```

Environment variables (e.g. AIClaw `[skills.exec.victoriametrics]`):

- `$VM_LOGS_URL` — gateway base URL (no path suffix); set via `vm_logs_url` in config or `export VM_LOGS_URL=...`
- `$VM_AK` / `$VM_SK` — Access Key / Secret Key

Omit `--url` / `--ak` / `--sk` when the same values are set in the environment.

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

- **Direct curl** times: RFC3339 in query params, e.g. `2026-03-07T09:00:00Z`. Collect JSON Lines with `| jq -s .`
- **`vl_query.py` times**: RFC3339 (`…Z` or offset), plain Unix seconds, or naive `YYYY-MM-DD HH:MM:SS` (treated as UTC)
- `facets` is the best single-call discovery tool on direct VictoriaLogs
- `stats_query` uses `time`; `stats_query_range` uses `start`/`end`/`step`
