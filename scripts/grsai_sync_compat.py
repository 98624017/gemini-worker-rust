#!/usr/bin/env python3
from __future__ import annotations

import argparse
import base64
import json
import os
import signal
import socket
import subprocess
import sys
import tempfile
import time
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any


DEFAULT_GO_PORT = 18787
DEFAULT_RUST_PORT = 18788
DEFAULT_MOCK_PORT = 19080
DEFAULT_PUBLIC_BASE_URL = "https://proxy.example.com"
DEFAULT_GRSAI_BASE_URL = "http://api.grsai.com"


def expected_grsai_request_body(
    *,
    model: str,
    prompt: str,
    urls: list[str],
    aspect_ratio: str,
    image_size: str,
) -> dict[str, Any]:
    return {
        "model": model,
        "prompt": prompt,
        "urls": urls,
        "aspectRatio": aspect_ratio,
        "imageSize": image_size,
        "shutProgress": True,
    }


def summarize_gemini_success(body: dict[str, Any]) -> dict[str, Any]:
    inline_data = body["candidates"][0]["content"]["parts"][0]["inlineData"]
    return {
        "mime_type": inline_data["mimeType"],
        "data": inline_data["data"],
    }


def summarize_openai_success(body: dict[str, Any]) -> dict[str, Any]:
    return {
        "image_url": body["data"][0]["url"],
    }


def summarize_error_response(status_code: int, body: dict[str, Any]) -> dict[str, Any]:
    error = body.get("error", {})
    return {
        "status_code": status_code,
        "error_code": error.get("code"),
    }


def run_command(
    args: list[str],
    *,
    cwd: str | None = None,
    env: dict[str, str] | None = None,
    timeout: float | None = None,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        args,
        cwd=cwd,
        env=env,
        text=True,
        capture_output=True,
        check=False,
        timeout=timeout,
    )


def find_free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def wait_http_ready(url: str, timeout_seconds: float) -> None:
    deadline = time.time() + timeout_seconds
    last_error = "service not ready"
    while time.time() < deadline:
        try:
            request = urllib.request.Request(url, method="GET")
            with urllib.request.urlopen(request, timeout=3.0) as response:
                if response.status in (200, 404):
                    return
        except urllib.error.HTTPError as exc:
            if exc.code == 404:
                return
            last_error = f"HTTP Error {exc.code}: {exc.reason}"
        except Exception as exc:  # noqa: BLE001
            last_error = str(exc)
        time.sleep(0.2)
    raise RuntimeError(f"timeout waiting for {url}: {last_error}")


def request_json(
    url: str,
    *,
    body: dict[str, Any] | None = None,
    headers: dict[str, str] | None = None,
    timeout_seconds: float = 15.0,
) -> tuple[int, dict[str, Any]]:
    data = None
    if body is not None:
        data = json.dumps(body, separators=(",", ":")).encode("utf-8")
    request = urllib.request.Request(
        url,
        data=data,
        headers=headers or {},
        method="POST" if body is not None else "GET",
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout_seconds) as response:
            return response.getcode(), json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        payload = exc.read().decode("utf-8")
        parsed = json.loads(payload) if payload else {}
        return exc.code, parsed


def build_gemini_request(output_mode: str | None) -> dict[str, Any]:
    image_config: dict[str, Any] = {
        "aspectRatio": "16:9",
        "imageSize": "2K",
    }
    if output_mode is not None:
        image_config["output"] = output_mode
    return {
        "contents": [
            {
                "role": "user",
                "parts": [
                    {"text": "两张图片合并"},
                    {"inlineData": {"data": "https://img.example.com/ref-1.png"}},
                    {"inlineData": {"data": "https://img.example.com/ref-2.png"}},
                ],
            }
        ],
        "generationConfig": {"imageConfig": image_config},
    }


def build_openai_request() -> dict[str, Any]:
    return {
        "model": "",
        "prompt": "两张图片合并",
        "images": [
            "https://img.example.com/ref-1.png",
            "https://img.example.com/ref-2.png",
        ],
        "aspect_ratio": "16:9",
        "imageSize": "2K",
        "response_format": "url",
    }


