use super::*;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

pub(super) fn spilled_runtime(directory: &std::path::Path) -> Arc<BlobRuntime> {
    Arc::new(BlobRuntime::new(BlobRuntimeConfig {
        inline_max_bytes: 8,
        request_hot_budget_bytes: 8,
        global_hot_budget_bytes: 8,
        spill_dir: directory.to_path_buf(),
    }))
}

pub(super) async fn finished<T>(task: &JoinHandle<T>) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !task.is_finished() {
        assert!(Instant::now() < deadline, "HTTP request did not finish");
        tokio::task::yield_now().await;
    }
}

pub(super) async fn wait_until_empty(directory: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while std::fs::read_dir(directory).unwrap().next().is_some() {
        assert!(
            Instant::now() < deadline,
            "request spill files were not removed"
        );
        tokio::task::yield_now().await;
    }
}

async fn open_generation_socket(harness: &Harness, body: Value) -> TcpStream {
    let address = harness.base_url.strip_prefix("http://").unwrap();
    let mut socket = TcpStream::connect(address).await.unwrap();
    let body = body.to_string();
    socket.write_all(format!(
        "POST /v1/images/generations HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()
    ).as_bytes()).await.unwrap();
    socket
}

#[tokio::test]
async fn image_preparation_timeout_preserves_stage_and_never_submits() {
    let source = ReferenceSource::new(
        image_bytes(64, 32, ImageFormat::Jpeg, Rgba([30, 60, 90, 255])),
        "image/jpeg",
    )
    .await;
    *source.gate.lock().await = Some(Arc::new(Notify::new()));
    let harness = Harness::with_image_client(
        reference_replies(),
        |config| {
            config.upstream_timeout = Duration::from_secs(5);
        },
        source.client(),
    )
    .await;
    let request = harness.request(generation(&source));
    let task = tokio::spawn(async move { request.send().await.unwrap() });
    source.wait_for_downloads(1).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(5) + Duration::from_millis(1)).await;
    finished(&task).await;
    let response = task.await.unwrap();
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    let error: Value = response.json().await.unwrap();
    assert_eq!(error["error"]["stage"], "magic_prepare_image");
    assert_eq!(error["error"]["kind"], "timeout");
    assert!(harness.mock.requests.lock().await.is_empty());
}

#[tokio::test]
async fn preparation_submission_and_polling_share_the_same_deadline_and_release_spills() {
    let source = ReferenceSource::new(
        image_bytes(64, 32, ImageFormat::Jpeg, Rgba([30, 60, 90, 255])),
        "image/jpeg",
    )
    .await;
    let image_gate = Arc::new(Notify::new());
    *source.gate.lock().await = Some(image_gate.clone());
    let directory = tempfile::tempdir().unwrap();
    let runtime = spilled_runtime(directory.path());
    let harness = Harness::with_image_services(
        reference_replies(),
        |config| {
            config.upstream_timeout = Duration::from_secs(30);
        },
        source.client(),
        None,
        Some(runtime),
    )
    .await;
    harness
        .mock
        .gates
        .lock()
        .await
        .insert(1, Arc::new(Notify::new()));
    let mut body = generation(&source);
    body["image"] = json!([source.url(), source.url()]);
    let request = harness.request(body);
    let task = tokio::spawn(async move { request.send().await.unwrap() });
    source.wait_for_downloads(1).await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(20)).await;
    *source.gate.lock().await = None;
    image_gate.notify_one();
    harness.wait_for_requests(2).await;
    assert!(
        std::fs::read_dir(directory.path())
            .unwrap()
            .next()
            .is_none()
    );
    tokio::time::advance(Duration::from_secs(9)).await;
    settle().await;
    assert!(!task.is_finished());
    tokio::time::advance(Duration::from_secs(1) + Duration::from_millis(1)).await;
    finished(&task).await;
    let response = task.await.unwrap();
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    let error: Value = response.json().await.unwrap();
    assert_eq!(error["error"]["stage"], "magic_poll");
    assert_eq!(error["error"]["task_id"], "reference-task");
    assert_eq!(harness.mock.requests.lock().await.len(), 2);
    wait_until_empty(directory.path()).await;
}

#[tokio::test]
async fn reference_decode_failure_releases_the_downloaded_blob() {
    let source = ReferenceSource::new(b"invalid image payload".to_vec(), "image/png").await;
    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::with_image_services(
        reference_replies(),
        |_| {},
        source.client(),
        None,
        Some(spilled_runtime(directory.path())),
    )
    .await;
    let response = harness.request(generation(&source)).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(harness.mock.requests.lock().await.is_empty());
    wait_until_empty(directory.path()).await;
}

