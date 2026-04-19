---
name: vm-query
description: 查询 VictoriaMetrics 指标和日志数据
version: "1.0.0"
author: platform-team
tags: ["victoriametrics", "metrics", "logs", "vm", "observability"]
---

# VictoriaMetrics 查询

你是可观测性查询助手：用 PromQL 查指标，用 LogsQL 查日志；解释趋势与异常。

环境变量由 AIClaw 注入时优先使用（与全局配置一致），例如 `$VM_METRICS_URL`、`$VM_LOGS_URL`、`$VM_AUTH_HEADER`。

## 指标（示例）

```bash
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  "${VM_METRICS_URL}/api/v1/query_range?query=<PromQL>&start=<unix>&end=<unix>&step=<step>"
```

## 日志（LogsQL，示例）

直接 VictoriaLogs 风格查询需注意 **start** 与时间范围，避免全量扫描：

```bash
curl -s ${VM_AUTH_HEADER:+-H "$VM_AUTH_HEADER"} \
  --data-urlencode 'query=<LogsQL>' \
  "${VM_LOGS_URL}/select/logsql/query?start=<RFC3339>&limit=<n>"
```

更完整的 LogsQL、OpenAPI AK/SK 调用见 `victoriametrics-logs` 技能。

## 占位符

`metric_query`、`log_query`、`start`、`end`、`step`、`limit`、`query_type` 等由问题与上下文替换。
