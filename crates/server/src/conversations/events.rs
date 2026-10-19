//! What a client may report, and what each event changes before it is stored.

use super::*;

pub(crate) fn normalize_event(mut value: Value) -> Option<Value> {
    let object = value.as_object_mut()?;
    let allowed: &[&str] = match object.get("type")?.as_str()? {
        "ready" => &["type", "native_session_id", "commands"],
        "settings" => &[
            "type",
            "model",
            "effort",
            "models",
            "permission_mode",
            "permission_modes",
        ],
        "cleared" => &["type"],
        "message" => &["type", "id", "role", "text", "delta"],
        "tool" => &[
            "type",
            "id",
            "name",
            "status",
            "text",
            "activity",
            "parent",
            "stats",
            "at",
            "background",
            "result_tokens",
        ],
        "approval" => &[
            "type",
            "id",
            "title",
            "text",
            "choices",
            "questions",
            "scope",
        ],
        "approval_resolved" => &["type", "id"],
        "turn" => &["type", "id", "status"],
        "usage" => &[
            "type",
            "input_tokens",
            "output_tokens",
            "context_tokens",
            "context_window",
            "turn",
            "message_id",
        ],
        "error" => &["type", "message"],
        "exit" => &["type"],
        _ => return None,
    };
    object.retain(|key, _| allowed.contains(&key.as_str()));
    let text = |key: &str| object.get(key).is_some_and(Value::is_string);
    let id = || {
        object
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty() && s.len() <= 256)
    };
    let valid = match object.get("type").and_then(Value::as_str)? {
        "ready" => {
            object.get("native_session_id").is_none_or(Value::is_string)
                // Command names are forwarded from the client's own list, so a
                // malformed entry drops the list rather than reaching the UI.
                && object.get("commands").is_none_or(|value| {
                    value.as_array().is_some_and(|names| {
                        names.len() <= 400
                            && names.iter().all(|name| {
                                name.as_str().is_some_and(|name| {
                                    !name.is_empty()
                                        && name.len() <= 64
                                        && name.chars().all(|c| {
                                            c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ':')
                                        })
                                })
                            })
                    })
                })
        }
        // A model list is the client's own; a malformed entry drops the list
        // rather than reaching the picker as something half-known.
        // Lengths are counted in characters, because that is what the bridge
        // clips by. Counting bytes here rejected a whole model list over a
        // description that was within its limit but contained a multi-byte
        // character, and a rejected event looks to the user like a client that
        // offers no models at all.
        "settings" => {
            let chars = |value: &Value, limit: usize| {
                value.as_str().is_some_and(|v| v.chars().count() <= limit)
            };
            object.get("model").is_none_or(|value| {
                value
                    .as_str()
                    .is_some_and(|v| !v.is_empty() && v.chars().count() <= 128)
            }) && object.get("effort").is_none_or(|value| {
                value
                    .as_str()
                    .is_some_and(|v| providers::EFFORT_LEVELS.contains(&v))
            }) && object.get("models").is_none_or(|value| {
                value.as_array().is_some_and(|rows| {
                    rows.len() <= 64
                        && rows.iter().all(|row| {
                            row.get("id").is_some_and(|v| {
                                v.as_str().is_some_and(|v| !v.is_empty()) && chars(v, 128)
                            }) && row.get("name").is_some_and(|v| chars(v, 128))
                                && row.get("description").is_none_or(|v| chars(v, 256))
                                && row.get("isDefault").is_none_or(Value::is_boolean)
                                && row.get("efforts").is_none_or(|v| {
                                    v.as_array().is_some_and(|levels| {
                                        levels.len() <= 16
                                            && levels.iter().all(|level| {
                                                level.as_str().is_some_and(|level| {
                                                    providers::EFFORT_LEVELS.contains(&level)
                                                })
                                            })
                                    })
                                })
                        })
                })
            })
        }
        "message" => {
            id() && text("text")
                && matches!(
                    object.get("role").and_then(Value::as_str),
                    Some("user" | "assistant")
                )
                && object.get("delta").is_none_or(Value::is_boolean)
        }
        "tool" => {
            id() && text("name")
                && object.get("text").is_none_or(Value::is_string)
                // Subagent progress, kept apart from `text` so it cannot displace
                // the tool's own input and result.
                && object.get("activity").is_none_or(Value::is_string)
                // The subagent call this one ran inside, and what the client
                // counted for a background task.
                && object.get("parent").is_none_or(|value| {
                    value.as_str().is_some_and(|s| !s.is_empty() && s.len() <= 256)
                })
                && object.get("at").is_none_or(Value::is_u64)
                && object.get("background").is_none_or(Value::is_boolean)
                && object.get("result_tokens").is_none_or(Value::is_u64)
                && object.get("stats").is_none_or(|value| {
                    value.as_object().is_some_and(|stats| {
                        stats.iter().all(|(key, value)| {
                            matches!(key.as_str(), "tokens" | "tools" | "ms") && value.is_u64()
                        })
                    })
                })
                && matches!(
                    object.get("status").and_then(Value::as_str),
                    Some("running" | "completed" | "failed")
                )
        }
        "approval" => {
            id() && text("title")
                && text("text")
                && object
                    .get("choices")
                    .and_then(Value::as_array)
                    .is_some_and(|choices| {
                        !choices.is_empty()
                            && choices.len() <= 4
                            && choices.iter().all(|v| {
                                matches!(
                                    v.as_str(),
                                    Some("accept" | "accept_session" | "decline" | "cancel")
                                )
                            })
                    })
                // What the request reaches for: an access kind, paths outside
                // the workspace, a command. Bounded, and only ever displayed.
                && object.get("scope").is_none_or(|scope| {
                    scope.as_object().is_some_and(|scope| {
                        scope.keys().all(|key| matches!(key.as_str(), "access" | "outside" | "command"))
                            && scope.get("access").is_none_or(|access| {
                                matches!(access.as_str(), Some("read" | "write" | "execute" | "network"))
                            })
                            && scope.get("outside").is_none_or(|paths| {
                                paths.as_array().is_some_and(|paths| {
                                    paths.len() <= 8
                                        && paths.iter().all(|path| path.as_str().is_some_and(|path| path.len() <= 1024))
                                })
                            })
                            && scope
                                .get("command")
                                .is_none_or(|command| command.as_str().is_some_and(|command| command.len() <= 2400))
                    })
                })
                && object.get("questions").is_none_or(|value| {
                    value.as_array().is_some_and(|qs| {
                        qs.len() <= 16
                            && qs.iter().all(|q| {
                                q["id"].is_string()
                                    && q["question"].is_string()
                                    && q["options"].as_array().is_some_and(|options| {
                                        options.len() <= 32
                                            && options.iter().all(|o| o["label"].is_string())
                                    })
                            })
                    })
                })
        }
        "approval_resolved" => id(),
        "turn" => {
            object.get("id").is_none_or(Value::is_string)
                && matches!(
                    object.get("status").and_then(Value::as_str),
                    Some("running" | "completed" | "failed" | "interrupted")
                )
        }
        "usage" => {
            ["input_tokens", "output_tokens", "context_tokens", "context_window"]
                .iter()
                .all(|key| {
                    object
                        .get(*key)
                        .is_none_or(|v| v.as_u64().is_some_and(|n| n <= 9_007_199_254_740_991))
                })
                // A zero window would make every share look like 100%, so it is
                // dropped rather than displayed.
                && object
                    .get("context_window")
                    .is_none_or(|v| v.as_u64().is_some_and(|n| n > 0))
                // The reply a cost belongs to, when the client names one.
                && object.get("message_id").is_none_or(|v| {
                    v.as_str().is_some_and(|s| !s.is_empty() && s.len() <= 256)
                })
                // What a turn cost, read and written, as the client summed it.
                && object.get("turn").is_none_or(|value| {
                    value.as_object().is_some_and(|turn| {
                        turn.iter().all(|(key, value)| {
                            matches!(key.as_str(), "input" | "output" | "cached")
                                && value.is_u64()
                        })
                    })
                })
        }
        "error" => text("message"),
        "exit" | "cleared" => true,
        _ => false,
    };
    if !valid {
        return None;
    }
    Some(value)
}
/// The event path, for tests that drive it without a bridge process.
#[cfg(test)]
pub(crate) async fn handle_event_for_test(
    state: &AppState,
    id: SessionId,
    runtime: &ChatRuntime,
    event: Value,
) -> Result<()> {
    handle_event(state, id, runtime, event).await
}

