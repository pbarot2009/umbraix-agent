use serde_json::Value;

/// Helpers for reading Gemini `generateContent` responses without brittle
/// full-struct deserialization (the API adds fields over time).
///
/// Response shape:
/// `candidates[0].content = {"role": "model", "parts": [...]}`,
/// where each part is either `{"text": ...}` or
/// `{"functionCall": {"name": ..., "args": {...}}}`.
///
/// Returns the raw model `Content` object (`{"role","parts"}`), if present.
pub fn candidate_content(res: &Value) -> Option<&Value> {
    res.get("candidates")?.as_array()?.first()?.get("content")
}

/// Extract `(tool_name, args)` for every `functionCall` part.
pub fn extract_function_calls(content: &Value) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    if let Some(parts) = content.get("parts").and_then(|p| p.as_array()) {
        for part in parts {
            if let Some(call) = part.get("functionCall") {
                let name = call
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or_default();
                if name.is_empty() {
                    tracing::warn!("Gemini returned nameless functionCall, skipping");
                    continue;
                }
                let args = call.get("args").cloned().unwrap_or(Value::Null);
                // Tool executors expect object args; coerce anything else to
                // Null with a warning instead of forwarding garbage.
                let args = if args.is_object() || args.is_null() {
                    args
                } else {
                    tracing::warn!(tool = %name, "non-object args coerced to null");
                    Value::Null
                };
                out.push((name.to_string(), args));
            }
        }
    }
    out
}

/// Extract the first `text` part, if any.
/// NOTE: with `candidateCount=1` the model returns a single candidate and
/// text normally precedes tool calls; multi-part text is concatenated by
/// [`extract_all_text`].
pub fn extract_text(content: &Value) -> Option<String> {
    extract_all_text(content)
}

/// Concatenate ALL text parts in a content object.
pub fn extract_all_text(content: &Value) -> Option<String> {
    let parts = content.get("parts")?.as_array()?;
    let mut out = String::new();
    for p in parts {
        if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(t);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Extract text from a full response (`candidates[0].content.parts[0].text`).
pub fn response_text(res: &Value) -> Option<String> {
    candidate_content(res).and_then(extract_text)
}

/// Build a `functionResponse` part to feed a tool result back to the model.
pub fn function_response_part(name: &str, result: &str) -> Value {
    serde_json::json!({
        "functionResponse": {
            "name": name,
            "response": { "result": result }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_calls_and_text() {
        let content = json!({
            "role": "model",
            "parts": [
                {"text": "hi"},
                {"functionCall": {"name": "purge_messages", "args": {"count": 5}}}
            ]
        });
        assert_eq!(extract_text(&content).as_deref(), Some("hi"));
        let calls = extract_function_calls(&content);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "purge_messages");
    }

    #[test]
    fn empty_candidate_is_none() {
        assert!(candidate_content(&json!({})).is_none());
        assert!(response_text(&json!({"candidates": []})).is_none());
    }
}
