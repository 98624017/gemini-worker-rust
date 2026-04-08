#[test]
fn fixture_outputs_match_documented_go_behavior() {
    let fixture = include_str!("fixtures/response_multi_image.json");
    let output = rust_sync_proxy::response_rewrite::keep_largest_inline_image(
        serde_json::from_str(fixture).unwrap(),
    );
    assert_eq!(
        output["candidates"][0]["content"]["parts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn stream_fixture_is_rewriteable() {
    let fixture = include_str!("fixtures/stream_sample.txt");
    let output = rust_sync_proxy::stream_rewrite::rewrite_sse_text_with(fixture, |value| {
        Ok(rust_sync_proxy::response_rewrite::keep_largest_inline_image(value))
    })
    .unwrap();
    assert!(output.contains("\"data\":\"bbbbbbbb\""));
}
