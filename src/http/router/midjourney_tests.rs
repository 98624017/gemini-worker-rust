use super::*;
use axum::extract::State;
use axum::serve::ListenerExt;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;

mod protocol_tests;
mod relax_tests;
mod selection_tests;
mod timing_tests;

#[derive(Clone, Debug)]
struct CapturedRequest {
    method: String,
    uri: String,
    headers: HeaderMap,
    body: Value,
}

#[derive(Clone)]
struct MagicMock {
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
    replies: Arc<Mutex<VecDeque<(StatusCode, String)>>>,
    gates: Arc<Mutex<HashMap<usize, Arc<Notify>>>>,
    disconnects: Arc<Mutex<HashSet<usize>>>,
    active_requests: Arc<AtomicUsize>,
}

struct Harness {
    base_url: String,
    client: reqwest::Client,
    mock: MagicMock,
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
    async fn new(replies: Vec<Value>) -> Self {
        Self::with_config(replies, |_| {}).await
    }

    async fn with_config(replies: Vec<Value>, configure: impl FnOnce(&mut Config)) -> Self {
        let mock = MagicMock {
            requests: Default::default(),
            replies: Arc::new(Mutex::new(
                replies
                    .into_iter()
                    .map(|value| (StatusCode::OK, value.to_string()))
                    .collect(),
            )),
            gates: Default::default(),
            disconnects: Default::default(),
            active_requests: Default::default(),
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = listener.local_addr().unwrap();
        let listener = listener.tap_io(|socket| socket.set_nodelay(true).unwrap());
        let mock_app = Router::new().fallback(mock_magic).with_state(mock.clone());
        let upstream_server = tokio::spawn(async move {
            axum::serve(listener, mock_app).await.unwrap();
        });

        let mut config = crate::test_config();
        config.upstream_base_url = format!("http://magic666.top:{}/v1/", upstream_addr.port());
        config.upstream_api_key = "env-key".into();
        config.admin_password = "admin-test".into();
        configure(&mut config);
        let upstream_client = reqwest::Client::builder()
            .no_proxy()
            .resolve("magic666.top", upstream_addr)
            .resolve("magic666.top.evil.example", upstream_addr)
            .resolve("other.example", upstream_addr)
            .build()
            .unwrap();
        let state = AppState {
            config: Arc::new(config.clone()),
            upstream_client,
            image_client: reqwest::Client::new(),
            uploader: Arc::new(Uploader::new(reqwest::Client::new(), config.clone())),
            admin: Some(Arc::new(AdminState::new("admin-test".into()))),
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
        let address = listener.local_addr().unwrap();
        let listener = listener.tap_io(|socket| socket.set_nodelay(true).unwrap());
        let downstream_server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            base_url: format!("http://{address}"),
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

    async fn wait_for_requests(&self, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while self.mock.requests.lock().await.len() < count {
            assert!(
                Instant::now() < deadline,
                "did not receive {count} upstream requests"
            );
            tokio::task::yield_now().await;
        }
        settle().await;
    }
}

async fn settle() {
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
}

struct ActiveRequest(Arc<AtomicUsize>);

impl Drop for ActiveRequest {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

async fn mock_magic(State(mock): State<MagicMock>, request: Request) -> Response {
    mock.active_requests.fetch_add(1, Ordering::Relaxed);
    let _active = ActiveRequest(mock.active_requests.clone());
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, MAX_REQUEST_BODY_BYTES).await.unwrap();
    let mut requests = mock.requests.lock().await;
    let index = requests.len();
    requests.push(CapturedRequest {
        method: parts.method.to_string(),
        uri: parts.uri.to_string(),
        headers: parts.headers,
        body: serde_json::from_slice(&body).unwrap_or(Value::Null),
    });
    drop(requests);
    let (status, body) = mock.replies.lock().await.pop_front().unwrap_or((
        StatusCode::NOT_FOUND,
        json!({"error": {"message": "unexpected request"}}).to_string(),
    ));
    let gate = mock.gates.lock().await.get(&index).cloned();
    if let Some(gate) = gate {
        gate.notified().await;
    }
    if mock.disconnects.lock().await.contains(&index) {
        let chunks = futures_util::stream::iter([
            Ok(Bytes::from_static(b"{")),
            Err(std::io::Error::from(std::io::ErrorKind::ConnectionReset)),
        ]);
        return Response::new(Body::from_stream(chunks));
    }
    (status, [(CONTENT_TYPE, "application/json")], body).into_response()
}

#[tokio::test]
async fn fast_chinese_generation_returns_candidates_through_http() {
    let harness = Harness::new(vec![
        json!({"code": 1, "result": "task-123"}),
        json!({
            "status": "SUCCESS", "finishTime": 1792596466000_i64,
            "imageUrls": ["https://img.example/1.png", {"url": "https://img.example/2.png"}, "https://img.example/1.png"],
            "imageUrl": "https://img.example/grid.png"
        }),
    ]).await;
    let response = harness
        .request(json!({
            "model": "mj_fast_imagine",
            "prompt": "橙色陶瓷狐狸，正文--fast不改 --relax --fast --turbo --ar 3:2 --v 8.1 --fast"
        }))
        .header(AUTHORIZATION, "Bearer selected-key")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], "application/json");
    let body: Value = response.json().await.unwrap();
    assert_eq!(
        body,
        json!({
            "created": 1792596466,
            "data": [{"url": "https://img.example/1.png"}, {"url": "https://img.example/2.png"}],
            "usage": {
                "input_tokens": 1024, "input_tokens_details": {"image_tokens": 1000, "text_tokens": 24},
                "output_tokens": 1024, "total_tokens": 2048,
                "output_tokens_details": {"image_tokens": 1024, "text_tokens": 0}
            }
        })
    );
    let requests = harness.mock.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].uri, "/mj/submit/imagine");
    assert_eq!(
        requests[0].body,
        json!({
            "mode": "FAST", "prompt": "橙色陶瓷狐狸，正文--fast不改 --ar 3:2 --v 8.1 --fast"
        })
    );
    assert_eq!(requests[1].method, "GET");
    assert_eq!(requests[1].uri, "/mj/task/task-123/fetch");
    for request in requests.iter() {
        assert_eq!(request.headers[AUTHORIZATION], "Bearer selected-key");
    }
}

