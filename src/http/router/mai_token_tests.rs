use super::*;

use axum::body::to_bytes;
use axum::extract::State;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::serve::ListenerExt;
use axum::{Router, http::HeaderMap};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

const TIMER_TICK: Duration = Duration::from_millis(1);
const PROXY_PREFIX: &str = "https://proxy.example/base/proxy/image?url=";
const EXTERNAL_PROXY_PREFIX: &str = "https://external.example/base/proxy/image?url=";

#[derive(Clone, Debug)]
struct CapturedRequest {
    method: String,
    uri: String,
    headers: HeaderMap,
    body: Value,
}

#[derive(Clone, Debug)]
struct MockReply {
    status: StatusCode,
    body: String,
}

impl MockReply {
    fn json(body: Value) -> Self {
        Self {
            status: StatusCode::OK,
            body: body.to_string(),
        }
    }

    fn status(status: StatusCode, body: Value) -> Self {
        Self {
            status,
            body: body.to_string(),
        }
    }

    fn raw(status: StatusCode, body: &str) -> Self {
        Self {
            status,
            body: body.to_string(),
        }
    }
}

#[derive(Clone, Default)]
struct MockState {
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
    replies: Arc<Mutex<VecDeque<MockReply>>>,
}

struct Harness {
    base_url: String,
    upstream_port: u16,
    client: reqwest::Client,
    mock: MockState,
    servers: Vec<JoinHandle<()>>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        for server in &self.servers {
            server.abort();
        }
    }
}

impl Harness {
    async fn new(replies: Vec<MockReply>) -> Self {
        Self::with_config(replies, |_| {}).await
    }

    async fn with_config(replies: Vec<MockReply>, configure: impl FnOnce(&mut Config)) -> Self {
        let mock = MockState {
            requests: Default::default(),
            replies: Arc::new(Mutex::new(replies.into_iter().collect())),
        };

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = listener.local_addr().unwrap();
        let upstream_server = tokio::spawn({
            let mock = mock.clone();
            async move {
                let app = Router::new().fallback(mock_upstream).with_state(mock);
                axum::serve(
                    listener.tap_io(|socket| socket.set_nodelay(true).unwrap()),
                    app,
                )
                .await
                .unwrap();
            }
        });

        let mut config = crate::test_config();
        config.upstream_base_url = format!("http://api.mai-token.com:{}/v1/", upstream_addr.port());
        config.upstream_api_key = "env-key".to_string();
        config.admin_password = "admin-test".to_string();
        config.external_image_proxy_prefix = EXTERNAL_PROXY_PREFIX.to_string();
        config.openai_image_upstream_url_proxy_prefix = PROXY_PREFIX.to_string();
        config.proxy_standard_output_urls = true;
        configure(&mut config);

        let upstream_client = reqwest::Client::builder()
            .no_proxy()
            .resolve("api.mai-token.com", upstream_addr)
            .resolve("api.mai-token.com.evil.example", upstream_addr)
            .resolve("other.example", upstream_addr)
            .build()
            .unwrap();
        let state = AppState {
            config: Arc::new(config.clone()),
            upstream_client,
            image_client: reqwest::Client::builder().no_proxy().build().unwrap(),
            uploader: Arc::new(Uploader::new(reqwest::Client::new(), config.clone())),
            admin: Some(Arc::new(AdminState::new("admin-test".to_string()))),
            request_inline_data_fetch_service: None,
            response_inline_data_fetch_service: None,
            blob_runtime: Arc::new(crate::test_blob_runtime(1024)),
            upstream_block_cache: UpstreamBlockCache::new(
                config.upstream_block_cache_ttl,
                config.upstream_block_cache_max_entries,
            )
            .map(Arc::new),
        };
        let app = Router::new()
            .route("/v1/images/generations", post(image_generations_action))
            .route("/v1beta/models/{*rest}", post(model_action))
            .route("/admin/api/logs", get(admin_logs_api))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let downstream_addr = listener.local_addr().unwrap();
        let downstream_server = tokio::spawn(async move {
            axum::serve(
                listener.tap_io(|socket| socket.set_nodelay(true).unwrap()),
                app,
            )
            .await
            .unwrap();
        });

        Self {
            base_url: format!("http://{downstream_addr}"),
            upstream_port: upstream_addr.port(),
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
            mock,
            servers: vec![upstream_server, downstream_server],
        }
    }

