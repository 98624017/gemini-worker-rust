use super::*;

pub(super) fn decode_item(item: &Value) -> (String, Vec<u8>) {
    let (prefix, data) = item.as_str().unwrap().split_once(',').unwrap();
    let mime = prefix
        .strip_prefix("data:")
        .unwrap()
        .strip_suffix(";base64")
        .unwrap();
    (mime.to_string(), STANDARD.decode(data).unwrap())
}

#[tokio::test]
async fn delayed_reference_never_reorders_items_or_submits_partial_images() {
    let jpeg = image_bytes(64, 32, ImageFormat::Jpeg, Rgba([80, 120, 180, 255]));
    let webp = image_bytes(32, 64, ImageFormat::WebP, Rgba([20, 40, 60, 77]));
    let slow = ReferenceSource::new(jpeg.clone(), "image/jpeg").await;
    let ready = ReferenceSource::new(webp.clone(), "image/webp").await;
    let gate = Arc::new(Notify::new());
    *slow.gate.lock().await = Some(gate.clone());
    let harness = slow.harness().await;
    let request = harness.request(json!({
        "model": "mj_fast_imagine", "prompt": "中文狐狸 --v 8.1",
        "image": [slow.url(), ready.url()]
    }));
    let task = tokio::spawn(async move { request.send().await.unwrap() });
    slow.wait_for_downloads(1).await;
    assert!(harness.mock.requests.lock().await.is_empty());
    assert!(!task.is_finished());
    gate.notify_one();
    let response = task.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let requests = harness.mock.requests.lock().await;
    let items = requests[0].body["base64Array"].as_array().unwrap();
    assert_eq!(decode_item(&items[0]), ("image/jpeg".into(), jpeg));
    assert_eq!(decode_item(&items[1]), ("image/webp".into(), webp));
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method == "POST")
            .count(),
        1
    );
}

