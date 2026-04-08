#[tokio::test]
async fn rejects_images_over_max_size() {
    let result =
        rust_sync_proxy::image_io::enforce_max_size(10 * 1024 * 1024 + 1, 10 * 1024 * 1024);
    assert!(result.is_err());
}
