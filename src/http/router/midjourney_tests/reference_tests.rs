use super::*;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use image::{DynamicImage, GenericImageView, ImageFormat, Rgba, RgbaImage};
use std::io::Cursor;
use std::net::SocketAddr;

mod cache_tests;
mod lifecycle_tests;

struct ReferenceSource {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<HeaderMap>>>,
    gate: Arc<Mutex<Option<Arc<Notify>>>>,
    server: JoinHandle<()>,
}

impl Drop for ReferenceSource {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl ReferenceSource {
    async fn new(bytes: Vec<u8>, mime: &'static str) -> Self {
        Self::with_status(bytes, mime, StatusCode::OK).await
    }

    async fn with_status(bytes: Vec<u8>, mime: &'static str, status: StatusCode) -> Self {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captures = requests.clone();
        let gate = Arc::new(Mutex::new(None::<Arc<Notify>>));
        let source_gate = gate.clone();
        let app = Router::new().fallback(move |request: Request| {
            let bytes = bytes.clone();
            let captures = captures.clone();
            let gate = source_gate.clone();
            async move {
                captures.lock().await.push(request.headers().clone());
                let gate = gate.lock().await.clone();
                if let Some(gate) = gate {
                    gate.notified().await;
                }
                (status, [(CONTENT_TYPE, mime)], bytes).into_response()
            }
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            address,
            requests,
            gate,
            server,
        }
    }

    fn url(&self) -> String {
        format!(
            "http://images.example:{}/reference.webp",
            self.address.port()
        )
    }

    async fn harness(&self) -> Harness {
        Harness::with_image_client(reference_replies(), |_| {}, self.client()).await
    }

    fn client(&self) -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .resolve("images.example", self.address)
            .build()
            .unwrap()
    }

    async fn wait_for_downloads(&self, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while self.requests.lock().await.len() < count {
            assert!(
                Instant::now() < deadline,
                "did not receive {count} image downloads"
            );
            tokio::task::yield_now().await;
        }
        settle().await;
    }
}

fn reference_replies() -> Vec<Value> {
    vec![
        json!({"code": 1, "result": "reference-task"}),
        json!({"status": "SUCCESS", "imageUrl": "https://result.example/grid.png"}),
    ]
}

fn generation(source: &ReferenceSource) -> Value {
    json!({"model": "mj_fast_imagine", "prompt": "中文狐狸 --v 8.1", "image": [source.url()]})
}

fn image_bytes(width: u32, height: u32, format: ImageFormat, pixel: Rgba<u8>) -> Vec<u8> {
    let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(width, height, pixel));
    let image = if format == ImageFormat::Jpeg {
        DynamicImage::ImageRgb8(image.to_rgb8())
    } else {
        image
    };
    let mut encoded = Cursor::new(Vec::new());
    image.write_to(&mut encoded, format).unwrap();
    encoded.into_inner()
}

fn decode_reference(request: &CapturedRequest) -> (String, Vec<u8>) {
    let items = request.body["base64Array"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    let (prefix, data) = items[0].as_str().unwrap().split_once(',').unwrap();
    let mime = prefix
        .strip_prefix("data:")
        .unwrap()
        .strip_suffix(";base64")
        .unwrap();
    (mime.to_string(), STANDARD.decode(data).unwrap())
}

#[tokio::test]
async fn png_reference_is_submitted_as_white_background_jpeg_from_http() {
    let source = ReferenceSource::new(
        image_bytes(64, 32, ImageFormat::Png, Rgba([20, 40, 60, 0])),
        "image/webp",
    )
    .await;
    let harness = source.harness().await;
    let response = harness
        .request(json!({
            "model": "mj_fast_imagine", "prompt": "中文狐狸 --v 8.1", "image": [source.url()]
        }))
        .header(AUTHORIZATION, "Bearer selected-key")
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
    assert_eq!(requests[0].body.as_object().unwrap().len(), 3);
    assert_eq!(requests[0].body["mode"], "FAST");
    assert_eq!(requests[0].body["prompt"], "中文狐狸 --v 8.1 --fast");
    let (mime, bytes) = decode_reference(&requests[0]);
    assert_eq!(mime, "image/jpeg");
    assert_jpeg_444(&bytes);
    let image = image::load_from_memory(&bytes).unwrap();
    assert_eq!(image.dimensions(), (64, 32));
    assert!(
        image
            .to_rgb8()
            .get_pixel(20, 10)
            .0
            .iter()
            .all(|&channel| channel >= 250)
    );
    let downloads = source.requests.lock().await;
    assert_eq!(downloads.len(), 1);
    assert!(!downloads[0].contains_key(AUTHORIZATION));
}

fn assert_jpeg_444(bytes: &[u8]) {
    assert_eq!(&bytes[..2], &[0xff, 0xd8]);
    let mut offset = 2;
    while offset + 4 <= bytes.len() {
        assert_eq!(bytes[offset], 0xff);
        let marker = bytes[offset + 1];
        let length = u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]) as usize;
        if matches!(marker, 0xc0 | 0xc2) {
            let frame = &bytes[offset + 4..offset + 2 + length];
            assert_eq!(frame[5], 3);
            assert_eq!([frame[7], frame[10], frame[13]], [0x11, 0x11, 0x11]);
            return;
        }
        offset += 2 + length;
    }
    panic!("JPEG is missing a supported frame marker");
}

