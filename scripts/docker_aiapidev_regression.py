#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path
from typing import Any


DEFAULT_IMAGE = "rust-sync-proxy:local"
DEFAULT_PROXY_HOST = "127.0.0.1"
DEFAULT_PROXY_PORT = 18790
DEFAULT_STARTUP_TIMEOUT_SECONDS = 60.0
DEFAULT_REQUEST_TIMEOUT_SECONDS = 450.0
DEFAULT_AIAPIDEV_BASE_URL = "https://www.aiapidev.com"
DEFAULT_GOOD_IMAGE_URLS = [
    "https://httpbin.org/image/png",
    "https://httpbin.org/image/jpeg",
]
DEFAULT_BAD_IMAGE_URLS = [
    "https://httpbin.org/image/png",
    "https://httpbin.org/status/404",
]


def run_command(args: list[str], timeout: float | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        args,
        check=False,
        text=True,
        capture_output=True,
        timeout=timeout,
    )


def find_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind((DEFAULT_PROXY_HOST, 0))
        return int(sock.getsockname()[1])


def build_aiapidev_request_body(
    image_urls: list[str],
    output_mode: str | None,
) -> dict[str, Any]:
    if len(image_urls) < 2:
        raise ValueError("at least two image URLs are required")

    parts: list[dict[str, Any]] = [{"text": "两张图片合并"}]
    for image_url in image_urls:
        mime_type = guess_mime_type_from_url(image_url)
        parts.append(
            {
                "inlineData": {
                    "data": image_url,
                    "mimeType": mime_type,
                }
            }
        )

    image_config: dict[str, Any] = {
        "aspectRatio": "3:4",
        "imageSize": "1K",
    }
    if output_mode is not None:
        image_config["output"] = output_mode

    return {
        "contents": [
            {
                "role": "user",
                "parts": parts,
            }
        ],
        "generationConfig": {
            "imageConfig": image_config,
            "responseModalities": ["IMAGE"],
        },
    }


def guess_mime_type_from_url(raw_url: str) -> str:
    lower = raw_url.lower()
    if lower.endswith(".jpg") or lower.endswith(".jpeg"):
        return "image/jpeg"
    if lower.endswith(".webp"):
        return "image/webp"
    if lower.endswith(".gif"):
        return "image/gif"
    return "image/png"


def wait_for_proxy(base_url: str, timeout_seconds: float) -> None:
    deadline = time.time() + timeout_seconds
    last_error = "proxy did not become ready"
    while time.time() < deadline:
        try:
            request = urllib.request.Request(f"{base_url}/not-found", method="GET")
            with urllib.request.urlopen(request, timeout=5.0):
                pass
        except urllib.error.HTTPError as exc:
            if exc.code == 404:
                return
            last_error = f"http {exc.code}"
        except Exception as exc:  # noqa: BLE001
            last_error = str(exc)
        time.sleep(1.0)
    raise RuntimeError(last_error)


def send_json_request(
    url: str,
    body: dict[str, Any],
    upstream_key: str,
    timeout_seconds: float,
) -> tuple[int, dict[str, Any]]:
    request = urllib.request.Request(
        url,
        data=json.dumps(body, separators=(",", ":")).encode("utf-8"),
        headers={
            "Content-Type": "application/json",
            "x-goog-api-key": f"{DEFAULT_AIAPIDEV_BASE_URL}|{upstream_key}",
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout_seconds) as response:
            return response.getcode(), json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        payload = exc.read().decode("utf-8")
        try:
            parsed = json.loads(payload)
        except json.JSONDecodeError:
            parsed = {"raw": payload}
        return exc.code, parsed


def validate_url_mode_response(body: dict[str, Any]) -> dict[str, Any]:
    candidate = body["candidates"][0]
    inline_data = candidate["content"]["parts"][0]["inlineData"]
    usage = body["usageMetadata"]
    assert candidate["content"]["role"] == "model"
    assert candidate["finishReason"] == "STOP"
    assert inline_data["data"].startswith("https://")
    assert usage["promptTokenCount"] == 1024
    assert usage["candidatesTokenCount"] == 1024
    assert usage["totalTokenCount"] == 2048
    return {
        "image_url": inline_data["data"],
        "usage_total": usage["totalTokenCount"],
    }


def validate_base64_mode_response(body: dict[str, Any]) -> dict[str, Any]:
    candidate = body["candidates"][0]
    inline_data = candidate["content"]["parts"][0]["inlineData"]
    usage = body["usageMetadata"]
    assert candidate["content"]["role"] == "model"
    assert candidate["finishReason"] == "STOP"
    assert inline_data["mimeType"].startswith("image/")
    assert len(inline_data["data"]) > 0
    assert usage["promptTokenCount"] == 1024
    assert usage["candidatesTokenCount"] == 1024
    assert usage["totalTokenCount"] == 2048
    return {
        "mime_type": inline_data["mimeType"],
        "data_len": len(inline_data["data"]),
        "usage_total": usage["totalTokenCount"],
    }


def validate_failure_response(status_code: int, body: dict[str, Any]) -> dict[str, Any]:
    assert status_code >= 400
    message = body.get("error", {}).get("message", "")
    assert message
    return {
        "status_code": status_code,
        "message": message,
    }


def run_standard_docker_smoke(image: str, output_dir: Path) -> dict[str, Any]:
    cmd = [
        sys.executable,
        str(Path(__file__).with_name("benchmark_docker_mock_upstream.py")),
        "--image",
        image,
        "--image-url",
        "https://httpbin.org/image/png",
        "--image-url",
        "https://httpbin.org/image/jpeg",
        "--image-url",
        "https://httpbin.org/image/webp",
        "--total-requests",
        "2",
        "--concurrency",
        "1",
        "--cooldown-seconds",
        "0",
        "--response-base64-bytes",
        "400000",
        "--output-dir",
        str(output_dir),
    ]
    result = run_command(cmd, timeout=600.0)
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or result.stdout.strip())
    return json.loads(result.stdout)


