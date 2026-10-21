use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

// Tokio 将截止时间向上取整到毫秒，额外推进一个 tick 以触发边界上的定时器。
const TIMER_TICK: Duration = Duration::from_millis(1);

fn generation() -> Value {
    json!({"model": "mj_fast_imagine", "prompt": "中文陶瓷狐狸 --v 8.1"})
}

fn submitted() -> Value {
    json!({"code": 1, "result": "timed-task"})
}

fn succeeded() -> Value {
    json!({"status": "SUCCESS", "imageUrl": "https://img.example/result.png"})
}

async fn finished<T>(task: &JoinHandle<T>) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !task.is_finished() {
        assert!(Instant::now() < deadline, "HTTP request did not finish");
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn pending_and_unknown_states_wait_for_success_at_ten_second_intervals() {
    let states = [
        "",
        "NOT_START",
        "SUBMITTED",
        "IN_PROGRESS",
        "NEW_UPSTREAM_STATE",
    ];
    let mut replies = vec![submitted()];
    replies.extend(
        states
            .iter()
            .map(|status| json!({"status": status, "progress": "100%"})),
    );
    replies.push(succeeded());
    let harness = Harness::new(replies).await;
    let request = harness.request(generation());
    let task = tokio::spawn(async move { request.send().await.unwrap() });
    harness.wait_for_requests(2).await;
    tokio::time::pause();
    for index in 0..states.len() {
        assert!(!task.is_finished());
        tokio::time::advance(Duration::from_secs(9)).await;
        settle().await;
        assert_eq!(harness.mock.requests.lock().await.len(), index + 2);
        tokio::time::advance(Duration::from_secs(1) + TIMER_TICK).await;
        harness.wait_for_requests(index + 3).await;
    }
    finished(&task).await;
    assert_eq!(task.await.unwrap().status(), StatusCode::OK);
    let requests = harness.mock.requests.lock().await;
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method == "POST")
            .count(),
        1
    );
}

#[tokio::test]
async fn only_polling_retries_transient_errors_and_resets_after_a_good_query() {
    let harness = Harness::new(vec![]).await;
    let mut replies = vec![(StatusCode::OK, submitted().to_string())];
    for _ in 0..4 {
        replies.push((StatusCode::SERVICE_UNAVAILABLE, "busy".into()));
    }
    replies.push((StatusCode::OK, json!({"status": "IN_PROGRESS"}).to_string()));
    for _ in 0..4 {
        replies.push((StatusCode::BAD_GATEWAY, "temporary".into()));
    }
    replies.push((StatusCode::OK, succeeded().to_string()));
    *harness.mock.replies.lock().await = replies.into();
    harness.mock.disconnects.lock().await.insert(2);
    let request = harness.request(generation());
    let task = tokio::spawn(async move { request.send().await.unwrap() });
    harness.wait_for_requests(2).await;
    tokio::time::pause();
    for count in 3..=11 {
        tokio::time::advance(Duration::from_secs(10) + TIMER_TICK).await;
        harness.wait_for_requests(count).await;
    }
    finished(&task).await;
    assert_eq!(task.await.unwrap().status(), StatusCode::OK);
    assert_eq!(
        harness
            .mock
            .requests
            .lock()
            .await
            .iter()
            .filter(|request| request.method == "POST")
            .count(),
        1
    );
}

#[tokio::test]
async fn five_consecutive_poll_failures_end_the_request_without_resubmission() {
    let harness = Harness::new(vec![]).await;
    let mut replies = vec![(StatusCode::OK, submitted().to_string())];
    for _ in 0..5 {
        replies.push((
            StatusCode::TOO_MANY_REQUESTS,
            json!({"error": {"message": "limit"}}).to_string(),
        ));
    }
    *harness.mock.replies.lock().await = replies.into();
    let request = harness.request(generation());
    let task = tokio::spawn(async move { request.send().await.unwrap() });
    harness.wait_for_requests(2).await;
    tokio::time::pause();
    for count in 3..=6 {
        tokio::time::advance(Duration::from_secs(10) + TIMER_TICK).await;
        harness.wait_for_requests(count).await;
    }
    finished(&task).await;
    let response = task.await.unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["task_id"], "timed-task");
    tokio::time::advance(Duration::from_secs(30)).await;
    settle().await;
    assert_eq!(harness.mock.requests.lock().await.len(), 6);
}