def mock_server_code(port: int) -> str:
    png_base64 = base64.b64encode(b"\x89PNG\r\n\x1a\n").decode("ascii")
    return f"""
import base64
import json
from http.server import BaseHTTPRequestHandler, HTTPServer

PNG_BYTES = base64.b64decode({png_base64!r})
REQUEST_LOG = {{"items": []}}

def build_success_payload():
    return {{
        "code": 0,
        "msg": "success",
        "data": {{
            "status": "succeeded",
            "results": [{{"url": "https://api.grsai.com/img/result.png"}}],
            "start_time": 1714800000,
            "end_time": 1714800002
        }}
    }}

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/healthz":
            self.send_response(200)
            self.end_headers()
            self.wfile.write(b"ok")
            return
        if self.path == "/img/result.png":
            self.send_response(200)
            self.send_header("Content-Type", "image/png")
            self.send_header("Content-Length", str(len(PNG_BYTES)))
            self.end_headers()
            self.wfile.write(PNG_BYTES)
            return
        if self.path == "/requests":
            body = json.dumps(REQUEST_LOG).encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        self.send_response(404)
        self.end_headers()

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length)
        parsed = json.loads(body.decode("utf-8"))
        auth = self.headers.get("Authorization")
        REQUEST_LOG["items"].append({{
            "path": self.path,
            "authorization": auth,
            "content_type": self.headers.get("Content-Type"),
            "body": parsed,
        }})
        if self.path.endswith("/v1/draw/nano-banana"):
            if auth == "Bearer bad-key":
                payload = json.dumps({{"code": 401, "msg": "invalid api key", "data": {{"failure_reason": "auth"}}}}).encode("utf-8")
                self.send_response(401)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)
                return
            if auth == "Bearer limited-key":
                payload = json.dumps({{"code": 429, "msg": "rate limited", "data": {{"failure_reason": "rate_limit"}}}}).encode("utf-8")
                self.send_response(429)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(payload)))
                self.end_headers()
                self.wfile.write(payload)
                return
            payload = json.dumps(build_success_payload()).encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
            return
        self.send_response(404)
        self.end_headers()

    def log_message(self, *args):
        return

HTTPServer(("127.0.0.1", {port}), Handler).serve_forever()
"""


@dataclass
class ManagedProcess:
    process: subprocess.Popen[str]
    log_path: Path

    def stop(self) -> None:
        if self.process.poll() is not None:
            return
        self.process.terminate()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=5)


def start_process(
    args: list[str],
    *,
    cwd: str,
    env: dict[str, str],
    log_path: Path,
) -> ManagedProcess:
    with log_path.open("w", encoding="utf-8") as log_file:
        process = subprocess.Popen(  # noqa: S603
            args,
            cwd=cwd,
            env=env,
            stdout=log_file,
            stderr=subprocess.STDOUT,
            text=True,
            preexec_fn=os.setsid,
        )
    return ManagedProcess(process=process, log_path=log_path)


def assert_equal(label: str, left: Any, right: Any) -> None:
    if left != right:
        raise AssertionError(
            f"{label} mismatch\\nLEFT: {json.dumps(left, ensure_ascii=False, indent=2)}\\nRIGHT: {json.dumps(right, ensure_ascii=False, indent=2)}"
        )