    fn request(&self, body: Value) -> reqwest::RequestBuilder {
        self.client
            .post(format!("{}/v1/images/generations", self.base_url))
            .json(&body)
    }

    fn other_host_token(&self) -> String {
        format!(
            "http://api.mai-token.com.evil.example:{}/v1/|other-key",
            self.upstream_port
        )
    }

    async fn wait_for_requests(&self, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if self.mock.requests.lock().await.len() >= count {
                settle().await;
                return;
            }
            assert!(
                Instant::now() < deadline,
                "did not receive {count} upstream requests"
            );
            tokio::task::yield_now().await;
        }
    }
}

async fn mock_upstream(State(mock): State<MockState>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, 4 * 1024 * 1024).await.unwrap();
    let captured = CapturedRequest {
        method: parts.method.to_string(),
        uri: parts.uri.to_string(),
        headers: parts.headers,
        body: serde_json::from_slice(&body).unwrap_or(Value::Null),
    };
    mock.requests.lock().await.push(captured);
    let reply = mock.replies.lock().await.pop_front().unwrap_or_else(|| {
        MockReply::status(
            StatusCode::NOT_FOUND,
            json!({"error": {"message": "unexpected request"}}),
        )
    });
    (
        reply.status,
        [(CONTENT_TYPE, "application/json")],
        reply.body,
    )
        .into_response()
}

async fn settle() {
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
}

fn generation_body() -> Value {
    json!({
        "model": "seedream-5.5",
        "prompt": "draw a glass house",
        "size": "2K",
        "aspect_ratio": "5:4",
        "n": 1,
        "watermark": false,
        "response_format": "url",
        "image": ["https://img.example/image.png"],
        "seed": 0,
    })
}

fn create_reply(task_id: &str) -> MockReply {
    MockReply::json(json!({"id": task_id, "status": "queued"}))
}

fn completed_reply(url: &str) -> MockReply {
    MockReply::json(json!({
        "status": "completed",
        "metadata": {"url": url},
    }))
}

fn proxy_url(url: &str) -> String {
    let encoded: String = url::form_urlencoded::byte_serialize(url.as_bytes()).collect();
    format!("{PROXY_PREFIX}{encoded}")
}

#[tokio::test]
async fn queued_processing_progress_100_and_completed_poll_through_http() {
    let harness = Harness::new(vec![
        create_reply("mai-queued"),
        MockReply::json(json!({"status": "queued"})),
        MockReply::json(json!({"status": "processing", "progress": 100})),
        completed_reply("https://img.example/first.png"),
    ])
    .await;
    let request = harness
        .request(generation_body())
        .bearer_auth("selected-key");
    let task = tokio::spawn(async move { request.send().await.unwrap() });

    harness.wait_for_requests(1).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(5) + TIMER_TICK).await;
    harness.wait_for_requests(2).await;
    settle().await;
    assert!(!task.is_finished(), "progress=100 must remain pending");
    tokio::time::advance(Duration::from_secs(5) + TIMER_TICK).await;
    harness.wait_for_requests(3).await;
    settle().await;
    assert!(!task.is_finished(), "progress=100 must remain pending");
    tokio::time::advance(Duration::from_secs(5) + TIMER_TICK).await;
    harness.wait_for_requests(4).await;

    let response = task.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(
        body["data"],
        json!([{"url": proxy_url("https://img.example/first.png")}])
    );
    assert!(body["created"].is_i64());
    assert_eq!(body["usage"]["total_tokens"], 2048);

    let requests = harness.mock.requests.lock().await;
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[0].method, "POST");
    assert!(requests[1..].iter().all(|request| request.method == "GET"));
    assert_eq!(requests[0].uri, "/v1/videos");
    assert_eq!(requests[1].uri, "/v1/videos/mai-queued");
    assert_eq!(requests[2].uri, "/v1/videos/mai-queued");
    assert_eq!(requests[3].uri, "/v1/videos/mai-queued");
    for request in requests.iter() {
        assert_eq!(request.headers[AUTHORIZATION], "Bearer selected-key");
    }
}

