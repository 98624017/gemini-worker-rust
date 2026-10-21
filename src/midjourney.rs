use axum::http::StatusCode;
use serde_json::{Value, json};
use url::Url;

const IMAGE_ALIASES: [&str; 3] = ["image", "images", "reference_images"];

#[derive(Clone, Copy, Debug)]
pub(crate) enum Mode {
    Fast,
    Relax,
}

impl Mode {
    fn from_model(model: &str) -> Option<Self> {
        match model {
            "mj_fast_imagine" => Some(Self::Fast),
            "mj_relax_imagine" => Some(Self::Relax),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "FAST",
            Self::Relax => "RELAX",
        }
    }

    pub(crate) fn submit_path(self) -> &'static str {
        match self {
            Self::Fast => "/mj/submit/imagine",
            Self::Relax => "/mj-relax/mj/submit/imagine",
        }
    }

    fn flag(self) -> &'static str {
        match self {
            Self::Fast => "--fast",
            Self::Relax => "--relax",
        }
    }
}

pub(crate) struct PreparedRequest {
    pub mode: Mode,
    pub payload: Value,
    pub reference_url: Option<String>,
}

#[derive(Debug)]
pub(crate) struct CompletedTask {
    pub urls: Vec<String>,
    pub created: Option<i64>,
}

#[derive(Debug)]
pub(crate) struct Error {
    pub status: StatusCode,
    pub source: &'static str,
    pub stage: &'static str,
    pub kind: &'static str,
    pub message: String,
    pub detail: Option<String>,
    pub business_code: Option<i64>,
    pub fail_reason: Option<String>,
}

impl Error {
    pub(crate) fn new(
        status: StatusCode,
        source: &'static str,
        stage: &'static str,
        kind: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status,
            source,
            stage,
            kind,
            message: message.into(),
            detail: None,
            business_code: None,
            fail_reason: None,
        }
    }

    pub(crate) fn protocol(stage: &'static str, message: impl AsRef<str>) -> Self {
        Self::new(
            StatusCode::BAD_GATEWAY,
            "upstream",
            stage,
            "invalid_response",
            format!("Magic 上游响应格式错误：{}", message.as_ref()),
        )
    }

    pub(crate) fn upstream(status: StatusCode, stage: &'static str, body: &Value) -> Self {
        let business_code = business_code(body);
        let fail_reason = nonempty_string(body.get("failReason")).map(str::to_string);
        let details = [
            body.get("failReason"),
            body.get("description"),
            body.get("message"),
            body.get("msg"),
            body.pointer("/error/message"),
            body.get("error"),
        ]
        .into_iter()
        .filter_map(nonempty_string)
        .collect::<Vec<_>>();
        let detail = details.first().copied();
        let mapped = classify_upstream_status(status, business_code, &details.join("\n"));
        let (kind, context) = match mapped {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                ("upstream_auth_failed", "Magic 上游鉴权失败")
            }
            StatusCode::TOO_MANY_REQUESTS => {
                ("upstream_rate_limited", "Magic 上游请求受限，请稍后再试")
            }
            StatusCode::SERVICE_UNAVAILABLE => (
                "upstream_capacity_exhausted",
                "Magic 上游暂无可用容量，请稍后再试",
            ),
            StatusCode::BAD_REQUEST => (
                "upstream_input_rejected",
                "Magic 上游拒绝了输入，请检查请求",
            ),
            _ => ("upstream_request_failed", "Magic 上游任务请求失败"),
        };
        let mut error = Self::new(
            mapped,
            "upstream",
            stage,
            kind,
            detail.map_or_else(
                || context.to_string(),
                |detail| format!("{context}：{detail}"),
            ),
        );
        error.business_code = business_code;
        error.fail_reason = fail_reason;
        error
    }

    fn with_body(mut self, body: &Value) -> Self {
        self.business_code = business_code(body);
        self.fail_reason = nonempty_string(body.get("failReason")).map(str::to_string);
        self
    }
}