def start_container(image: str, port: int) -> tuple[str, str]:
    container_name = f"rust-sync-proxy-aiapidev-{uuid.uuid4().hex[:8]}"
    cmd = [
        "docker",
        "run",
        "-d",
        "--rm",
        "--name",
        container_name,
        "-p",
        f"{port}:8787",
        image,
    ]
    result = run_command(cmd, timeout=30.0)
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or result.stdout.strip())
    return container_name, result.stdout.strip()


def stop_container(container_name: str) -> None:
    if not container_name:
        return
    run_command(["docker", "rm", "-f", container_name], timeout=15.0)


def create_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Docker regression runner for rust-sync-proxy + aiapidev flows."
    )
    parser.add_argument("--image", default=DEFAULT_IMAGE)
    parser.add_argument(
        "--proxy-port",
        type=int,
        default=0,
        help="Host port mapped to container 8787. Default: auto-allocate.",
    )
    parser.add_argument(
        "--startup-timeout-seconds",
        type=float,
        default=DEFAULT_STARTUP_TIMEOUT_SECONDS,
    )
    parser.add_argument(
        "--request-timeout-seconds",
        type=float,
        default=DEFAULT_REQUEST_TIMEOUT_SECONDS,
    )
    parser.add_argument(
        "--real-key",
        default=os.environ.get("AIAPIDEV_TEST_KEY", ""),
        help="Real aiapidev key. Defaults to AIAPIDEV_TEST_KEY env.",
    )
    parser.add_argument(
        "--skip-standard-smoke",
        action="store_true",
        help="Skip the existing standard docker smoke wrapper.",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=Path("/tmp") / f"rust-sync-proxy-aiapidev-{int(time.time())}",
    )
    return parser


def main() -> int:
    parser = create_parser()
    args = parser.parse_args()
    if not args.real_key:
        parser.error("--real-key or AIAPIDEV_TEST_KEY is required")

    args.output_dir.mkdir(parents=True, exist_ok=True)

    summary: dict[str, Any] = {
        "image": args.image,
        "standardSmoke": None,
        "aiapidev": {},
    }

    if not args.skip_standard_smoke:
        summary["standardSmoke"] = run_standard_docker_smoke(
            args.image,
            args.output_dir / "standard-smoke",
        )

    port = args.proxy_port or find_free_port()
    base_url = f"http://{DEFAULT_PROXY_HOST}:{port}"
    container_name = ""

    try:
        container_name, container_id = start_container(args.image, port)
        summary["containerName"] = container_name
        summary["containerId"] = container_id
        summary["proxyBaseUrl"] = base_url
        wait_for_proxy(base_url, args.startup_timeout_seconds)

        status_code, url_body = send_json_request(
            f"{base_url}/v1beta/models/gemini-3-pro-image-preview:generateContent",
            build_aiapidev_request_body(DEFAULT_GOOD_IMAGE_URLS, "url"),
            args.real_key,
            args.request_timeout_seconds,
        )
        if status_code != 200:
            raise RuntimeError(f"url mode failed: status={status_code} body={url_body}")
        summary["aiapidev"]["outputUrl"] = validate_url_mode_response(url_body)

        status_code, base64_body = send_json_request(
            f"{base_url}/v1beta/models/gemini-3-pro-image-preview:generateContent",
            build_aiapidev_request_body(DEFAULT_GOOD_IMAGE_URLS, None),
            args.real_key,
            args.request_timeout_seconds,
        )
        if status_code != 200:
            raise RuntimeError(f"base64 mode failed: status={status_code} body={base64_body}")
        summary["aiapidev"]["base64"] = validate_base64_mode_response(base64_body)

        status_code, failure_body = send_json_request(
            f"{base_url}/v1beta/models/gemini-3-pro-image-preview:generateContent",
            build_aiapidev_request_body(DEFAULT_BAD_IMAGE_URLS, "url"),
            args.real_key,
            args.request_timeout_seconds,
        )
        summary["aiapidev"]["badSourceFailure"] = validate_failure_response(
            status_code,
            failure_body,
        )

        (args.output_dir / "summary.json").write_text(
            json.dumps(summary, indent=2, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )
        print(json.dumps(summary, indent=2, ensure_ascii=False))
        return 0
    finally:
        stop_container(container_name)


if __name__ == "__main__":
    sys.exit(main())
