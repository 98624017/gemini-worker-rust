use anyhow::Result;
use serde_json::Value;

pub fn rewrite_sse_text_with<F>(input: &str, mut rewriter: F) -> Result<String>
where
    F: FnMut(Value) -> Result<Value>,
{
    let mut output = String::with_capacity(input.len());

    for line in input.split_inclusive('\n') {
        let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
        let line_ending = &line[trimmed.len()..];

        if !trimmed.starts_with("data:") {
            output.push_str(trimmed);
            output.push_str(line_ending);
            continue;
        }

        let raw = trimmed.trim_start_matches("data:").trim();
        if raw.is_empty() || raw == "[DONE]" {
            output.push_str(trimmed);
            output.push_str(line_ending);
            continue;
        }

        match serde_json::from_str::<Value>(raw) {
            Ok(value) => {
                let rewritten = rewriter(value)?;
                output.push_str("data: ");
                output.push_str(&serde_json::to_string(&rewritten)?);
                output.push_str(line_ending);
            }
            Err(_) => {
                output.push_str(trimmed);
                output.push_str(line_ending);
            }
        }
    }

    Ok(output)
}