#[tokio::test]
async fn jpeg_references_keep_small_bytes_and_resize_only_above_2048() {
    for (input, expected) in [
        ((2047, 100), (2047, 100)),
        ((2048, 100), (2048, 100)),
        ((2049, 1025), (2048, 1024)),
        ((1025, 2049), (1024, 2048)),
        ((4097, 1), (2048, 1)),
        ((1, 4097), (1, 2048)),
    ] {
        let original = image_bytes(
            input.0,
            input.1,
            ImageFormat::Jpeg,
            Rgba([80, 120, 180, 255]),
        );
        let source = ReferenceSource::new(original.clone(), "image/png").await;
        let harness = source.harness().await;
        let response = harness.request(generation(&source)).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "input={input:?}");
        let requests = harness.mock.requests.lock().await;
        let (mime, bytes) = decode_reference(&requests[0]);
        assert_eq!(mime, "image/jpeg");
        assert_eq!(
            image::load_from_memory(&bytes).unwrap().dimensions(),
            expected
        );
        if input == expected {
            assert_eq!(bytes, original);
        } else {
            assert_jpeg_444(&bytes);
        }
    }
}

#[tokio::test]
async fn webp_references_preserve_bytes_or_resize_losslessly_with_alpha() {
    for (input, expected) in [((100, 64), (100, 64)), ((2049, 17), (2048, 17))] {
        let original = image_bytes(input.0, input.1, ImageFormat::WebP, Rgba([35, 70, 105, 77]));
        let source = ReferenceSource::new(original.clone(), "application/octet-stream").await;
        let harness = source.harness().await;
        let response = harness.request(generation(&source)).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let requests = harness.mock.requests.lock().await;
        let (mime, bytes) = decode_reference(&requests[0]);
        assert_eq!(mime, "image/webp");
        let decoded = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), expected);
        assert_eq!(decoded.get_pixel(10, 5).0, [35, 70, 105, 77]);
        if input == expected {
            assert_eq!(bytes, original);
        }
    }
}

#[tokio::test]
async fn tall_png_shrinks_with_nearest_proportions_before_jpeg_conversion() {
    let source = ReferenceSource::new(
        image_bytes(17, 2049, ImageFormat::Png, Rgba([20, 40, 60, 0])),
        "image/png",
    )
    .await;
    let harness = source.harness().await;
    let response = harness.request(generation(&source)).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let requests = harness.mock.requests.lock().await;
    let (mime, bytes) = decode_reference(&requests[0]);
    assert_eq!(mime, "image/jpeg");
    assert_eq!(
        image::load_from_memory(&bytes).unwrap().dimensions(),
        (17, 2048)
    );
    assert_jpeg_444(&bytes);
}

