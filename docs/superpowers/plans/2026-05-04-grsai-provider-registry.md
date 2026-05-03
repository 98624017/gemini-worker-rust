# grsai Provider Registry Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a unified provider registry and port the Go `grsai` synchronous single-request channel into the Rust sync proxy for both Gemini and OpenAI image routes.

**Architecture:** Add focused provider-selection logic in `src/provider.rs`, pure `grsai` protocol logic in `src/grsai.rs`, then route Gemini and OpenAI requests through provider resolution before choosing `grsai`, `aiapidev`, or transparent forwarding. Keep downstream error JSON compatible with the current Rust proxy and preserve unknown-upstream transparent behavior.

**Tech Stack:** Rust 2024, axum 0.8, reqwest 0.12, serde_json, tokio, tower integration tests.

---

## File Structure

- Create `src/provider.rs`
  - Owns `ProviderKind`, provider host matching, and `resolve_provider(base_url)`.
  - Does not know request or response JSON formats.
- Create `src/grsai.rs`
  - Owns `grsai` model mapping, Gemini/OpenAI parameter extraction, upstream request body construction, JSON/SSE response parsing, and downstream payload helpers.
  - Contains only protocol logic and small payload builders; route-level network calls stay in `src/http/router.rs`.
- Modify `src/lib.rs`
  - Exports the new `provider` and `grsai` modules for integration tests.
- Modify `src/http/router.rs`
  - Replaces direct `is_aiapidev_base_url` branching with `provider::resolve_provider`.
  - Adds `forward_grsai_gemini_request` and `forward_grsai_openai_image_request`.
  - Keeps existing `aiapidev` and transparent branches intact.
- Create `tests/provider_test.rs`
  - Tests provider matching and fallback behavior.
- Create `tests/grsai_test.rs`
  - Tests pure `grsai` request construction and response parsing.
- Modify `src/http/router.rs` test module
  - Tests `grsai` Gemini/OpenAI flows with a proxied `reqwest::Client`, so provider
    matching still sees `api.grsai.com` while the HTTP request is intercepted locally.
- Modify `README.md`
  - Documents `grsai` support and the provider selection behavior.

## Task 1: Provider Registry

**Files:**
- Create: `src/provider.rs`
- Modify: `src/lib.rs`
- Test: `tests/provider_test.rs`

- [ ] **Step 1: Write provider matching tests**

Create `tests/provider_test.rs`:

```rust
use rust_sync_proxy::provider::{ProviderKind, resolve_provider};

#[test]
fn grsai_provider_matches_root_and_subdomains() {
    assert_eq!(resolve_provider("https://api.grsai.com").kind, ProviderKind::Grsai);
    assert_eq!(resolve_provider("https://grsai.com").kind, ProviderKind::Grsai);
    assert_eq!(
        resolve_provider("https://sub.api.grsai.com").kind,
        ProviderKind::Grsai
    );
}

#[test]
fn grsai_provider_rejects_suffix_tricks() {
    assert_eq!(
        resolve_provider("https://evilgrsai.com").kind,
        ProviderKind::Transparent
    );
    assert_eq!(
        resolve_provider("https://grsai.com.evil.com").kind,
        ProviderKind::Transparent
    );
}

#[test]
fn aiapidev_provider_matches_existing_hosts() {
    assert_eq!(
        resolve_provider("https://aiapidev.com").kind,
        ProviderKind::Aiapidev
    );
    assert_eq!(
        resolve_provider("https://www.aiapidev.com").kind,
        ProviderKind::Aiapidev
    );
}

#[test]
fn unknown_or_invalid_base_url_uses_transparent_provider() {
    assert_eq!(
        resolve_provider("https://magic666.top").kind,
        ProviderKind::Transparent
    );
    assert_eq!(resolve_provider("not a url").kind, ProviderKind::Transparent);
}
```

- [ ] **Step 2: Run provider tests and verify they fail**

Run:

```bash
timeout 60s cargo test --test provider_test
```

Expected: compile failure because `rust_sync_proxy::provider` does not exist.

- [ ] **Step 3: Implement provider registry**

Create `src/provider.rs`:

```rust
use url::Url;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderKind {
    Grsai,
    Aiapidev,
    Transparent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Provider {
    pub kind: ProviderKind,
    pub name: &'static str,
}

pub fn resolve_provider(base_url: &str) -> Provider {
    if is_grsai_base_url(base_url) {
        return Provider {
            kind: ProviderKind::Grsai,
            name: "grsai",
        };
    }
    if is_aiapidev_base_url(base_url) {
        return Provider {
            kind: ProviderKind::Aiapidev,
            name: "aiapidev",
        };
    }
    Provider {
        kind: ProviderKind::Transparent,
        name: "transparent",
    }
}

pub fn is_grsai_base_url(raw: &str) -> bool {
    let Some(host) = parse_host(raw) else {
        return false;
    };
    host == "grsai.com" || host.ends_with(".grsai.com")
}

pub fn is_aiapidev_base_url(raw: &str) -> bool {
    let Some(host) = parse_host(raw) else {
        return false;
    };
    matches!(host.as_str(), "aiapidev.com" | "www.aiapidev.com")
}

fn parse_host(raw: &str) -> Option<String> {
    Url::parse(raw)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_ascii_lowercase))
}
```

Modify `src/lib.rs`:

```rust
pub mod provider;
```

Place it with the other `pub mod` declarations.

- [ ] **Step 4: Run provider tests and verify they pass**

Run:

```bash
timeout 60s cargo test --test provider_test
```

Expected: all tests in `provider_test` pass.

- [ ] **Step 5: Commit**

```bash
git add src/provider.rs src/lib.rs tests/provider_test.rs
git commit -m "feat: add provider registry"
```

## Task 2: grsai Pure Protocol Logic

**Files:**
- Create: `src/grsai.rs`
- Modify: `src/lib.rs`
- Test: `tests/grsai_test.rs`

- [ ] **Step 1: Write grsai unit tests**

Create `tests/grsai_test.rs`:

```rust
use axum::http::StatusCode;
use serde_json::{Value, json};

use rust_sync_proxy::grsai::{
    GrsaiSource, build_grsai_request_body, extract_gemini_params, extract_openai_params,
    normalize_model, parse_grsai_response,
};

#[test]
fn gemini_model_mapping_matches_go_provider() {
    assert_eq!(
        normalize_model("gemini-3-pro-image-preview", GrsaiSource::Gemini),
        "nano-banana-pro"
    );
    assert_eq!(
        normalize_model("gemini-2.5-flash-image", GrsaiSource::Gemini),
        "nano-banana-fast"
    );
    assert_eq!(
        normalize_model("gemini-3.1-flash-image-preview", GrsaiSource::Gemini),
        "nano-banana-2"
    );
    assert_eq!(normalize_model("", GrsaiSource::Gemini), "nano-banana-fast");
    assert_eq!(
        normalize_model("custom-model", GrsaiSource::Gemini),
        "custom-model"
    );
}

#[test]
fn openai_model_mapping_defaults_only_when_empty() {
    assert_eq!(normalize_model("", GrsaiSource::OpenAi), "nano-banana-fast");
    assert_eq!(
        normalize_model("gpt-image-2", GrsaiSource::OpenAi),
        "gpt-image-2"
    );
}

#[test]
fn gemini_params_extract_user_prompt_urls_and_image_config() {
    let body = json!({
        "contents": [
            {
                "role": "model",
                "parts": [{"text": "ignored"}]
            },
            {
                "role": "user",
                "parts": [
                    {"text": "first line"},
                    {"inlineData": {"mimeType": "image/png", "data": "https://img.example.com/a.png"}},
                    {"text": "second line"}
                ]
            }
        ],
        "generationConfig": {
            "imageConfig": {
                "aspectRatio": "16:9",
                "imageSize": "2K",
                "output": "url"
            }
        }
    });

    let params = extract_gemini_params(
        &body,
        "gemini-3-pro-image-preview",
        Some("aspectRatio=1:1&image_size=4K&output=url"),
    )
    .unwrap();

    assert_eq!(params.model, "nano-banana-pro");
    assert_eq!(params.prompt, "first line\nsecond line");
    assert_eq!(params.urls, vec!["https://img.example.com/a.png"]);
    assert_eq!(params.aspect_ratio, "1:1");
    assert_eq!(params.image_size, "4K");
    assert_eq!(params.output, "url");
}

#[test]
fn openai_params_accept_aliases_and_defaults() {
    let body = json!({
        "model_name": "",
        "prompt": "draw a pear",
        "images": ["https://img.example.com/ref.png"]
    });

    let params = extract_openai_params(&body).unwrap();

    assert_eq!(params.model, "nano-banana-fast");
    assert_eq!(params.prompt, "draw a pear");
    assert_eq!(params.urls, vec!["https://img.example.com/ref.png"]);
    assert_eq!(params.aspect_ratio, "auto");
    assert_eq!(params.image_size, "1K");
}

#[test]
fn request_body_matches_go_grsai_provider() {
    let params = extract_openai_params(&json!({
        "model": "nano-banana-fast",
        "prompt": "draw",
        "urls": ["https://img.example.com/ref.png"],
        "aspect_ratio": "auto",
        "image_size": "1K"
    }))
    .unwrap();

    let request_body = build_grsai_request_body(&params);

    assert_eq!(request_body["model"], "nano-banana-fast");
    assert_eq!(request_body["prompt"], "draw");
    assert_eq!(request_body["urls"], json!(["https://img.example.com/ref.png"]));
    assert_eq!(request_body["aspectRatio"], "auto");
    assert_eq!(request_body["imageSize"], "1K");
    assert_eq!(request_body["shutProgress"], true);
}

#[test]
fn parses_json_success_response() {
    let body = br#"{"code":0,"msg":"success","data":{"status":"succeeded","results":[{"url":"https://api.grsai.com/img/123.png"}],"start_time":10,"end_time":12}}"#;

    let parsed = parse_grsai_response(StatusCode::OK, body).unwrap();

    assert_eq!(parsed.status, "succeeded");
    assert_eq!(parsed.image_urls, vec!["https://api.grsai.com/img/123.png"]);
    assert_eq!(parsed.start_time, Some(10));
    assert_eq!(parsed.end_time, Some(12));
}

#[test]
fn parses_sse_success_response_from_last_data_line() {
    let body = b"data: {\"code\":0,\"msg\":\"progress\",\"data\":{\"status\":\"running\"}}\n\ndata: {\"code\":0,\"msg\":\"success\",\"data\":{\"status\":\"succeeded\",\"results\":[{\"url\":\"https://api.grsai.com/img/456.png\"}]}}\n\ndata: [DONE]\n";

    let parsed = parse_grsai_response(StatusCode::OK, body).unwrap();

    assert_eq!(parsed.status, "succeeded");
    assert_eq!(parsed.image_urls, vec!["https://api.grsai.com/img/456.png"]);
}

#[test]
fn parses_business_error_as_grsai_error() {
    let body = br#"{"code":401,"msg":"invalid api key","data":{"failure_reason":"auth"}}"#;

    let err = parse_grsai_response(StatusCode::OK, body).unwrap_err();

    assert_eq!(err.http_status, StatusCode::UNAUTHORIZED);
    assert_eq!(err.message, "invalid api key");
    assert_eq!(err.upstream_code, Some(401));
    assert_eq!(err.failure_reason.as_deref(), Some("auth"));
}

#[test]
fn parse_error_when_no_data_lines_exist() {
    let err = parse_grsai_response(StatusCode::OK, b"hello").unwrap_err();

    assert_eq!(err.http_status, StatusCode::BAD_GATEWAY);
    assert!(err.message.contains("解析上游服务响应失败"));
}
```

- [ ] **Step 2: Run grsai tests and verify they fail**

Run:

```bash
timeout 60s cargo test --test grsai_test
```

Expected: compile failure because `rust_sync_proxy::grsai` does not exist.

- [ ] **Step 3: Implement `src/grsai.rs`**

Create `src/grsai.rs` with these public types and functions:

```rust
use axum::http::StatusCode;
use serde_json::{Value, json};
use url::form_urlencoded;

pub const IMAGE_GENERATION_PATH: &str = "/v1/draw/nano-banana";
pub const DEFAULT_MODEL: &str = "nano-banana-fast";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrsaiSource {
    Gemini,
    OpenAi,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrsaiImageParams {
    pub model: String,
    pub prompt: String,
    pub urls: Vec<String>,
    pub aspect_ratio: String,
    pub image_size: String,
    pub output: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GrsaiResult {
    pub status: String,
    pub image_urls: Vec<String>,
    pub failure_reason: String,
    pub error_detail: String,
    pub start_time: Option<i64>,
    pub end_time: Option<i64>,
    pub raw_data: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GrsaiError {
    pub http_status: StatusCode,
    pub message: String,
    pub upstream_code: Option<i64>,
    pub failure_reason: Option<String>,
    pub body_text: String,
    pub raw_json: Option<Value>,
}

pub fn normalize_model(raw_model: &str, source: GrsaiSource) -> String {
    let model = raw_model.trim();
    if source == GrsaiSource::Gemini {
        match model.to_ascii_lowercase().as_str() {
            "gemini-3-pro-image-preview" => "nano-banana-pro".to_string(),
            "gemini-2.5-flash-image" => DEFAULT_MODEL.to_string(),
            "gemini-3.1-flash-image-preview" => "nano-banana-2".to_string(),
            "" => DEFAULT_MODEL.to_string(),
            _ => model.to_string(),
        }
    } else if model.is_empty() {
        DEFAULT_MODEL.to_string()
    } else {
        model.to_string()
    }
}

pub fn extract_gemini_params(
    body: &Value,
    raw_model: &str,
    query: Option<&str>,
) -> Result<GrsaiImageParams, GrsaiError> {
    let content = select_gemini_content(body);
    let parts = content
        .and_then(|content| content.get("parts"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut texts = Vec::new();
    let mut urls = Vec::new();
    for part in parts {
        if let Some(text) = part.get("text").and_then(Value::as_str).map(str::trim) {
            if !text.is_empty() {
                texts.push(text.to_string());
            }
        }
        if let Some(data) = part
            .get("inlineData")
            .or_else(|| part.get("inline_data"))
            .and_then(Value::as_object)
            .and_then(|inline_data| inline_data.get("data"))
            .and_then(Value::as_str)
            .map(str::trim)
        {
            if !data.is_empty() {
                urls.push(data.to_string());
            }
        }
    }

    let image_config = body
        .pointer("/generationConfig/imageConfig")
        .or_else(|| body.pointer("/generation_config/image_config"));
    let aspect_ratio = first_query_value(query, &["aspectRatio", "aspect_ratio"])
        .or_else(|| first_json_string(image_config, &["aspectRatio", "aspect_ratio"]))
        .unwrap_or_else(|| "auto".to_string());
    let image_size = first_query_value(query, &["imageSize", "image_size"])
        .or_else(|| first_json_string(image_config, &["imageSize", "image_size"]))
        .unwrap_or_else(|| "1K".to_string());
    let output = first_query_value(query, &["output"])
        .or_else(|| first_json_string(image_config, &["output"]))
        .unwrap_or_default();

    Ok(GrsaiImageParams {
        model: normalize_model(raw_model, GrsaiSource::Gemini),
        prompt: texts.join("\n"),
        urls,
        aspect_ratio,
        image_size,
        output,
    })
}

pub fn extract_openai_params(body: &Value) -> Result<GrsaiImageParams, GrsaiError> {
    let model = first_json_string(Some(body), &["model", "model_name"]).unwrap_or_default();
    let prompt = first_json_string(Some(body), &["prompt"]).unwrap_or_default();
    let urls = first_json_string_array(body, &["urls", "images"]);
    let aspect_ratio = first_json_string(Some(body), &["aspect_ratio", "aspectRatio"])
        .unwrap_or_else(|| "auto".to_string());
    let image_size = first_json_string(Some(body), &["image_size", "imageSize"])
        .unwrap_or_else(|| "1K".to_string());

    Ok(GrsaiImageParams {
        model: normalize_model(&model, GrsaiSource::OpenAi),
        prompt,
        urls,
        aspect_ratio,
        image_size,
        output: "url".to_string(),
    })
}

pub fn build_grsai_request_body(params: &GrsaiImageParams) -> Value {
    json!({
        "model": params.model,
        "prompt": params.prompt,
        "urls": params.urls,
        "aspectRatio": params.aspect_ratio,
        "imageSize": params.image_size,
        "shutProgress": true,
    })
}

pub fn parse_grsai_response(status: StatusCode, body: &[u8]) -> Result<GrsaiResult, GrsaiError> {
    let body_text = String::from_utf8_lossy(body).to_string();
    let parsed = parse_body_text(&body_text).map_err(|message| parse_error(message, &body_text))?;
    let body_map = parsed.as_object().ok_or_else(|| GrsaiError {
        http_status: StatusCode::BAD_GATEWAY,
        message: "解析上游服务响应失败，请稍后再试".to_string(),
        upstream_code: None,
        failure_reason: None,
        body_text: body_text.clone(),
        raw_json: Some(parsed.clone()),
    })?;

    let code = body_map.get("code").and_then(Value::as_i64);
    let message = body_map
        .get("msg")
        .or_else(|| body_map.get("message"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();

    if !status.is_success() {
        return Err(GrsaiError {
            http_status: if status.as_u16() >= 400 {
                status
            } else {
                guess_http_status(code, &message)
            },
            message: non_empty_or(message, format!("上游服务返回 HTTP {}", status.as_u16())),
            upstream_code: code,
            failure_reason: extract_failure_reason(body_map),
            body_text,
            raw_json: Some(parsed),
        });
    }

    if code.is_some_and(|value| value != 0) {
        return Err(GrsaiError {
            http_status: guess_http_status(code, &message),
            message: non_empty_or(message, "上游服务处理失败，请稍后再试".to_string()),
            upstream_code: code,
            failure_reason: extract_failure_reason(body_map),
            body_text,
            raw_json: Some(parsed),
        });
    }

    let data = if code.is_some() {
        body_map.get("data").unwrap_or(&parsed)
    } else {
        &parsed
    };
    let data_map = data.as_object().ok_or_else(|| parse_error("解析上游服务响应失败，请稍后再试".to_string(), &body_text))?;
    let status_value = data_map
        .get("status")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| parse_error("解析上游服务响应失败，请稍后再试".to_string(), &body_text))?;
    let image_urls = data_map
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("url").and_then(Value::as_str).map(str::trim))
        .filter(|url| !url.is_empty())
        .map(ToOwned::to_owned)
        .collect();

    Ok(GrsaiResult {
        status: status_value.to_string(),
        image_urls,
        failure_reason: data_map
            .get("failure_reason")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        error_detail: data_map
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        start_time: data_map.get("start_time").and_then(Value::as_i64),
        end_time: data_map.get("end_time").and_then(Value::as_i64),
        raw_data: data.clone(),
    })
}

fn select_gemini_content(body: &Value) -> Option<&Value> {
    let contents = body.get("contents").and_then(Value::as_array)?;
    contents
        .iter()
        .find(|item| item.get("role").and_then(Value::as_str) == Some("user"))
        .or_else(|| contents.first())
}

fn first_json_string(parent: Option<&Value>, keys: &[&str]) -> Option<String> {
    let parent = parent?;
    keys.iter()
        .filter_map(|key| parent.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn first_json_string_array(parent: &Value, keys: &[&str]) -> Vec<String> {
    for key in keys {
        if let Some(items) = parent.get(*key).and_then(Value::as_array) {
            return items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect();
        }
    }
    Vec::new()
}

fn first_query_value(query: Option<&str>, keys: &[&str]) -> Option<String> {
    query
        .into_iter()
        .flat_map(|query| form_urlencoded::parse(query.as_bytes()))
        .find(|(key, value)| keys.contains(&key.as_ref()) && !value.trim().is_empty())
        .map(|(_, value)| value.trim().to_string())
}

fn parse_body_text(text: &str) -> Result<Value, String> {
    let body = text.trim();
    if body.is_empty() {
        return Err("解析上游服务响应失败，请稍后再试".to_string());
    }
    if body.starts_with('{') || body.starts_with('[') {
        return serde_json::from_str(body).map_err(|_| "解析上游服务响应失败，请稍后再试".to_string());
    }
    let payload = body
        .lines()
        .filter_map(|line| line.trim().strip_prefix("data:").map(str::trim))
        .filter(|payload| !payload.eq_ignore_ascii_case("[DONE]"))
        .last()
        .ok_or_else(|| "解析上游服务响应失败，请稍后再试".to_string())?;
    serde_json::from_str(payload).map_err(|_| "解析上游服务响应失败，请稍后再试".to_string())
}

fn extract_failure_reason(body_map: &serde_json::Map<String, Value>) -> Option<String> {
    body_map
        .get("data")
        .and_then(Value::as_object)
        .and_then(|data| data.get("failure_reason"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_string())
}

fn guess_http_status(code: Option<i64>, message: &str) -> StatusCode {
    match code {
        Some(400) => StatusCode::BAD_REQUEST,
        Some(401) => StatusCode::UNAUTHORIZED,
        Some(403) => StatusCode::FORBIDDEN,
        Some(404) => StatusCode::NOT_FOUND,
        Some(409) => StatusCode::CONFLICT,
        Some(422) => StatusCode::UNPROCESSABLE_ENTITY,
        Some(429) => StatusCode::TOO_MANY_REQUESTS,
        _ if is_likely_auth_error(message) => StatusCode::UNAUTHORIZED,
        _ => StatusCode::BAD_GATEWAY,
    }
}

fn is_likely_auth_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("unauthorized")
        || lower.contains("forbidden")
        || lower.contains("invalid api key")
        || lower.contains("invalid key")
        || lower.contains("invalid token")
        || lower.contains("api key")
        || lower.contains("apikey")
        || lower.contains("authorization")
}

fn non_empty_or(value: String, fallback: String) -> String {
    if value.trim().is_empty() {
        fallback
    } else {
        value
    }
}

fn parse_error(message: String, body_text: &str) -> GrsaiError {
    GrsaiError {
        http_status: StatusCode::BAD_GATEWAY,
        message,
        upstream_code: None,
        failure_reason: None,
        body_text: body_text.to_string(),
        raw_json: None,
    }
}
```

Modify `src/lib.rs`:

```rust
pub mod grsai;
```

- [ ] **Step 4: Run grsai unit tests and verify they pass**

Run:

```bash
timeout 60s cargo test --test grsai_test
```

Expected: all tests in `grsai_test` pass.

- [ ] **Step 5: Commit**

```bash
git add src/grsai.rs src/lib.rs tests/grsai_test.rs
git commit -m "feat: add grsai protocol parser"
```

