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
                    continue;
                }
                let args = call.get("args").cloned().unwrap_or(Value::Null);
                out.push((name.to_string(), args));
            }
        }
    }
    out
}

/// Extract the first `text` part, if any.
pub fn extract_text(content: &Value) -> Option<String> {
    content
        .get("parts")?
        .as_array()?
        .iter()
        .find_map(|p| p.get("text")?.as_str())
        .map(|s| s.to_string())
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
