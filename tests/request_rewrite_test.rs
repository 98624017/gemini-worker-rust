use serde_json::json;

#[test]
fn collects_unique_inline_data_urls_and_enforces_limit() {
    let body = json!({
        "contents": [{
            "parts": [
                {"inlineData": {"data": "https://img.example/a.png"}},
                {"inlineData": {"data": "https://img.example/a.png"}}
            ]
        }]
    });

    let scan = rust_sync_proxy::request_rewrite::scan_inline_data_urls(&body).unwrap();
    assert_eq!(scan.unique_urls.len(), 1);
    assert_eq!(scan.total_refs, 2);
}