## Task 3: Route Gemini Through Provider Registry

**Files:**
- Modify: `src/http/router.rs`
- Test: `src/http/router.rs` internal test module and existing forwarding tests

- [ ] **Step 1: Write Gemini grsai flow tests**

Append these helpers and tests inside `#[cfg(test)] mod tests` in `src/http/router.rs`.
They intentionally call `forward_gemini_request` directly with `resolved.base_url =
"http://api.grsai.com"` and a proxied `reqwest::Client`. This keeps provider
matching realistic while intercepting outbound HTTP locally.

```rust
#[derive(Clone, Default)]
struct GrsaiMockState {
    requests: Arc<Mutex<Vec<Value>>>,
    auth_headers: Arc<Mutex<Vec<String>>>,
}

#[tokio::test]
async fn forward_gemini_request_uses_grsai_sync_provider_for_output_url() {
    let mock = GrsaiMockState::default();
    let state = spawn_grsai_proxy_app_state(mock.clone()).await;
    let resolved = ResolvedUpstream {
        base_url: "http://api.grsai.com".to_string(),
        api_key: "override-key".to_string(),
    };
    let mut config = crate::test_config();
    config.external_image_proxy_prefix = "https://proxy.example.com/fetch?url=".to_string();
    config.proxy_special_upstream_urls = true;
    let state = AppState {
        config: Arc::new(config.clone()),
        uploader: Arc::new(Uploader::new(reqwest::Client::new(), config)),
        ..state
    };
    let request_body = json!({
        "contents": [{
            "role": "user",
            "parts": [
                {"text": "draw a banana"},
                {"inlineData": {"mimeType": "image/png", "data": "https://img.example.com/ref.png"}}
            ]
        }],
        "generationConfig": {
            "imageConfig": {
                "aspectRatio": "16:9",
                "imageSize": "2K"
            }
        }
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1beta/models/gemini-3-pro-image-preview:generateContent?output=url")
        .header(CONTENT_TYPE, "application/json")
        .body(Body::empty())
        .unwrap();
    let (parts, _) = request.into_parts();

    let (response, admin_entry) = forward_gemini_request(
        state,
        resolved,
        "/v1beta/models/gemini-3-pro-image-preview:generateContent".to_string(),
        parts,
        request_body,
        RequestLogSnapshot::default(),
        None,
    )
    .await
    .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(admin_entry.output_mode, "url");
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json_body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json_body["candidates"][0]["content"]["parts"][0]["inlineData"]["data"],
        "https://proxy.example.com/fetch?url=https%3A%2F%2Fapi.grsai.com%2Fimg%2F123.png"
    );

    let requests = mock.requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["model"], "nano-banana-pro");
    assert_eq!(requests[0]["prompt"], "draw a banana");
    assert_eq!(requests[0]["urls"], json!(["https://img.example.com/ref.png"]));
    assert_eq!(requests[0]["aspectRatio"], "16:9");
    assert_eq!(requests[0]["imageSize"], "2K");
    assert_eq!(requests[0]["shutProgress"], true);
    assert_eq!(mock.auth_headers.lock().await.as_slice(), ["Bearer override-key"]);
}

#[tokio::test]
async fn forward_gemini_request_uses_grsai_sync_provider_for_base64() {
    let mock = GrsaiMockState::default();
    let mut state = spawn_grsai_proxy_app_state(mock).await;
    state.response_inline_data_fetch_service = None;
    let resolved = ResolvedUpstream {
        base_url: "http://api.grsai.com".to_string(),
        api_key: "override-key".to_string(),
    };
    let request_body = json!({
        "contents": [{
            "parts": [{"text": "draw"}]
        }]
    });
    let request = Request::builder()
        .method("POST")
        .uri("/v1beta/models/gemini-2.5-flash-image:generateContent")
        .header(CONTENT_TYPE, "application/json")
        .body(Body::empty())
        .unwrap();
    let (parts, _) = request.into_parts();

    let (response, _admin_entry) = forward_gemini_request(
        state,
        resolved,
        "/v1beta/models/gemini-2.5-flash-image:generateContent".to_string(),
        parts,
        request_body,
        RequestLogSnapshot::default(),
        None,
    )
    .await
    .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json_body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json_body["candidates"][0]["content"]["parts"][0]["inlineData"]["data"],
        base64::engine::general_purpose::STANDARD.encode([1_u8, 2, 3])
    );
}

async fn spawn_grsai_proxy_app_state(mock_state: GrsaiMockState) -> AppState {
    spawn_grsai_proxy_app_state_with_response_and_state(
        mock_state,
        json!({
            "code": 0,
            "msg": "success",
            "data": {
                "status": "succeeded",
                "results": [{"url": "http://api.grsai.com/img/123.png"}]
            }
        }),
    )
    .await
}

async fn spawn_grsai_proxy_app_state_with_response(response_body: Value) -> AppState {
    spawn_grsai_proxy_app_state_with_response_and_state(GrsaiMockState::default(), response_body)
        .await
}

async fn spawn_grsai_proxy_app_state_with_response_and_state(
    mock_state: GrsaiMockState,
    response_body: Value,
) -> AppState {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let service = service_fn(move |request| {
            let state = mock_state.clone();
            let response_body = response_body.clone();
            async move { Ok::<_, Infallible>(mock_grsai_proxy(state, response_body, request).await) }
        });
        axum::serve(listener, Shared::new(service)).await.unwrap();
    });

    let config = crate::test_config();
    AppState {
        upstream_client: reqwest::Client::builder()
            .proxy(reqwest::Proxy::http(format!("http://127.0.0.1:{}", address.port())).unwrap())
            .build()
            .unwrap(),
        image_client: reqwest::Client::builder()
            .proxy(reqwest::Proxy::http(format!("http://127.0.0.1:{}", address.port())).unwrap())
            .build()
            .unwrap(),
        uploader: Arc::new(Uploader::new(reqwest::Client::new(), config.clone())),
        admin: None,
        request_inline_data_fetch_service: None,
        response_inline_data_fetch_service: None,
        blob_runtime: Arc::new(crate::test_blob_runtime(8 * 1024 * 1024)),
        upstream_block_cache: None,
        config: Arc::new(config),
    }
}

async fn mock_grsai_proxy(state: GrsaiMockState, response_body: Value, request: Request) -> Response {
    let path = extract_proxy_path(request.uri());
    if path == "/v1/draw/nano-banana" {
        let headers = request.headers().clone();
        let body = to_bytes(request.into_body(), usize::MAX).await.unwrap();
        let parsed: Value = serde_json::from_slice(&body).unwrap();
        state.requests.lock().await.push(parsed);
        state.auth_headers.lock().await.push(
            headers
                .get(AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_string(),
        );
        return Json(response_body).into_response();
    }
    if path == "/img/123.png" {
        return ([(CONTENT_TYPE, "image/png")], vec![1_u8, 2, 3]).into_response();
    }
    StatusCode::NOT_FOUND.into_response()
}
```

- [ ] **Step 2: Run Gemini grsai flow tests and verify they fail**

Run:

```bash
timeout 60s cargo test forward_gemini_request_uses_grsai
```

Expected: failures because the router still transparent-forwards to the Gemini path instead of `grsai` `/v1/draw/nano-banana`.

- [ ] **Step 3: Add provider resolution in Gemini routing**

Modify imports in `src/http/router.rs`:

```rust
use crate::provider::{ProviderKind, resolve_provider};
use crate::upload::{Uploader, wrap_external_proxy_url};
```

Keep `wrap_external_proxy_url` in the existing upload import if it already exists.

In `forward_gemini_request`, replace the direct `is_aiapidev` boolean with:

```rust
let provider = resolve_provider(&resolved.base_url);
```

Change the `if is_aiapidev { ... }` branch to:

```rust
if provider.kind == ProviderKind::Grsai {
    return forward_grsai_gemini_request(
        state,
        resolved,
        target_path,
        request_query,
        body,
        output_mode,
        request_log,
        block_cache_key,
    )
    .await;
}

if provider.kind == ProviderKind::Aiapidev {
    let external_proxy_prefix = state.config.resolved_external_image_proxy_prefix();
    let rewritten_body = rewrite_aiapidev_request_body(
        body,
        &external_proxy_prefix,
        &state.config.image_fetch_external_proxy_domains,
    );
    let target_path = rewrite_aiapidev_model_path(&target_path);
    let request_upstream = if admin_enabled {
        let request_upstream_bytes = serde_json::to_vec(&rewritten_body)
            .map_err(|err| ForwardRequestFailure::new(err, admin_entry.clone()))?;
        admin::maybe_sanitize_json_for_log(&request_upstream_bytes, true)
    } else {
        None
    };
    let upstream_build_started = Instant::now();
    let response = handle_aiapidev_response(
        &resolved,
        &target_path,
        request_query.as_deref(),
        rewritten_body,
        output_mode,
        &state.upstream_client,
        &state.image_client,
        state.response_inline_data_fetch_service.as_ref(),
        state.config.as_ref(),
        state.upstream_block_cache.as_ref(),
        block_cache_key.as_ref(),
    )
    .await;
    let upstream_build_ms = upstream_build_started.elapsed().as_millis() as i64;

    admin_entry.upstream_build_ms = upstream_build_ms;
    admin_entry.request_upstream = request_upstream
        .as_ref()
        .map(|value| value.pretty.clone())
        .unwrap_or_default();
    admin_entry.request_upstream_images = request_upstream
        .as_ref()
        .map(|value| value.image_urls.clone())
        .unwrap_or_default();
    admin_entry.status_code = response.status().as_u16();
    return Ok((response, admin_entry));
}
```

In `model_action` error handling, compute `provider` before the match result and use it for the aiapidev-specific fallback:

```rust
let provider = resolve_provider(&resolved.base_url);
```

Then replace `if !is_aiapidev_upstream` with:

```rust
if provider.kind != ProviderKind::Aiapidev {
```

- [ ] **Step 4: Add Gemini grsai forwarding helper**

Add this helper near `forward_gemini_request` in `src/http/router.rs`:

```rust
async fn forward_grsai_gemini_request(
    state: AppState,
    resolved: ResolvedUpstream,
    target_path: String,
    request_query: Option<String>,
    body: Value,
    output_mode: OutputMode,
    request_log: RequestLogSnapshot,
    block_cache_key: Option<UpstreamBlockCacheKey>,
) -> Result<(Response, AdminLogEntry), ForwardRequestFailure> {
    let mut admin_entry = request_log.base_entry();
    admin_entry.output_mode = match output_mode {
        OutputMode::Base64 => "base64".to_string(),
        OutputMode::Url => "url".to_string(),
    };
    let raw_model = target_path
        .strip_prefix("/v1beta/models/")
        .and_then(|value| value.strip_suffix(":generateContent"))
        .unwrap_or_default();
    let params = crate::grsai::extract_gemini_params(&body, raw_model, request_query.as_deref())
        .map_err(|err| ForwardRequestFailure::new(anyhow!(err.message), admin_entry.clone()))?;
    if params.prompt.trim().is_empty() {
        admin_entry.status_code = StatusCode::BAD_REQUEST.as_u16();
        admin_entry.error_source = "proxy".to_string();
        admin_entry.error_stage = "validate_request".to_string();
        admin_entry.error_kind = "invalid_request".to_string();
        admin_entry.error_message = "Field \"prompt\" is required and must be a string.".to_string();
        let response = proxy_error_response(
            StatusCode::BAD_REQUEST,
            "Field \"prompt\" is required and must be a string.",
            "validate_request",
            "invalid_request",
        );
        return Ok((response, admin_entry));
    }

    let upstream_body = crate::grsai::build_grsai_request_body(&params);
    let request_upstream = if state.admin.is_some() {
        let request_upstream_bytes = serde_json::to_vec(&upstream_body)
            .map_err(|err| ForwardRequestFailure::new(err, admin_entry.clone()))?;
        admin::maybe_sanitize_json_for_log(&request_upstream_bytes, true)
    } else {
        None
    };

    let upstream_started = Instant::now();
    let upstream_url = build_upstream_url(
        &resolved.base_url,
        crate::grsai::IMAGE_GENERATION_PATH,
        None,
    )
    .map_err(|err| ForwardRequestFailure::new(err, admin_entry.clone()))?;
    let upstream_response = state
        .upstream_client
        .post(upstream_url)
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, format!("Bearer {}", resolved.api_key))
        .json(&upstream_body)
        .send()
        .await
        .map_err(|err| ForwardRequestFailure::new(err, admin_entry.clone()))?;
    let status = upstream_response.status();
    let body_bytes = upstream_response.bytes().await.map_err(|err| {
        ForwardRequestFailure::new(
            StructuredProxyError::new(
                MSG_READ_UPSTREAM_BODY_FAILED,
                "read_upstream_body",
                "body_truncated",
                err.to_string(),
            ),
            admin_entry.clone(),
        )
    })?;
    admin_entry.upstream_build_ms = upstream_started.elapsed().as_millis() as i64;
    admin_entry.request_upstream = request_upstream
        .as_ref()
        .map(|value| value.pretty.clone())
        .unwrap_or_default();
    admin_entry.request_upstream_images = request_upstream
        .as_ref()
        .map(|value| value.image_urls.clone())
        .unwrap_or_default();

    let result = match crate::grsai::parse_grsai_response(status, &body_bytes) {
        Ok(result) => result,
        Err(err) => {
            let response = proxy_error_response(
                err.http_status,
                &downstream_grsai_error_message(&err),
                "parse_upstream_response",
                "upstream_error",
            );
            maybe_store_upstream_block_error(
                state.upstream_block_cache.as_ref(),
                block_cache_key.as_ref(),
                err.http_status,
                HeaderValue::from_static("application/json"),
                &body_bytes,
            )
            .await;
            admin_entry.status_code = err.http_status.as_u16();
            admin_entry.error_source = "upstream".to_string();
            admin_entry.error_stage = "parse_upstream_response".to_string();
            admin_entry.error_kind = "upstream_error".to_string();
            admin_entry.error_message = downstream_grsai_error_message(&err);
            admin_entry.error_detail = err.body_text;
            return Ok((response, admin_entry));
        }
    };

    if result.status != "succeeded" || result.image_urls.is_empty() {
        let response = proxy_error_response(
            StatusCode::BAD_GATEWAY,
            MSG_OPENAI_IMAGE_MISSING_DATA,
            "parse_upstream_response",
            "missing_image_url",
        );
        admin_entry.status_code = StatusCode::BAD_GATEWAY.as_u16();
        admin_entry.error_source = "upstream".to_string();
        admin_entry.error_stage = "parse_upstream_response".to_string();
        admin_entry.error_kind = "missing_image_url".to_string();
        admin_entry.error_message = MSG_OPENAI_IMAGE_MISSING_DATA.to_string();
        admin_entry.error_detail = format!("status={}, failure_reason={}", result.status, result.failure_reason);
        return Ok((response, admin_entry));
    }

    let final_json = build_grsai_gemini_response(
        &result.image_urls,
        output_mode,
        &state.image_client,
        state.response_inline_data_fetch_service.as_ref(),
        state.config.as_ref(),
    )
    .await
    .map_err(|err| ForwardRequestFailure::new(err, admin_entry.clone()))?;
    let final_body = serde_json::to_vec(&final_json)
        .map_err(|err| ForwardRequestFailure::new(err, admin_entry.clone()))?;
    let mut response = Response::new(Body::from(final_body));
    *response.status_mut() = StatusCode::OK;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    admin_entry.status_code = StatusCode::OK.as_u16();
    Ok((response, admin_entry))
}
```

