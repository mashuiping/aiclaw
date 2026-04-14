#!/usr/bin/env python3
"""
VictoriaMetrics metrics query script with AK/SK authentication via aiops platform API.

Supports two endpoints:
  - /v1/openapi/monitor/queryHistoryPromql  (PromQL range queries)
  - /v1/openapi/monitor/queryLatestMetrics  (latest metric values)

Replicates the HMAC-SHA256 auth scheme from aiops-api-go-client middleware/auth.

Usage:
    python vm_query.py query-promql --query QUERY --start START --end END --step STEP [OPTIONS]
    python vm_query.py query-latest --dimensions DIM [--item-names ITEMS] [OPTIONS]

Environment variables can also be used as fallback:
    VM_METRICS_URL, VM_AK, VM_SK
"""

import argparse
import base64
import hmac
import hashlib
import json
import os
import sys
import time
import urllib.parse
import urllib.request


AC = "k8s-aiops"


def encrypt(content: str, key: str) -> str:
    """Replicate Go encrypt() from aiops-api-go-client middleware/auth."""
    key = key.replace(" ", "+")
    key = key.replace("-", "+")
    key = key.replace("_", "/")
    while len(key) % 4 != 0:
        key += "="
    bcode = base64.standard_b64decode(key)
    sigbyte = hmac.new(bcode, content.encode(), hashlib.sha256).digest()
    sigstr = base64.urlsafe_b64encode(sigbyte).decode()
    signature = sigstr.rstrip("=")
    return signature


def get_signature(uri: str, ak: str, sk: str) -> tuple[str, str]:
    """Replicate Go getSignature() from aiops-api-go-client middleware/auth.

    Returns (timestamp_ms_str, signature).
    """
    timestamp_ms = int(time.time() * 1000)
    timestamp_day = timestamp_ms // 86400000
    timestamp_ms_str = str(timestamp_ms)

    sign_str = f"{ak}\n{timestamp_ms}\n{uri}"
    identity = f"{ak}:{timestamp_day}"

    tmp_signature = encrypt(identity, sk)
    signature = encrypt(sign_str, tmp_signature)

    return timestamp_ms_str, signature


def build_request(url: str, ak: str, sk: str) -> urllib.request.Request:
    """Build authenticated request with AK/SK headers."""
    parsed = urllib.parse.urlparse(url)
    uri = parsed.path
    if parsed.query:
        uri = f"{parsed.path}?{parsed.query}"

    timestamp_ms, signature = get_signature(uri, ak, sk)

    req = urllib.request.Request(url)
    req.add_header("Content-Type", "application/json")
    req.add_header("x-alogic-now", timestamp_ms)
    req.add_header("x-alogic-app", ak)
    req.add_header("x-alogic-ac", AC)
    req.add_header("x-alogic-signature", signature)
    req.add_header("x-original-url", uri)

    return req


def post_json(url: str, ak: str, sk: str, body: dict) -> dict:
    """POST JSON and return parsed response body."""
    req = build_request(url, ak, sk)
    req.data = json.dumps(body).encode("utf-8")

    with urllib.request.urlopen(req) as resp:
        return json.loads(resp.read().decode("utf-8"))


def rfc3339_to_unix(ts: str) -> int:
    """Convert RFC3339 timestamp to Unix seconds."""
    # Handle both formats: with and without timezone
    try:
        from datetime import datetime
        dt = datetime.fromisoformat(ts.replace("Z", "+00:00"))
        return int(dt.timestamp())
    except Exception:
        # Fallback: assume it's already a Unix timestamp string
        return int(ts)


def query_history_promql(
    url: str,
    ak: str,
    sk: str,
    query: str,
    start: str,
    end: str,
    step: int | str,
    data_source_xid: str | None = None,
    region_id: str | None = None,
) -> dict:
    """
    Query VictoriaMetrics history using PromQL via aiops platform.

    Args:
        url:      Base URL of aiops platform (e.g. https://aiops.example.com)
        ak:       Access Key
        sk:       Secret Key
        query:    PromQL query expression
        start:    Start time (RFC3339 or Unix seconds)
        end:      End time (RFC3339 or Unix seconds)
        step:     Query resolution step in seconds (int)
        data_source_xid: (optional) data source xid
        region_id:        (optional) region id
    """
    # Convert RFC3339 to Unix timestamps if needed
    try:
        start_ts = rfc3339_to_unix(start) if not start.isdigit() else int(start)
    except (ValueError, TypeError):
        start_ts = int(start)
    try:
        end_ts = rfc3339_to_unix(end) if not end.isdigit() else int(end)
    except (ValueError, TypeError):
        end_ts = int(end)

    body = {
        "query": query,
        "start": start_ts,
        "end": end_ts,
        "step": int(step),
    }
    if data_source_xid:
        body["data_source_xid"] = data_source_xid
    if region_id:
        body["region_id"] = region_id

    endpoint = f"{url}/v1/openapi/monitor/queryHistoryPromql"
    return post_json(endpoint, ak, sk, body)


