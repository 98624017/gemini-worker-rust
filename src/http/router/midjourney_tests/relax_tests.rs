use super::*;

#[tokio::test]
async fn relax_chinese_generation_uses_slow_path_and_a_single_relax_flag() {
    let harness = Harness::new(vec![
        json!({"code": 1, "result": "relax-task"}),
        json!({
            "status": "SUCCESS", "finishTime": 1792596466000_i64,
            "imageUrls": ["https://img.example/relax-1.png", {"url": "https://img.example/relax-2.png"}]
        }),
    ])
    .await;
    let response = harness
        .request(json!({
            "model": "mj_relax_imagine",
            "prompt": "橙色陶瓷狐狸，正文--fast不改 --fast --relax --turbo --ar 3:2 --v 8.1 --relax"
        }))
        .bearer_auth("selected-key")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["created"], 1792596466_i64);
    assert_eq!(
        body["data"],
        json!([
            {"url": "https://img.example/relax-1.png"},
            {"url": "https://img.example/relax-2.png"}
        ])
    );
    let requests = harness.mock.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].uri, "/mj-relax/mj/submit/imagine");
    assert_eq!(
        requests[0].body,
        json!({
            "mode": "RELAX",
            "prompt": "橙色陶瓷狐狸，正文--fast不改 --ar 3:2 --v 8.1 --relax"
        })
    );
    assert_eq!(requests[1].method, "GET");
    assert_eq!(requests[1].uri, "/mj/task/relax-task/fetch");
    for request in requests.iter() {
        assert_eq!(request.headers[AUTHORIZATION], "Bearer selected-key");
    }
    let submitted = requests[0].body.clone();
    drop(requests);
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
    let entry = &logs["items"][0];
    assert_eq!(entry["upstreamMode"], "RELAX");
    assert_eq!(entry["upstreamTaskId"], "relax-task");
    assert_eq!(entry["upstreamStage"], "magic_complete");
    let logged: Value = serde_json::from_str(entry["requestUpstream"].as_str().unwrap()).unwrap();
    assert_eq!(logged, submitted);
    assert!(!logs.to_string().contains("selected-key"));
    assert!(!logs.to_string().contains("env-key"));
}

#[tokio::test]
async fn relax_waits_for_success_before_returning_the_openai_response() {
    let harness = Harness::new(vec![
        json!({"code": 21, "result": "waiting-relax"}),
        json!({"status": "IN_PROGRESS", "progress": "100%"}),
        json!({"status": "SUCCESS", "imageUrl": "https://img.example/relax.png"}),
    ])
    .await;
    let request =
        harness.request(json!({"model": "mj_relax_imagine", "prompt": "中文狐狸 --v 8.1"}));
    let task = tokio::spawn(async move { request.send().await.unwrap() });
    harness.wait_for_requests(2).await;
    tokio::time::pause();
    assert!(!task.is_finished());
    tokio::time::advance(Duration::from_secs(9)).await;
    settle().await;
    assert!(!task.is_finished());
    assert_eq!(harness.mock.requests.lock().await.len(), 2);
    tokio::time::advance(Duration::from_secs(1) + Duration::from_millis(1)).await;
    harness.wait_for_requests(3).await;
    let response = task.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(
        body["data"],
        json!([{"url": "https://img.example/relax.png"}])
    );
    let requests = harness.mock.requests.lock().await;
    assert_eq!(requests[0].uri, "/mj-relax/mj/submit/imagine");
    assert_eq!(requests[1].uri, "/mj/task/waiting-relax/fetch");
    assert_eq!(requests[2].uri, "/mj/task/waiting-relax/fetch");
}

#[tokio::test]
async fn relax_task_failure_keeps_diagnostics_and_never_resubmits() {
    for status in ["FAILURE", "FAILED", "CANCEL", "CANCELLED", "MODAL"] {
        let harness = Harness::new(vec![
            json!({"code": 22, "result": "failed-relax"}),
            json!({"status": status, "code": 6, "failReason": "任务失败 env-key selected-key"}),
        ])
        .await;
        let response = harness
            .request(json!({"model": "mj_relax_imagine", "prompt": "中文狐狸 --v 8.1"}))
            .bearer_auth("selected-key")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY, "{status}");
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"]["stage"], "magic_poll");
        assert_eq!(body["error"]["business_code"], 6);
        assert_eq!(body["error"]["task_id"], "failed-relax");
        assert_eq!(
            body["error"]["failReason"],
            "任务失败 [redacted] [redacted]"
        );
        assert!(body["error"]["message"].as_str().unwrap().contains(status));
        assert!(body.get("data").is_none());
        let requests = harness.mock.requests.lock().await;
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].uri, "/mj-relax/mj/submit/imagine");
    }
}

#[tokio::test]
async fn relax_submission_and_query_share_the_request_deadline() {
    let harness = Harness::with_config(
        vec![
            json!({"code": 1, "result": "deadline-relax"}),
            json!({"status": "SUCCESS", "imageUrl": "https://img.example/late.png"}),
        ],
        |config| config.upstream_timeout = Duration::from_secs(30),
    )
    .await;
    let submit_gate = Arc::new(Notify::new());
    harness
        .mock
        .gates
        .lock()
        .await
        .insert(0, submit_gate.clone());
    harness
        .mock
        .gates
        .lock()
        .await
        .insert(1, Arc::new(Notify::new()));
    let request =
        harness.request(json!({"model": "mj_relax_imagine", "prompt": "中文狐狸 --v 8.1"}));
    let task = tokio::spawn(async move { request.send().await.unwrap() });
    harness.wait_for_requests(1).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(20)).await;
    submit_gate.notify_one();
    harness.wait_for_requests(2).await;
    tokio::time::advance(Duration::from_secs(9)).await;
    settle().await;
    assert!(!task.is_finished());
    tokio::time::advance(Duration::from_secs(1) + Duration::from_millis(1)).await;
    let deadline = Instant::now() + Duration::from_secs(3);
    while !task.is_finished() {
        assert!(
            Instant::now() < deadline,
            "RELAX exceeded the request budget"
        );
        tokio::task::yield_now().await;
    }
    let response = task.await.unwrap();
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["stage"], "magic_poll");
    assert_eq!(body["error"]["task_id"], "deadline-relax");
    let requests = harness.mock.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].uri, "/mj-relax/mj/submit/imagine");
}

#[tokio::test]
async fn invalid_relax_input_is_rejected_before_task_creation() {
    let harness = Harness::new(vec![]).await;
    for (field, value) in [
        ("prompt", json!(" \n\t")),
        ("response_format", json!(null)),
        ("response_format", json!("b64_json")),
        ("image", json!(null)),
        (
            "image",
            json!(["https://img.example/a.png", "https://img.example/b.png"]),
        ),
        ("images", json!([1])),
        ("reference_images", json!(["QUJDREVGRw=="])),
        ("image", json!(["data:image/png;base64,AQID"])),
        ("n", json!(1)),
        ("size", json!("1024x1024")),
        ("quality", json!("high")),
        ("stream", json!(false)),
    ] {
        let mut request = json!({"model": "mj_relax_imagine", "prompt": "中文狐狸 --v 8.1"});
        request[field] = value;
        let response = harness.request(request).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{field}");
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["error"]["stage"], "magic_validate_request");
        assert_eq!(body["error"]["kind"], "invalid_request");
    }
    assert!(harness.mock.requests.lock().await.is_empty());
}