#[tokio::test]
async fn mai_output_without_dedicated_proxy_prefix_keeps_upstream_url() {
    let harness = Harness::with_config(
        vec![
            create_reply("raw-url-task"),
            completed_reply("https://img.example/raw.png"),
        ],
        |config| config.openai_image_upstream_url_proxy_prefix.clear(),
    )
    .await;
    let response = harness
        .request(generation_body())
        .bearer_auth("selected-key")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(
        body["data"],
        json!([{"url": "https://img.example/raw.png"}])
    );
    assert!(
        !body["data"][0]["url"]
            .as_str()
            .unwrap()
            .starts_with(EXTERNAL_PROXY_PREFIX)
    );
    assert_eq!(harness.mock.requests.lock().await.len(), 2);
}

#[tokio::test]
async fn request_body_merges_reference_aliases_and_preserves_seed_zero() {
    let harness = Harness::new(vec![
        create_reply("body-task"),
        completed_reply("https://img.example/one.png"),
    ])
    .await;
    let mut body = generation_body();
    body["reference_images"] = json!(["https://img.example/reference.png"]);
    body["images"] = json!(["https://img.example/second.png"]);

    let response = harness
        .request(body)
        .header(AUTHORIZATION, "Bearer selected-key")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let requests = harness.mock.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].body,
        json!({
            "model": "seedream-5.5",
            "prompt": "draw a glass house",
            "n": 1,
            "quality": "1440p",
            "aspect_ratio": "5:4",
            "image_urls": [
                "https://img.example/reference.png",
                "https://img.example/second.png",
                "https://img.example/image.png",
            ],
            "seed": 0,
        })
    );
    assert_eq!(requests[1].uri, "/v1/videos/body-task");
}

#[tokio::test]
async fn mai_selection_is_exact_host_and_model_and_gemini_stays_transparent() {
    let harness = Harness::new(vec![
        MockReply::json(json!({
            "created": 1712345678,
            "data": [{"url": "https://img.example/other-model.png"}]
        })),
        MockReply::json(json!({
            "created": 1712345679,
            "data": [{"url": "https://img.example/other-host.png"}]
        })),
        MockReply::json(json!({
            "candidates": [{"content": {"parts": [{"text": "transparent"}]}}]
        })),
    ])
    .await;

    let response = harness
        .request(json!({"model": "seedream-5-pro", "prompt": "other model"}))
        .bearer_auth("selected-key")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = harness
        .request(generation_body())
        .header("x-goog-api-key", harness.other_host_token())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = harness
        .client
        .post(format!(
            "{}/v1beta/models/gemini-3-pro-image-preview:generateContent",
            harness.base_url
        ))
        .header(AUTHORIZATION, "Bearer selected-key")
        .json(&json!({
            "contents": [{"parts": [{"text": "transparent gemini"}]}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let requests = harness.mock.requests.lock().await;
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].uri, "/v1/images/generations");
    assert!(requests[0].body.get("contract_version").is_none());
    assert_eq!(requests[1].uri, "/v1/images/generations");
    assert!(requests[1].body.get("contract_version").is_none());
    assert_eq!(
        requests[2].uri,
        "/v1beta/models/gemini-3-pro-image-preview:generateContent"
    );
    assert_eq!(
        requests[2].body["contents"][0]["parts"][0]["text"],
        "transparent gemini"
    );
}

#[tokio::test]
async fn mai_allows_ten_references_but_eleven_and_other_channels_are_rejected_locally() {
    let references = (0..10)
        .map(|index| format!("https://img.example/{index}.png"))
        .collect::<Vec<_>>();
    let harness = Harness::new(vec![
        create_reply("ten-task"),
        completed_reply("https://img.example/result.png"),
    ])
    .await;
    let response = harness
        .request(json!({
            "model": "seedream-5.5",
            "prompt": "ten references",
            "image": references,
        }))
        .bearer_auth("selected-key")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let requests = harness.mock.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].body["image_urls"].as_array().unwrap().len(), 10);
    drop(requests);

    let eleven = (0..11)
        .map(|index| format!("https://img.example/{index}.png"))
        .collect::<Vec<_>>();
    let harness = Harness::new(vec![]).await;
    let response = harness
        .request(json!({
            "model": "seedream-5.5",
            "prompt": "eleven references",
            "image": eleven,
        }))
        .bearer_auth("selected-key")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(harness.mock.requests.lock().await.is_empty());

    let seven = (0..7)
        .map(|index| format!("https://img.example/{index}.png"))
        .collect::<Vec<_>>();
    let harness = Harness::new(vec![]).await;
    let response = harness
        .request(json!({
            "model": "seedream-5.5",
            "prompt": "other channel",
            "image": seven,
        }))
        .header("x-goog-api-key", harness.other_host_token())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(harness.mock.requests.lock().await.is_empty());
}

