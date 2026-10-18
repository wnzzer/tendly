//! Reading what app-server sends: the paths in a request, its model list,
//! an item's status and a request's key.

use super::*;
use std::time::Duration;

/// A command's output is streamed in many small deltas, and each used to go
/// out as the whole block so far at `status:"running"`. For a chatty command
/// -- a progress bar, a build log -- that is hundreds of ever-growing events,
/// every one also written to history: a single `winget install` once wrote
/// 500+ events and megabytes, enough to trip the conversation's 4 MB
/// backpressure and cut the turn short so it never finished. A running update
/// is worth forwarding only once the output has grown by [`OUTPUT_EMIT_BYTES`],
/// or [`OUTPUT_EMIT_INTERVAL`] has passed so a slow trickle still shows. The
/// closing `item/completed` carries the full text either way, so skipping the
/// ones between loses nothing.
pub(super) const OUTPUT_EMIT_BYTES: usize = 2048;
pub(super) const OUTPUT_EMIT_INTERVAL: Duration = Duration::from_millis(200);

/// Whether a command's streamed output, having `grew` bytes since the last
/// update `elapsed` ago, is worth forwarding now rather than waiting.
pub(super) fn should_emit_output(grew: usize, elapsed: Duration) -> bool {
    grew >= OUTPUT_EMIT_BYTES || elapsed >= OUTPUT_EMIT_INTERVAL
}

/// `pathsIn`: absolute paths anywhere in a permission request.
pub(super) fn paths_in(value: &Value, found: &mut Vec<Value>, depth: usize) {
    if depth > 6 || found.len() >= 16 {
        return;
    }
    match value {
        Value::String(text) => {
            let bytes = text.as_bytes();
            let windows = bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && matches!(bytes[2], b'\\' | b'/');
            if text.starts_with('/') || windows {
                found.push(value.clone());
            }
        }
        Value::Array(items) => items
            .iter()
            .for_each(|item| paths_in(item, found, depth + 1)),
        Value::Object(object) => object
            .values()
            .for_each(|item| paths_in(item, found, depth + 1)),
        _ => {}
    }
}

/// `codexModels`: the models app-server's model/list offers, as it describes them.
pub(super) fn codex_models(result: &Value) -> Option<Vec<Value>> {
    let rows = result
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| result.get("models").and_then(Value::as_array))
        .or_else(|| result.as_array())?;
    Some(
        rows.iter()
            .filter_map(|row| {
                let object = row.as_object()?;
                let id = ["id", "model", "name"]
                    .iter()
                    .map(|key| object.get(*key))
                    .find(|value| value.is_some_and(|v| !v.is_null()))
                    .flatten()
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())?;
                if object.get("hidden") == Some(&Value::Bool(true)) {
                    return None;
                }
                let efforts: Option<Vec<Value>> = object
                    .get("supportedReasoningEfforts")
                    .and_then(Value::as_array)
                    .map(|levels| {
                        levels
                            .iter()
                            .filter_map(|level| {
                                level
                                    .as_str()
                                    .or_else(|| {
                                        level.get("reasoningEffort").and_then(Value::as_str)
                                    })
                                    .map(|level| json!(level))
                            })
                            .collect()
                    });
                let mut entry = Map::new();
                entry.insert("id".into(), json!(id));
                entry.insert(
                    "name".into(),
                    json!(
                        object
                            .get("displayName")
                            .and_then(Value::as_str)
                            .unwrap_or(id)
                    ),
                );
                if object.get("isDefault") == Some(&Value::Bool(true)) {
                    entry.insert("isDefault".into(), json!(true));
                }
                if let Some(description) = object.get("description").and_then(Value::as_str) {
                    entry.insert("description".into(), json!(description));
                }
                if let Some(efforts) = efforts.filter(|e| !e.is_empty()) {
                    entry.insert("efforts".into(), Value::Array(efforts));
                }
                Some(Value::Object(entry))
            })
            .collect(),
    )
}

pub(super) fn item_status(item: &Value) -> &'static str {
    let status = item.get("status").and_then(Value::as_str);
    if matches!(status, Some("failed" | "declined" | "cancelled"))
        || item.get("success") == Some(&Value::Bool(false))
    {
        "failed"
    } else if status == Some("inProgress") {
        "running"
    } else {
        "completed"
    }
}

/// `requestKey`: a native request id, keyed by type so `1` and `"1"` differ.
pub(super) fn request_key(id: &Value) -> String {
    match id {
        Value::String(s) => format!("string:{s}"),
        Value::Number(n) => format!("number:{n}"),
        Value::Null => "object:null".into(),
        other => format!(
            "{}:{}",
            if other.is_boolean() {
                "boolean"
            } else {
                "object"
            },
            js_string(other)
        ),
    }
}