#[tokio::test]
async fn a_failed_later_reference_stops_the_whole_request_and_releases_prepared_images() {
    use super::lifecycle_tests::{spilled_runtime, wait_until_empty};
    let first = ReferenceSource::new(
        image_bytes(64, 32, ImageFormat::Jpeg, Rgba([30, 60, 90, 255])),
        "image/jpeg",
    )
    .await;
    for model in ["mj_fast_imagine", "mj_relax_imagine"] {
        for (status, payload, expected, kind) in [
            (
                StatusCode::NOT_FOUND,
                b"missing".as_slice(),
                StatusCode::BAD_GATEWAY,
                "image_fetch_failed",
            ),
            (
                StatusCode::OK,
                b"broken png".as_slice(),
                StatusCode::BAD_REQUEST,
                "invalid_reference_image",
            ),
        ] {
            let second = ReferenceSource::with_status(payload.to_vec(), "image/png", status).await;
            let directory = tempfile::tempdir().unwrap();
            let harness = Harness::with_image_services(
                reference_replies(),
                |_| {},
                first.client(),
                None,
                Some(spilled_runtime(directory.path())),
            )
            .await;
            let response = harness.request(json!({
                "model": model, "prompt": "中文狐狸", "image": [first.url(), second.url(), first.url()]
            })).send().await.unwrap();
            assert_eq!(response.status(), expected);
            let error: Value = response.json().await.unwrap();
            assert_eq!(error["error"]["kind"], kind);
            assert_eq!(error["error"]["stage"], "magic_prepare_image");
            assert!(
                error["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("参考图片 2")
            );
            assert!(harness.mock.requests.lock().await.is_empty());
            wait_until_empty(directory.path()).await;
        }
    }
    assert_eq!(first.requests.lock().await.len(), 4);
}

#[tokio::test]
async fn invalid_aliases_or_members_reject_the_whole_request_before_any_download() {
    let source = ReferenceSource::new(
        image_bytes(1, 1, ImageFormat::Png, Rgba([0, 0, 0, 255])),
        "image/png",
    )
    .await;
    for model in ["mj_fast_imagine", "mj_relax_imagine"] {
        let harness = source.harness().await;
        for alias in ["image", "images", "reference_images"] {
            for invalid in [
                Value::Null,
                json!("https://images.example/a.png"),
                json!([source.url(), 7]),
                json!([source.url(), "data:image/png;base64,AQID"]),
                json!([source.url(), "invalid-url"]),
            ] {
                let mut body = json!({"model": model, "prompt": "中文狐狸", "image": [source.url()], "images": [], "reference_images": []});
                body[alias] = invalid;
                let response = harness.request(body).send().await.unwrap();
                assert_eq!(response.status(), StatusCode::BAD_REQUEST);
                let error: Value = response.json().await.unwrap();
                assert_eq!(error["error"]["stage"], "magic_validate_request");
            }
        }
        assert!(harness.mock.requests.lock().await.is_empty());
    }
    assert!(source.requests.lock().await.is_empty());
}

#[tokio::test]
async fn both_modes_submit_two_reference_data_uris_in_input_order() {
    let jpeg = image_bytes(64, 32, ImageFormat::Jpeg, Rgba([80, 120, 180, 255]));
    let webp = image_bytes(32, 64, ImageFormat::WebP, Rgba([20, 40, 60, 77]));
    let first = ReferenceSource::new(jpeg.clone(), "image/png").await;
    let second = ReferenceSource::new(webp.clone(), "image/jpeg").await;
    for (model, mode, path, flag) in [
        ("mj_fast_imagine", "FAST", "/mj/submit/imagine", "--fast"),
        (
            "mj_relax_imagine",
            "RELAX",
            "/mj-relax/mj/submit/imagine",
            "--relax",
        ),
    ] {
        let harness = first.harness().await;
        let response = harness
            .request(json!({
                "model": model, "prompt": "中文陶瓷狐狸 --fast --relax --v 8.1 --ar 3:2",
                "image": [first.url(), second.url()]
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "{}",
            response.text().await.unwrap()
        );
        let requests = harness.mock.requests.lock().await;
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].uri, path);
        assert_eq!(requests[0].body["mode"], mode);
        assert_eq!(
            requests[0].body["prompt"],
            format!("中文陶瓷狐狸 --v 8.1 --ar 3:2 {flag}")
        );
        let items = requests[0].body["base64Array"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(decode_item(&items[0]), ("image/jpeg".into(), jpeg.clone()));
        assert_eq!(decode_item(&items[1]), ("image/webp".into(), webp.clone()));
    }
}

#[tokio::test]
async fn seven_merged_references_are_rejected_before_downloading_in_both_modes() {
    let source = ReferenceSource::new(
        image_bytes(1, 1, ImageFormat::Jpeg, Rgba([30, 60, 90, 255])),
        "image/jpeg",
    )
    .await;
    for model in ["mj_fast_imagine", "mj_relax_imagine"] {
        let harness = source.harness().await;
        let response = harness
            .request(json!({
                "model": model, "prompt": "中文狐狸",
                "reference_images": vec![source.url(); 3],
                "images": vec![source.url(); 3], "image": [source.url()]
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let error: Value = response.json().await.unwrap();
        assert_eq!(error["error"]["stage"], "magic_validate_request");
        assert!(error["error"]["message"].as_str().unwrap().contains("6"));
        assert!(harness.mock.requests.lock().await.is_empty());
    }
    assert!(source.requests.lock().await.is_empty());
}

#[tokio::test]
async fn six_merged_references_keep_alias_order_duplicates_and_per_image_formats() {
    let webp = image_bytes(32, 64, ImageFormat::WebP, Rgba([20, 40, 60, 77]));
    let first = ReferenceSource::new(webp.clone(), "image/png").await;
    let second = ReferenceSource::new(
        image_bytes(2049, 17, ImageFormat::Png, Rgba([20, 40, 60, 0])),
        "image/webp",
    )
    .await;
    for model in ["mj_fast_imagine", "mj_relax_imagine"] {
        let harness = first.harness().await;
        let response = harness
            .request(json!({
                "model": model, "prompt": "中文狐狸 --v 8.1",
                "image": [second.url(), first.url()], "images": [first.url(), second.url()],
                "reference_images": [second.url(), first.url()]
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let requests = harness.mock.requests.lock().await;
        let items = requests[0].body["base64Array"].as_array().unwrap();
        assert_eq!(items.len(), 6);
        let png = decode_item(&items[0]);
        assert_eq!(png.0, "image/jpeg");
        assert_jpeg_444(&png.1);
        let decoded = image::load_from_memory(&png.1).unwrap();
        assert_eq!(decoded.dimensions(), (2048, 17));
        assert!(
            decoded
                .to_rgb8()
                .get_pixel(10, 5)
                .0
                .iter()
                .all(|&channel| channel >= 250)
        );
        assert_eq!(items[0], items[3]);
        assert_eq!(items[0], items[4]);
        for index in [1, 2, 5] {
            assert_eq!(
                decode_item(&items[index]),
                ("image/webp".into(), webp.clone())
            );
        }
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
        let logged: Value =
            serde_json::from_str(logs["items"][0]["requestUpstream"].as_str().unwrap()).unwrap();
        let summaries = logged["base64Array"].as_array().unwrap();
        assert_eq!(summaries.len(), 6);
        for (index, mime) in [
            "image/jpeg",
            "image/webp",
            "image/webp",
            "image/jpeg",
            "image/jpeg",
            "image/webp",
        ]
        .iter()
        .enumerate()
        {
            assert!(summaries[index].as_str().unwrap().contains(mime));
        }
        assert!(!logs.to_string().contains("data:image/"));
        assert!(!logs.to_string().contains(&STANDARD.encode(&webp)));
    }
}

#[tokio::test]
async fn timeout_on_later_reference_identifies_the_image_and_cleans_earlier_blobs() {
    use super::lifecycle_tests::{finished, spilled_runtime, wait_until_empty};
    let first = ReferenceSource::new(
        image_bytes(64, 32, ImageFormat::Jpeg, Rgba([30, 60, 90, 255])),
        "image/jpeg",
    )
    .await;
    let second = ReferenceSource::new(
        image_bytes(32, 64, ImageFormat::WebP, Rgba([20, 40, 60, 77])),
        "image/webp",
    )
    .await;
    *second.gate.lock().await = Some(Arc::new(Notify::new()));
    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::with_image_services(
        reference_replies(),
        |config| config.upstream_timeout = Duration::from_secs(5),
        first.client(),
        None,
        Some(spilled_runtime(directory.path())),
    )
    .await;
    let request = harness.request(json!({
        "model": "mj_relax_imagine", "prompt": "中文狐狸 --v 8.1",
        "image": [first.url(), second.url(), first.url()]
    }));
    let task = tokio::spawn(async move { request.send().await.unwrap() });
    second.wait_for_downloads(1).await;
    assert!(
        std::fs::read_dir(directory.path())
            .unwrap()
            .next()
            .is_some()
    );
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(5) + Duration::from_millis(1)).await;
    finished(&task).await;
    let response = task.await.unwrap();
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    let error: Value = response.json().await.unwrap();
    assert_eq!(error["error"]["stage"], "magic_prepare_image");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("参考图片 2"),
        "{error}"
    );
    assert!(harness.mock.requests.lock().await.is_empty());
    assert_eq!(first.requests.lock().await.len(), 1);
    wait_until_empty(directory.path()).await;
}