#[tokio::test]
async fn create_http_errors_are_returned_without_recreating_the_task() {
    for status in [
        StatusCode::UNAUTHORIZED,
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::INTERNAL_SERVER_ERROR,
    ] {
        let harness = Harness::new(vec![MockReply::status(
            status,
            json!({"error": {"message": "upstream unavailable"}}),
        )])
        .await;
        let response = harness
            .request(generation_body())
            .bearer_auth("selected-key")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"]["message"], "upstream unavailable");
        let requests = harness.mock.requests.lock().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].headers[AUTHORIZATION], "Bearer selected-key");
    }
}

#[tokio::test]
async fn polling_429_retries_the_same_task_without_a_second_post() {
    let harness = Harness::new(vec![
        MockReply::json(json!({"task_id": "retry-task", "status": "queued"})),
        MockReply::status(
            StatusCode::TOO_MANY_REQUESTS,
            json!({"error": {"message": "poll later"}}),
        ),
        completed_reply("https://img.example/retried.png"),
    ])
    .await;
    let request = harness
        .request(generation_body())
        .bearer_auth("selected-key");
    let task = tokio::spawn(async move { request.send().await.unwrap() });
    harness.wait_for_requests(1).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(5) + TIMER_TICK).await;
    harness.wait_for_requests(2).await;
    tokio::time::advance(Duration::from_secs(5) + TIMER_TICK).await;
    harness.wait_for_requests(3).await;

    let response = task.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let requests = harness.mock.requests.lock().await;
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[1].method, "GET");
    assert_eq!(requests[2].method, "GET");
    assert_eq!(requests[1].uri, "/v1/videos/retry-task");
    assert_eq!(requests[2].uri, "/v1/videos/retry-task");
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method == "POST")
            .count(),
        1
    );
}

#[tokio::test]
async fn failed_task_stops_polling_with_an_explicit_error() {
    let harness = Harness::new(vec![
        create_reply("failed-task"),
        MockReply::json(json!({
            "status": "failed",
            "error": {"message": "content rejected"}
        })),
    ])
    .await;
    let response = harness
        .request(generation_body())
        .bearer_auth("selected-key")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body: Value = response.json().await.unwrap();
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("content rejected"))
    );
    assert_eq!(body["error"]["upstream_task_id"], "failed-task");
    let requests = harness.mock.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[1].method, "GET");
}