pub(super) async fn handle_event(
    state: &AppState,
    id: SessionId,
    runtime: &ChatRuntime,
    event: Value,
) -> Result<()> {
    match event["type"].as_str() {
        Some("ready") => {
            if let Some(native_id) = event["native_session_id"].as_str() {
                if !crate::native_history::valid_id(native_id) {
                    return Err(ApiError::bad("Invalid native conversation identifier"));
                }
                let native_id = native_id.to_owned();
                db(state, move |s| s.set_native_conversation_id(id, &native_id)).await?;
            }
        }
        Some("turn") => match event["status"].as_str() {
            Some("running") => runtime.busy.store(true, Ordering::Release),
            Some("completed" | "failed" | "interrupted") => {
                runtime.busy.store(false, Ordering::Release)
            }
            _ => return Err(ApiError::bad("Invalid turn state")),
        },
        Some("approval") => {
            let request = event["id"]
                .as_str()
                .filter(|s| s.len() <= 200)
                .ok_or_else(|| ApiError::bad("Invalid approval identifier"))?;
            let choices = event["choices"]
                .as_array()
                .ok_or_else(|| ApiError::bad("Missing native approval choices"))?
                .iter()
                .filter_map(|v| v.as_str())
                .filter(|s| matches!(*s, "accept" | "accept_session" | "decline" | "cancel"))
                .map(str::to_owned)
                .collect();
            let first = {
                let mut approvals = runtime.approvals.lock().expect("approvals");
                let first = approvals.is_empty();
                approvals.insert(request.into(), choices);
                first
            };
            // The session list says a session is waiting on you, not just
            // running, so every view can tell the two apart.
            if first {
                db(state, move |s| {
                    s.set_session_status(id, SessionStatus::Waiting)
                })
                .await?;
            }
        }
        Some("approval_resolved") => {
            if let Some(request) = event["id"].as_str() {
                let last = {
                    let mut approvals = runtime.approvals.lock().expect("approvals");
                    approvals.remove(request).is_some() && approvals.is_empty()
                };
                if last && runtime.running() {
                    db(state, move |s| {
                        s.set_session_status(id, SessionStatus::Running)
                    })
                    .await?;
                }
            }
        }
        Some("message") if event["role"] == "user" => return Ok(()), // The durable submission already contains this exact user message.
        _ => {}
    }
    let is_ready = event["type"] == "ready";
    persist(state, id, event).await?;
    if is_ready {
        runtime.ready.store(true, Ordering::Release);
        runtime.changed.notify_waiters();
    }
    Ok(())
}
