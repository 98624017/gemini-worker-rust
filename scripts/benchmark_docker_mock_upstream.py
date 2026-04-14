#!/usr/bin/env python3
from __future__ import annotations

import argparse
import base64
import csv
import json
import os
import re
import socket
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, Iterable


DEFAULT_RESPONSE_BASE64_BYTES = 20 * 1024 * 1024
DEFAULT_PROXY_PORT = 18788
DEFAULT_SAMPLE_INTERVAL_SECONDS = 1.0
DEFAULT_ADMIN_LOG_POLL_INTERVAL_SECONDS = 0.2
DEFAULT_COOLDOWN_SECONDS = 60.0
DEFAULT_REQUEST_TIMEOUT_SECONDS = 300.0
DEFAULT_ADMIN_PASSWORD = "bench-admin"
DEFAULT_UPSTREAM_API_KEY = "bench-upstream-key"
DEFAULT_IMAGE = "rust-sync-proxy:jemalloc-test"
DEFAULT_MOCK_BIND_HOST = "0.0.0.0"
DEFAULT_MOCK_LOCALHOST = "127.0.0.1"
VMRSS_PATTERN = re.compile(r"^VmRSS:\s+(\d+)\s+kB$", re.MULTILINE)
PASSTHROUGH_ENV_NAMES = (
    "IMAGE_HOST_MODE",
    "EXTERNAL_IMAGE_PROXY_PREFIX",
    "IMAGE_FETCH_TIMEOUT_MS",
    "IMAGE_TLS_HANDSHAKE_TIMEOUT_MS",
    "IMAGE_FETCH_INSECURE_SKIP_VERIFY",
    "IMAGE_FETCH_EXTERNAL_PROXY_DOMAINS",
    "INLINE_DATA_URL_CACHE_DIR",
    "INLINE_DATA_URL_CACHE_TTL_MS",
    "INLINE_DATA_URL_CACHE_MAX_BYTES",
    "INLINE_DATA_URL_MEMORY_CACHE_MAX_BYTES",
    "INLINE_DATA_URL_BACKGROUND_FETCH_WAIT_TIMEOUT_MS",
    "INLINE_DATA_URL_BACKGROUND_FETCH_TOTAL_TIMEOUT_MS",
    "INLINE_DATA_URL_BACKGROUND_FETCH_MAX_INFLIGHT",
    "INSTANCE_MEMORY_BYTES",
    "BLOB_INLINE_MAX_BYTES",
    "BLOB_REQUEST_HOT_BUDGET_BYTES",
    "BLOB_GLOBAL_HOT_BUDGET_BYTES",
    "BLOB_SPILL_DIR",
    "UPLOAD_TIMEOUT_MS",
    "UPLOAD_TLS_HANDSHAKE_TIMEOUT_MS",
    "UPLOAD_INSECURE_SKIP_VERIFY",
    "LEGACY_UGUU_UPLOAD_URL",
    "LEGACY_KEFAN_UPLOAD_URL",
    "R2_ENDPOINT",
    "R2_BUCKET",
    "R2_ACCESS_KEY_ID",
    "R2_SECRET_ACCESS_KEY",
    "R2_PUBLIC_BASE_URL",
    "R2_OBJECT_PREFIX",
    "MALLOC_CONF",
)