#[tokio::test]
async fn submit_and_slow_poll_share_one_absolute_deadline() {
    let harness = Harness::with_config(vec![submitted(), succeeded()], |config| {
        config.upstream_timeout = Duration::from_secs(30);
    })
    .await;
    let submit_gate = Arc::new(Notify::new());
    let poll_gate = Arc::new(Notify::new());
    harness
        .mock
        .gates
        .lock()
        .await
        .insert(0, submit_gate.clone());
    harness.mock.gates.lock().await.insert(1, poll_gate);
    let request = harness.request(generation());
    let task = tokio::spawn(async move { request.send().await.unwrap() });
    harness.wait_for_requests(1).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(20)).await;
    submit_gate.notify_one();
    harness.wait_for_requests(2).await;
    tokio::time::advance(Duration::from_secs(9)).await;
    settle().await;
    assert!(!task.is_finished());
    tokio::time::advance(Duration::from_secs(1) + TIMER_TICK).await;
    finished(&task).await;
    let response = task.await.unwrap();
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["stage"], "magic_poll");
    assert_eq!(body["error"]["task_id"], "timed-task");
    assert_eq!(harness.mock.requests.lock().await.len(), 2);
}

#[tokio::test]
async fn polling_sleep_is_bounded_by_remaining_budget() {
    let harness = Harness::with_config(
        vec![submitted(), json!({"status": "IN_PROGRESS"})],
        |config| {
            config.upstream_timeout = Duration::from_secs(5);
        },
    )
    .await;
    let request = harness.request(generation());
    let task = tokio::spawn(async move { request.send().await.unwrap() });
    harness.wait_for_requests(2).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(5) + TIMER_TICK).await;
    finished(&task).await;
    assert_eq!(task.await.unwrap().status(), StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(harness.mock.requests.lock().await.len(), 2);
}

#[tokio::test]
async fn default_budget_is_six_hundred_seconds_including_submission() {
    let harness = Harness::new(vec![submitted()]).await;
    harness
        .mock
        .gates
        .lock()
        .await
        .insert(0, Arc::new(Notify::new()));
    let request = harness.request(generation());
    let task = tokio::spawn(async move { request.send().await.unwrap() });
    harness.wait_for_requests(1).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(450)).await;
    settle().await;
    assert!(!task.is_finished());
    tokio::time::advance(Duration::from_secs(150) + TIMER_TICK).await;
    finished(&task).await;
    let response = task.await.unwrap();
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(
        response.json::<Value>().await.unwrap()["error"]["stage"],
        "magic_submit"
    );
    assert_eq!(harness.mock.requests.lock().await.len(), 1);
}

#[tokio::test]
async fn interrupted_submission_is_not_replayed() {
    let harness = Harness::new(vec![submitted(), succeeded()]).await;
    harness.mock.disconnects.lock().await.insert(0);
    let response = harness.request(generation()).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["stage"], "magic_submit");
    assert!(body["error"].get("detail").is_none());
    assert_eq!(harness.mock.requests.lock().await.len(), 1);
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
    let detail = logs["items"][0]["errorDetail"].as_str().unwrap();
    assert!(
        detail.contains("connection"),
        "missing network cause: {detail}"
    );
    assert!(!detail.contains("env-key"));
}

#[tokio::test]
async fn real_downstream_disconnect_stops_polling_and_releases_inflight_query() {
    for pending_query in [false, true] {
        let harness = Harness::new(vec![
            submitted(),
            json!({"status": "IN_PROGRESS"}),
            succeeded(),
        ])
        .await;
        if pending_query {
            harness
                .mock
                .gates
                .lock()
                .await
                .insert(1, Arc::new(Notify::new()));
        }
        let address = harness.base_url.strip_prefix("http://").unwrap();
        let mut socket = TcpStream::connect(address).await.unwrap();
        let body = generation().to_string();
        socket.write_all(format!(
            "POST /v1/images/generations HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()
        ).as_bytes()).await.unwrap();
        harness.wait_for_requests(2).await;
        drop(socket);
        // TCP 断开依赖真实 I/O 就绪，先观察取消完成再推进虚拟轮询时钟。
        let logs = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
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
                if logs["items"][0]["statusCode"] == 499
                    && harness.mock.active_requests.load(Ordering::Relaxed) == 0
                {
                    break logs;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("downstream cancellation did not release upstream query and record status 499");
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(30)).await;
        settle().await;
        assert_eq!(harness.mock.requests.lock().await.len(), 2);
        assert_eq!(harness.mock.active_requests.load(Ordering::Relaxed), 0);
        assert_eq!(logs["items"][0]["statusCode"], 499);
        assert_eq!(logs["items"][0]["upstreamTaskId"], "timed-task");
        tokio::time::resume();
    }
}

