use super::*;

#[tokio::test]
async fn fast_selection_requires_exact_host_and_model() {
    for (host, model) in [
        ("magic666.top.evil.example", "mj_fast_imagine"),
        ("other.example", "mj_fast_imagine"),
        ("magic666.top", "mj_relax_imagine"),
        ("magic666.top", "mj_fast_upscale"),
        ("magic666.top", "MJ_FAST_IMAGINE"),
        ("magic666.top", "gpt-image-2"),
    ] {
        let harness = Harness::with_config(
            vec![json!({
                "created": 1234, "data": [{"url": "https://img.example/original.png"}]
            })],
            |config| {
                let mut base = Url::parse(&config.upstream_base_url).unwrap();
                base.set_host(Some(host)).unwrap();
                config.upstream_base_url = base.to_string();
            },
        )
        .await;
        let response = harness
            .request(json!({"model": model, "prompt": "原请求 --relax", "n": 1}))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "host={host}, model={model}"
        );
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["created"], 1234);
        assert_eq!(body["data"][0]["url"], "https://img.example/original.png");
        let requests = harness.mock.requests.lock().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].uri, "/v1/images/generations");
        assert_eq!(requests[0].body["model"], model);
        assert_eq!(requests[0].body["prompt"], "原请求 --relax");
        assert_eq!(requests[0].body["n"], 1);
    }
}

#[tokio::test]
async fn origin_paths_and_empty_image_aliases_work_even_with_force_b64_configuration() {
    for suffix in ["", "/", "/v1", "/v1/"] {
        let mut origin = String::new();
        let harness = Harness::with_config(
            vec![
                json!({"code": 1, "result": "path-task"}),
                json!({"status": "SUCCESS", "imageUrl": "https://img.example/result.png"}),
            ],
            |config| {
                let mut base = Url::parse(&config.upstream_base_url).unwrap();
                base.set_path(suffix);
                config.upstream_base_url = base.to_string();
                origin = base.origin().ascii_serialization();
                config.openai_image_b64_json_upstream_domains = vec!["magic666.top".into()];
            },
        )
        .await;
        let response = harness.request(json!({
            "model": "mj_fast_imagine", "prompt": "正文x--fast 保留--faster --turbo --ar 1:1 --v 8.1",
            "image": [], "images": [], "reference_images": [], "response_format": "UrL"
        })).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "base path={suffix}");
        let requests = harness.mock.requests.lock().await;
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].uri, "/mj/submit/imagine");
        assert_eq!(requests[1].uri, "/mj/task/path-task/fetch");
        assert_eq!(
            requests[0].headers["host"],
            origin.strip_prefix("http://").unwrap()
        );
        assert_eq!(
            requests[0].body,
            json!({
                "mode": "FAST", "prompt": "正文x--fast 保留--faster --ar 1:1 --v 8.1 --fast"
            })
        );
    }
}

#[tokio::test]
async fn upstream_override_and_goog_key_precedence_apply_to_submit_and_query() {
    let mut magic_base = String::new();
    let harness = Harness::with_config(
        vec![
            json!({"code": 1, "result": "task/segment?encoded=yes"}),
            json!({"status": "SUCCESS", "imageUrl": "https://img.example/result.png"}),
        ],
        |config| {
            magic_base = config.upstream_base_url.clone();
            config.upstream_base_url = config
                .upstream_base_url
                .replace("magic666.top", "other.example");
        },
    )
    .await;
    let response = harness
        .request(json!({"model": "mj_fast_imagine", "prompt": "测试中文"}))
        .header("x-goog-api-key", format!("{magic_base}|goog-selected-key"))
        .header(AUTHORIZATION, "Bearer ignored-key")
        .query(&[("output", "url"), ("ignored", "parameter")])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let requests = harness.mock.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].uri, "/mj/submit/imagine");
    assert_eq!(
        requests[1].uri,
        "/mj/task/task%2Fsegment%3Fencoded=yes/fetch"
    );
    for request in requests.iter() {
        assert_eq!(request.headers[AUTHORIZATION], "Bearer goog-selected-key");
        assert!(!request.headers.contains_key("x-goog-api-key"));
    }
}

#[tokio::test]
async fn magic_gemini_entry_keeps_the_existing_protocol() {
    let harness = Harness::new(vec![json!({"candidates": [{"content": {
        "parts": [{"text": "普通 Gemini 响应"}]
    }}]})])
    .await;
    let response = harness
        .client
        .post(format!(
            "{}/v1beta/models/demo:generateContent",
            harness.base_url
        ))
        .json(&json!({"contents": [{"parts": [{"text": "中文正文"}]}]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = response.json().await.unwrap();
    assert_eq!(
        body["candidates"][0]["content"]["parts"][0]["text"],
        "普通 Gemini 响应"
    );
    let requests = harness.mock.requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].uri, "/v1beta/models/demo:generateContent");
    assert_eq!(
        requests[0].body["contents"][0]["parts"][0]["text"],
        "中文正文"
    );
}

#[tokio::test]
async fn generic_review_failures_are_not_cached_but_existing_policy_blocks_are() {
    for (reason, expected_status, expected_requests) in [
        ("Prompt图片未通过审核", StatusCode::BAD_GATEWAY, 4),
        (
            "请求无法用于生成图像，请调整输入",
            StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS,
            2,
        ),
    ] {
        let mut replies = Vec::new();
        for _ in 0..2 {
            replies.push(json!({"code": 1, "result": "review-task"}));
            replies.push(json!({"status": "FAILURE", "failReason": reason, "progress": "100%"}));
        }
        let harness = Harness::new(replies).await;
        for _ in 0..2 {
            let response = harness
                .request(json!({"model": "mj_fast_imagine", "prompt": "中文狐狸"}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), expected_status);
        }
        assert_eq!(harness.mock.requests.lock().await.len(), expected_requests);
    }
}