def build_base64_payload(target_len: int) -> str:
    if target_len <= 0:
        raise ValueError("target_len must be positive")
    if target_len % 4 != 0:
        raise ValueError("target_len must be a multiple of 4 for exact base64 sizing")
    raw_len = (target_len // 4) * 3
    payload = base64.b64encode(b"\x89" * raw_len).decode("ascii")
    if len(payload) != target_len:
        raise ValueError(f"generated payload length {len(payload)} != target {target_len}")
    return payload


def build_request_body(
    image_urls: list[str],
    output_mode: str = "url",
) -> dict[str, Any]:
    if len(image_urls) != 3:
        raise ValueError("exactly three image URLs are required")
    body = {
        "output": "url",
        "contents": [
            {
                "parts": [
                    {"inlineData": {"data": image_url}} for image_url in image_urls
                ]
            }
        ],
    }
    if output_mode == "url":
        return body
    if output_mode == "base64":
        body.pop("output", None)
        return body
    raise ValueError("output_mode must be 'url' or 'base64'")


def append_cache_buster(raw_url: str, request_id: int) -> str:
    parsed = urllib.parse.urlparse(raw_url)
    query = urllib.parse.parse_qsl(parsed.query, keep_blank_values=True)
    query.append(("bench_request_id", str(request_id)))
    return urllib.parse.urlunparse(
        parsed._replace(query=urllib.parse.urlencode(query))
    )


def build_request_body_bytes(
    image_urls: list[str],
    output_mode: str,
    request_id: int,
    cache_bust_urls: bool,
) -> bytes:
    effective_urls = image_urls
    if cache_bust_urls:
        effective_urls = [append_cache_buster(url, request_id) for url in image_urls]
    request_body = build_request_body(effective_urls, output_mode)
    return json.dumps(request_body, separators=(",", ":")).encode("utf-8")


def build_scenario_metadata(output_mode: str, warm_cache: bool) -> dict[str, str]:
    cache_state = "hit" if warm_cache else "miss"
    return {
        "scenario": f"{cache_state}/{output_mode}",
        "cache_state": cache_state,
        "output_mode": output_mode,
    }


def build_request_targets(
    proxy_base_url: str,
    direct_base_url: str,
    model: str = "bench",
) -> dict[str, str]:
    suffix = f"/v1beta/models/{model}:generateContent"
    return {
        "direct": f"{direct_base_url}{suffix}",
        "proxy": f"{proxy_base_url}{suffix}",
    }


def build_mock_response(base64_payload: str) -> bytes:
    body = {
        "thoughtSignature": "bench-secret",
        "candidates": [
            {
                "finishReason": "STOP",
                "content": {
                    "parts": [
                        {
                            "inlineData": {
                                "mimeType": "image/png",
                                "data": base64_payload,
                            }
                        }
                    ]
                },
            }
        ],
    }
    return json.dumps(body, separators=(",", ":")).encode("utf-8")


def basic_auth_header(username: str, password: str) -> str:
    token = base64.b64encode(f"{username}:{password}".encode("utf-8")).decode("ascii")
    return f"Basic {token}"


def percentile_ms(values: list[float], percentile: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    rank = round((len(ordered) - 1) * percentile)
    return ordered[max(0, min(rank, len(ordered) - 1))]


def average_ms(values: list[float]) -> float:
    if not values:
        return 0.0
    return sum(values) / len(values)


def ensure_dir(path: Path) -> None:
    path.mkdir(parents=True, exist_ok=True)


def find_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind((DEFAULT_MOCK_BIND_HOST, 0))
        return int(sock.getsockname()[1])


def read_json(url: str, headers: dict[str, str], timeout: float) -> dict[str, Any]:
    request = urllib.request.Request(url, headers=headers, method="GET")
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return json.loads(response.read().decode("utf-8"))


def read_admin_logs(base_url: str, auth_header: str, timeout: float) -> list[dict[str, Any]]:
    payload = read_json(
        f"{base_url}/admin/api/logs",
        {"Authorization": auth_header},
        timeout=timeout,
    )
    items = payload.get("items", [])
    if isinstance(items, list):
        return items
    return []


def write_csv(path: Path, rows: list[dict[str, Any]], fieldnames: Iterable[str]) -> None:
    with path.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=list(fieldnames))
        writer.writeheader()
        writer.writerows(rows)


def extract_stage_stats_row(payload: dict[str, Any], sampled_at: float) -> dict[str, Any]:
    return {
        "timestamp": sampled_at,
        "totalRequests": payload.get("totalRequests", 0),
        "errorRequests": payload.get("errorRequests", 0),
        "cacheHits": payload.get("cacheHits", 0),
        "spillCount": payload.get("spillCount", 0),
        "spillBytesTotal": payload.get("spillBytesTotal", 0),
        "requestParseMs": payload.get("requestParseMs", 0),
        "requestImagePrepareMs": payload.get("requestImagePrepareMs", 0),
        "requestImageMaterializeMs": payload.get("requestImageMaterializeMs", 0),
        "requestImageFetchWorkMs": payload.get("requestImageFetchWorkMs", 0),
        "requestImageStoreWorkMs": payload.get("requestImageStoreWorkMs", 0),
        "requestEncodeMs": payload.get("requestEncodeMs", 0),
        "upstreamBuildMs": payload.get("upstreamBuildMs", 0),
        "responseProcessMs": payload.get("responseProcessMs", 0),
        "uploadMs": payload.get("uploadMs", 0),
    }


def newest_admin_log_id(items: list[dict[str, Any]]) -> int:
    newest = 0
    for item in items:
        raw_id = item.get("id", 0)
        if isinstance(raw_id, int):
            newest = max(newest, raw_id)
    return newest


def select_new_admin_log_items(
    items: list[dict[str, Any]],
    baseline_log_id: int,
) -> list[dict[str, Any]]:
    selected = [
        item
        for item in items
        if isinstance(item.get("id"), int) and item["id"] > baseline_log_id
    ]
    selected.sort(key=lambda item: item["id"])
    return selected


def extract_admin_log_stage_row(item: dict[str, Any]) -> dict[str, Any]:
    return {
        "id": item.get("id", 0),
        "statusCode": item.get("statusCode", 0),
        "durationMs": item.get("durationMs", 0),
        "requestParseMs": item.get("requestParseMs", 0),
        "requestImagePrepareMs": item.get("requestImagePrepareMs", 0),
        "requestImageMaterializeMs": item.get("requestImageMaterializeMs", 0),
        "requestImageFetchWorkMs": item.get("requestImageFetchWorkMs", 0),
        "requestImageStoreWorkMs": item.get("requestImageStoreWorkMs", 0),
        "requestEncodeMs": item.get("requestEncodeMs", 0),
        "upstreamBuildMs": item.get("upstreamBuildMs", 0),
        "responseProcessMs": item.get("responseProcessMs", 0),
        "uploadMs": item.get("uploadMs", 0),
        "errorStage": item.get("errorStage", ""),
        "errorKind": item.get("errorKind", ""),
    }


def build_admin_log_stage_summary(items: list[dict[str, Any]]) -> dict[str, float]:
    fields = [
        "durationMs",
        "requestParseMs",
        "requestImagePrepareMs",
        "requestImageMaterializeMs",
        "requestImageFetchWorkMs",
        "requestImageStoreWorkMs",
        "requestEncodeMs",
        "upstreamBuildMs",
        "responseProcessMs",
        "uploadMs",
    ]
    summary: dict[str, float] = {}
    for field in fields:
        values = [
            float(item[field])
            for item in items
            if isinstance(item.get(field), (int, float))
        ]
        summary[f"avg{field[0].upper()}{field[1:]}"] = round(average_ms(values), 3)
    return summary


def merge_admin_log_items(
    items_by_id: dict[int, dict[str, Any]],
    items: list[dict[str, Any]],
    baseline_log_id: int,
    max_seen_id: int,
    gap_detected: bool,
) -> tuple[int, bool]:
    for item in select_new_admin_log_items(items, baseline_log_id):
        item_id = item["id"]
        if item_id > max_seen_id + 1:
            gap_detected = True
        items_by_id.setdefault(item_id, item)
        max_seen_id = max(max_seen_id, item_id)
    return max_seen_id, gap_detected


def run_command(args: list[str], timeout: float | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        args,
        check=False,
        text=True,
        capture_output=True,
        timeout=timeout,
    )


class MockGenerateContentHandler(BaseHTTPRequestHandler):
    response_body: bytes = b""

    def do_GET(self) -> None:  # noqa: N802
        if self.path == "/healthz":
            body = b"ok"
            self.send_response(200)
            self.send_header("Content-Type", "text/plain; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return

        self.send_response(404)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def do_POST(self) -> None:  # noqa: N802
        length = int(self.headers.get("Content-Length", "0"))
        if length:
            self.rfile.read(length)

        if self.path.startswith("/v1beta/models/") and self.path.endswith(":generateContent"):
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(self.response_body)))
            self.end_headers()
            self.wfile.write(self.response_body)
            return

        self.send_response(404)
        self.send_header("Content-Length", "0")
        self.end_headers()

    def log_message(self, format: str, *args: Any) -> None:
        return


