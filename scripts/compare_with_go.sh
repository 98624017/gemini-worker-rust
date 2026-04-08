#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GO_IMPL_ROOT="${GO_IMPL_ROOT:-}"
TMP_DIR="$(mktemp -d)"
MOCK_PORT="${MOCK_PORT:-19080}"
GO_PORT="${GO_PORT:-18787}"
RUST_PORT="${RUST_PORT:-18788}"

cleanup() {
  set +e
  [[ -n "${GO_PID:-}" ]] && kill "$GO_PID" >/dev/null 2>&1
  [[ -n "${RUST_PID:-}" ]] && kill "$RUST_PID" >/dev/null 2>&1
  [[ -n "${MOCK_PID:-}" ]] && kill "$MOCK_PID" >/dev/null 2>&1
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 1
  }
}

wait_http() {
  local url="$1"
  for _ in $(seq 1 80); do
    if curl -sS -o /dev/null "$url" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.25
  done
  echo "timeout waiting for $url" >&2
  return 1
}

require_cmd curl
require_cmd jq
require_cmd python3
require_cmd go

if [[ -z "$GO_IMPL_ROOT" ]]; then
  echo "GO_IMPL_ROOT is required, for example:" >&2
  echo "  GO_IMPL_ROOT=/path/to/go-implementation bash ./scripts/compare_with_go.sh" >&2
  exit 1
fi

if [[ ! -f "$GO_IMPL_ROOT/main.go" ]]; then
  echo "GO_IMPL_ROOT does not look like the Go proxy root: $GO_IMPL_ROOT" >&2
  exit 1
fi

python3 - "$MOCK_PORT" >"$TMP_DIR/mock.log" 2>&1 <<'PY' &
import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

port = int(sys.argv[1])

generate_payload = {
    "thoughtSignature": "secret",
    "candidates": [{
        "finishReason": "STOP",
        "content": {
            "parts": [
                {"inlineData": {"mimeType": "image/png", "data": "aaaa"}},
                {"text": "kept"},
                {"inlineData": {"mimeType": "image/png", "data": "aaaaaaaa"}}
            ]
        }
    }]
}

stream_payload = (
    "event: message\n"
    "data: {\"thoughtSignature\":\"secret\",\"candidates\":[{\"content\":{\"parts\":[{\"inlineData\":{\"mimeType\":\"image/png\",\"data\":\"aaaa\"}},{\"inlineData\":{\"mimeType\":\"image/png\",\"data\":\"bbbbbbbb\"}}]}}]}\n"
    "\n"
    "data: [DONE]\n"
)

markdown_payload = {
    "candidates": [{
        "finishReason": "STOP",
        "content": {
            "parts": [
                {"text": "before"},
                {"text": "![img](https://example.com/path/demo.png)"},
                {"text": "after"}
            ]
        }
    }]
}

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/healthz":
            self.send_response(200)
            self.end_headers()
            self.wfile.write(b"ok")
            return
        self.send_response(404)
        self.end_headers()

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        _ = self.rfile.read(length)

        if self.path.startswith("/v1beta/models/demo:generateContent"):
            payload = json.dumps(generate_payload).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
            return

        if self.path.startswith("/v1beta/models/markdown:generateContent"):
            payload = json.dumps(markdown_payload).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
            return

        if self.path.startswith("/v1beta/models/demo:streamGenerateContent"):
            payload = stream_payload.encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
            return

        self.send_response(404)
        self.end_headers()

    def do_PUT(self):
        length = int(self.headers.get("Content-Length", "0"))
        _ = self.rfile.read(length)

        if self.path.startswith("/bucket/images/"):
            self.send_response(200)
            self.end_headers()
            return

        self.send_response(404)
        self.end_headers()

    def log_message(self, fmt, *args):
        return

HTTPServer(("127.0.0.1", port), Handler).serve_forever()
PY
MOCK_PID=$!

wait_http "http://127.0.0.1:${MOCK_PORT}/healthz"

(
  cd "$GO_IMPL_ROOT"
  PORT="$GO_PORT" \
  UPSTREAM_BASE_URL="http://127.0.0.1:${MOCK_PORT}" \
  UPSTREAM_API_KEY="env-key" \
  IMAGE_HOST_MODE="r2" \
  R2_ENDPOINT="http://127.0.0.1:${MOCK_PORT}" \
  R2_BUCKET="bucket" \
  R2_ACCESS_KEY_ID="test-key" \
  R2_SECRET_ACCESS_KEY="test-secret" \
  R2_PUBLIC_BASE_URL="https://img.example.com" \
  R2_OBJECT_PREFIX="images" \
  ADMIN_PASSWORD="pw" \
  PUBLIC_BASE_URL="https://proxy.example.com" \
  go run .
) >"$TMP_DIR/go.log" 2>&1 &
GO_PID=$!

(
  cd "$REPO_ROOT"
  PORT="$RUST_PORT" \
  UPSTREAM_BASE_URL="http://127.0.0.1:${MOCK_PORT}" \
  UPSTREAM_API_KEY="env-key" \
  IMAGE_HOST_MODE="r2" \
  R2_ENDPOINT="http://127.0.0.1:${MOCK_PORT}" \
  R2_BUCKET="bucket" \
  R2_ACCESS_KEY_ID="test-key" \
  R2_SECRET_ACCESS_KEY="test-secret" \
  R2_PUBLIC_BASE_URL="https://img.example.com" \
  R2_OBJECT_PREFIX="images" \
  ADMIN_PASSWORD="pw" \
  PUBLIC_BASE_URL="https://proxy.example.com" \
  "$HOME/.cargo/bin/cargo" run --manifest-path "$REPO_ROOT/Cargo.toml"
) >"$TMP_DIR/rust.log" 2>&1 &
RUST_PID=$!

