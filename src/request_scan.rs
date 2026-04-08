use anyhow::{Result, anyhow};
use serde_json::Value;

const MAX_INLINE_DATA_URLS: usize = 5;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestImageRef {
    pub json_pointer: String,
    pub url: String,
}

pub fn scan_request_image_urls(body: &Value) -> Result<Vec<RequestImageRef>> {
    let mut matches = Vec::new();
    walk(body, "", &mut matches)?;
    Ok(matches)
}

fn walk(node: &Value, path: &str, matches: &mut Vec<RequestImageRef>) -> Result<()> {
    match node {
        Value::Object(map) => {
            if let Some(Value::Object(inline_data)) = map.get("inlineData") {
                if let Some(Value::String(data)) = inline_data.get("data") {
                    if is_http_url(data) {
                        matches.push(RequestImageRef {
                            json_pointer: format!("{path}/inlineData"),
                            url: data.clone(),
                        });
                        if matches.len() > MAX_INLINE_DATA_URLS {
                            return Err(anyhow!("too many inlineData URLs"));
                        }
                    }
                }
            }

            for (key, child) in map {
                let child_path = format!("{path}/{}", escape_json_pointer_token(key));
                walk(child, &child_path, matches)?;
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                let child_path = format!("{path}/{index}");
                walk(child, &child_path, matches)?;
            }
        }
        _ => {}
    }

    Ok(())
}

fn escape_json_pointer_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

fn is_http_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}