@dataclass
class RequestResult:
    request_id: int
    started_at: float
    duration_ms: float
    status_code: int
    ok: bool
    error: str


def build_comparison_summary(
    direct_results: list[RequestResult],
    proxy_results: list[RequestResult],
) -> dict[str, float]:
    direct_success_ms = [result.duration_ms for result in direct_results if result.ok]
    proxy_success_ms = [result.duration_ms for result in proxy_results if result.ok]
    direct_total_ms = round(average_ms(direct_success_ms), 3)
    proxy_total_ms = round(average_ms(proxy_success_ms), 3)
    direct_p50_ms = round(percentile_ms(direct_success_ms, 0.50), 3)
    proxy_p50_ms = round(percentile_ms(proxy_success_ms, 0.50), 3)
    direct_p95_ms = round(percentile_ms(direct_success_ms, 0.95), 3)
    proxy_p95_ms = round(percentile_ms(proxy_success_ms, 0.95), 3)
    return {
        "direct_total_ms": direct_total_ms,
        "proxy_total_ms": proxy_total_ms,
        "proxy_overhead_ms": round(proxy_total_ms - direct_total_ms, 3),
        "direct_p50_ms": direct_p50_ms,
        "proxy_p50_ms": proxy_p50_ms,
        "proxy_overhead_p50_ms": round(proxy_p50_ms - direct_p50_ms, 3),
        "direct_p95_ms": direct_p95_ms,
        "proxy_p95_ms": proxy_p95_ms,
        "proxy_overhead_p95_ms": round(proxy_p95_ms - direct_p95_ms, 3),
    }