pub(crate) fn matches(base_url: &str, body: &Value) -> bool {
    body.get("model")
        .and_then(Value::as_str)
        .and_then(Mode::from_model)
        .is_some()
        && is_magic_upstream(base_url)
}

pub(crate) fn is_magic_upstream(base_url: &str) -> bool {
    Url::parse(base_url)
        .ok()
        .is_some_and(|url| url.host_str() == Some("magic666.top"))
}

pub(crate) fn prepare_request(body: Value) -> Result<PreparedRequest, Error> {
    let object = body
        .as_object()
        .ok_or_else(|| invalid_request("请求内容必须是 JSON 对象"))?;
    for field in ["model", "prompt"] {
        if nonempty_string(object.get(field)).is_none() {
            return Err(invalid_request(format!("{field} 必须是非空字符串")));
        }
    }
    let mode = Mode::from_model(object.get("model").and_then(Value::as_str).unwrap()).ok_or_else(
        || invalid_request("此 Magic 生图路径仅支持 mj_fast_imagine 或 mj_relax_imagine"),
    )?;
    if let Some(format) = object.get("response_format")
        && !format
            .as_str()
            .is_some_and(|format| format.eq_ignore_ascii_case("url"))
    {
        return Err(invalid_request("response_format 只支持 url，请调整后再试"));
    }
    for field in ["n", "size", "quality", "stream", "base64Array"] {
        if object.contains_key(field) {
            return Err(invalid_request(format!(
                "Magic 生图不支持下游 {field} 字段；参考图请使用 HTTP(S) URL 数组，生成参数请通过 prompt 设置"
            )));
        }
    }

    let body = crate::openai_image::normalize_request_body(body, false)
        .map_err(|error| invalid_request(error.to_string()))?;
    let images = IMAGE_ALIASES
        .iter()
        .find_map(|alias| body.get(*alias).and_then(Value::as_array));
    if images.is_some_and(|images| images.len() > 1) {
        return Err(invalid_request(
            "Magic 当前仅支持一张参考图片，请合并别名后保留一张 URL",
        ));
    }
    let reference_url = images
        .and_then(|images| images.first())
        .and_then(Value::as_str)
        .map(str::to_string);
    let prompt = body.get("prompt").and_then(Value::as_str).unwrap();
    Ok(PreparedRequest {
        mode,
        payload: json!({"mode": mode.as_str(), "prompt": normalize_prompt(prompt, mode)}),
        reference_url,
    })
}

pub(crate) fn submitted_task(body: &Value) -> Result<String, Error> {
    let code = body
        .get("code")
        .and_then(Value::as_i64)
        .ok_or_else(|| Error::protocol("magic_submit", "缺少整数 code 字段").with_body(body))?;
    if !matches!(code, 1 | 21 | 22) {
        return Err(Error::upstream(StatusCode::OK, "magic_submit", body));
    }
    nonempty_string(body.get("result"))
        .map(str::to_string)
        .ok_or_else(|| Error::protocol("magic_submit", "缺少非空字符串任务 ID").with_body(body))
}

pub(crate) fn task_result(body: &Value) -> Result<Option<CompletedTask>, Error> {
    let status = body
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::protocol("magic_poll", "缺少字符串 status 字段").with_body(body))?;
    match status {
        "SUCCESS" => Ok(Some(CompletedTask {
            urls: result_urls(body).map_err(|error| error.with_body(body))?,
            created: ["finishTime", "submitTime"].into_iter().find_map(|field| {
                body.get(field)
                    .and_then(Value::as_i64)
                    .filter(|timestamp| *timestamp > 0)
                    .map(|timestamp| timestamp / 1000)
            }),
        })),
        "FAILURE" | "FAILED" | "CANCEL" | "CANCELLED" | "MODAL" => {
            let mut error = Error::upstream(StatusCode::OK, "magic_poll", body);
            let context = if status == "MODAL" {
                "Magic 任务需要额外交互，当前同步生图不支持"
            } else {
                "Magic 生图任务已失败或取消"
            };
            error.message = format!("{context}（{status}）：{}", error.message);
            error.kind = if status == "MODAL" {
                "upstream_interaction_required"
            } else {
                error.kind
            };
            Err(error)
        }
        _ => Ok(None),
    }
}

