use super::*;

fn generation_request(harness: &Harness) -> reqwest::RequestBuilder {
    harness.request(json!({"model": "mj_fast_imagine", "prompt": "中文狐狸 --v 8.1"}))
}

async fn error_body(response: reqwest::Response, status: StatusCode, stage: &str) -> Value {
    assert_eq!(response.status(), status);
    assert_eq!(response.headers()[CONTENT_TYPE], "application/json");
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["source"], "upstream", "{body}");
    assert_eq!(body["error"]["stage"], stage, "{body}");
    assert!(!body["error"]["message"].as_str().unwrap().is_empty());
    assert!(body.get("data").is_none(), "{body}");
    body
}

async fn admin_logs(harness: &Harness) -> Value {
    let response = harness
        .client
        .get(format!("{}/admin/api/logs", harness.base_url))
        .basic_auth("user", Some("admin-test"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response.json().await.unwrap()
}

#[tokio::test]
async fn accepted_submission_codes_query_the_returned_task_once() {
    for code in [1, 21, 22] {
        let task_id = format!("accepted-{code}");
        let harness = Harness::new(vec![
            json!({"code": code, "result": format!(" {task_id} ")}),
            json!({"status": "SUCCESS", "imageUrl": "https://img.example/result.png"}),
        ])
        .await;
        let response = generation_request(&harness).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "code={code}");
        let body: Value = response.json().await.unwrap();
        assert_eq!(
            body["data"],
            json!([{"url": "https://img.example/result.png"}])
        );
        let requests = harness.mock.requests.lock().await;
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].uri, "/mj/submit/imagine");
        assert_eq!(requests[1].method, "GET");
        assert_eq!(requests[1].uri, format!("/mj/task/{task_id}/fetch"));
    }
}

#[tokio::test]
async fn successful_http_submission_still_checks_business_failure_and_classification() {
    let cases = [
        (0, "未分类任务错误", StatusCode::BAD_GATEWAY),
        (3, "暂时不可用", StatusCode::SERVICE_UNAVAILABLE),
        (23, "暂时不可用", StatusCode::SERVICE_UNAVAILABLE),
        (30, "暂时不可用", StatusCode::SERVICE_UNAVAILABLE),
        (4, "请求被拒绝", StatusCode::BAD_REQUEST),
        (24, "请求被拒绝", StatusCode::BAD_REQUEST),
        (99, "没有可用实例", StatusCode::SERVICE_UNAVAILABLE),
        (
            99,
            "No available account instance",
            StatusCode::SERVICE_UNAVAILABLE,
        ),
        (99, "队列满", StatusCode::SERVICE_UNAVAILABLE),
        (99, "分组负载饱和", StatusCode::SERVICE_UNAVAILABLE),
        (99, "输入违反内容政策", StatusCode::BAD_REQUEST),
    ];
    for (code, reason, status) in cases {
        let harness = Harness::new(vec![json!({
            "code": code, "result": "not-an-accepted-task", "failReason": reason
        })])
        .await;
        let body = error_body(
            generation_request(&harness).send().await.unwrap(),
            status,
            "magic_submit",
        )
        .await;
        assert_eq!(body["error"]["business_code"], code);
        assert_eq!(body["error"]["failReason"], reason);
        assert!(body["error"]["message"].as_str().unwrap().contains(reason));
        assert_eq!(harness.mock.requests.lock().await.len(), 1);
    }
}

