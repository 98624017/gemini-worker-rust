use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::routing::post;
use axum::{Json, http::HeaderMap};
use serde_json::json;
use tokio::net::TcpListener;

#[derive(Clone, Default)]
struct UploadCapture;

#[tokio::test]
async fn output_url_response_rewrites_uploaded_url_with_external_proxy_prefix() {
    let upload_server = Router::new()
        .route("/uguu", post(mock_legacy_upload))
        .with_state(UploadCapture);
    let upload_addr = spawn_server(upload_server).await;

    let mut config = rust_sync_proxy::test_config();
    config.image_host_mode = "legacy".to_string();
    config.legacy_uguu_upload_url = format!("http://{upload_addr}/uguu");
    let uploader = rust_sync_proxy::upload::Uploader::new(reqwest::Client::new(), config);

    let runtime = rust_sync_proxy::test_blob_runtime(8 * 1024 * 1024);
    let mut body = json!({
        "candidates": [{
            "content": {
                "parts": [{
                    "inlineData": {
                        "mimeType": "image/png",
                        "data": "AQID"
                    }
                }]
            }
        }]
    });

    rust_sync_proxy::response_materialize::finalize_output_urls(
        &mut body,
        &runtime,
        &uploader,
        "https://external-proxy.example/fetch?url=",
    )
    .await
    .unwrap();

    assert_eq!(
        body["candidates"][0]["content"]["parts"][0]["inlineData"]["data"],
        "https://external-proxy.example/fetch?url=https%3A%2F%2Fimg.example.com%2Fa.png"
    );
}

async fn mock_legacy_upload(
    State(_capture): State<UploadCapture>,
    _headers: HeaderMap,
    _body: Bytes,
) -> Json<serde_json::Value> {
    Json(json!({
        "success": true,
        "files": [{
            "url": "https://img.example.com/a.png"
        }]
    }))
}

async fn spawn_server(app: Router) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    address
}