wait_http "http://127.0.0.1:${GO_PORT}/does-not-exist"
wait_http "http://127.0.0.1:${RUST_PORT}/does-not-exist"

NON_STREAM_REQ='{"contents":[{"parts":[{"text":"hello"}]}]}'

curl -fsS \
  -H 'Content-Type: application/json' \
  -d "$NON_STREAM_REQ" \
  "http://127.0.0.1:${GO_PORT}/v1beta/models/demo:generateContent" \
  | jq -S . >"$TMP_DIR/go-generate.json"

curl -fsS \
  -H 'Content-Type: application/json' \
  -d "$NON_STREAM_REQ" \
  "http://127.0.0.1:${RUST_PORT}/v1beta/models/demo:generateContent" \
  | jq -S . >"$TMP_DIR/rust-generate.json"

diff -u "$TMP_DIR/go-generate.json" "$TMP_DIR/rust-generate.json"

URL_REQ='{"output":"url","contents":[{"parts":[{"text":"hello"}]}]}'

normalize_output_url_json() {
  jq -S '
    walk(
      if type == "object"
         and has("inlineData")
         and (.inlineData | type == "object")
         and (.inlineData.data | type == "string")
         and (.inlineData.data | startswith("https://img.example.com/images/"))
      then .inlineData.data = "https://img.example.com/__R2_OBJECT__"
      else .
      end
    )
  '
}

normalize_output_url_stream() {
  sed -E 's#https://img\.example\.com/images/[^"[:space:]]+#https://img.example.com/__R2_OBJECT__#g'
}

curl -fsS \
  -H 'Content-Type: application/json' \
  -d "$URL_REQ" \
  "http://127.0.0.1:${GO_PORT}/v1beta/models/demo:generateContent" \
  | normalize_output_url_json >"$TMP_DIR/go-generate-url.json"

curl -fsS \
  -H 'Content-Type: application/json' \
  -d "$URL_REQ" \
  "http://127.0.0.1:${RUST_PORT}/v1beta/models/demo:generateContent" \
  | normalize_output_url_json >"$TMP_DIR/rust-generate-url.json"

diff -u "$TMP_DIR/go-generate-url.json" "$TMP_DIR/rust-generate-url.json"

curl -fsS \
  -H 'Content-Type: application/json' \
  -d "$NON_STREAM_REQ" \
  "http://127.0.0.1:${GO_PORT}/v1beta/models/demo:streamGenerateContent" \
  >"$TMP_DIR/go-stream.txt"

curl -fsS \
  -H 'Content-Type: application/json' \
  -d "$NON_STREAM_REQ" \
  "http://127.0.0.1:${RUST_PORT}/v1beta/models/demo:streamGenerateContent" \
  >"$TMP_DIR/rust-stream.txt"

diff -u "$TMP_DIR/go-stream.txt" "$TMP_DIR/rust-stream.txt"

curl -fsS \
  -H 'Content-Type: application/json' \
  -d "$URL_REQ" \
  "http://127.0.0.1:${GO_PORT}/v1beta/models/demo:streamGenerateContent" \
  | normalize_output_url_stream >"$TMP_DIR/go-stream-url.txt"

curl -fsS \
  -H 'Content-Type: application/json' \
  -d "$URL_REQ" \
  "http://127.0.0.1:${RUST_PORT}/v1beta/models/demo:streamGenerateContent" \
  | normalize_output_url_stream >"$TMP_DIR/rust-stream-url.txt"

diff -u "$TMP_DIR/go-stream-url.txt" "$TMP_DIR/rust-stream-url.txt"

MARKDOWN_REQ='{"output":"url","contents":[{"parts":[{"text":"hello"}]}]}'

curl -fsS \
  -H 'Content-Type: application/json' \
  -d "$MARKDOWN_REQ" \
  "http://127.0.0.1:${GO_PORT}/v1beta/models/markdown:generateContent" \
  | jq -S . >"$TMP_DIR/go-markdown.json"

curl -fsS \
  -H 'Content-Type: application/json' \
  -d "$MARKDOWN_REQ" \
  "http://127.0.0.1:${RUST_PORT}/v1beta/models/markdown:generateContent" \
  | jq -S . >"$TMP_DIR/rust-markdown.json"

diff -u "$TMP_DIR/go-markdown.json" "$TMP_DIR/rust-markdown.json"

AUTH_HEADER="Authorization: Basic $(printf 'user:pw' | base64)"
curl -fsS -H "$AUTH_HEADER" "http://127.0.0.1:${GO_PORT}/admin/api/stats" \
  | jq -S '.totalDurationMs = 0' >"$TMP_DIR/go-admin-stats.json"
curl -fsS -H "$AUTH_HEADER" "http://127.0.0.1:${RUST_PORT}/admin/api/stats" \
  | jq -S '.totalDurationMs = 0' >"$TMP_DIR/rust-admin-stats.json"

diff -u "$TMP_DIR/go-admin-stats.json" "$TMP_DIR/rust-admin-stats.json"

echo "Go/Rust compatibility check passed."