Add these helpers below the forwarding helper:

```rust
fn downstream_grsai_error_message(err: &crate::grsai::GrsaiError) -> String {
    if err.http_status == StatusCode::UNAUTHORIZED {
        return "上游服务鉴权失败，请检查 API Key".to_string();
    }
    if err.http_status == StatusCode::TOO_MANY_REQUESTS {
        return "上游服务请求过于频繁，请稍后再试".to_string();
    }
    if err.message.contains("解析上游服务响应失败") {
        return MSG_PARSE_UPSTREAM_JSON_FAILED.to_string();
    }
    MSG_UPSTREAM_REQUEST_FAILED.to_string()
}

async fn build_grsai_gemini_response(
    image_urls: &[String],
    output_mode: OutputMode,
    image_client: &reqwest::Client,
    fetch_service: Option<&Arc<InlineDataUrlFetchService>>,
    config: &Config,
) -> Result<Value> {
    let image_url = image_urls
        .first()
        .ok_or_else(|| anyhow!(MSG_OPENAI_IMAGE_MISSING_DATA))?;
    let inline_data = match output_mode {
        OutputMode::Url => {
            let external_proxy_prefix = config.resolved_external_image_proxy_prefix();
            let data = if config.proxy_special_upstream_urls && !external_proxy_prefix.is_empty() {
                wrap_external_proxy_url(&external_proxy_prefix, image_url)
            } else {
                image_url.to_string()
            };
            json!({
                "mimeType": grsai_guess_image_mime_type(image_url),
                "data": data,
            })
        }
        OutputMode::Base64 => {
            let fetched = if let Some(fetch_service) = fetch_service {
                let fetched = fetch_service.fetch(image_url).await?;
                crate::image_io::FetchedInlineData {
                    mime_type: fetched.mime_type,
                    bytes: fetched.bytes,
                }
            } else {
                crate::image_io::fetch_image_as_inline_data(
                    image_client,
                    image_url,
                    crate::image_io::DEFAULT_MAX_IMAGE_BYTES,
                )
                .await?
            };
            json!({
                "mimeType": fetched.mime_type,
                "data": base64::engine::general_purpose::STANDARD.encode(fetched.bytes),
            })
        }
    };
    Ok(json!({
        "candidates": [{
            "content": {
                "role": "model",
                "parts": [{"inlineData": inline_data}],
            },
            "finishReason": "STOP",
            "safetyRatings": []
        }],
        "usageMetadata": {
            "promptTokenCount": 1,
            "candidatesTokenCount": 1,
            "totalTokenCount": 2
        }
    }))
}

fn grsai_guess_image_mime_type(raw_url: &str) -> &'static str {
    let lower = raw_url.to_ascii_lowercase();
    if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else {
        "image/png"
    }
}
```

- [ ] **Step 5: Run targeted Gemini tests**

Run:

```bash
timeout 60s cargo test forward_gemini_request_uses_grsai
timeout 60s cargo test --test http_forwarding_test generate_content_forwards_expected_upstream_request_and_output_url_result
```

Expected: the two new Gemini grsai tests pass, and the existing transparent forwarding test still passes.

- [ ] **Step 6: Commit**

```bash
git add src/http/router.rs
git commit -m "feat: route gemini grsai provider"
```

## Task 4: Route OpenAI Image Through grsai Provider

**Files:**
- Modify: `src/http/router.rs`
- Test: `src/http/router.rs` internal test module

- [ ] **Step 1: Add OpenAI grsai flow tests**

Append these tests inside `#[cfg(test)] mod tests` in `src/http/router.rs`. Reuse
`GrsaiMockState` and `spawn_grsai_proxy_app_state` from Task 3.

```rust
#[tokio::test]
async fn forward_openai_image_request_uses_grsai_sync_provider() {
    let mock = GrsaiMockState::default();
    let mut state = spawn_grsai_proxy_app_state(mock.clone()).await;
    let mut config = crate::test_config();
    config.openai_image_upstream_url_proxy_prefix = "https://openai-proxy.example.com/fetch?url=".to_string();
    state.config = Arc::new(config.clone());
    state.uploader = Arc::new(Uploader::new(reqwest::Client::new(), config));
    let resolved = ResolvedUpstream {
        base_url: "http://api.grsai.com".to_string(),
        api_key: "override-key".to_string(),
    };

    let (response, admin_entry) = forward_openai_image_request(
        state,
        resolved,
        json!({
            "model": "",
            "prompt": "draw a banana",
            "images": ["https://img.example.com/ref.png"],
            "aspect_ratio": "auto",
            "image_size": "1K"
        }),
        None,
        RequestLogSnapshot::default(),
        None,
    )
    .await
    .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(admin_entry.output_mode, "url");
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json_body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json_body["data"][0]["url"],
        "https://openai-proxy.example.com/fetch?url=http%3A%2F%2Fapi.grsai.com%2Fimg%2F123.png"
    );
    assert!(json_body["usage"].is_object());

    let requests = mock.requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["model"], "nano-banana-fast");
    assert_eq!(requests[0]["prompt"], "draw a banana");
    assert_eq!(requests[0]["urls"], json!(["https://img.example.com/ref.png"]));
    assert_eq!(requests[0]["aspectRatio"], "auto");
    assert_eq!(requests[0]["imageSize"], "1K");
    assert_eq!(requests[0]["shutProgress"], true);
    assert_eq!(mock.auth_headers.lock().await.as_slice(), ["Bearer override-key"]);
}

#[tokio::test]
async fn forward_openai_image_request_grsai_missing_image_url_returns_current_proxy_error_shape() {
    let state = spawn_grsai_proxy_app_state_with_response(json!({
        "code": 0,
        "msg": "success",
        "data": {
            "status": "succeeded",
            "results": []
        }
    }))
    .await;
    let resolved = ResolvedUpstream {
        base_url: "http://api.grsai.com".to_string(),
        api_key: "override-key".to_string(),
    };

    let (response, _admin_entry) = forward_openai_image_request(
        state,
        resolved,
        json!({"prompt": "draw"}),
        None,
        RequestLogSnapshot::default(),
        None,
    )
    .await
    .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json_body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json_body["error"]["code"], 502);
    assert_eq!(json_body["error"]["source"], "proxy");
    assert_eq!(json_body["error"]["stage"], "parse_upstream_response");
}
```

- [ ] **Step 2: Run OpenAI grsai tests and verify they fail**

Run:

```bash
timeout 60s cargo test forward_openai_image_request_uses_grsai
```

Expected: failures because OpenAI still forwards to `/v1/images/generations`.

- [ ] **Step 3: Add OpenAI provider branch**

In `forward_openai_image_request`, add:

```rust
let provider = resolve_provider(&resolved.base_url);
```

Place it after `cache_tracking` is created and before the reference image count validation.

Replace the current aiapidev condition:

```rust
if is_aiapidev_base_url(&resolved.base_url) {
```

with:

```rust
if provider.kind == ProviderKind::Grsai {
    return forward_grsai_openai_image_request(
        state,
        resolved,
        request_body,
        request_query,
        request_log,
        block_cache_key,
    )
    .await;
}

if provider.kind == ProviderKind::Aiapidev {
```

- [ ] **Step 4: Add OpenAI grsai forwarding helper**

