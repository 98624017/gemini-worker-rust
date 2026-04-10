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


def build_request_body(image_urls: list[str]) -> dict[str, Any]:
    if len(image_urls) != 3:
        raise ValueError("exactly three image URLs are required")
    return {
        "output": "url",
        "contents": [
            {
                "parts": [
                    {"inlineData": {"data": image_url}} for image_url in image_urls
                ]
            }
        ],
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


def write_csv(path: Path, rows: list[dict[str, Any]], fieldnames: Iterable[str]) -> None:
    with path.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=list(fieldnames))
        writer.writeheader()
        writer.writerows(rows)


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
            stats_rows.append(
                {
                    "timestamp": sampled_at,
                    "totalRequests": payload.get("totalRequests", 0),
                    "errorRequests": payload.get("errorRequests", 0),
                    "cacheHits": payload.get("cacheHits", 0),
                    "spillCount": payload.get("spillCount", 0),
                    "spillBytesTotal": payload.get("spillBytesTotal", 0),
                }
            )
        stop_event.wait(sample_interval_seconds)


def send_request(
    base_url: str,
    request_body_bytes: bytes,
    timeout_seconds: float,
    request_id: int,
) -> RequestResult:
    started_at = time.time()
    request = urllib.request.Request(
        f"{base_url}/v1beta/models/bench:generateContent",
        data=request_body_bytes,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout_seconds) as response:
            status_code = response.status
            body = json.loads(response.read().decode("utf-8"))
        image_url = (
            body["candidates"][0]["content"]["parts"][0]["inlineData"]["data"]
        )
        ok = status_code == 200 and isinstance(image_url, str) and image_url.startswith("http")
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
    base_url: str,
    request_body_bytes: bytes,
    concurrency: int,
    total_requests: int | None,
    duration_seconds: float | None,
    timeout_seconds: float,
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
            worker_results.append(
                send_request(base_url, request_body_bytes, timeout_seconds, request_id)
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
    auth_header = basic_auth_header("bench", DEFAULT_ADMIN_PASSWORD)
    rss_rows: list[dict[str, Any]] = []
    stats_rows: list[dict[str, Any]] = []
    request_rows: list[dict[str, Any]] = []
    stop_event = threading.Event()

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

    try:
        run_result = run_command(docker_run_args, timeout=30.0)
        if run_result.returncode != 0:
            raise RuntimeError(run_result.stderr.strip() or run_result.stdout.strip())
        container_id = run_result.stdout.strip()

        wait_for_proxy(base_url, auth_header, timeout_seconds=60.0)

        rss_thread.start()
        stats_thread.start()

        request_body = build_request_body(args.image_urls)
        request_body_bytes = json.dumps(request_body, separators=(",", ":")).encode("utf-8")
        results = run_load(
            base_url=base_url,
            request_body_bytes=request_body_bytes,
            concurrency=args.concurrency,
            total_requests=args.total_requests,
            duration_seconds=args.duration_seconds,
            timeout_seconds=args.request_timeout_seconds,
        )

        for result in results:
            request_rows.append(
                {
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
    finally:
        stop_event.set()
        rss_thread.join(timeout=5.0)
        stats_thread.join(timeout=5.0)
        mock_server.shutdown()
        mock_server.server_close()
        if container_name:
            run_command(["docker", "rm", "-f", container_name], timeout=15.0)

    success_durations = [row["durationMs"] for row in request_rows if row["ok"]]
    peak_rss_kb = max((row["vmrssKb"] for row in rss_rows), default=0)
    final_stats = stats_rows[-1] if stats_rows else {}

    summary = {
        "image": args.image,
        "containerName": container_name,
        "containerId": container_id,
        "proxyBaseUrl": base_url,
        "mockUpstreamUrl": f"http://{DEFAULT_MOCK_LOCALHOST}:{mock_port}",
        "requestImageUrls": args.image_urls,
        "concurrency": args.concurrency,
        "totalRequests": len(request_rows),
        "successRequests": sum(1 for row in request_rows if row["ok"]),
        "failedRequests": sum(1 for row in request_rows if not row["ok"]),
        "p50Ms": round(percentile_ms(success_durations, 0.50), 3),
        "p95Ms": round(percentile_ms(success_durations, 0.95), 3),
        "p99Ms": round(percentile_ms(success_durations, 0.99), 3),
        "peakRssKb": peak_rss_kb,
        "finalSpillCount": final_stats.get("spillCount", 0),
        "finalSpillBytesTotal": final_stats.get("spillBytesTotal", 0),
        "mallocConf": args.malloc_conf,
        "responseBase64Bytes": args.response_base64_bytes,
        "cooldownSeconds": args.cooldown_seconds,
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
        ],
    )
    write_csv(
        args.output_dir / "requests.csv",
        request_rows,
        fieldnames=[
            "requestId",
            "startedAt",
            "durationMs",
            "statusCode",
            "ok",
            "error",
        ],
    )
    (args.output_dir / "summary.json").write_text(
        json.dumps(summary, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(summary, indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
