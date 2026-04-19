#!/usr/bin/env python3
"""
Query logs via OpenAPI LogService.QueryLog with AK/SK authentication.

Auth matches aiops-api-go-client ``middleware/auth``. The HTTP call matches
``LogServiceQueryLog``: POST ``/v1/openapi/log/queryLog`` with a JSON body
(``aiops.api.sdk.v1.QueryLogReq``), not GET query parameters.

Usage:
    python vl_query.py --url URL --ak AK --sk SK --query QUERY --start START [OPTIONS]

--start / --end: RFC3339, Unix seconds, or naive YYYY-MM-DD HH:MM:SS (interpreted as UTC).

Environment variables can also be used as fallback:
    VM_LOGS_URL, VM_AK, VM_SK
"""

import argparse
import base64
import hmac
import hashlib
import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime, timezone
from typing import Any


AC = "k8s-aiops"


def encrypt(content: str, key: str) -> str:
    """Replicate Go encrypt() function.

    First call: key is SK string (undergoes base64 decode + replacements)
    Second call: key is tmp_signature string from first call
    """
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
    """
    Replicate Go getSignature() function.

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


def build_request(
    url: str,
    ak: str,
    sk: str,
    method: str = "GET",
    data: bytes | None = None,
    content_type: str | None = None,
) -> urllib.request.Request:
    """Build request with AK/SK auth headers (matches middleware/auth RoundTrip)."""
    parsed = urllib.parse.urlparse(url)
    uri = parsed.path
    if parsed.query:
        uri = f"{parsed.path}?{parsed.query}"

    timestamp_ms, signature = get_signature(uri, ak, sk)

    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("x-alogic-now", timestamp_ms)
    req.add_header("x-alogic-app", ak)
    req.add_header("x-alogic-ac", AC)
    req.add_header("x-alogic-signature", signature)
    req.add_header("x-original-url", uri)
    if content_type:
        req.add_header("Content-Type", content_type)

    return req


def parse_time_to_unix(s: str) -> int:
    """QueryLogReq uses int32 epoch seconds (proto); accept unix digits, RFC3339, or naive ISO datetime."""
    s = s.strip()
    if s.isdigit() or (s.startswith("-") and s[1:].isdigit()):
        return int(s)
    if s.endswith("Z"):
        s = s[:-1] + "+00:00"
    dt = datetime.fromisoformat(s)
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return int(dt.timestamp())


def query_logs(
    url: str,
    ak: str,
    sk: str,
    query: str,
    start: str,
    end: str | None = None,
    limit: int = 100,
    fields: str | None = None,
    region_id: str | None = None,
    data_source_xid: str | None = None,
    page_index: int | None = None,
    reverse: bool | None = None,
) -> list[Any]:
    """
    Call OpenAPI LogService.QueryLog (POST /v1/openapi/log/queryLog), same as Go client.

    Request body matches aiops.api.sdk.v1.QueryLogReq (JSON snake_case). Returns the
    ``data`` array from QueryLogResp (list of log row objects).
    """
    if fields:
        sys.stderr.write(
            "Warning: --fields is not part of QueryLogReq; ignoring for this endpoint.\n"
        )

    base = url.rstrip("/")
    path = "/v1/openapi/log/queryLog"
    full_url = f"{base}{path}"

    body_obj: dict = {
        "query": query,
        "start": parse_time_to_unix(start),
    }
    if end:
        body_obj["end"] = parse_time_to_unix(end)
    if limit:
        body_obj["page_size"] = limit
    if region_id:
        body_obj["region_id"] = region_id
    if data_source_xid:
        body_obj["data_source_xid"] = data_source_xid
    if page_index is not None:
        body_obj["page_index"] = page_index
    if reverse is not None:
        body_obj["reverse"] = reverse

    payload = json.dumps(body_obj, separators=(",", ":")).encode("utf-8")
    req = build_request(
        full_url,
        ak,
        sk,
        method="POST",
        data=payload,
        content_type="application/json",
    )

    try:
        with urllib.request.urlopen(req) as resp:
            body = resp.read().decode("utf-8")
    except urllib.error.HTTPError as e:
        err_body = e.read().decode("utf-8", errors="replace")
        sys.stderr.write(f"HTTP {e.code}: {err_body}\n")
        raise

    parsed = json.loads(body)
    status = parsed.get("status") or {}
    code = status.get("code", 0)
    if code != 0:
        msg = status.get("message", "")
        raise RuntimeError(f"API status code={code} message={msg!r}")

    data = parsed.get("data")
    if data is None:
        return []
    if not isinstance(data, list):
        return [data]
    return data


def main():
    parser = argparse.ArgumentParser(
        description="Query logs via POST /v1/openapi/log/queryLog with AK/SK authentication."
    )
    parser.add_argument(
        "--url",
        help="API gateway base URL without path (e.g. https://aiops.example.com:30443)",
    )
    parser.add_argument("--ak", help="Access Key")
    parser.add_argument("--sk", help="Secret Key")
    parser.add_argument("--query", required=True, help="LogsQL query expression")
    parser.add_argument(
        "--start",
        required=True,
        help="Start: RFC3339, Unix seconds, or YYYY-MM-DD HH:MM:SS (naive = UTC)",
    )
    parser.add_argument(
        "--end",
        help="End: same formats as --start (optional)",
    )
    parser.add_argument(
        "--limit",
        type=int,
        default=100,
        help="Page size (maps to page_size in QueryLogReq, default: 100)",
    )
    parser.add_argument(
        "--fields",
        help="Not used by QueryLogReq; if set, a warning is printed and value is ignored",
    )
    parser.add_argument("--region-id", help="region_id in request body (optional)")
    parser.add_argument("--data-source-xid", help="data_source_xid in request body (optional)")
    parser.add_argument("--page-index", type=int, help="page_index in request body (optional)")
    parser.add_argument(
        "--reverse",
        action=argparse.BooleanOptionalAction,
        default=None,
        help="reverse sort (optional)",
    )
    parser.add_argument(
        "--format",
        choices=["json", "jsonl"],
        default="json",
        help="Output format: json (pretty) or jsonl (raw lines). Default: json",
    )

    args = parser.parse_args()

    # Fall back to env vars
    url = args.url or os.environ.get("VM_LOGS_URL", "")
    ak = args.ak or os.environ.get("VM_AK", "")
    sk = args.sk or os.environ.get("VM_SK", "")

    if not url:
        sys.stderr.write("Error: --url not provided and VM_LOGS_URL not set\n")
        sys.exit(1)
    if not ak:
        sys.stderr.write("Error: --ak not provided and VM_AK not set\n")
        sys.exit(1)
    if not sk:
        sys.stderr.write("Error: --sk not provided and VM_SK not set\n")
        sys.exit(1)

    logs = query_logs(
        url=url,
        ak=ak,
        sk=sk,
        query=args.query,
        start=args.start,
        end=args.end,
        limit=args.limit,
        fields=args.fields,
        region_id=args.region_id,
        data_source_xid=args.data_source_xid,
        page_index=args.page_index,
        reverse=args.reverse,
    )

    if args.format == "json":
        print(json.dumps(logs, indent=2, ensure_ascii=False))
    else:
        for entry in logs:
            print(json.dumps(entry, ensure_ascii=False))


if __name__ == "__main__":
    main()