#[tokio::test]
async fn upstream_authentication_and_rate_limit_http_statuses_are_preserved() {
    for (during_poll, status) in [
        (false, StatusCode::UNAUTHORIZED),
        (false, StatusCode::FORBIDDEN),
        (false, StatusCode::TOO_MANY_REQUESTS),
        (true, StatusCode::UNAUTHORIZED),
        (true, StatusCode::FORBIDDEN),
    ] {
        let harness = Harness::new(if during_poll {
            vec![json!({"code": 1, "result": "auth-task"})]
        } else {
            vec![]
        })
        .await;
        harness.mock.replies.lock().await.push_back((
            status,
            json!({"code": 3, "description": "上游凭据或限流错误"}).to_string(),
        ));
        let body = error_body(
            generation_request(&harness).send().await.unwrap(),
            status,
            if during_poll {
                "magic_poll"
            } else {
                "magic_submit"
            },
        )
        .await;
        assert_eq!(body["error"]["business_code"], 3);
        if during_poll {
            assert_eq!(body["error"]["task_id"], "auth-task");
        }
        assert_eq!(
            harness.mock.requests.lock().await.len(),
            if during_poll { 2 } else { 1 }
        );
    }
}

#[tokio::test]
async fn non_success_http_submission_cannot_be_accepted_by_its_business_code() {
    for status in [
        StatusCode::INTERNAL_SERVER_ERROR,
        StatusCode::BAD_GATEWAY,
        StatusCode::SERVICE_UNAVAILABLE,
    ] {
        let harness = Harness::new(vec![]).await;
        harness.mock.replies.lock().await.push_back((
            status,
            json!({"code": 1, "result": "must-not-poll", "description": "一般上游故障"})
                .to_string(),
        ));
        let body = error_body(
            generation_request(&harness).send().await.unwrap(),
            StatusCode::BAD_GATEWAY,
            "magic_submit",
        )
        .await;
        assert_eq!(body["error"]["business_code"], 1);
        assert_eq!(harness.mock.requests.lock().await.len(), 1);
    }
}

#[tokio::test]
async fn malformed_submission_never_starts_polling_or_returns_success() {
    let cases = [
        "not json".to_string(),
        json!(null).to_string(),
        json!([]).to_string(),
        json!({}).to_string(),
        json!({"code": "1", "result": "task"}).to_string(),
        json!({"code": 1}).to_string(),
        json!({"code": 1, "result": null}).to_string(),
        json!({"code": 1, "result": 123}).to_string(),
        json!({"code": 1, "result": []}).to_string(),
        json!({"code": 1, "result": ""}).to_string(),
        json!({"code": 1, "result": " \n\t"}).to_string(),
    ];
    for raw in cases {
        let harness = Harness::new(vec![]).await;
        harness
            .mock
            .replies
            .lock()
            .await
            .push_back((StatusCode::OK, raw.clone()));
        let body = error_body(
            generation_request(&harness).send().await.unwrap(),
            StatusCode::BAD_GATEWAY,
            "magic_submit",
        )
        .await;
        assert_eq!(body["error"]["kind"], "invalid_response", "raw={raw}");
        assert_eq!(harness.mock.requests.lock().await.len(), 1, "raw={raw}");
    }
}

#[tokio::test]
async fn malformed_poll_response_stops_immediately_with_task_diagnostics() {
    for raw in [
        "not json".to_string(),
        json!(null).to_string(),
        json!([]).to_string(),
        json!({}).to_string(),
        json!({"status": null}).to_string(),
        json!({"status": 100, "progress": "100%"}).to_string(),
    ] {
        let harness = Harness::new(vec![json!({"code": 1, "result": "broken-task"})]).await;
        harness
            .mock
            .replies
            .lock()
            .await
            .push_back((StatusCode::OK, raw.clone()));
        let body = error_body(
            generation_request(&harness).send().await.unwrap(),
            StatusCode::BAD_GATEWAY,
            "magic_poll",
        )
        .await;
        assert_eq!(body["error"]["kind"], "invalid_response", "raw={raw}");
        assert_eq!(body["error"]["task_id"], "broken-task");
        assert_eq!(harness.mock.requests.lock().await.len(), 2);
    }
}

