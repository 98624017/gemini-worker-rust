use rust_sync_proxy::mai_token::{
    MAX_REFERENCE_IMAGES, TaskState, build_request_body, is_seedream_55_request, parse_task_state,
    task_id,
};
use serde_json::json;

#[test]
fn builds_legacy_seedream_request_and_preserves_seed_zero() {
    let body = json!({
        "model": "seedream-5.5",
        "prompt": "draw a glass house",
        "n": 1,
        "size": "1k",
        "aspect_ratio": "5:4",
        "reference_images": ["https://img.example/reference.png"],
        "images": ["https://img.example/second.png"],
        "image": ["https://img.example/third.png"],
        "seed": 0,
        "watermark": false,
        "response_format": "url",
        "duration": 10
    });

    assert!(is_seedream_55_request(&body));
    assert!(!is_seedream_55_request(
        &json!({"model": "seedream-5.5-preview"})
    ));

    let request = build_request_body(&body).unwrap();
    assert_eq!(
        request,
        json!({
            "model": "seedream-5.5",
            "prompt": "draw a glass house",
            "n": 1,
            "aspect_ratio": "5:4",
            "quality": "1440p",
            "image_urls": [
                "https://img.example/reference.png",
                "https://img.example/second.png",
                "https://img.example/third.png"
            ],
            "seed": 0
        })
    );
    assert!(request.get("contract_version").is_none());
    assert!(request.get("async").is_none());
    assert!(request.get("resolution").is_none());
    assert!(request.get("ratio").is_none());
    assert!(request.get("references").is_none());
    assert!(request.get("watermark").is_none());
    assert!(request.get("response_format").is_none());
    assert!(request.get("duration").is_none());
}

#[test]
fn supports_auto_and_fixed_ratio_variants() {
    for ratio in [
        "1:1", "16:9", "9:16", "4:3", "3:4", "5:4", "4:5", "3:2", "2:3", "21:9", "auto",
    ] {
        let request = build_request_body(&json!({
            "model": "seedream-5.5",
            "prompt": "draw a tree",
            "size": "2K",
            "aspect_ratio": ratio,
        }))
        .unwrap();
        assert_eq!(request["aspect_ratio"], ratio);
        assert_eq!(request["quality"], "1440p");
    }
}

#[test]
fn rejects_invalid_request_fields_and_reference_urls() {
    let invalid = [
        json!({"model": "seedream-5.5", "prompt": ""}),
        json!({"model": "seedream-5.5", "prompt": "   "}),
        json!({"model": "seedream-5.5", "prompt": "x", "size": "4K"}),
        json!({"model": "seedream-5.5", "prompt": "x", "n": 2}),
        json!({"model": "seedream-5.5", "prompt": "x", "n": "1"}),
        json!({"model": "seedream-5.5", "prompt": "x", "n": 1.0}),
        json!({"model": "seedream-5.5", "prompt": "x", "n": 1.5}),
        json!({"model": "seedream-5.5", "prompt": "x", "aspect_ratio": "10:1"}),
        json!({"model": "seedream-5.5", "prompt": "x", "aspect_ratio": 1}),
        json!({"model": "seedream-5.5", "prompt": "x", "seed": 1.5}),
        json!({"model": "seedream-5.5", "prompt": "x", "image": ["file:///tmp/a.png"]}),
        json!({"model": "seedream-5.5", "prompt": "x", "image": ["https://"]}),
    ];
    for body in invalid {
        assert!(
            build_request_body(&body).is_err(),
            "unexpectedly accepted {body}"
        );
    }

    let too_many = json!({
        "model": "seedream-5.5",
        "prompt": "x",
        "reference_images": (0..=MAX_REFERENCE_IMAGES)
            .map(|index| format!("https://img.example/{index}.png"))
            .collect::<Vec<_>>(),
    });
    assert!(build_request_body(&too_many).is_err());
}

#[test]
fn extracts_task_id_from_both_supported_aliases() {
    assert_eq!(task_id(&json!({"id": "task-1"})).unwrap(), "task-1");
    assert_eq!(task_id(&json!({"task_id": "task-2"})).unwrap(), "task-2");
    assert!(task_id(&json!({"id": "", "task_id": null})).is_err());
}

#[test]
fn parses_pending_completed_and_failed_states() {
    for status in ["queued", "in_progress", "processing"] {
        assert_eq!(
            parse_task_state(&json!({"status": status})).unwrap(),
            TaskState::Pending
        );
    }
    assert_eq!(
        parse_task_state(&json!({"status": "unknown"})).unwrap(),
        TaskState::Unknown
    );
    assert_eq!(
        parse_task_state(&json!({"status": "processing", "progress": 100})).unwrap(),
        TaskState::Pending
    );

    assert_eq!(
        parse_task_state(&json!({
            "status": "completed",
            "metadata": {"url": "https://img.example/first.png"}
        }))
        .unwrap(),
        TaskState::Completed(vec!["https://img.example/first.png".to_string()])
    );
    assert_eq!(
        parse_task_state(&json!({"status": "failed", "error": {"message": "content rejected"}}))
            .unwrap(),
        TaskState::Failed("content rejected".to_string())
    );
    assert_eq!(
        parse_task_state(&json!({"status": "failed", "error": "quota exceeded"})).unwrap(),
        TaskState::Failed("quota exceeded".to_string())
    );
    assert_eq!(
        parse_task_state(&json!({"status": "failed", "error": {}, "message": "fallback"})).unwrap(),
        TaskState::Failed("fallback".to_string())
    );
}

#[test]
fn rejects_incomplete_or_unknown_task_responses() {
    for body in [
        json!({"status": "completed", "data": [{"url": "https://img.example/old.png"}]}),
        json!({"status": "completed", "metadata": {}}),
        json!({"status": "completed", "metadata": {"url": ""}}),
        json!({"status": "completed", "metadata": {"url": "not-a-url"}}),
        json!({"status": "completed"}),
        json!({"status": "unexpected"}),
        json!({"progress": 100}),
    ] {
        assert!(
            parse_task_state(&body).is_err(),
            "unexpectedly accepted {body}"
        );
    }
}
