use std::error::Error as StdError;
use std::fmt;

use axum::http::{HeaderMap, StatusCode};
use serde_json::Value;
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedUpstream {
    pub base_url: String,
    pub api_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolveUpstreamErrorKind {
    MissingApiKey,
    InvalidOverride,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolveUpstreamError {
    kind: ResolveUpstreamErrorKind,
    message: String,
}

impl ResolveUpstreamError {
    fn missing_api_key() -> Self {
        Self {
            kind: ResolveUpstreamErrorKind::MissingApiKey,
            message: "Missing upstream apiKey".to_string(),
        }
    }

    fn invalid_override(message: impl Into<String>) -> Self {
        Self {
            kind: ResolveUpstreamErrorKind::InvalidOverride,
            message: message.into(),
        }
    }

    pub fn status_code(&self) -> StatusCode {
        match self.kind {
            ResolveUpstreamErrorKind::MissingApiKey => StatusCode::UNAUTHORIZED,
            ResolveUpstreamErrorKind::InvalidOverride => StatusCode::BAD_REQUEST,
        }
    }
}

impl fmt::Display for ResolveUpstreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl StdError for ResolveUpstreamError {}

pub fn resolve_upstream<I, K, V>(
    headers: I,
    default_base_url: &str,
    default_api_key: &str,
) -> Result<ResolvedUpstream, ResolveUpstreamError>
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: AsRef<str>,
{
    resolve_upstream_for_request(headers, &Value::Null, default_base_url, default_api_key)
}

pub fn resolve_upstream_for_request<I, K, V>(
    headers: I,
    request_body: &Value,
    default_base_url: &str,
    default_api_key: &str,
) -> Result<ResolvedUpstream, ResolveUpstreamError>
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: AsRef<str>,
{
    let mut x_goog_api_key = None;
    let mut authorization = None;

    for (key, value) in headers {
        let key = key.as_ref();
        let value = value.as_ref().trim();
        if key.eq_ignore_ascii_case("x-goog-api-key") && !value.is_empty() {
            x_goog_api_key = Some(value.to_string());
        } else if key.eq_ignore_ascii_case("authorization") && !value.is_empty() {
            authorization = Some(value.to_string());
        }
    }

    let token = x_goog_api_key.or_else(|| {
        authorization.and_then(|value| value.strip_prefix("Bearer ").map(ToOwned::to_owned))
    });

    let mut base_url = default_base_url.trim().to_string();
    let mut api_key = default_api_key.trim().to_string();

    if let Some(token) = token {
        let override_upstream = if token.contains(',') {
            parse_dual_upstream_token(&token, request_body)?
        } else {
            parse_single_upstream_token(&token)?
        };

        if let Some(custom_base) = override_upstream.base_url {
            base_url = custom_base;
        }
        if let Some(custom_key) = override_upstream.api_key {
            api_key = custom_key;
        }
    }

    if api_key.is_empty() {
        return Err(ResolveUpstreamError::missing_api_key());
    }

    Ok(ResolvedUpstream { base_url, api_key })
}

pub fn resolve_upstream_from_header_map(
    headers: &HeaderMap,
    default_base_url: &str,
    default_api_key: &str,
) -> Result<ResolvedUpstream, ResolveUpstreamError> {
    let pairs = headers.iter().filter_map(|(name, value)| {
        value
            .to_str()
            .ok()
            .map(|text| (name.as_str().to_string(), text.to_string()))
    });
    resolve_upstream(pairs, default_base_url, default_api_key)
}

pub fn resolve_upstream_for_request_from_header_map(
    headers: &HeaderMap,
    request_body: &Value,
    default_base_url: &str,
    default_api_key: &str,
) -> Result<ResolvedUpstream, ResolveUpstreamError> {
    let pairs = headers.iter().filter_map(|(name, value)| {
        value
            .to_str()
            .ok()
            .map(|text| (name.as_str().to_string(), text.to_string()))
    });
    resolve_upstream_for_request(pairs, request_body, default_base_url, default_api_key)
}

pub fn is_aiapidev_base_url(raw: &str) -> bool {
    let Ok(parsed) = Url::parse(raw) else {
        return false;
    };
    matches!(
        parsed.host_str(),
        Some("www.aiapidev.com") | Some("aiapidev.com")
    )
}

pub fn rewrite_aiapidev_model_path(path: &str) -> String {
    let Some(model_part) = path.strip_prefix("/v1beta/models/") else {
        return path.to_string();
    };
    let Some((model, suffix)) = model_part.split_once(":generateContent") else {
        return path.to_string();
    };

    let mapped_model = match model {
        "gemini-3-pro-image-preview" => "nanobananapro",
        "gemini-3.1-flash-image-preview" => "nanobanana2",
        _ => model,
    };

    format!("/v1beta/models/{mapped_model}:generateContent{suffix}")
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct UpstreamOverride {
    base_url: Option<String>,
    api_key: Option<String>,
}

fn parse_single_upstream_token(token: &str) -> Result<UpstreamOverride, ResolveUpstreamError> {
    if let Some((custom_base, custom_key)) = token.split_once('|') {
        let custom_base = custom_base.trim();
        let custom_key = custom_key.trim();
        let mut parsed = UpstreamOverride::default();
        if !custom_base.is_empty() {
            validate_http_base_url(custom_base)?;
            parsed.base_url = Some(custom_base.to_string());
        }
        if !custom_key.is_empty() {
            parsed.api_key = Some(custom_key.to_string());
        }
        return Ok(parsed);
    }

    Ok(UpstreamOverride {
        base_url: None,
        api_key: Some(token.trim().to_string()),
    })
}

fn parse_dual_upstream_token(
    token: &str,
    request_body: &Value,
) -> Result<UpstreamOverride, ResolveUpstreamError> {
    let parts: Vec<&str> = token
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();

    if parts.len() != 2 {
        return Err(ResolveUpstreamError::invalid_override(
            "invalid dual upstream token: expected exactly two <baseUrl>|<apiKey> groups",
        ));
    }

    let first = parse_strict_upstream_pair(parts[0])?;
    let second = parse_strict_upstream_pair(parts[1])?;
    if request_targets_4k(request_body) {
        Ok(second)
    } else {
        Ok(first)
    }
}

fn parse_strict_upstream_pair(token: &str) -> Result<UpstreamOverride, ResolveUpstreamError> {
    let Some((custom_base, custom_key)) = token.split_once('|') else {
        return Err(ResolveUpstreamError::invalid_override(
            "invalid dual upstream token: each group must be <baseUrl>|<apiKey>",
        ));
    };
    let custom_base = custom_base.trim();
    let custom_key = custom_key.trim();
    if custom_base.is_empty() || custom_key.is_empty() {
        return Err(ResolveUpstreamError::invalid_override(
            "invalid dual upstream token: baseUrl and apiKey must both be non-empty",
        ));
    }
    validate_http_base_url(custom_base)?;
    Ok(UpstreamOverride {
        base_url: Some(custom_base.to_string()),
        api_key: Some(custom_key.to_string()),
    })
}

fn request_targets_4k(request_body: &Value) -> bool {
    request_body
        .pointer("/generationConfig/imageConfig/imageSize")
        .or_else(|| request_body.pointer("/generation_config/image_config/image_size"))
        .and_then(Value::as_str)
        .map(|value| value.trim().eq_ignore_ascii_case("4k"))
        .unwrap_or(false)
}

fn validate_http_base_url(raw: &str) -> Result<(), ResolveUpstreamError> {
    let parsed =
        Url::parse(raw).map_err(|err| ResolveUpstreamError::invalid_override(err.to_string()))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(ResolveUpstreamError::invalid_override(
            "custom baseUrl must use http or https",
        ));
    }
    if parsed.host_str().unwrap_or_default().is_empty() {
        return Err(ResolveUpstreamError::invalid_override(
            "custom baseUrl host is empty",
        ));
    }
    Ok(())
}
