//! Reading what Claude Code reports: usage and context, models, commands,
//! and the one-line summaries of tool calls.

use super::*;

pub(super) fn non_negative(value: Option<&Value>) -> Option<i64> {
    safe_integer(value).filter(|n| *n >= 0)
}

/// `contextTokens`: the input tokens in the window, cache reads and writes
/// included, from the client's own fields.
pub(super) fn context_tokens(usage: Option<&Value>) -> Option<Value> {
    let parts = [
        "input_tokens",
        "cache_read_input_tokens",
        "cache_creation_input_tokens",
    ]
    .map(|key| non_negative(usage.and_then(|u| u.get(key))));
    if parts.iter().all(Option::is_none) {
        return None;
    }
    Some(json!(parts.iter().map(|p| p.unwrap_or(0)).sum::<i64>()))
}

/// `modelEntries`: the models the client lists at initialize time, with the
/// effort levels each states it supports.
pub(super) fn model_entries(value: Option<&Value>) -> Option<Vec<Value>> {
    let entries = value?.as_array()?;
    Some(
        entries
            .iter()
            .filter_map(|entry| {
                let id = entry.get("value")?.as_str().filter(|v| !v.is_empty())?;
                let mut row = Map::new();
                row.insert("id".into(), json!(id));
                row.insert(
                    "name".into(),
                    json!(
                        entry
                            .get("displayName")
                            .and_then(Value::as_str)
                            .unwrap_or(id)
                    ),
                );
                if let Some(description) = entry.get("description").and_then(Value::as_str) {
                    row.insert("description".into(), json!(description));
                }
                if entry.get("supportsEffort") == Some(&Value::Bool(true))
                    && let Some(levels) =
                        entry.get("supportedEffortLevels").filter(|l| l.is_array())
                {
                    row.insert("efforts".into(), levels.clone());
                }
                Some(Value::Object(row))
            })
            .collect(),
    )
}

/// `commandNames`: commands as objects at initialize time and as names in the
/// later init message, reduced to names.
pub(super) fn command_names(value: Option<&Value>) -> Option<Vec<String>> {
    let entries = value?.as_array()?;
    Some(
        entries
            .iter()
            .filter_map(|entry| match entry {
                Value::String(name) => Some(name.clone()),
                Value::Object(object) => object.get("name")?.as_str().map(str::to_owned),
                _ => None,
            })
            .collect(),
    )
}