def run_compat_check(go_root: Path, rust_root: Path) -> dict[str, Any]:
    temp_dir = Path(tempfile.mkdtemp(prefix="grsai-sync-compat-"))
    mock_port = find_free_port()
    go_port = find_free_port()
    rust_port = find_free_port()
    mock_log = temp_dir / "mock.log"
    go_log = temp_dir / "go.log"
    rust_log = temp_dir / "rust.log"

    env_base = os.environ.copy()
    env_base.update(
        {
            "PUBLIC_BASE_URL": DEFAULT_PUBLIC_BASE_URL,
            "ADMIN_PASSWORD": "pw",
            "HTTP_PROXY": f"http://127.0.0.1:{mock_port}",
            "http_proxy": f"http://127.0.0.1:{mock_port}",
            "NO_PROXY": "127.0.0.1,localhost",
            "no_proxy": "127.0.0.1,localhost",
        }
    )

    mock_process = start_process(
        [sys.executable, "-c", mock_server_code(mock_port)],
        cwd=str(rust_root),
        env=env_base,
        log_path=mock_log,
    )
    go_env = env_base | {
        "PORT": str(go_port),
        "BANANA_BASE_URL": DEFAULT_GRSAI_BASE_URL,
    }
    rust_env = env_base | {
        "PORT": str(rust_port),
        "UPSTREAM_BASE_URL": DEFAULT_GRSAI_BASE_URL,
        "UPSTREAM_API_KEY": "env-key",
    }
    go_process = start_process(
        ["go", "run", "."],
        cwd=str(go_root),
        env=go_env,
        log_path=go_log,
    )
    rust_process = start_process(
        [str(Path.home() / ".cargo/bin/cargo"), "run"],
        cwd=str(rust_root),
        env=rust_env,
        log_path=rust_log,
    )

    try:
        wait_http_ready(f"http://127.0.0.1:{mock_port}/healthz", 10.0)
        wait_http_ready(f"http://127.0.0.1:{go_port}/health", 20.0)
        wait_http_ready(f"http://127.0.0.1:{rust_port}/not-found", 20.0)

        gemini_headers = {
            "Content-Type": "application/json",
            "x-goog-api-key": "env-key",
        }
        openai_headers = {
            "Content-Type": "application/json",
            "Authorization": "Bearer env-key",
        }

        go_status, go_gemini_url = request_json(
            f"http://127.0.0.1:{go_port}/v1beta/models/gemini-3-pro-image-preview:generateContent?output=url",
            body=build_gemini_request("url"),
            headers=gemini_headers,
        )
        rust_status, rust_gemini_url = request_json(
            f"http://127.0.0.1:{rust_port}/v1beta/models/gemini-3-pro-image-preview:generateContent?output=url",
            body=build_gemini_request("url"),
            headers=gemini_headers,
        )
        assert_equal("gemini url status", go_status, rust_status)
        assert_equal(
            "gemini url summary",
            summarize_gemini_success(go_gemini_url),
            summarize_gemini_success(rust_gemini_url),
        )

        go_status, go_openai = request_json(
            f"http://127.0.0.1:{go_port}/v1/images/generations",
            body=build_openai_request(),
            headers=openai_headers,
        )
        rust_status, rust_openai = request_json(
            f"http://127.0.0.1:{rust_port}/v1/images/generations",
            body=build_openai_request(),
            headers=openai_headers,
        )
        assert_equal("openai status", go_status, rust_status)
        go_openai_summary = summarize_openai_success(go_openai)
        rust_openai_summary = summarize_openai_success(rust_openai)
        assert_equal("openai summary", go_openai_summary, rust_openai_summary)

        gemini_bad_headers = {
            "Content-Type": "application/json",
            "x-goog-api-key": "bad-key",
        }
        go_gemini_auth_status, go_gemini_auth_error = request_json(
            f"http://127.0.0.1:{go_port}/v1beta/models/gemini-2.5-flash-image:generateContent?output=url",
            body=build_gemini_request("url"),
            headers=gemini_bad_headers,
        )
        rust_gemini_auth_status, rust_gemini_auth_error = request_json(
            f"http://127.0.0.1:{rust_port}/v1beta/models/gemini-2.5-flash-image:generateContent?output=url",
            body=build_gemini_request("url"),
            headers=gemini_bad_headers,
        )
        assert_equal(
            "gemini auth error summary",
            summarize_error_response(go_gemini_auth_status, go_gemini_auth_error),
            summarize_error_response(rust_gemini_auth_status, rust_gemini_auth_error),
        )

        openai_limited_headers = {
            "Content-Type": "application/json",
            "Authorization": "Bearer limited-key",
        }
        go_openai_rate_limit_status, go_openai_rate_limit = request_json(
            f"http://127.0.0.1:{go_port}/v1/images/generations",
            body=build_openai_request(),
            headers=openai_limited_headers,
        )
        rust_openai_rate_limit_status, rust_openai_rate_limit = request_json(
            f"http://127.0.0.1:{rust_port}/v1/images/generations",
            body=build_openai_request(),
            headers=openai_limited_headers,
        )
        assert_equal(
            "openai rate limit summary",
            summarize_error_response(go_openai_rate_limit_status, go_openai_rate_limit),
            summarize_error_response(rust_openai_rate_limit_status, rust_openai_rate_limit),
        )

        _, request_log = request_json(f"http://127.0.0.1:{mock_port}/requests")
        items = request_log["items"]
        expected = [
            expected_grsai_request_body(
                model="nano-banana-pro",
                prompt="两张图片合并",
                urls=[
                    "https://img.example.com/ref-1.png",
                    "https://img.example.com/ref-2.png",
                ],
                aspect_ratio="16:9",
                image_size="2K",
            ),
            expected_grsai_request_body(
                model="nano-banana-fast",
                prompt="两张图片合并",
                urls=[
                    "https://img.example.com/ref-1.png",
                    "https://img.example.com/ref-2.png",
                ],
                aspect_ratio="16:9",
                image_size="2K",
            ),
            expected_grsai_request_body(
                model="nano-banana-fast",
                prompt="两张图片合并",
                urls=[
                    "https://img.example.com/ref-1.png",
                    "https://img.example.com/ref-2.png",
                ],
                aspect_ratio="16:9",
                image_size="2K",
            ),
            expected_grsai_request_body(
                model="nano-banana-fast",
                prompt="两张图片合并",
                urls=[
                    "https://img.example.com/ref-1.png",
                    "https://img.example.com/ref-2.png",
                ],
                aspect_ratio="16:9",
                image_size="2K",
            ),
        ]
        draw_items = [item for item in items if item["path"].endswith("/v1/draw/nano-banana")]
        go_bodies = [item["body"] for item in draw_items[0::2]]
        rust_bodies = [item["body"] for item in draw_items[1::2]]
        assert_equal("go request bodies", expected, go_bodies)
        assert_equal("rust request bodies", expected, rust_bodies)

        return {
            "go_port": go_port,
            "rust_port": rust_port,
            "mock_port": mock_port,
            "gemini_url": summarize_gemini_success(go_gemini_url),
            "openai_url": go_openai_summary,
            "gemini_auth_error": summarize_error_response(
                go_gemini_auth_status, go_gemini_auth_error
            ),
            "openai_rate_limit": summarize_error_response(
                go_openai_rate_limit_status, go_openai_rate_limit
            ),
            "request_count": len(items),
            "checks": {
                "gemini_url": "matched",
                "openai_url": "matched",
                "gemini_auth_error": "matched",
                "openai_rate_limit": "matched",
            },
            "temp_dir": str(temp_dir),
        }
    finally:
        for managed in [go_process, rust_process, mock_process]:
            try:
                os.killpg(os.getpgid(managed.process.pid), signal.SIGTERM)
            except ProcessLookupError:
                pass
            managed.stop()


def create_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Compare Go and Rust grsai sync behavior against the same mock upstream."
    )
    parser.add_argument(
        "--go-root",
        default=os.environ.get("GO_IMPL_ROOT", ""),
        help="Path to the Go banana-proxy root. Defaults to GO_IMPL_ROOT.",
    )
    parser.add_argument(
        "--rust-root",
        default=str(Path(__file__).resolve().parent.parent),
        help="Path to the Rust rust-sync-proxy root.",
    )
    return parser


def main() -> int:
    parser = create_parser()
    args = parser.parse_args()
    if not args.go_root:
        parser.error("--go-root or GO_IMPL_ROOT is required")

    summary = run_compat_check(Path(args.go_root), Path(args.rust_root))
    print(json.dumps(summary, indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
