//! What the bridge does to every string and value it handles: `clip`,
//! JavaScript's truthiness and `String`, client ids, secrets, paths and the
//! check of a launch.
use crate::chat_adapter::ChatInit;
use serde_json::Value;
use std::path::{Component, Path, PathBuf};

/// `clip`: no terminal escapes or control characters, and at most `limit`
/// bytes, cut where a multi-byte character may be split.
pub fn clip(value: &str, limit: usize) -> String {
    let mut cleaned = String::with_capacity(value.len());
    let chars: Vec<char> = value.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        // \x1b[ params intermediates final, as /\u001b\[[0-?]*[ -/]*[@-~]/.
        if chars[i] == '\u{1b}' && chars.get(i + 1) == Some(&'[') {
            let mut j = i + 2;
            while j < chars.len() && ('0'..='?').contains(&chars[j]) {
                j += 1;
            }
            while j < chars.len() && (' '..='/').contains(&chars[j]) {
                j += 1;
            }
            if j < chars.len() && ('@'..='~').contains(&chars[j]) {
                i = j + 1;
                continue;
            }
        }
        cleaned.push(chars[i]);
        i += 1;
    }
    cleaned.retain(|c| !matches!(c as u32, 0x00..=0x08 | 0x0b | 0x0c | 0x0e..=0x1f));
    if cleaned.len() <= limit {
        return cleaned;
    }
    let mut cut = String::from_utf8_lossy(&cleaned.as_bytes()[..limit]).into_owned();
    cut.push_str("\n[truncated]");
    cut
}

/// `clip` of anything: what is not a string becomes empty.
pub fn clip_value(value: Option<&Value>, limit: usize) -> String {
    value
        .and_then(Value::as_str)
        .map(|v| clip(v, limit))
        .unwrap_or_default()
}

/// `nativeId`: an id a client may use for a session or thread.
pub fn native_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes.next().is_some_and(|b| b.is_ascii_alphanumeric())
        && value.len() <= 128
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Take a resume ID from the launch arguments, refusing a malformed one or one
/// that disagrees with the ID already known. Returns whether it was accepted.
pub fn take_resume(resume: &mut Option<String>, id: String) -> bool {
    if !native_id(&id) || resume.as_ref().is_some_and(|known| *known != id) {
        return false;
    }
    *resume = Some(id);
    true
}

/// JavaScript truthiness of a JSON value.
pub fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// `String(value)`, for the template strings the bridge builds.
pub fn js_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Array(items) => items
            .iter()
            .map(|item| match item {
                Value::Null => String::new(),
                other => js_string(other),
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

/// `JSON.stringify`, which keeps a value's own key order (serde_json is built
/// with `preserve_order` for exactly this).
pub fn stringify(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

/// An integer JavaScript would count as `Number.isSafeInteger`.
pub fn safe_integer(value: Option<&Value>) -> Option<i64> {
    let number = value?.as_f64()?;
    (number.fract() == 0.0 && number.abs() <= 9_007_199_254_740_991.0).then_some(number as i64)
}

/// A token count as the bridge reads one: a safe, non-negative integer, else 0.
pub fn count(value: Option<&Value>) -> i64 {
    safe_integer(value).filter(|n| *n >= 0).unwrap_or(0)
}

/// `redactEvents`: every secret-looking variable's value in the client's
/// environment, longest first, becomes `[redacted]` wherever it appears.
pub fn secrets_of(environment: impl Iterator<Item = (String, String)>) -> Vec<String> {
    const NAMES: [&str; 6] = [
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PRIVATE_KEY",
        "API_KEY",
        "AUTHORIZATION",
    ];
    let mut secrets: Vec<String> = environment
        .filter(|(key, value)| {
            let key = key.to_uppercase();
            NAMES.iter().any(|name| key.contains(name)) && value.encode_utf16().count() >= 4
        })
        .map(|(_, value)| value)
        .collect();
    secrets.sort_by_key(|value| std::cmp::Reverse(value.encode_utf16().count()));
    secrets
}

pub fn scrub(value: Value, secrets: &[String]) -> Value {
    match value {
        Value::String(text) => Value::String(secrets.iter().fold(text, |text, secret| {
            text.replace(secret.as_str(), "[redacted]")
        })),
        Value::Array(items) => Value::Array(items.into_iter().map(|v| scrub(v, secrets)).collect()),
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .map(|(key, item)| (key, scrub(item, secrets)))
                .collect(),
        ),
        other => other,
    }
}

/// `path.resolve(root, value)`, lexically, as the bridge reports paths.
pub fn resolve(root: &Path, value: &str) -> PathBuf {
    let joined = root.join(value);
    let mut out = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `validateChatInput(init, true)`.
pub fn valid_init(init: &ChatInit) -> bool {
    !init.program.is_empty()
        && !init.program.contains('\0')
        && Path::new(&init.cwd).is_absolute()
        && init.args.len() <= 512
        && !init.args.iter().any(|arg| arg.contains('\0'))
        && serde_json::to_string(&init.args).is_ok_and(|s| s.len() <= 64 * 1024)
        && init.resume_id.as_deref().is_none_or(native_id)
}