def start_mock_server(base64_payload_len: int) -> tuple[ThreadingHTTPServer, int]:
    port = find_free_port()
    handler = type(
        "ConfiguredMockGenerateContentHandler",
        (MockGenerateContentHandler,),
        {"response_body": build_mock_response(build_base64_payload(base64_payload_len))},
    )
    # Bind on all interfaces so the Docker container can reach the host mock
    # through host.docker.internal while the host can still access it via 127.0.0.1.
    server = ThreadingHTTPServer((DEFAULT_MOCK_BIND_HOST, port), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server, port


def wait_for_proxy(base_url: str, auth_header: str, timeout_seconds: float) -> None:
    deadline = time.time() + timeout_seconds
    headers = {"Authorization": auth_header}
    last_error = "proxy did not become ready"
    while time.time() < deadline:
        try:
            read_json(f"{base_url}/admin/api/stats", headers, timeout=5.0)
            return
        except Exception as exc:  # noqa: BLE001
            last_error = str(exc)
            time.sleep(1.0)
    raise RuntimeError(last_error)


def parse_vmrss_kb(status_text: str) -> int | None:
    match = VMRSS_PATTERN.search(status_text)
    if not match:
        return None
    return int(match.group(1))


def sample_rss(
    container_name: str,
    stop_event: threading.Event,
    sample_interval_seconds: float,
    rss_rows: list[dict[str, Any]],
) -> None:
    while not stop_event.is_set():
        sampled_at = time.time()
        result = run_command(
            ["docker", "exec", container_name, "cat", "/proc/1/status"],
            timeout=10.0,
        )
        vmrss_kb = None
        if result.returncode == 0:
            vmrss_kb = parse_vmrss_kb(result.stdout)
        if vmrss_kb is not None:
            rss_rows.append({"timestamp": sampled_at, "vmrssKb": vmrss_kb})
        stop_event.wait(sample_interval_seconds)


def sample_admin_stats(
    base_url: str,
    auth_header: str,
    stop_event: threading.Event,
    sample_interval_seconds: float,
    stats_rows: list[dict[str, Any]],
) -> None:
    headers = {"Authorization": auth_header}
    while not stop_event.is_set():
        sampled_at = time.time()
        try:
            payload = read_json(f"{base_url}/admin/api/stats", headers, timeout=10.0)
        except Exception:  # noqa: BLE001
            payload = None
        if payload is not None:
            stats_rows.append(extract_stage_stats_row(payload, sampled_at))
        stop_event.wait(sample_interval_seconds)


def sample_admin_logs(
    base_url: str,
    auth_header: str,
    stop_event: threading.Event,
    sample_interval_seconds: float,
    baseline_log_id: int,
    items_by_id: dict[int, dict[str, Any]],
    state_lock: threading.Lock,
    log_capture_state: dict[str, Any],
) -> None:
    while not stop_event.is_set():
        try:
            items = read_admin_logs(base_url, auth_header, timeout=10.0)
        except Exception:  # noqa: BLE001
            items = None
        if items is not None:
            with state_lock:
                max_seen_id, gap_detected = merge_admin_log_items(
                    items_by_id,
                    items,
                    baseline_log_id,
                    int(log_capture_state["max_seen_id"]),
                    bool(log_capture_state["gap_detected"]),
                )
                log_capture_state["max_seen_id"] = max_seen_id
                log_capture_state["gap_detected"] = gap_detected
        stop_event.wait(sample_interval_seconds)


def send_request(
    request_url: str,
    request_body_bytes: bytes,
    timeout_seconds: float,
    request_id: int,
    expect_url_payload: bool,
) -> RequestResult:
    started_at = time.time()
    request = urllib.request.Request(
        request_url,
        data=request_body_bytes,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout_seconds) as response:
            status_code = response.status
            body = json.loads(response.read().decode("utf-8"))
        inline_data = body["candidates"][0]["content"]["parts"][0]["inlineData"]["data"]
        if expect_url_payload:
            ok = (
                status_code == 200
                and isinstance(inline_data, str)
                and inline_data.startswith("http")
            )
        else:
            ok = status_code == 200 and isinstance(inline_data, str) and len(inline_data) > 0
        error = "" if ok else "unexpected response payload"
        duration_ms = (time.time() - started_at) * 1000.0
        return RequestResult(request_id, started_at, duration_ms, status_code, ok, error)
    except urllib.error.HTTPError as exc:
        duration_ms = (time.time() - started_at) * 1000.0
        return RequestResult(
            request_id,
            started_at,
            duration_ms,
            exc.code,
            False,
            f"http error: {exc.reason}",
        )
    except Exception as exc:  # noqa: BLE001
        duration_ms = (time.time() - started_at) * 1000.0
        return RequestResult(
            request_id,
            started_at,
            duration_ms,
            0,
            False,
            str(exc),
        )


def run_load(
    request_url: str,
    image_urls: list[str],
    output_mode: str,
    concurrency: int,
    total_requests: int | None,
    duration_seconds: float | None,
    timeout_seconds: float,
    expect_url_payload: bool,
    cache_bust_urls: bool,
) -> list[RequestResult]:
    next_request_id = 0
    deadline = time.time() + duration_seconds if duration_seconds is not None else None
    request_lock = threading.Lock()

    def allocate_request_id() -> int | None:
        nonlocal next_request_id
        with request_lock:
            if total_requests is not None and next_request_id >= total_requests:
                return None
            if deadline is not None and time.time() >= deadline:
                return None
            next_request_id += 1
            return next_request_id

    def worker() -> list[RequestResult]:
        worker_results: list[RequestResult] = []
        while True:
            request_id = allocate_request_id()
            if request_id is None:
                return worker_results
            request_body_bytes = build_request_body_bytes(
                image_urls,
                output_mode,
                request_id,
                cache_bust_urls,
            )
            worker_results.append(
                send_request(
                    request_url,
                    request_body_bytes,
                    timeout_seconds,
                    request_id,
                    expect_url_payload,
                )
            )

    all_results: list[RequestResult] = []
    with ThreadPoolExecutor(max_workers=concurrency) as executor:
        futures = [executor.submit(worker) for _ in range(concurrency)]
        for future in futures:
            all_results.extend(future.result())
    all_results.sort(key=lambda result: result.request_id)
    return all_results


def collect_container_env(args: argparse.Namespace, mock_port: int) -> list[str]:
    env_pairs = [
        ("PORT", "8787"),
        ("RUST_LOG", "info"),
        ("ADMIN_PASSWORD", DEFAULT_ADMIN_PASSWORD),
        ("UPSTREAM_BASE_URL", f"http://host.docker.internal:{mock_port}"),
        ("UPSTREAM_API_KEY", DEFAULT_UPSTREAM_API_KEY),
    ]
    if args.malloc_conf:
        env_pairs.append(("MALLOC_CONF", args.malloc_conf))

    for name in PASSTHROUGH_ENV_NAMES:
        value = os.environ.get(name)
        if value is None or value == "":
            continue
        if name == "MALLOC_CONF" and args.malloc_conf:
            continue
        env_pairs.append((name, value))

    docker_args: list[str] = []
    for key, value in env_pairs:
        docker_args.extend(["-e", f"{key}={value}"])
    return docker_args


def create_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Docker benchmark for rust-sync-proxy with real request images and mock upstream."
    )
    parser.add_argument(
        "--image",
        default=DEFAULT_IMAGE,
        help="Docker image name to run. Default: %(default)s",
    )
    parser.add_argument(
        "--output-mode",
        choices=("base64", "url"),
        default="url",
        help="Expected proxy output mode for this benchmark run. Default: %(default)s",
    )
    parser.add_argument(
        "--warm-cache",
        action="store_true",
        help="Warm request-side image cache before measured proxy requests.",
    )
    parser.add_argument(
        "--image-url",
        action="append",
        required=True,
        dest="image_urls",
        help="Real request image URL. Pass exactly three times.",
    )
    parser.add_argument(
        "--proxy-port",
        type=int,
        default=DEFAULT_PROXY_PORT,
        help="Host port mapped to container 8787. Default: %(default)s",
    )
    parser.add_argument(
        "--concurrency",
        type=int,
        default=2,
        help="Concurrent request workers. Default: %(default)s",
    )
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument(
        "--total-requests",
        type=int,
        help="Total requests to send.",
    )
    group.add_argument(
        "--duration-seconds",
        type=float,
        help="Run load generation for this many seconds.",
    )
    parser.add_argument(
        "--cooldown-seconds",
        type=float,
        default=DEFAULT_COOLDOWN_SECONDS,
        help="Keep sampling after load for RSS decay observation. Default: %(default)s",
    )
    parser.add_argument(
        "--sample-interval-seconds",
        type=float,
        default=DEFAULT_SAMPLE_INTERVAL_SECONDS,
        help="Sampling interval for RSS and stats. Default: %(default)s",
    )
    parser.add_argument(
        "--request-timeout-seconds",
        type=float,
        default=DEFAULT_REQUEST_TIMEOUT_SECONDS,
        help="Per-request timeout. Default: %(default)s",
    )
    parser.add_argument(
        "--response-base64-bytes",
        type=int,
        default=DEFAULT_RESPONSE_BASE64_BYTES,
        help="Exact mock upstream base64 size in bytes. Must be a multiple of 4.",
    )
    parser.add_argument(
        "--malloc-conf",
        default=os.environ.get("MALLOC_CONF", ""),
        help="Override MALLOC_CONF passed to the container.",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=Path("benchmark-output") / time.strftime("%Y%m%d-%H%M%S"),
        help="Directory for summary and samples. Default: %(default)s",
    )
    return parser