#[tokio::test]
async fn malformed_create_missing_id_and_completed_without_url_are_502() {
    let cases = [
        (
            vec![MockReply::raw(StatusCode::OK, "{not-json")],
            false,
            None,
        ),
        (
            vec![MockReply::json(json!({"status": "queued"}))],
            false,
            None,
        ),
        (
            vec![
                create_reply("empty-task"),
                MockReply::json(json!({
                    "status": "completed",
                    "metadata": {},
                })),
            ],
            true,
            Some("empty-task"),
        ),
        (
            vec![MockReply::json(json!({
                "id": "create-failed-task",
                "status": "failed",
                "error": {"message": "creation failed"},
            }))],
            false,
            Some("create-failed-task"),
        ),
        (
            vec![MockReply::json(json!({
                "task_id": "bad-status-task",
                "status": "unexpected",
            }))],
            false,
            Some("bad-status-task"),
        ),
    ];
    for (replies, expect_poll, expected_task_id) in cases {
        let harness = Harness::new(replies).await;
        let response = harness
            .request(generation_body())
            .bearer_auth("selected-key")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"]["code"], 502);
        let requests = harness.mock.requests.lock().await;
        assert_eq!(requests.len(), if expect_poll { 2 } else { 1 });
        assert_eq!(requests[0].method, "POST");
        if let Some(expected_task_id) = expected_task_id {
            assert_eq!(body["error"]["upstream_task_id"], expected_task_id);
        }
    }
}

#[tokio::test]
async fn short_deadline_returns_timeout_with_task_id_in_error_and_admin_log() {
    let harness = Harness::with_config(
        vec![
            create_reply("deadline-task"),
            MockReply::json(json!({"status": "queued"})),
        ],
        |config| config.upstream_timeout = Duration::from_millis(250),
    )
    .await;
    let response = tokio::time::timeout(
        Duration::from_secs(2),
        harness
            .request(generation_body())
            .bearer_auth("selected-key")
            .send(),
    )
    .await
    .expect("deadline request did not finish")
    .unwrap();
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["upstream_task_id"], "deadline-task");

    let logs: Value = harness
        .client
        .get(format!("{}/admin/api/logs", harness.base_url))
        .basic_auth("user", Some("admin-test"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(logs["items"][0]["upstreamTaskId"], "deadline-task");
    assert_eq!(harness.mock.requests.lock().await.len(), 1);
}

#[tokio::test]
async fn transient_unknown_status_continues_polling_until_completed() {
    let harness = Harness::new(vec![
        MockReply::json(json!({"id": "mai-unknown-ok", "status": "unknown"})),
        MockReply::json(json!({"status": "unknown", "progress": 0})),
        MockReply::json(json!({"status": "unknown", "progress": 0})),
        MockReply::json(json!({"status": "in_progress", "progress": 10})),
        completed_reply("https://img.example/recovered.png"),
    ])
    .await;
    let request = harness
        .request(generation_body())
        .bearer_auth("selected-key");
    let task = tokio::spawn(async move { request.send().await.unwrap() });

    harness.wait_for_requests(1).await;
    tokio::time::pause();
    for expected_requests in 2..=5 {
        tokio::time::advance(Duration::from_secs(5) + TIMER_TICK).await;
        harness.wait_for_requests(expected_requests).await;
    }

    let response = task.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(
        body["data"],
        json!([{"url": proxy_url("https://img.example/recovered.png")}])
    );
    let requests = harness.mock.requests.lock().await;
    assert_eq!(requests.len(), 5);
    assert_eq!(requests[0].method, "POST");
    assert!(requests[1..].iter().all(|request| request.method == "GET"));
}

#[tokio::test]
async fn persistent_unknown_status_returns_502_after_limit() {
    let mut replies = vec![create_reply("stuck-unknown-task")];
    replies.extend((0..18).map(|_| MockReply::json(json!({"status": "unknown", "progress": 0}))));
    let harness = Harness::new(replies).await;
    let request = harness
        .request(generation_body())
        .bearer_auth("selected-key");
    let task = tokio::spawn(async move { request.send().await.unwrap() });

    harness.wait_for_requests(1).await;
    tokio::time::pause();
    for expected_requests in 2..=19 {
        tokio::time::advance(Duration::from_secs(5) + TIMER_TICK).await;
        harness.wait_for_requests(expected_requests).await;
    }

    let response = task.await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], 502);
    assert_eq!(body["error"]["kind"], "invalid_task_response");
    assert_eq!(body["error"]["upstream_task_id"], "stuck-unknown-task");
    assert_eq!(harness.mock.requests.lock().await.len(), 19);
}