#[tokio::test]
async fn aliases_and_relax_reuse_the_single_reference_pipeline() {
    let source = ReferenceSource::new(
        image_bytes(8, 8, ImageFormat::Png, Rgba([255, 0, 0, 128])),
        "image/png",
    )
    .await;
    for alias in ["image", "images", "reference_images"] {
        let harness = source.harness().await;
        let mut body = json!({"model": "mj_relax_imagine", "prompt": "中文狐狸 --fast --v 8.1", "image": [], "images": [], "reference_images": []});
        body[alias] = json!([source.url()]);
        let response = harness.request(body).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let requests = harness.mock.requests.lock().await;
        assert_eq!(requests[0].uri, "/mj-relax/mj/submit/imagine");
        assert_eq!(requests[0].body["mode"], "RELAX");
        assert_eq!(requests[0].body["prompt"], "中文狐狸 --v 8.1 --relax");
        let (mime, bytes) = decode_reference(&requests[0]);
        assert_eq!(mime, "image/jpeg");
        assert_jpeg_444(&bytes);
        let rgb = image::load_from_memory(&bytes).unwrap().to_rgb8();
        let pixel = rgb.get_pixel(4, 4).0;
        assert!(
            pixel[0] >= 250 && (125..=130).contains(&pixel[1]) && (125..=130).contains(&pixel[2])
        );
    }
}

#[tokio::test]
async fn merged_multiple_images_and_private_base64_are_rejected_before_any_download() {
    let source = ReferenceSource::new(
        image_bytes(1, 1, ImageFormat::Png, Rgba([0, 0, 0, 255])),
        "image/png",
    )
    .await;
    let harness = source.harness().await;
    let mut cases = vec![
        json!({"image": [source.url(), source.url()]}),
        json!({"reference_images": [source.url()], "images": [source.url()]}),
        json!({"images": [source.url()], "image": [source.url()]}),
    ];
    for value in [
        Value::Null,
        json!([]),
        json!(["data:image/jpeg;base64,AQID"]),
        json!("AQID"),
    ] {
        cases.push(json!({"base64Array": value}));
    }
    for fields in cases {
        let mut body = json!({"model": "mj_fast_imagine", "prompt": "中文狐狸"});
        body.as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        let response = harness.request(body).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response.json::<Value>().await.unwrap()["error"]["stage"],
            "magic_validate_request"
        );
    }
    assert!(source.requests.lock().await.is_empty());
    assert!(harness.mock.requests.lock().await.is_empty());
}

#[tokio::test]
async fn unsupported_and_corrupt_references_fail_before_creating_magic_tasks() {
    for bytes in [
        b"GIF89a\x01\x00\x01\x00\x00\x00\x00".to_vec(),
        b"\x89PNG\r\n\x1a\ncorrupt-image".to_vec(),
        b"ordinary text".to_vec(),
    ] {
        let source = ReferenceSource::new(bytes, "image/jpeg").await;
        let harness = source.harness().await;
        let response = harness.request(generation(&source)).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let error: Value = response.json().await.unwrap();
        assert_eq!(error["error"]["stage"], "magic_prepare_image");
        assert_eq!(error["error"]["kind"], "invalid_reference_image");
        assert!(harness.mock.requests.lock().await.is_empty());
    }
}

#[tokio::test]
async fn missing_oversized_and_forbidden_image_sources_fail_without_a_task() {
    for (bytes, status) in [
        (b"missing".to_vec(), StatusCode::NOT_FOUND),
        (
            vec![0; crate::image_io::REQUEST_MAX_IMAGE_BYTES + 1],
            StatusCode::OK,
        ),
    ] {
        let source = ReferenceSource::with_status(bytes, "image/jpeg", status).await;
        let harness = source.harness().await;
        let response = harness.request(generation(&source)).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let error: Value = response.json().await.unwrap();
        assert_eq!(error["error"]["stage"], "magic_prepare_image");
        assert_eq!(error["error"]["kind"], "image_fetch_failed");
        assert!(harness.mock.requests.lock().await.is_empty());
    }
    let harness = Harness::new(vec![]).await;
    let response = harness.request(json!({"model": "mj_fast_imagine", "prompt": "中文狐狸", "image": ["http://127.0.0.1:9/reference.png"]})).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert!(harness.mock.requests.lock().await.is_empty());
}
