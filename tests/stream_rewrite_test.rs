use anyhow::Result;
use serde_json::json;

#[test]
fn rewrites_sse_data_chunks_without_buffering_whole_stream() -> Result<()> {
    let input = concat!(
        "event: message\n",
        "data: {\"candidates\":[{\"content\":{\"parts\":[{\"inlineData\":{\"mimeType\":\"image/png\",\"data\":\"aaaa\"}},{\"inlineData\":{\"mimeType\":\"image/png\",\"data\":\"aaaaaaaa\"}}]}}]}\n",
        "\n",
        "data: [DONE]\n"
    );

    let output = rust_sync_proxy::stream_rewrite::rewrite_sse_text_with(input, |value| {
        Ok(rust_sync_proxy::response_rewrite::keep_largest_inline_image(value))
    })?;

    assert!(output.contains("event: message\n"));
    assert!(output.contains("data: [DONE]\n"));
    assert!(output.contains("\"data\":\"aaaaaaaa\""));
    assert!(!output.contains("\"data\":\"aaaa\""));
    Ok(())
}

#[test]
fn preserves_non_json_sse_data_lines() -> Result<()> {
    let input = "data: not-json\n";
    let output = rust_sync_proxy::stream_rewrite::rewrite_sse_text_with(input, |value| {
        Ok(json!({ "wrapped": value }))
    })?;
    assert_eq!(output, input);
    Ok(())
}