#[tokio::test]
async fn successful_candidates_keep_order_deduplicate_and_use_compatible_usage() {
    let harness = Harness::new(vec![
        json!({"code": 1, "result": "candidate-task"}),
        json!({
            "status": "SUCCESS", "finishTime": 1792596466999_i64,
            "submitTime": 1792596400000_i64,
            "imageUrls": [
                {"url": "https://img.example/2.png"}, "http://img.example/1.png",
                "https://img.example/2.png", {"url": "https://img.example/3.png"},
                {"url": "http://img.example/1.png"}, "https://img.example/4.png"
            ],
            "imageUrl": "invalid fallback ignored when candidates exist"
        }),
    ])
    .await;
    let response = generation_request(&harness).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], "application/json");
    let body: Value = response.json().await.unwrap();
    assert_eq!(
        body,
        json!({
            "created": 1792596466,
            "data": [
                {"url": "https://img.example/2.png"}, {"url": "http://img.example/1.png"},
                {"url": "https://img.example/3.png"}, {"url": "https://img.example/4.png"}
            ],
            "usage": {
                "input_tokens": 1024, "input_tokens_details": {"image_tokens": 1000, "text_tokens": 24},
                "output_tokens": 1024, "total_tokens": 2048,
                "output_tokens_details": {"image_tokens": 1024, "text_tokens": 0}
            }
        })
    );
    assert_eq!(harness.mock.requests.lock().await.len(), 2);
}

#[tokio::test]
async fn v82_resolution_flags_return_original_candidates_in_both_modes() {
    for (model, mode, path, flag) in [
        ("mj_fast_imagine", "FAST", "/mj/submit/imagine", "--fast"),
        (
            "mj_relax_imagine",
            "RELAX",
            "/mj-relax/mj/submit/imagine",
            "--relax",
        ),
    ] {
        for (resolution, image_id) in [
            ("--sd", "be70613e-ef23-446d-aa34-562e51f8cac7"),
            ("--hd", "50ae7195-40a3-4578-bd3e-5c1a1083e6d3"),
        ] {
            let candidates: Vec<_> = (0..4)
                .map(|index| {
                    json!({"url": format!(
                        "https://imagine-cdn.youchuan.cn/{image_id}_0_{index}.png"
                    )})
                })
                .collect();
            let harness = Harness::new(vec![
                json!({"code": 1, "result": "resolution-task"}),
                json!({
                    "status": "SUCCESS",
                    "imageUrls": candidates,
                    "imageUrl": "https://img.example/grid.png"
                }),
            ])
            .await;
            let prompt = format!("中文狐狸 --v 8.2 --ar 1:1 {resolution}");
            let response = harness
                .request(json!({
                    "model": model, "prompt": format!("{prompt} --turbo"),
                    "response_format": "url"
                }))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{model} {resolution}");
            let body: Value = response.json().await.unwrap();
            assert_eq!(body["data"], json!(candidates), "{model} {resolution}");
            let requests = harness.mock.requests.lock().await;
            assert_eq!(requests.len(), 2);
            assert_eq!(requests[0].uri, path);
            assert_eq!(
                requests[0].body,
                json!({"mode": mode, "prompt": format!("{prompt} {flag}")})
            );
            assert_eq!(requests[1].uri, "/mj/task/resolution-task/fetch");
        }
    }
}

#[tokio::test]
async fn only_missing_or_empty_candidate_lists_fall_back_to_main_image() {
    for candidates in [None, Some(json!([]))] {
        let mut result = json!({"status": "SUCCESS", "imageUrl": "https://img.example/grid.png"});
        if let Some(candidates) = candidates {
            result["imageUrls"] = candidates;
        }
        let harness =
            Harness::new(vec![json!({"code": 1, "result": "fallback-task"}), result]).await;
        let response = generation_request(&harness).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = response.json().await.unwrap();
        assert_eq!(
            body["data"],
            json!([{"url": "https://img.example/grid.png"}])
        );
    }
}

