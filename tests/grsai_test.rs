use axum::http::StatusCode;
use serde_json::json;

use rust_sync_proxy::grsai::{
    GrsaiSource, build_grsai_request_body, extract_gemini_params, extract_openai_params,
    normalize_model, parse_grsai_response,
};

#[test]
fn gemini_model_mapping_matches_go_provider() {
    assert_eq!(
        normalize_model("gemini-3-pro-image-preview", GrsaiSource::Gemini),
        "nano-banana-pro"
    );
    assert_eq!(
        normalize_model("gemini-2.5-flash-image", GrsaiSource::Gemini),
        "nano-banana-fast"
    );
    assert_eq!(
        normalize_model("gemini-3.1-flash-image-preview", GrsaiSource::Gemini),
        "nano-banana-2"
    );
    assert_eq!(normalize_model("", GrsaiSource::Gemini), "nano-banana-fast");
    assert_eq!(
        normalize_model("custom-model", GrsaiSource::Gemini),
        "custom-model"
    );
}

#[test]
fn openai_model_mapping_defaults_only_when_empty() {
    assert_eq!(normalize_model("", GrsaiSource::OpenAi), "nano-banana-fast");
    assert_eq!(
        normalize_model("gpt-image-2", GrsaiSource::OpenAi),
        "gpt-image-2"
    );
}

#[test]
fn gemini_params_extract_user_prompt_urls_and_image_config() {
    let body = json!({
        "contents": [
            {
                "role": "model",
                "parts": [{"text": "ignored"}]
            },
            {
                "role": "user",
                "parts": [
                    {"text": "first line"},
                    {"inlineData": {"mimeType": "image/png", "data": "https://img.example.com/a.png"}},
                    {"text": "second line"}
                ]
            }
        ],
        "generationConfig": {
            "imageConfig": {
                "aspectRatio": "16:9",
                "imageSize": "2K",
                "output": "url"
            }
        }
    });

    let params = extract_gemini_params(
        &body,
        "gemini-3-pro-image-preview",
        Some("aspectRatio=1:1&image_size=4K&output=url"),
    )
    .unwrap();

    assert_eq!(params.model, "nano-banana-pro");
    assert_eq!(params.prompt, "first line\nsecond line");
    assert_eq!(params.urls, vec!["https://img.example.com/a.png"]);
    assert_eq!(params.aspect_ratio, "1:1");
    assert_eq!(params.image_size, "4K");
    assert_eq!(params.output, "url");
}

#[test]
fn openai_params_accept_aliases_and_defaults() {
    let body = json!({
        "model_name": "",
        "prompt": "draw a pear",
        "images": ["https://img.example.com/ref.png"]
    });

    let params = extract_openai_params(&body).unwrap();

    assert_eq!(params.model, "nano-banana-fast");
    assert_eq!(params.prompt, "draw a pear");
    assert_eq!(params.urls, vec!["https://img.example.com/ref.png"]);
    assert_eq!(params.aspect_ratio, "auto");
    assert_eq!(params.image_size, "1K");
}

#[test]
fn request_body_matches_go_grsai_provider() {
    let params = extract_openai_params(&json!({
        "model": "nano-banana-fast",
        "prompt": "draw",
        "urls": ["https://img.example.com/ref.png"],
        "aspect_ratio": "auto",
        "image_size": "1K"
    }))
    .unwrap();

    let request_body = build_grsai_request_body(&params);

    assert_eq!(request_body["model"], "nano-banana-fast");
    assert_eq!(request_body["prompt"], "draw");
    assert_eq!(
        request_body["urls"],
        json!(["https://img.example.com/ref.png"])
    );
    assert_eq!(request_body["aspectRatio"], "auto");
    assert_eq!(request_body["imageSize"], "1K");
    assert_eq!(request_body["shutProgress"], true);
}

#[test]
fn parses_json_success_response() {
    let body = br#"{"code":0,"msg":"success","data":{"status":"succeeded","results":[{"url":"https://api.grsai.com/img/123.png"}],"start_time":10,"end_time":12}}"#;

    let parsed = parse_grsai_response(StatusCode::OK, body).unwrap();

    assert_eq!(parsed.status, "succeeded");
    assert_eq!(parsed.image_urls, vec!["https://api.grsai.com/img/123.png"]);
    assert_eq!(parsed.start_time, Some(10));
    assert_eq!(parsed.end_time, Some(12));
}

#[test]
fn parses_sse_success_response_from_last_data_line() {
    let body = b"data: {\"code\":0,\"msg\":\"progress\",\"data\":{\"status\":\"running\"}}\n\ndata: {\"code\":0,\"msg\":\"success\",\"data\":{\"status\":\"succeeded\",\"results\":[{\"url\":\"https://api.grsai.com/img/456.png\"}]}}\n\ndata: [DONE]\n";

    let parsed = parse_grsai_response(StatusCode::OK, body).unwrap();

    assert_eq!(parsed.status, "succeeded");
    assert_eq!(parsed.image_urls, vec!["https://api.grsai.com/img/456.png"]);
}

#[test]
fn parses_business_error_as_grsai_error() {
    let body = br#"{"code":401,"msg":"invalid api key","data":{"failure_reason":"auth"}}"#;

    let err = parse_grsai_response(StatusCode::OK, body).unwrap_err();

    assert_eq!(err.http_status, StatusCode::UNAUTHORIZED);
    assert_eq!(err.message, "invalid api key");
    assert_eq!(err.upstream_code, Some(401));
    assert_eq!(err.failure_reason.as_deref(), Some("auth"));
}

#[test]
fn parse_error_when_no_data_lines_exist() {
    let err = parse_grsai_response(StatusCode::OK, b"hello").unwrap_err();

    assert_eq!(err.http_status, StatusCode::BAD_GATEWAY);
    assert!(err.message.contains("解析上游服务响应失败"));
}
