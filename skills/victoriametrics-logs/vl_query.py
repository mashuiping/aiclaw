#!/usr/bin/env python3
"""
VictoriaLogs query script with AK/SK authentication.

Replicates the HMAC-SHA256 auth scheme from aiops-api-go-client middleware/auth.

Usage:
    python vl_query.py --url URL --ak AK --sk SK --query QUERY --start START [OPTIONS]

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
import urllib.parse
import urllib.request


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


def build_request(url: str, ak: str, sk: str, **kwargs) -> urllib.request.Request:
    """Build VictoriaLogs request with AK/SK auth headers."""
    parsed = urllib.parse.urlparse(url)
    uri = parsed.path
    if parsed.query:
        uri = f"{parsed.path}?{parsed.query}"

    timestamp_ms, signature = get_signature(uri, ak, sk)

    req = urllib.request.Request(url)
    req.add_header("x-alogic-now", timestamp_ms)
    req.add_header("x-alogic-app", ak)
    req.add_header("x-alogic-ac", AC)
    req.add_header("x-alogic-signature", signature)
    req.add_header("x-original-url", uri)

    return req


def query_logs(
    url: str,
    ak: str,
    sk: str,
    query: str,
    start: str,
    end: str | None = None,
    limit: int = 100,
    fields: str | None = None,
) -> list[dict]:
    """
    Query VictoriaLogs /select/logsql/query endpoint.

    Returns a list of log entries (dicts).
    """
    params = {
        "query": query,
        "start": start,
    }
    if end:
        params["end"] = end
    if limit:
        params["limit"] = str(limit)
    if fields:
        params["fields"] = fields

    query_str = urllib.parse.urlencode(params)
    full_url = f"{url}/select/logsql/query?{query_str}"

    req = build_request(full_url, ak, sk)

    with urllib.request.urlopen(req) as resp:
        body = resp.read().decode("utf-8")

    # Response is JSON Lines (one JSON object per line)
    logs = []
    for line in body.splitlines():
        line = line.strip()
        if line:
            logs.append(json.loads(line))

    return logs


def main():
    parser = argparse.ArgumentParser(
        description="Query VictoriaLogs with AK/SK authentication."
    )
    parser.add_argument("--url", help="VictoriaLogs base URL (e.g. https://vlselect.example.com)")
    parser.add_argument("--ak", help="Access Key")
    parser.add_argument("--sk", help="Secret Key")
    parser.add_argument("--query", required=True, help="LogsQL query expression")
    parser.add_argument("--start", required=True, help="Start time (RFC3339, e.g. 2026-04-11T00:00:00Z)")
    parser.add_argument("--end", help="End time (RFC3339, optional)")
    parser.add_argument("--limit", type=int, default=100, help="Max log entries (default: 100)")
    parser.add_argument("--fields", help="Comma-separated fields to return (optional)")
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
    )

    if args.format == "json":
        print(json.dumps(logs, indent=2, ensure_ascii=False))
    else:
        for entry in logs:
            print(json.dumps(entry, ensure_ascii=False))


if __name__ == "__main__":
    main()