#[tokio::test]
async fn invalid_fields_are_rejected_before_creating_a_task() {
    let harness = Harness::new(vec![]).await;
    let valid = json!({"model": "mj_fast_imagine", "prompt": "中文狐狸 --v 8.1"});
    let mut cases = Vec::new();
    for field in ["model", "prompt"] {
        for value in [Value::Null, json!(1), json!([]), json!(""), json!(" \n\t")] {
            let mut body = valid.clone();
            body[field] = value;
            cases.push(body);
        }
        let mut body = valid.clone();
        body.as_object_mut().unwrap().remove(field);
        cases.push(body);
    }
    for value in [
        Value::Null,
        json!(false),
        json!([]),
        json!("b64_json"),
        json!("jpeg"),
    ] {
        let mut body = valid.clone();
        body["response_format"] = value;
        cases.push(body);
    }
    for alias in ["image", "images", "reference_images"] {
        for value in [
            Value::Null,
            json!("https://img.example/ref.png"),
            json!([1]),
            json!([""]),
            json!(["data:image/png;base64,AQID"]),
            json!(["file:///tmp/ref.png"]),
            json!(["https://img.example/ref.png"]),
        ] {
            let mut body = valid.clone();
            body[alias] = value;
            cases.push(body);
        }
    }
    for field in ["n", "size", "quality", "stream"] {
        let mut body = valid.clone();
        body[field] = Value::Null;
        cases.push(body);
    }
    for body in cases {
        let response = harness.request(body.clone()).send().await.unwrap();
        let status = response.status();
        let error: Value = response.json().await.unwrap();
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "input={body}, response={error}"
        );
        assert_eq!(error["error"]["source"], "proxy");
        assert_eq!(error["error"]["stage"], "magic_validate_request");
        assert_eq!(error["error"]["kind"], "invalid_request");
        assert!(
            error["error"]["message"]
                .as_str()
                .unwrap()
                .chars()
                .any(|ch| ('\u{4e00}'..='\u{9fff}').contains(&ch))
        );
    }
    assert!(harness.mock.requests.lock().await.is_empty());
}