Add this helper near `forward_openai_image_request`:

```rust
async fn forward_grsai_openai_image_request(
    state: AppState,
    resolved: ResolvedUpstream,
    request_body: Value,
    request_query: Option<String>,
    request_log: RequestLogSnapshot,
    block_cache_key: Option<UpstreamBlockCacheKey>,
) -> Result<(Response, AdminLogEntry), ForwardRequestFailure> {
    let mut admin_entry = request_log.base_entry();
    admin_entry.output_mode = "url".to_string();
    let params = crate::grsai::extract_openai_params(&request_body)
        .map_err(|err| ForwardRequestFailure::new(anyhow!(err.message), admin_entry.clone()))?;
    if params.prompt.trim().is_empty() {
        admin_entry.status_code = StatusCode::BAD_REQUEST.as_u16();
        admin_entry.error_source = "proxy".to_string();
        admin_entry.error_stage = "validate_request".to_string();
        admin_entry.error_kind = "invalid_request".to_string();
        admin_entry.error_message = "Field \"prompt\" is required and must be a string.".to_string();
        let response = proxy_error_response(
            StatusCode::BAD_REQUEST,
            "Field \"prompt\" is required and must be a string.",
            "validate_request",
            "invalid_request",
        );
        return Ok((response, admin_entry));
    }

    let upstream_body = crate::grsai::build_grsai_request_body(&params);
    let request_upstream = if state.admin.is_some() {
        let request_upstream_bytes = serde_json::to_vec(&upstream_body)
            .map_err(|err| ForwardRequestFailure::new(err, admin_entry.clone()))?;
        admin::maybe_sanitize_json_for_log(&request_upstream_bytes, true)
    } else {
        None
    };

    let upstream_started = Instant::now();
    let upstream_url = build_upstream_url(
        &resolved.base_url,
        crate::grsai::IMAGE_GENERATION_PATH,
        request_query.as_deref(),
    )
    .map_err(|err| ForwardRequestFailure::new(err, admin_entry.clone()))?;
    let upstream_response = state
        .upstream_client
        .post(upstream_url)
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, format!("Bearer {}", resolved.api_key))
        .json(&upstream_body)
        .send()
        .await
        .map_err(|err| ForwardRequestFailure::new(err, admin_entry.clone()))?;
    let status = upstream_response.status();
    let body_bytes = upstream_response.bytes().await.map_err(|err| {
        ForwardRequestFailure::new(
            StructuredProxyError::new(
                MSG_READ_UPSTREAM_BODY_FAILED,
                "read_upstream_body",
                "body_truncated",
                err.to_string(),
            ),
            admin_entry.clone(),
        )
    })?;
    admin_entry.upstream_build_ms = upstream_started.elapsed().as_millis() as i64;
    admin_entry.request_upstream = request_upstream
        .as_ref()
        .map(|value| value.pretty.clone())
        .unwrap_or_default();
    admin_entry.request_upstream_images = request_upstream
        .as_ref()
        .map(|value| value.image_urls.clone())
        .unwrap_or_default();

    let result = match crate::grsai::parse_grsai_response(status, &body_bytes) {
        Ok(result) => result,
        Err(err) => {
            let response = proxy_error_response(
                err.http_status,
                &downstream_grsai_error_message(&err),
                "parse_upstream_response",
                "upstream_error",
            );
            maybe_store_upstream_block_error(
                state.upstream_block_cache.as_ref(),
                block_cache_key.as_ref(),
                err.http_status,
                HeaderValue::from_static("application/json"),
                &body_bytes,
            )
            .await;
            admin_entry.status_code = err.http_status.as_u16();
            admin_entry.error_source = "upstream".to_string();
            admin_entry.error_stage = "parse_upstream_response".to_string();
            admin_entry.error_kind = "upstream_error".to_string();
            admin_entry.error_message = downstream_grsai_error_message(&err);
            admin_entry.error_detail = err.body_text;
            return Ok((response, admin_entry));
        }
    };

    if result.status != "succeeded" || result.image_urls.is_empty() {
        let response = proxy_error_response(
            StatusCode::BAD_GATEWAY,
            MSG_OPENAI_IMAGE_MISSING_DATA,
            "parse_upstream_response",
            "missing_image_url",
        );
        admin_entry.status_code = StatusCode::BAD_GATEWAY.as_u16();
        admin_entry.error_source = "proxy".to_string();
        admin_entry.error_stage = "parse_upstream_response".to_string();
        admin_entry.error_kind = "missing_image_url".to_string();
        admin_entry.error_message = MSG_OPENAI_IMAGE_MISSING_DATA.to_string();
        admin_entry.error_detail = format!("status={}, failure_reason={}", result.status, result.failure_reason);
        return Ok((response, admin_entry));
    }

    let uploaded: Vec<_> = result
        .image_urls
        .iter()
        .map(|image_url| crate::openai_image::UploadedImage {
            url: build_openai_image_output_url(state.config.as_ref(), "direct", image_url),
        })
        .collect();
    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let final_json = crate::openai_image::build_response_payload_from_uploaded(&uploaded, created);
    let final_body = serde_json::to_vec(&final_json)
        .map_err(|err| ForwardRequestFailure::new(err, admin_entry.clone()))?;
    let mut response = Response::new(Body::from(final_body));
    *response.status_mut() = StatusCode::OK;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    admin_entry.status_code = StatusCode::OK.as_u16();
    Ok((response, admin_entry))
}
```

- [ ] **Step 5: Run targeted OpenAI tests**

Run:

```bash
timeout 60s cargo test forward_openai_image_request_uses_grsai
timeout 60s cargo test --test http_forwarding_test image_generations_invalid_json_returns_bad_request
```

Expected: new OpenAI grsai tests pass and existing OpenAI request parsing behavior remains unchanged.

- [ ] **Step 6: Commit**

```bash
git add src/http/router.rs
git commit -m "feat: route openai grsai provider"
```

## Task 5: Preserve aiapidev and Transparent Regression Behavior

**Files:**
- Modify: `src/upstream.rs`
- Modify: `src/http/router.rs`
- Test: existing tests

- [ ] **Step 1: Remove duplicate aiapidev host helper usage**

`src/upstream.rs` still has `is_aiapidev_base_url`. Keep it during the transition if tests reference it directly, but route selection should call `provider::resolve_provider`.

In `src/http/router.rs`, run:

```bash
rg -n "is_aiapidev_base_url" src/http/router.rs
```

Expected after Tasks 3 and 4: no direct route-selection calls remain. Imports can still exist only if helper tests inside `router.rs` need them.

- [ ] **Step 2: Run aiapidev regression tests**

Run:

```bash
timeout 60s cargo test aiapidev_flow_polls_task_and_rewrites_result
timeout 60s cargo test aiapidev_openai_image_flow_polls_task_and_returns_openai_response
timeout 60s cargo test forward_gemini_request_rewrites_aiapidev_base64_inline_data_before_create_call
timeout 60s cargo test forward_openai_image_request_uses_aiapidev_async_flow
```

Expected: all four tests pass.

- [ ] **Step 3: Run transparent forwarding regression tests**

Run:

```bash
timeout 60s cargo test --test http_forwarding_test generate_content_forwards_expected_upstream_request_and_output_url_result
timeout 60s cargo test --test http_forwarding_test upstream_json_error_preserves_message_and_adds_proxy_metadata
timeout 60s cargo test forward_openai_image_request_keeps_happyapi_generations_when_images_missing
timeout 60s cargo test forward_openai_image_request_uses_happyapi_edits_multipart_when_images_exist
```

Expected: all four tests pass.

- [ ] **Step 4: Commit regression-safe cleanup**

