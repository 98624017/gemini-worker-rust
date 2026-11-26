use anyhow::{Result, anyhow};
use serde_json::{Map, Value};
use url::Url;

pub const MAX_REFERENCE_IMAGES: usize = 10;

const MODEL: &str = "seedream-5.5";
const ALLOWED_RATIOS: [&str; 11] = [
    "1:1", "16:9", "9:16", "4:3", "3:4", "5:4", "4:5", "3:2", "2:3", "21:9", "auto",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskState {
    Pending,
    Unknown,
    Completed(Vec<String>),
    Failed(String),
}

pub fn is_seedream_55_request(body: &Value) -> bool {
    body.get("model").and_then(Value::as_str) == Some(MODEL)
}

pub fn build_request_body(body: &Value) -> Result<Value> {
    let object = body
        .as_object()
        .ok_or_else(|| anyhow!("MAI 请求内容必须是 JSON 对象"))?;

    if object.get("model").and_then(Value::as_str) != Some(MODEL) {
        return Err(anyhow!("MAI 仅支持 model 为 seedream-5.5"));
    }

    let prompt = object
        .get("prompt")
        .and_then(Value::as_str)
        .filter(|prompt| !prompt.trim().is_empty())
        .ok_or_else(|| anyhow!("MAI prompt 必须是非空字符串"))?;

    if let Some(n) = object.get("n")
        && !(n.as_i64() == Some(1) || n.as_u64() == Some(1))
    {
        return Err(anyhow!("MAI n 只支持整数 1"));
    }

    if let Some(size) = object.get("size") {
        let size = size
            .as_str()
            .ok_or_else(|| anyhow!("MAI size 必须是字符串，且只支持 1K 或 2K"))?;
        if !size.eq_ignore_ascii_case("1K") && !size.eq_ignore_ascii_case("2K") {
            return Err(anyhow!("MAI size 只支持 1K 或 2K，不支持 4K"));
        }
    }

    let ratio = match object.get("aspect_ratio") {
        None => "1:1",
        Some(Value::String(ratio)) if ALLOWED_RATIOS.contains(&ratio.as_str()) => ratio,
        Some(Value::String(_)) => {
            return Err(anyhow!("MAI aspect_ratio 不是支持的比例"));
        }
        Some(_) => return Err(anyhow!("MAI aspect_ratio 必须是字符串")),
    };

    let image_urls = collect_references(object)?;

    if let Some(seed) = object.get("seed")
        && !(seed.is_i64() || seed.is_u64())
    {
        return Err(anyhow!("MAI seed 只允许整数"));
    }

    let mut request = Map::new();
    request.insert("model".to_string(), Value::String(MODEL.to_string()));
    request.insert("prompt".to_string(), Value::String(prompt.to_string()));
    request.insert("n".to_string(), Value::from(1));
    request.insert("aspect_ratio".to_string(), Value::String(ratio.to_string()));
    request.insert("quality".to_string(), Value::String("1440p".to_string()));
    if !image_urls.is_empty() {
        request.insert(
            "image_urls".to_string(),
            Value::Array(image_urls.into_iter().map(Value::String).collect()),
        );
    }
    if let Some(seed) = object.get("seed") {
        request.insert("seed".to_string(), seed.clone());
    }

    Ok(Value::Object(request))
}

pub fn task_id(body: &Value) -> Result<String> {
    let object = body
        .as_object()
        .ok_or_else(|| anyhow!("MAI 任务响应必须是 JSON 对象"))?;
    let mut invalid_field = None;

    for field in ["id", "task_id"] {
        match object.get(field) {
            Some(Value::String(value)) if !value.trim().is_empty() => return Ok(value.clone()),
            Some(_) => invalid_field = Some(field),
            None => {}
        }
    }

    if let Some(field) = invalid_field {
        return Err(anyhow!("MAI 任务响应中的 {field} 必须是非空字符串"));
    }
    Err(anyhow!("MAI 任务响应缺少非空任务 ID"))
}

pub fn parse_task_state(body: &Value) -> Result<TaskState> {
    let object = body
        .as_object()
        .ok_or_else(|| protocol_error("任务响应必须是 JSON 对象"))?;
    let status = object
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| protocol_error("缺少字符串 status 字段"))?;

    match status {
        "queued" | "in_progress" | "processing" => Ok(TaskState::Pending),
        "unknown" => Ok(TaskState::Unknown),
        "completed" => Ok(TaskState::Completed(parse_completed_urls(object)?)),
        "failed" => Ok(TaskState::Failed(
            failure_reason(object).unwrap_or_else(|| "任务失败".to_string()),
        )),
        _ => Err(protocol_error(&format!("未知任务状态：{status}"))),
    }
}

fn collect_references(object: &Map<String, Value>) -> Result<Vec<String>> {
    let mut image_urls = Vec::new();

    for field in ["reference_images", "images", "image"] {
        let Some(value) = object.get(field) else {
            continue;
        };
        let images = value
            .as_array()
            .ok_or_else(|| anyhow!("MAI {field} 必须是图片 URL 数组"))?;
        for image in images {
            if image_urls.len() == MAX_REFERENCE_IMAGES {
                return Err(anyhow!("MAI 参考图片最多支持 {} 张", MAX_REFERENCE_IMAGES));
            }
            let source = image
                .as_str()
                .ok_or_else(|| anyhow!("MAI {field} 中的图片地址必须是字符串"))?;
            validate_http_url(source, "MAI 参考图片")?;
            image_urls.push(source.to_string());
        }
    }

    Ok(image_urls)
}

fn parse_completed_urls(object: &Map<String, Value>) -> Result<Vec<String>> {
    let url = object
        .get("metadata")
        .and_then(Value::as_object)
        .and_then(|metadata| metadata.get("url"))
        .and_then(Value::as_str)
        .filter(|url| !url.trim().is_empty())
        .ok_or_else(|| protocol_error("completed 状态缺少非空 metadata.url"))?;
    validate_http_url(url, "MAI 完成结果图片")
        .map_err(|_| protocol_error("completed 状态包含非法图片 URL"))?;
    Ok(vec![url.to_string()])
}

fn failure_reason(object: &Map<String, Value>) -> Option<String> {
    let candidates = [
        object
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str),
        object.get("error").and_then(Value::as_str),
        object.get("message").and_then(Value::as_str),
    ];
    candidates
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|message| !message.is_empty())
        .map(str::to_string)
}

fn validate_http_url(raw: &str, subject: &str) -> Result<()> {
    let parsed = Url::parse(raw).map_err(|_| anyhow!("{subject}必须是完整 URL"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(anyhow!("{subject} URL 必须使用 http 或 https"));
    }
    if parsed.host_str().is_none_or(|host| host.trim().is_empty()) {
        return Err(anyhow!("{subject} URL 缺少主机名"));
    }
    Ok(())
}

fn protocol_error(message: &str) -> anyhow::Error {
    anyhow!("MAI 协议响应错误：{message}")
}