#[tokio::test]
async fn malformed_candidates_are_protocol_errors_even_when_main_image_is_valid() {
    for candidates in [
        json!(null),
        json!("https://img.example/candidate.png"),
        json!({"url": "https://img.example/1.png"}),
        json!(["https://img.example/valid.png", null]),
        json!([{}]),
        json!([{"url": null}]),
        json!([{"url": 12}]),
        json!([false]),
        json!([12]),
        json!([""]),
        json!(["/relative.png"]),
        json!(["https://"]),
        json!(["ftp://img.example/image.png"]),
        json!(["data:image/png;base64,AQID"]),
    ] {
        let harness = Harness::new(vec![
            json!({"code": 1, "result": "invalid-image-task"}),
            json!({"status": "SUCCESS", "imageUrls": candidates, "imageUrl": "https://img.example/grid.png"}),
        ])
        .await;
        let body = error_body(
            generation_request(&harness).send().await.unwrap(),
            StatusCode::BAD_GATEWAY,
            "magic_result",
        )
        .await;
        assert_eq!(
            body["error"]["kind"], "invalid_response",
            "candidates={candidates}"
        );
        assert_eq!(body["error"]["task_id"], "invalid-image-task");
        assert_eq!(harness.mock.requests.lock().await.len(), 2);
    }
    for result in [
        json!({"status": "SUCCESS"}),
        json!({"status": "SUCCESS", "imageUrls": []}),
        json!({"status": "SUCCESS", "imageUrl": null}),
        json!({"status": "SUCCESS", "imageUrl": "file:///tmp/image.png"}),
    ] {
        let harness = Harness::new(vec![
            json!({"code": 1, "result": "missing-image-task"}),
            result,
        ])
        .await;
        let body = error_body(
            generation_request(&harness).send().await.unwrap(),
            StatusCode::BAD_GATEWAY,
            "magic_result",
        )
        .await;
        assert_eq!(body["error"]["kind"], "invalid_response");
    }
}

#[tokio::test]
async fn created_uses_valid_millisecond_timestamps_then_local_time_and_keeps_url_prefix() {
    for (mut result, expected) in [
        (
            json!({"finishTime": 1792596466999_i64, "submitTime": 1792596400000_i64}),
            Some(1792596466_i64),
        ),
        (
            json!({"submitTime": 1792596400999_i64}),
            Some(1792596400_i64),
        ),
        (
            json!({"finishTime": 0, "submitTime": 1792596400999_i64}),
            Some(1792596400_i64),
        ),
        (
            json!({"finishTime": -1, "submitTime": 1792596400999_i64}),
            Some(1792596400_i64),
        ),
        (
            json!({"finishTime": "1792596466999", "submitTime": 1792596400999_i64}),
            Some(1792596400_i64),
        ),
        (
            json!({"finishTime": 1.5, "submitTime": 1792596400999_i64}),
            Some(1792596400_i64),
        ),
        (json!({}), None),
        (json!({"finishTime": null, "submitTime": null}), None),
        (json!({"finishTime": 0, "submitTime": -1}), None),
    ] {
        result["status"] = json!("SUCCESS");
        result["imageUrl"] = json!("https://img.example/grid.png?x=1&y=2");
        let harness = Harness::with_config(
            vec![json!({"code": 1, "result": "timestamp-task"}), result],
            |config| {
                config.openai_image_upstream_url_proxy_prefix =
                    "https://proxy.example/image?url=".into();
            },
        )
        .await;
        let before = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let response = generation_request(&harness).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = response.json().await.unwrap();
        let after = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let created = body["created"].as_i64().unwrap();
        if let Some(expected) = expected {
            assert_eq!(created, expected);
        } else {
            assert!((before..=after).contains(&created));
        }
        assert_eq!(
            body["data"],
            json!([{
                "url": "https://proxy.example/image?url=https%3A%2F%2Fimg.example%2Fgrid.png%3Fx%3D1%26y%3D2"
            }])
        );
        assert_eq!(harness.mock.requests.lock().await.len(), 2);
    }
}