def query_latest_metrics(
    url: str,
    ak: str,
    sk: str,
    dimensions: list[dict] | None = None,
    item_name_list: list[str] | None = None,
    config: dict | None = None,
    data_source_xid: str | None = None,
    region_id: str | None = None,
) -> dict:
    """
    Query latest metric values via aiops platform.

    Args:
        url:             Base URL of aiops platform
        ak:              Access Key
        sk:              Secret Key
        dimensions:      List of {"name": str, "value": [str]} dicts
        item_name_list:  List of metric item names
        config:          Additional config dict
        data_source_xid: (optional) data source xid
        region_id:       (optional) region id
    """
    body = {}
    if dimensions:
        body["dimensions"] = dimensions
    if item_name_list:
        body["item_name_list"] = item_name_list
    if config:
        body["config"] = config
    if data_source_xid:
        body["data_source_xid"] = data_source_xid
    if region_id:
        body["region_id"] = region_id

    endpoint = f"{url}/v1/openapi/monitor/queryLatestMetrics"
    return post_json(endpoint, ak, sk, body)


def parse_dimensions(dims_str: str) -> list[dict]:
    """Parse dimension string like 'name:val1,val2;name2:val3' into list of dicts."""
    dimensions = []
    for part in dims_str.split(";"):
        part = part.strip()
        if not part:
            continue
        if ":" not in part:
            dimensions.append({"name": part, "value": []})
        else:
            name, values_str = part.split(":", 1)
            values = [v.strip() for v in values_str.split(",") if v.strip()]
            dimensions.append({"name": name.strip(), "value": values})
    return dimensions


def main():
    parser = argparse.ArgumentParser(
        description="Query VictoriaMetrics metrics via aiops platform REST API."
    )
    sub = parser.add_subparsers(dest="command", required=True)

    # promql subcommand
    p = sub.add_parser("query-promql", help="PromQL range query")
    p.add_argument("--query", required=True, help="PromQL expression")
    p.add_argument("--start", required=True, help="Start time (RFC3339 or Unix seconds)")
    p.add_argument("--end", required=True, help="End time (RFC3339 or Unix seconds)")
    p.add_argument("--step", required=True, help="Step in seconds (int)")
    p.add_argument("--data-source-xid", help="Data source xid (optional)")
    p.add_argument("--region-id", help="Region ID (optional)")

    # latest subcommand
    l = sub.add_parser("query-latest", help="Query latest metric values")
    l.add_argument("--dimensions", help="Dimensions as 'name:val1,val2;name2:val3'")
    l.add_argument("--item-names", help="Comma-separated item names")
    l.add_argument("--config", help="JSON config object")
    l.add_argument("--data-source-xid", help="Data source xid (optional)")
    l.add_argument("--region-id", help="Region ID (optional)")

    # shared
    parser.add_argument("--url", help="Aiops platform base URL")
    parser.add_argument("--ak", help="Access Key")
    parser.add_argument("--sk", help="Secret Key")
    parser.add_argument(
        "--format",
        choices=["json", "prometheus"],
        default="json",
        help="Output format: json (full response) or prometheus (pretty-printed result). Default: json",
    )

    args = parser.parse_args()

    # Fall back to env vars
    url = args.url or os.environ.get("VM_METRICS_URL", "")
    ak = args.ak or os.environ.get("VM_AK", "")
    sk = args.sk or os.environ.get("VM_SK", "")

    if not url:
        sys.stderr.write("Error: --url not provided and VM_METRICS_URL not set\n")
        sys.exit(1)
    if not ak:
        sys.stderr.write("Error: --ak not provided and VM_AK not set\n")
        sys.exit(1)
    if not sk:
        sys.stderr.write("Error: --sk not provided and VM_SK not set\n")
        sys.exit(1)

    if args.command == "query-promql":
        result = query_history_promql(
            url=url,
            ak=ak,
            sk=sk,
            query=args.query,
            start=args.start,
            end=args.end,
            step=args.step,
            data_source_xid=args.data_source_xid,
            region_id=args.region_id,
        )
    else:
        dimensions = None
        if args.dimensions:
            dimensions = parse_dimensions(args.dimensions)

        item_name_list = None
        if args.item_names:
            item_name_list = [x.strip() for x in args.item_names.split(",") if x.strip()]

        config = None
        if args.config:
            config = json.loads(args.config)

        result = query_latest_metrics(
            url=url,
            ak=ak,
            sk=sk,
            dimensions=dimensions,
            item_name_list=item_name_list,
            config=config,
            data_source_xid=args.data_source_xid,
            region_id=args.region_id,
        )

    if args.format == "prometheus":
        # Extract Prometheus-format result from item_data.data.result
        try:
            item_data = result.get("item_data", {})
            data = item_data.get("data", {})
            result_type = data.get("resultType", "")
            results = data.get("result", [])
            print(f"# resultType: {result_type}")
            for r in results:
                metric = r.get("metric", {})
                metric_str = " ".join(f'{k}="{v}"' for k, v in sorted(metric.items()))
                if result_type == "matrix":
                    for value in r.get("values", []):
                        ts, val = value
                        print(f"{metric_str} {val} {ts}")
                else:
                    value = r.get("value", [])
                    if value:
                        ts, val = value
                        print(f"{metric_str} {val} {ts}")
        except Exception as e:
            sys.stderr.write(f"Warning: could not parse prometheus format: {e}\n")
            print(json.dumps(result, indent=2, ensure_ascii=False))
    else:
        print(json.dumps(result, indent=2, ensure_ascii=False))


if __name__ == "__main__":
    main()
