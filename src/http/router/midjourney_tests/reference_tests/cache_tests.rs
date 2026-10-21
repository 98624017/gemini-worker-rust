use super::*;

#[tokio::test]
async fn legacy_cache_hits_misses_and_disabled_cache_keep_all_mj_png_references() {
    let mut png = image_bytes(32, 16, ImageFormat::Png, Rgba([10, 20, 30, 255]));
    png.resize(crate::image_io::REQUEST_PNG_WEBP_THRESHOLD_BYTES + 1, 0);
    for variant in ["disabled", "miss", "legacy-hit"] {
        let source = ReferenceSource::new(png.clone(), "image/png").await;
        let mut config = crate::test_config();
        config.enable_request_image_webp_optimization = true;
        config.inline_data_url_background_fetch_max_inflight = 0;
        config.inline_data_url_cache_dir.clear();
        config.inline_data_url_memory_cache_max_bytes = if variant == "disabled" {
            0
        } else {
            32 * 1024 * 1024
        };
        config.inline_data_url_background_fetch_total_timeout = Duration::ZERO;
        let service = InlineDataUrlFetchService::from_config(
            &config,
            source.client(),
            crate::image_io::REQUEST_MAX_IMAGE_BYTES,
            false,
        );
        let warmed = if variant == "legacy-hit" {
            let cached = service
                .as_ref()
                .unwrap()
                .fetch(&source.url())
                .await
                .unwrap();
            assert_eq!(cached.mime_type, "image/webp");
            assert!(!cached.from_cache);
            Some(cached.bytes)
        } else {
            None
        };
        let harness = Harness::with_image_services(
            reference_replies(),
            |target| {
                target.enable_image_compression = false;
                target.enable_request_image_webp_optimization = true;
            },
            source.client(),
            service.clone(),
            None,
        )
        .await;
        let mut body = generation(&source);
        body["image"] = json!([source.url(), source.url()]);
        let response = harness.request(body).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{variant}");
        let requests = harness.mock.requests.lock().await;
        let items = requests[0].body["base64Array"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], items[1]);
        let (mime, bytes) = super::multiple_tests::decode_item(&items[0]);
        assert_eq!(mime, "image/jpeg", "{variant}");
        assert_eq!(
            image::load_from_memory(&bytes).unwrap().dimensions(),
            (32, 16)
        );
        assert_jpeg_444(&bytes);
        drop(requests);
        let mj_downloads = if warmed.is_some() { 3 } else { 2 };
        assert_eq!(source.requests.lock().await.len(), mj_downloads);
        if let Some(service) = service {
            let cached = service.fetch(&source.url()).await.unwrap();
            assert_eq!(cached.mime_type, "image/webp");
            assert_eq!(cached.from_cache, warmed.is_some());
            if let Some(warmed) = warmed {
                assert_eq!(cached.bytes, warmed);
                assert_eq!(source.requests.lock().await.len(), 3);
            } else {
                assert_eq!(source.requests.lock().await.len(), 3);
                assert!(service.fetch(&source.url()).await.unwrap().from_cache);
            }
        }
    }
}

#[tokio::test]
async fn temporary_image_review_failure_does_not_become_a_block_cache_hit() {
    let source = ReferenceSource::new(
        image_bytes(8, 8, ImageFormat::Jpeg, Rgba([30, 60, 90, 255])),
        "image/jpeg",
    )
    .await;
    let mut replies = vec![json!({"code": 4, "description": "Prompt图片未通过审核"})];
    replies.extend(reference_replies());
    let harness = Harness::with_image_client(replies, |_| {}, source.client()).await;
    let body = generation(&source);
    let response = harness.request(body.clone()).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = harness.request(body).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(source.requests.lock().await.len(), 2);
    let requests = harness.mock.requests.lock().await;
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method == "POST")
            .count(),
        2
    );
}

#[tokio::test]
async fn admin_reference_log_contains_mime_and_size_without_data_or_keys() {
    let original = image_bytes(64, 32, ImageFormat::Jpeg, Rgba([80, 120, 180, 255]));
    let source = ReferenceSource::new(original.clone(), "image/jpeg").await;
    let harness = source.harness().await;
    let response = harness
        .request(generation(&source))
        .bearer_auth("selected-key")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
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
    assert_eq!(entry["upstreamStage"], "magic_complete");
    assert_eq!(entry["upstreamTaskId"], "reference-task");
    let payload = entry["requestUpstream"].as_str().unwrap();
    assert!(payload.contains("mime=image/jpeg"));
    assert!(payload.contains(&format!("bytes={}", original.len())));
    for secret in [
        "data:image/",
        "selected-key",
        "env-key",
        &STANDARD.encode(&original),
    ] {
        assert!(!logs.to_string().contains(secret));
    }
}

#[tokio::test]
async fn admin_upstream_logs_bound_long_prompts_without_truncating_submission() {
    let harness = Harness::new(reference_replies()).await;
    let prompt = "中文狐狸".repeat(admin::ADMIN_MAX_BODY_BYTES_PER_ENTRY);
    let response = harness
        .request(json!({"model": "mj_fast_imagine", "prompt": prompt}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let _: Value = response.json().await.unwrap();
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

    let logged = logs["items"][0]["requestUpstream"].as_str().unwrap();
    assert!(logged.len() <= admin::ADMIN_MAX_BODY_BYTES_PER_ENTRY + "\n...[truncated]".len());
    assert!(logged.ends_with("\n...[truncated]"));
    let requests = harness.mock.requests.lock().await;
    assert_eq!(requests[0].body["prompt"], format!("{prompt} --fast"));
}

#[tokio::test]
async fn reference_download_failure_logs_connection_cause_without_source_url() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            drop(socket);
        }
    });
    let source = ReferenceSource {
        address,
        requests: Default::default(),
        gate: Default::default(),
        server,
    };
    let harness = source.harness().await;
    let response = harness
        .request(json!({
            "model": "mj_fast_imagine", "prompt": "中文狐狸 --v 8.1",
            "image": [format!("{}?token=source-only-token", source.url())]
        }))
        .bearer_auth("selected-key")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["stage"], "magic_prepare_image");
    assert!(body["error"].get("detail").is_none());
    assert!(harness.mock.requests.lock().await.is_empty());
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
        detail.to_ascii_lowercase().contains("connection"),
        "{detail}"
    );
    for secret in ["source-only-token", "selected-key", "env-key"] {
        assert!(!detail.contains(secret), "{detail}");
    }
}