def main() -> int:
    parser = create_parser()
    args = parser.parse_args()

    if len(args.image_urls) != 3:
        parser.error("exactly three --image-url arguments are required")
    if args.concurrency <= 0:
        parser.error("--concurrency must be positive")
    if args.total_requests is not None and args.total_requests <= 0:
        parser.error("--total-requests must be positive")
    if args.duration_seconds is not None and args.duration_seconds <= 0:
        parser.error("--duration-seconds must be positive")
    if args.cooldown_seconds < 0:
        parser.error("--cooldown-seconds cannot be negative")
    if args.response_base64_bytes % 4 != 0:
        parser.error("--response-base64-bytes must be a multiple of 4")

    ensure_dir(args.output_dir)

    mock_server, mock_port = start_mock_server(args.response_base64_bytes)
    container_name = f"rust-sync-proxy-bench-{uuid.uuid4().hex[:8]}"
    base_url = f"http://127.0.0.1:{args.proxy_port}"
    scenario_metadata = build_scenario_metadata(args.output_mode, args.warm_cache)
    auth_header = basic_auth_header("bench", DEFAULT_ADMIN_PASSWORD)
    rss_rows: list[dict[str, Any]] = []
    stats_rows: list[dict[str, Any]] = []
    request_rows: list[dict[str, Any]] = []
    admin_log_items: list[dict[str, Any]] = []
    benchmark_log_items: list[dict[str, Any]] = []
    direct_results: list[RequestResult] = []
    proxy_results: list[RequestResult] = []
    stop_event = threading.Event()
    baseline_log_id = 0
    admin_log_items_by_id: dict[int, dict[str, Any]] = {}
    admin_log_state_lock = threading.Lock()
    admin_log_capture_state = {
        "max_seen_id": 0,
        "gap_detected": False,
    }

    docker_run_args = [
        "docker",
        "run",
        "-d",
        "--rm",
        "--name",
        container_name,
        "--add-host",
        "host.docker.internal:host-gateway",
        "-p",
        f"{args.proxy_port}:8787",
        *collect_container_env(args, mock_port),
        args.image,
    ]

    container_id = ""
    rss_thread = threading.Thread(
        target=sample_rss,
        args=(container_name, stop_event, args.sample_interval_seconds, rss_rows),
        daemon=True,
    )
    stats_thread = threading.Thread(
        target=sample_admin_stats,
        args=(base_url, auth_header, stop_event, args.sample_interval_seconds, stats_rows),
        daemon=True,
    )
    admin_log_thread: threading.Thread | None = None

    try:
        run_result = run_command(docker_run_args, timeout=30.0)
        if run_result.returncode != 0:
            raise RuntimeError(run_result.stderr.strip() or run_result.stdout.strip())
        container_id = run_result.stdout.strip()

        wait_for_proxy(base_url, auth_header, timeout_seconds=60.0)

        request_targets = build_request_targets(
            proxy_base_url=base_url,
            direct_base_url=f"http://{DEFAULT_MOCK_LOCALHOST}:{mock_port}",
        )
        direct_results = run_load(
            request_url=request_targets["direct"],
            image_urls=args.image_urls,
            output_mode=args.output_mode,
            concurrency=args.concurrency,
            total_requests=args.total_requests,
            duration_seconds=args.duration_seconds,
            timeout_seconds=args.request_timeout_seconds,
            expect_url_payload=False,
            cache_bust_urls=not args.warm_cache,
        )

        if args.warm_cache:
            request_body_bytes = build_request_body_bytes(
                args.image_urls,
                args.output_mode,
                request_id=0,
                cache_bust_urls=False,
            )
            warmup_result = send_request(
                request_targets["proxy"],
                request_body_bytes,
                args.request_timeout_seconds,
                request_id=0,
                expect_url_payload=args.output_mode == "url",
            )
            if not warmup_result.ok:
                raise RuntimeError(f"proxy warmup failed: {warmup_result.error}")

        baseline_log_id = newest_admin_log_id(
            read_admin_logs(base_url, auth_header, timeout=10.0)
        )
        with admin_log_state_lock:
            admin_log_capture_state["max_seen_id"] = baseline_log_id

        rss_thread.start()
        stats_thread.start()
        admin_log_thread = threading.Thread(
            target=sample_admin_logs,
            args=(
                base_url,
                auth_header,
                stop_event,
                max(
                    0.01,
                    min(
                        args.sample_interval_seconds,
                        DEFAULT_ADMIN_LOG_POLL_INTERVAL_SECONDS,
                    ),
                ),
                baseline_log_id,
                admin_log_items_by_id,
                admin_log_state_lock,
                admin_log_capture_state,
            ),
            daemon=True,
        )
        admin_log_thread.start()

        proxy_results = run_load(
            request_url=request_targets["proxy"],
            image_urls=args.image_urls,
            output_mode=args.output_mode,
            concurrency=args.concurrency,
            total_requests=args.total_requests,
            duration_seconds=args.duration_seconds,
            timeout_seconds=args.request_timeout_seconds,
            expect_url_payload=args.output_mode == "url",
            cache_bust_urls=not args.warm_cache,
        )

        for target_name, results in (("direct", direct_results), ("proxy", proxy_results)):
            for result in results:
                request_rows.append(
                    {
                        "target": target_name,
                        "requestId": result.request_id,
                        "startedAt": result.started_at,
                        "durationMs": round(result.duration_ms, 3),
                        "statusCode": result.status_code,
                        "ok": result.ok,
                        "error": result.error,
                    }
                )

        if args.cooldown_seconds > 0:
            time.sleep(args.cooldown_seconds)

        admin_log_items = read_admin_logs(base_url, auth_header, timeout=10.0)
        with admin_log_state_lock:
            max_seen_id, gap_detected = merge_admin_log_items(
                admin_log_items_by_id,
                admin_log_items,
                baseline_log_id,
                int(admin_log_capture_state["max_seen_id"]),
                bool(admin_log_capture_state["gap_detected"]),
            )
            admin_log_capture_state["max_seen_id"] = max_seen_id
            admin_log_capture_state["gap_detected"] = gap_detected
    finally:
        stop_event.set()
        if rss_thread.is_alive():
            rss_thread.join(timeout=5.0)
        if stats_thread.is_alive():
            stats_thread.join(timeout=5.0)
        if admin_log_thread is not None and admin_log_thread.is_alive():
            admin_log_thread.join(timeout=5.0)
        mock_server.shutdown()
        mock_server.server_close()
        if container_name:
            run_command(["docker", "rm", "-f", container_name], timeout=15.0)

    with admin_log_state_lock:
        benchmark_log_items = [
            admin_log_items_by_id[item_id]
            for item_id in sorted(admin_log_items_by_id)
        ]
        admin_log_gap_detected = bool(admin_log_capture_state["gap_detected"])

    if admin_log_gap_detected:
        raise RuntimeError(
            "admin 日志滚动缓冲区已覆盖部分 benchmark 样本，阶段统计不完整"
        )

    comparison = build_comparison_summary(direct_results, proxy_results)
    proxy_request_rows = [row for row in request_rows if row["target"] == "proxy"]
    success_durations = [row["durationMs"] for row in proxy_request_rows if row["ok"]]
    peak_rss_kb = max((row["vmrssKb"] for row in rss_rows), default=0)
    final_stats = stats_rows[-1] if stats_rows else {}
    admin_log_stage_summary = build_admin_log_stage_summary(benchmark_log_items)

    summary = {
        "image": args.image,
        "containerName": container_name,
        "containerId": container_id,
        "proxyBaseUrl": base_url,
        "mockUpstreamUrl": f"http://{DEFAULT_MOCK_LOCALHOST}:{mock_port}",
        "requestImageUrls": args.image_urls,
        "concurrency": args.concurrency,
        "scenario": scenario_metadata["scenario"],
        "cacheState": scenario_metadata["cache_state"],
        "outputMode": scenario_metadata["output_mode"],
        "totalRequests": len(proxy_request_rows),
        "successRequests": sum(1 for row in proxy_request_rows if row["ok"]),
        "failedRequests": sum(1 for row in proxy_request_rows if not row["ok"]),
        "p50Ms": round(percentile_ms(success_durations, 0.50), 3),
        "p95Ms": round(percentile_ms(success_durations, 0.95), 3),
        "p99Ms": round(percentile_ms(success_durations, 0.99), 3),
        "directRequestCount": len(direct_results),
        "directSuccessRequests": sum(1 for result in direct_results if result.ok),
        "directFailedRequests": sum(1 for result in direct_results if not result.ok),
        **comparison,
        "peakRssKb": peak_rss_kb,
        "finalSpillCount": final_stats.get("spillCount", 0),
        "finalSpillBytesTotal": final_stats.get("spillBytesTotal", 0),
        "adminLogRequestCount": len(benchmark_log_items),
        "mallocConf": args.malloc_conf,
        "responseBase64Bytes": args.response_base64_bytes,
        "cooldownSeconds": args.cooldown_seconds,
        **admin_log_stage_summary,
    }

    write_csv(
        args.output_dir / "rss-samples.csv",
        rss_rows,
        fieldnames=["timestamp", "vmrssKb"],
    )
    write_csv(
        args.output_dir / "stats-samples.csv",
        stats_rows,
        fieldnames=[
            "timestamp",
            "totalRequests",
            "errorRequests",
            "cacheHits",
            "spillCount",
            "spillBytesTotal",
            "requestParseMs",
            "requestImagePrepareMs",
            "requestImageMaterializeMs",
            "requestImageFetchWorkMs",
            "requestImageStoreWorkMs",
            "requestEncodeMs",
            "upstreamBuildMs",
            "responseProcessMs",
            "uploadMs",
        ],
    )
    write_csv(
        args.output_dir / "requests.csv",
        request_rows,
        fieldnames=[
            "target",
            "requestId",
            "startedAt",
            "durationMs",
            "statusCode",
            "ok",
            "error",
        ],
    )
    write_csv(
        args.output_dir / "admin-log-stage-rows.csv",
        [extract_admin_log_stage_row(item) for item in benchmark_log_items],
        fieldnames=[
            "id",
            "statusCode",
            "durationMs",
            "requestParseMs",
            "requestImagePrepareMs",
            "requestImageMaterializeMs",
            "requestImageFetchWorkMs",
            "requestImageStoreWorkMs",
            "requestEncodeMs",
            "upstreamBuildMs",
            "responseProcessMs",
            "uploadMs",
            "errorStage",
            "errorKind",
        ],
    )
    (args.output_dir / "admin-logs.json").write_text(
        json.dumps({"items": benchmark_log_items}, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    (args.output_dir / "summary.json").write_text(
        json.dumps(summary, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(summary, indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