fn invalid_request(message: impl Into<String>) -> Error {
    Error::new(
        StatusCode::BAD_REQUEST,
        "proxy",
        "magic_validate_request",
        "invalid_request",
        message,
    )
}

fn normalize_prompt(prompt: &str, mode: Mode) -> String {
    let flag = mode.flag();
    let mut normalized = String::with_capacity(prompt.len() + flag.len() + 1);
    for piece in prompt.split_inclusive(char::is_whitespace) {
        let token = piece.trim_end_matches(char::is_whitespace);
        if !matches!(token, "--fast" | "--relax" | "--turbo") {
            normalized.push_str(piece);
        }
    }
    let prompt = normalized.trim_end();
    if prompt.is_empty() {
        flag.to_string()
    } else {
        format!("{prompt} {flag}")
    }
}

fn result_urls(body: &Value) -> Result<Vec<String>, Error> {
    let mut urls = Vec::new();
    match body.get("imageUrls") {
        None => {}
        Some(Value::Array(values)) => {
            for value in values {
                let raw = value
                    .as_str()
                    .or_else(|| value.get("url").and_then(Value::as_str))
                    .ok_or_else(|| {
                        Error::protocol("magic_result", "imageUrls 中存在非法图片地址")
                    })?;
                validate_result_url(raw)?;
                if !urls.iter().any(|url| url == raw) {
                    urls.push(raw.to_string());
                }
            }
        }
        Some(_) => return Err(Error::protocol("magic_result", "imageUrls 必须是数组")),
    }
    if urls.is_empty() {
        let raw = body
            .get("imageUrl")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::protocol("magic_result", "成功任务缺少图片 URL"))?;
        validate_result_url(raw)?;
        urls.push(raw.to_string());
    }
    Ok(urls)
}

fn validate_result_url(raw: &str) -> Result<(), Error> {
    if !Url::parse(raw).ok().is_some_and(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some_and(|host| !host.is_empty())
    }) {
        return Err(Error::protocol(
            "magic_result",
            "图片地址必须是包含主机名的完整 HTTP(S) URL",
        ));
    }
    Ok(())
}

fn nonempty_string(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn business_code(body: &Value) -> Option<i64> {
    body.get("code")
        .or_else(|| body.pointer("/error/code"))
        .and_then(Value::as_i64)
}

fn classify_upstream_status(status: StatusCode, code: Option<i64>, detail: &str) -> StatusCode {
    if matches!(
        status,
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS
    ) {
        return status;
    }
    let detail = detail.to_ascii_lowercase();
    if matches!(code, Some(3 | 23 | 30))
        || [
            "无可用实例",
            "没有可用实例",
            "没有可用的实例",
            "队列已满",
            "队列满",
            "分组饱和",
            "分组负载饱和",
            "no available instance",
            "no available account instance",
            "no instance available",
            "queue is full",
            "queue full",
            "group saturated",
            "group is saturated",
        ]
        .iter()
        .any(|pattern| detail.contains(pattern))
    {
        StatusCode::SERVICE_UNAVAILABLE
    } else if matches!(code, Some(4 | 24))
        || [
            "敏感词",
            "违反内容政策",
            "违反政策",
            "违规提示词",
            "输入违规",
            "sensitive word",
            "content policy",
            "policy violation",
            "banned word",
        ]
        .iter()
        .any(|pattern| detail.contains(pattern))
    {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::BAD_GATEWAY
    }
}