If this task only removes unused imports, commit those changes:

```bash
git add src/http/router.rs src/upstream.rs
git commit -m "refactor: use provider registry for route selection"
```

If there are no file changes after the regression tests, record no commit for this task.

## Task 6: grsai Error and SSE Integration Coverage

**Files:**
- Modify: `src/http/router.rs`

- [ ] **Step 1: Add integration tests for SSE and business errors**

Append these tests inside `#[cfg(test)] mod tests` in `src/http/router.rs`. Reuse
`spawn_grsai_proxy_app_state_with_response` from Task 3.

```rust
#[tokio::test]
async fn forward_gemini_request_grsai_accepts_sse_final_payload() {
    let state = spawn_grsai_proxy_app_state_with_text_response(
        "data: {\"code\":0,\"msg\":\"success\",\"data\":{\"status\":\"succeeded\",\"results\":[{\"url\":\"http://api.grsai.com/img/sse.png\"}]}}\n\ndata: [DONE]\n",
    )
    .await;
    let resolved = ResolvedUpstream {
        base_url: "http://api.grsai.com".to_string(),
        api_key: "override-key".to_string(),
    };
    let request = Request::builder()
        .method("POST")
        .uri("/v1beta/models/gemini-3-pro-image-preview:generateContent?output=url")
        .header(CONTENT_TYPE, "application/json")
        .body(Body::empty())
        .unwrap();
    let (parts, _) = request.into_parts();

    let (response, _admin_entry) = forward_gemini_request(
        state,
        resolved,
        "/v1beta/models/gemini-3-pro-image-preview:generateContent".to_string(),
        parts,
        json!({"contents": [{"parts": [{"text": "draw"}]}]}),
        RequestLogSnapshot::default(),
        None,
    )
    .await
    .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json_body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json_body["candidates"][0]["content"]["parts"][0]["inlineData"]["data"],
        "http://api.grsai.com/img/sse.png"
    );
}

#[tokio::test]
async fn forward_gemini_request_grsai_business_error_keeps_rust_error_shape() {
    let state = spawn_grsai_proxy_app_state_with_response(json!({
        "code": 401,
        "msg": "invalid api key",
        "data": {"failure_reason": "auth"}
    }))
    .await;
    let resolved = ResolvedUpstream {
        base_url: "http://api.grsai.com".to_string(),
        api_key: "override-key".to_string(),
    };
    let request = Request::builder()
        .method("POST")
        .uri("/v1beta/models/gemini-3-pro-image-preview:generateContent")
        .header(CONTENT_TYPE, "application/json")
        .body(Body::empty())
        .unwrap();
    let (parts, _) = request.into_parts();

    let (response, _admin_entry) = forward_gemini_request(
        state,
        resolved,
        "/v1beta/models/gemini-3-pro-image-preview:generateContent".to_string(),
        parts,
        json!({"contents": [{"parts": [{"text": "draw"}]}]}),
        RequestLogSnapshot::default(),
        None,
    )
    .await
    .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json_body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json_body["error"]["code"], 401);
    assert_eq!(json_body["error"]["source"], "proxy");
    assert_eq!(json_body["error"]["stage"], "parse_upstream_response");
    assert_eq!(json_body["error"]["kind"], "upstream_error");
}
```

Add this helper next to the other grsai mock helpers if Task 3 did not already add a
text-response variant:

```rust
async fn spawn_grsai_proxy_app_state_with_text_response(response_body: &'static str) -> AppState {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let service = service_fn(move |request: Request| async move {
            let path = extract_proxy_path(request.uri());
            if path == "/v1/draw/nano-banana" {
                return Ok::<_, Infallible>(
                    ([(CONTENT_TYPE, "text/event-stream")], response_body).into_response(),
                );
            }
            Ok::<_, Infallible>(StatusCode::NOT_FOUND.into_response())
        });
        axum::serve(listener, Shared::new(service)).await.unwrap();
    });

    let config = crate::test_config();
    AppState {
        upstream_client: reqwest::Client::builder()
            .proxy(reqwest::Proxy::http(format!("http://127.0.0.1:{}", address.port())).unwrap())
            .build()
            .unwrap(),
        image_client: reqwest::Client::builder()
            .proxy(reqwest::Proxy::http(format!("http://127.0.0.1:{}", address.port())).unwrap())
            .build()
            .unwrap(),
        uploader: Arc::new(Uploader::new(reqwest::Client::new(), config.clone())),
        admin: None,
        request_inline_data_fetch_service: None,
        response_inline_data_fetch_service: None,
        blob_runtime: Arc::new(crate::test_blob_runtime(8 * 1024 * 1024)),
        upstream_block_cache: None,
        config: Arc::new(config),
    }
}
```

- [ ] **Step 2: Run grsai flow tests**

Run:

```bash
timeout 60s cargo test grsai
```

Expected: all grsai integration tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/http/router.rs
git commit -m "test: cover grsai sse and errors"
```

## Task 7: Documentation

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Update README support summary**

Add a small provider support section near the current status or route sections:

```markdown
## 支持的上游 Provider

请求会先根据解析出的 `UPSTREAM_BASE_URL` 或请求头覆盖地址选择 provider：

| Provider | 匹配域名 | 行为 |
| --- | --- | --- |
| `grsai` | `grsai.com`、`*.grsai.com` | 同步单次请求到 `/v1/draw/nano-banana` |
| `aiapidev` | `aiapidev.com`、`www.aiapidev.com` | 创建任务并轮询结果 |
| `transparent` | 其他上游 | 保持现有 Gemini/OpenAI 透明转发 |

`grsai` 支持 Gemini `generateContent` 和 OpenAI `/v1/images/generations`。
Gemini 模型映射：

| Gemini 模型名 | grsai 模型名 |
| --- | --- |
| `gemini-3-pro-image-preview` | `nano-banana-pro` |
| `gemini-2.5-flash-image` | `nano-banana-fast` |
| `gemini-3.1-flash-image-preview` | `nano-banana-2` |
```

- [ ] **Step 2: Run doc-adjacent compile check**

Run:

```bash
timeout 60s cargo test --test provider_test --test grsai_test
```

Expected: provider and grsai unit tests pass after docs-only edit.

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: document provider registry"
```

## Task 8: Final Verification

**Files:**
- No planned edits

- [ ] **Step 1: Format check**

Run:

```bash
timeout 60s cargo fmt -- --check
```

Expected: command exits 0. If it fails, run `cargo fmt`, inspect the diff, and commit formatting with the task that introduced the formatting drift.

- [ ] **Step 2: Focused test suite**

Run:

```bash
timeout 60s cargo test --test provider_test
timeout 60s cargo test --test grsai_test
timeout 60s cargo test grsai
timeout 60s cargo test --test http_forwarding_test
```

Expected: all four commands exit 0.

- [ ] **Step 3: Full test suite**

Run:

```bash
timeout 60s cargo test
```

Expected: exits 0. If this exceeds 60 seconds, rerun the failing or unverified subsets with narrower filters and record the timeout in the final handoff.

- [ ] **Step 4: Inspect final diff**

Run:

```bash
git status --short
git log --oneline -8
```

Expected: working tree is clean after commits, and recent commits correspond to the tasks above.

## Self-Review

- Spec coverage: provider registry, grsai Gemini/OpenAI support, unknown transparent fallback, aiapidev preservation, Rust-style errors, SSE parsing, URL/base64 output, and tests are each covered by tasks.
- Placeholder scan: this plan contains no open requirement markers.
- Type consistency: `ProviderKind`, `GrsaiImageParams`, `GrsaiResult`, `GrsaiError`, `forward_grsai_gemini_request`, and `forward_grsai_openai_image_request` are introduced before route usage.