#[tokio::test]
async fn downstream_disconnect_during_submission_releases_all_large_reference_and_request_blobs() {
    let mut jpeg = image_bytes(64, 32, ImageFormat::Jpeg, Rgba([30, 60, 90, 255]));
    jpeg.resize(128 * 1024, 0);
    let source = ReferenceSource::new(jpeg.clone(), "image/jpeg").await;
    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::with_image_services(
        reference_replies(),
        |_| {},
        source.client(),
        None,
        Some(spilled_runtime(directory.path())),
    )
    .await;
    harness
        .mock
        .gates
        .lock()
        .await
        .insert(0, Arc::new(Notify::new()));
    let mut body = generation(&source);
    body["image"] = json!([source.url(), source.url()]);
    let socket = open_generation_socket(&harness, body).await;
    harness.wait_for_requests(1).await;
    {
        let requests = harness.mock.requests.lock().await;
        let items = requests[0].body["base64Array"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], items[1]);
        assert_eq!(super::multiple_tests::decode_item(&items[1]).1, jpeg);
    }
    assert!(
        std::fs::read_dir(directory.path())
            .unwrap()
            .next()
            .is_some()
    );
    drop(socket);
    wait_until_empty(directory.path()).await;
    let deadline = Instant::now() + Duration::from_secs(3);
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
        if let Some(entry) = logs["items"].as_array().unwrap().first() {
            assert_eq!(entry["statusCode"], 499);
            assert_eq!(entry["errorStage"], "magic_submit");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "cancelled request was not recorded"
        );
        tokio::task::yield_now().await;
    }
    assert_eq!(harness.mock.requests.lock().await.len(), 1);
}

#[tokio::test]
async fn concurrent_reference_preparations_are_bounded_and_cancellation_releases_a_slot() {
    let source = ReferenceSource::new(
        image_bytes(1, 1, ImageFormat::Jpeg, Rgba([30, 60, 90, 255])),
        "image/jpeg",
    )
    .await;
    *source.gate.lock().await = Some(Arc::new(Notify::new()));
    let harness = source.harness().await;
    let mut sockets = Vec::new();
    for _ in 0..5 {
        sockets.push(open_generation_socket(&harness, generation(&source)).await);
    }
    source.wait_for_downloads(4).await;
    assert_eq!(source.requests.lock().await.len(), 4);
    drop(sockets.remove(0));
    source.wait_for_downloads(5).await;
    drop(sockets);
    settle().await;
    assert!(harness.mock.requests.lock().await.is_empty());
}

#[tokio::test]
async fn disconnect_during_a_later_download_cleans_prior_images_and_keeps_shared_services_alive() {
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
        |_| {},
        first.client(),
        None,
        Some(spilled_runtime(directory.path())),
    )
    .await;
    let socket = open_generation_socket(
        &harness,
        json!({
            "model": "mj_relax_imagine", "prompt": "中文狐狸",
            "image": [first.url(), second.url(), first.url()]
        }),
    )
    .await;
    second.wait_for_downloads(1).await;
    assert!(
        std::fs::read_dir(directory.path())
            .unwrap()
            .next()
            .is_some()
    );
    drop(socket);
    wait_until_empty(directory.path()).await;
    assert!(harness.mock.requests.lock().await.is_empty());
    assert_eq!(first.requests.lock().await.len(), 1);
    let response = harness.request(generation(&first)).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    wait_until_empty(directory.path()).await;
    assert_eq!(harness.mock.requests.lock().await.len(), 2);
}

#[tokio::test]
async fn unavailable_spill_storage_fails_reference_preparation_without_a_submission() {
    let source = ReferenceSource::new(
        image_bytes(64, 32, ImageFormat::Jpeg, Rgba([30, 60, 90, 255])),
        "image/jpeg",
    )
    .await;
    let directory = tempfile::tempdir().unwrap();
    let blocked = directory.path().join("not-a-directory");
    std::fs::write(&blocked, b"occupied").unwrap();
    let harness = Harness::with_image_services(
        reference_replies(),
        |_| {},
        source.client(),
        None,
        Some(spilled_runtime(&blocked)),
    )
    .await;
    let response = harness
        .request(json!({
            "model": "mj_fast_imagine", "prompt": "中文狐狸", "image": [source.url(), source.url()]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let error: Value = response.json().await.unwrap();
    assert_eq!(error["error"]["stage"], "magic_prepare_image");
    assert_eq!(error["error"]["kind"], "image_storage_failed");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("参考图片 1")
    );
    assert!(harness.mock.requests.lock().await.is_empty());
    assert_eq!(source.requests.lock().await.len(), 1);
}