pub(super) fn is_uuid(id: &str) -> bool {
    let bytes = id.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

/// The client echoes `uuid` back when it reads a message; Tendly's ids are
/// UUIDs already, and any other id goes out as a fresh one.
pub(super) fn wire_id(id: &str) -> String {
    if is_uuid(id) {
        id.to_owned()
    } else {
        uuid::Uuid::new_v4().to_string()
    }
}

/// `keepsSelection`: whether a listed selection still describes the model
/// the client resolved it to.
pub fn keeps_selection(models: Option<&[Value]>, selected: Option<&str>, reported: &str) -> bool {
    let listed = selected.is_some_and(|selected| {
        models.is_some_and(|models| {
            models
                .iter()
                .any(|entry| entry.get("id").and_then(Value::as_str) == Some(selected))
        })
    });
    let Some(selected) = selected.filter(|s| !s.is_empty() && *s != reported && listed) else {
        return selected == Some(reported);
    };
    if selected == "default" {
        return true;
    }
    let base = match (selected.find('['), selected.ends_with(']')) {
        (Some(at), true) => &selected[..at],
        _ => selected,
    };
    reported.to_lowercase().contains(&base.to_lowercase())
}

pub(super) fn first_line_short(line: &str) -> String {
    let first = line.split('\n').next().unwrap_or("");
    if first.encode_utf16().count() > 80 {
        let units: Vec<u16> = first.encode_utf16().take(79).collect();
        format!("{}\u{2026}", String::from_utf16_lossy(&units))
    } else {
        first.to_owned()
    }
}

/// `callSummary`: a few words on what a call does, from what its input says.
pub fn call_summary(input: Option<&Value>) -> Option<String> {
    let input = input?.as_object()?;
    let text = |key: &str| {
        input
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
    };
    let file = |path: &str| {
        let parts: Vec<&str> = path.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
        parts[parts.len().saturating_sub(2)..].join("/")
    };
    let path = text("file_path")
        .or_else(|| text("notebook_path"))
        .or_else(|| text("path"));
    let pattern = text("pattern");
    let url = text("url");
    if let Some(description) = text("description") {
        return Some(first_line_short(&description));
    }
    if let Some(command) = text("command") {
        return Some(first_line_short(&command));
    }
    if let Some(pattern) = pattern {
        return Some(first_line_short(&match &path {
            Some(path) => format!("{pattern} \u{00b7} {}", file(path)),
            None => pattern,
        }));
    }
    if let Some(path) = path {
        return Some(file(&path));
    }
    if let Some(query) = text("query") {
        return Some(first_line_short(&query));
    }
    let url = url?;
    let bare = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(&url);
    Some(first_line_short(bare))
}

/// `turnUsage`: a turn's tokens from its result.
pub fn turn_usage(usage: Option<&Value>) -> Option<Value> {
    let usage = usage.filter(|u| u.is_object())?;
    let n = |key: &str| non_negative(usage.get(key)).unwrap_or(0);
    let cached = n("cache_read_input_tokens");
    let input = n("input_tokens") + cached + n("cache_creation_input_tokens");
    let output = n("output_tokens");
    if input == 0 && output == 0 {
        return None;
    }
    Some(json!({"input": input, "output": output, "cached": cached}))
}

/// `resultTokens`: roughly how many tokens a tool result puts into context.
pub fn result_tokens(text: &str) -> i64 {
    let (mut ascii, mut other) = (0f64, 0f64);
    for c in text.chars() {
        if (c as u32) < 128 {
            ascii += 1.0;
        } else {
            other += 1.0;
        }
    }
    (ascii / 4.0 + other).round() as i64
}

/// The one case-insensitive test of a failed result's diagnostic:
/// `/oauth|subscription.*(?:not|unsupported)|authentication.*(?:not|unsupported)/i`.
pub(super) fn unsupported_authentication(diagnostic: &str) -> bool {
    let lower = diagnostic.to_lowercase();
    let followed = |head: &str| {
        lower.find(head).is_some_and(|at| {
            let rest = &lower[at + head.len()..];
            rest.contains("not") || rest.contains("unsupported")
        })
    };
    lower.contains("oauth") || followed("subscription") || followed("authentication")
}

/// Why a turn failed, from what the client reports. Claude Code names the
/// cause in `result` and the endpoint's status in `api_error_status`; it
/// leaves `errors` empty, so reading only that made every failed request
/// look like the same guess. Mirrors `failureReason` in chat-claude.mjs.
pub(super) fn failure_reason(message: &Value) -> String {
    let status = safe_integer(message.get("api_error_status")).filter(|s| (100..600).contains(s));
    let advice = match status {
        Some(401 | 403) => "Check the key, and that the endpoint takes it as an x-api-key header.",
        Some(404) => "Check the model name, and that the endpoint serves Anthropic Messages.",
        Some(429) => "The endpoint is limiting requests; try again later.",
        Some(500..=599) => "The endpoint failed; try again later.",
        _ => "Check its native account, endpoint and permission settings.",
    };
    let code = status.map(|s| format!(" (HTTP {s})")).unwrap_or_default();
    let said = message
        .get("result")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|said| !said.is_empty())
        .map(|said| format!(" Claude Code said: {}", clip(said, 600)))
        .unwrap_or_default();
    format!("Claude Code could not complete the turn{code}. {advice}{said}")
}

pub(super) fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}