#[tokio::test]
async fn terminal_failures_preserve_reason_and_task_even_at_full_progress() {
    for status in ["FAILURE", "FAILED", "CANCEL", "CANCELLED", "MODAL"] {
        let harness = Harness::new(vec![
            json!({"code": 1, "result": "failed-task"}),
            json!({"status": status, "progress": "100%", "code": 6,
                "failReason": "任务未生成图片，请调整后重试", "imageUrl": "https://img.example/not-success.png"}),
        ])
        .await;
        let body = error_body(
            generation_request(&harness).send().await.unwrap(),
            StatusCode::BAD_GATEWAY,
            "magic_poll",
        )
        .await;
        assert_eq!(body["error"]["business_code"], 6);
        assert_eq!(body["error"]["failReason"], "任务未生成图片，请调整后重试");
        assert_eq!(body["error"]["task_id"], "failed-task");
        let message = body["error"]["message"].as_str().unwrap();
        assert!(message.contains(status));
        assert!(message.contains("任务未生成图片，请调整后重试"));
        if status == "MODAL" {
            assert_eq!(body["error"]["kind"], "upstream_interaction_required");
            assert!(message.contains("额外交互"));
        }
        assert_eq!(harness.mock.requests.lock().await.len(), 2);
    }
}

#[tokio::test]
async fn admin_logs_record_actual_magic_request_task_stage_and_elapsed_time() {
    let harness = Harness::new(vec![
        json!({"code": 1, "result": "logged-task"}),
        json!({"status": "SUCCESS", "imageUrl": "https://img.example/result.png"}),
    ])
    .await;
    let response = generation_request(&harness).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let _: Value = response.json().await.unwrap();
    let logs = admin_logs(&harness).await;
    let items = logs["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    let entry = &items[0];
    assert_eq!(entry["upstreamMode"], "FAST");
    assert_eq!(entry["upstreamTaskId"], "logged-task");
    assert_eq!(entry["upstreamStage"], "magic_complete");
    assert_eq!(entry["statusCode"], 200);
    assert_eq!(entry["upstreamStatusCode"], 200);
    for field in [
        "durationMs",
        "requestParseMs",
        "upstreamBuildMs",
        "responseProcessMs",
    ] {
        assert!(entry[field].as_i64().unwrap() >= 0, "{field}: {entry}");
    }
    let payload: Value = serde_json::from_str(entry["requestUpstream"].as_str().unwrap()).unwrap();
    assert_eq!(
        payload,
        json!({"mode": "FAST", "prompt": "中文狐狸 --v 8.1 --fast"})
    );
    assert_eq!(harness.mock.requests.lock().await[0].body, payload);
    assert!(!logs.to_string().contains("env-key"));
}

#[tokio::test]
async fn echoed_keys_are_redacted_from_errors_and_admin_diagnostics() {
    for selected_key in [None, Some("selected-key")] {
        let reason = if selected_key.is_some() {
            "上游返回 env-key 和 selected-key，任务失败"
        } else {
            "上游返回 env-key，任务失败"
        };
        let harness = Harness::new(vec![
            json!({"code": 1, "result": "redacted-task"}),
            json!({"status": "FAILED", "code": 6, "failReason": reason}),
        ])
        .await;
        let mut request = generation_request(&harness);
        if let Some(key) = selected_key {
            request = request.bearer_auth(key);
        }
        let body = error_body(
            request.send().await.unwrap(),
            StatusCode::BAD_GATEWAY,
            "magic_poll",
        )
        .await;
        let logs = admin_logs(&harness).await;
        for output in [body.to_string(), logs.to_string()] {
            assert!(!output.contains("env-key"), "{output}");
            assert!(!output.contains("selected-key"), "{output}");
            assert!(output.contains("[redacted]"), "{output}");
        }
        let entry = &logs["items"][0];
        assert_eq!(entry["upstreamMode"], "FAST");
        assert_eq!(entry["upstreamTaskId"], "redacted-task");
        assert_eq!(entry["upstreamStage"], "magic_poll");
        assert_eq!(entry["errorStage"], "magic_poll");
        let requests = harness.mock.requests.lock().await;
        assert_eq!(requests.len(), 2);
        for request in requests.iter() {
            assert_eq!(
                request.headers[AUTHORIZATION],
                format!("Bearer {}", selected_key.unwrap_or("env-key"))
            );
        }
    }
}