#[tokio::test]
async fn reading_the_downstream_body_consumes_the_same_request_budget() {
    let harness = Harness::with_config(vec![submitted(), succeeded()], |config| {
        config.upstream_timeout = Duration::from_secs(30);
    })
    .await;
    harness
        .mock
        .gates
        .lock()
        .await
        .insert(1, Arc::new(Notify::new()));
    let address = harness.base_url.strip_prefix("http://").unwrap();
    let mut socket = TcpStream::connect(address).await.unwrap();
    let body = generation().to_string();
    socket.write_all(format!(
        "POST /v1/images/generations HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nExpect: 100-continue\r\nConnection: close\r\nContent-Length: {}\r\n\r\n", body.len()
    ).as_bytes()).await.unwrap();
    let mut interim = [0; 25];
    socket.read_exact(&mut interim).await.unwrap();
    assert_eq!(&interim, b"HTTP/1.1 100 Continue\r\n\r\n");
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(20)).await;
    socket.write_all(body.as_bytes()).await.unwrap();
    let response = tokio::spawn(async move {
        let mut bytes = Vec::new();
        socket.read_to_end(&mut bytes).await.unwrap();
        String::from_utf8(bytes).unwrap()
    });
    harness.wait_for_requests(2).await;
    tokio::time::advance(Duration::from_secs(10) + TIMER_TICK).await;
    finished(&response).await;
    let response = response.await.unwrap();
    assert!(response.starts_with("HTTP/1.1 504"), "{response}");
    assert!(response.contains("timed-task"));
    assert_eq!(harness.mock.requests.lock().await.len(), 2);
}

#[tokio::test]
async fn explicit_input_rejection_is_not_retried_as_a_temporary_http_failure() {
    let harness = Harness::new(vec![submitted()]).await;
    harness.mock.replies.lock().await.push_back((
        StatusCode::SERVICE_UNAVAILABLE,
        json!({"code": 24, "description": "提示词包含敏感词"}).to_string(),
    ));
    let request = harness.request(generation());
    let task = tokio::spawn(async move { request.send().await.unwrap() });
    harness.wait_for_requests(2).await;
    finished(&task).await;
    assert_eq!(task.await.unwrap().status(), StatusCode::BAD_REQUEST);
    assert_eq!(harness.mock.requests.lock().await.len(), 2);
}

#[tokio::test]
async fn stalled_downstream_body_times_out_without_creating_a_task() {
    let harness = Harness::with_config(vec![], |config| {
        config.upstream_timeout = Duration::from_secs(5);
    })
    .await;
    let address = harness.base_url.strip_prefix("http://").unwrap();
    let mut socket = TcpStream::connect(address).await.unwrap();
    let body = generation().to_string();
    socket.write_all(format!(
        "POST /v1/images/generations HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nExpect: 100-continue\r\nConnection: close\r\nContent-Length: {}\r\n\r\n", body.len()
    ).as_bytes()).await.unwrap();
    let mut interim = [0; 25];
    socket.read_exact(&mut interim).await.unwrap();
    assert_eq!(&interim, b"HTTP/1.1 100 Continue\r\n\r\n");
    let response = tokio::spawn(async move {
        let mut bytes = Vec::new();
        socket.read_to_end(&mut bytes).await.unwrap();
        String::from_utf8(bytes).unwrap()
    });
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(5) + TIMER_TICK).await;
    finished(&response).await;
    let response = response.await.unwrap();
    assert!(response.starts_with("HTTP/1.1 504"), "{response}");
    assert!(response.contains("read_request_body"));
    assert!(harness.mock.requests.lock().await.is_empty());
}
