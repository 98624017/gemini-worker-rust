use anyhow::{Result, anyhow};
use axum::http::HeaderMap;
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedUpstream {
    pub base_url: String,
    pub api_key: String,
}

pub fn resolve_upstream<I, K, V>(
    headers: I,
    default_base_url: &str,
    default_api_key: &str,
) -> Result<ResolvedUpstream>
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
        if let Some((custom_base, custom_key)) = token.split_once('|') {
            let custom_base = custom_base.trim();
            let custom_key = custom_key.trim();
            if !custom_base.is_empty() {
                validate_http_base_url(custom_base)?;
                base_url = custom_base.to_string();
            }
            if !custom_key.is_empty() {
                api_key = custom_key.to_string();
            }
        } else {
            api_key = token.trim().to_string();
        }
    }

    if api_key.is_empty() {
        return Err(anyhow!("Missing upstream apiKey"));
    }

    Ok(ResolvedUpstream { base_url, api_key })
}

pub fn resolve_upstream_from_header_map(
    headers: &HeaderMap,
    default_base_url: &str,
    default_api_key: &str,
) -> Result<ResolvedUpstream> {
    let pairs = headers.iter().filter_map(|(name, value)| {
        value
            .to_str()
            .ok()
            .map(|text| (name.as_str().to_string(), text.to_string()))
    });
    resolve_upstream(pairs, default_base_url, default_api_key)
}

fn validate_http_base_url(raw: &str) -> Result<()> {
    let parsed = Url::parse(raw)?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(anyhow!("custom baseUrl must use http or https"));
    }
    if parsed.host_str().unwrap_or_default().is_empty() {
        return Err(anyhow!("custom baseUrl host is empty"));
    }
    Ok(())
}
