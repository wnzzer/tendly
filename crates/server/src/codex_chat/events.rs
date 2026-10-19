//! Turning app-server notifications into conversation events: turns,
//! messages, reasoning, token usage, and items such as commands, file changes
//! and tool calls.

use super::*;

impl Codex {
    pub(super) fn on_notification(&mut self, message: Value) {
        if !message.is_object() {
            return;
        }
        if truthy(message.get("method")) && message.get("id").is_some() {
            self.request(message);
            return;
        }
        let empty = Value::Object(Map::new());
        let p = message
            .get("params")
            .filter(|v| !v.is_null())
            .unwrap_or(&empty)
            .clone();
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        if truthy(p.get("threadId"))
            && let Some(current) = &self.core.native_session_id
            && p.get("threadId") != Some(&json!(current))
        {
            return;
        }
        match method {
            "serverRequest/resolved" => {
                let key = request_key(p.get("requestId").unwrap_or(&Value::Null));
                self.core.resolve_native(&key);
                return;
            }
            "thread/tokenUsage/updated" => {
                let usage = p.get("tokenUsage");
                let last = usage.and_then(|u| u.get("last")).filter(|v| !v.is_null());
                let chosen = last.or_else(|| usage.and_then(|u| u.get("total")));
                let context = last.and_then(|l| l.get("totalTokens")).cloned();
                let window = usage.and_then(|u| u.get("modelContextWindow")).cloned();
                self.core
                    .usage(chosen.cloned().as_ref(), context.as_ref(), window.as_ref());
                if let (Some(turn), Some(last)) = (self.core.active, last) {
                    let native = self.core.turns[turn].extra.native_turn.as_ref();
                    if !truthy(p.get("turnId")) || !truthy(native) || p.get("turnId") == native {
                        let input = count(last.get("inputTokens"));
                        let output = count(last.get("outputTokens"));
                        let cached = count(last.get("cachedInputTokens"));
                        if input != 0 || output != 0 {
                            self.core.turns[turn].extra.turn_usage =
                                Some(json!({"input":input,"output":output,"cached":cached}));
                        }
                    }
                }
                return;
            }
            "error" => {
                self.core.error("Codex reported a native turn error. Check its account, endpoint and permission settings.");
                return;
            }
            _ => {}
        }
        let Some(turn) = self.core.active else {
            return;
        };
        if truthy(p.get("turnId"))
            && truthy(self.core.turns[turn].extra.native_turn.as_ref())
            && p.get("turnId") != self.core.turns[turn].extra.native_turn.as_ref()
        {
            return;
        }
        match method {
            "turn/started" => {
                self.core.turns[turn].extra.native_turn =
                    p.get("turn").and_then(|t| t.get("id")).cloned();
                if self.core.turns[turn].interrupted {
                    self.send_interrupt(turn);
                }
            }
            "turn/completed" => {
                let status = match p
                    .get("turn")
                    .and_then(|t| t.get("status"))
                    .and_then(Value::as_str)
                {
                    Some("interrupted") => "interrupted",
                    Some("failed") => "failed",
                    _ => "completed",
                };
                if let Some(spent) = self.core.turns[turn].extra.turn_usage.clone() {
                    self.core.emit(json!({"type":"usage","turn":spent}));
                }
                self.core.finish(status);
                if status == "interrupted" {
                    self.deferred.clear();
                } else {
                    self.continue_deferred();
                }
            }
            "item/agentMessage/delta" | "item/plan/delta" => {
                if let (Some(id), Some(delta)) = (
                    p.get("itemId").and_then(Value::as_str),
                    p.get("delta").and_then(Value::as_str),
                ) {
                    let text = clip(delta, MAX_TEXT);
                    self.core.emit(json!({"type":"message","id":id,"role":"assistant","text":text,"delta":true}));
                }
            }
            "item/commandExecution/outputDelta" => {
                let Some(id) = p.get("itemId").and_then(Value::as_str).map(str::to_owned) else {
                    return;
                };
                let (name, text) = self
                    .tools
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| ("Command".into(), String::new()));
                let delta = p.get("delta").and_then(Value::as_str).unwrap_or("");
                let text = clip(&format!("{text}{delta}"), MAX_TEXT);
                self.tools.insert(id.clone(), (name.clone(), text.clone()));
                // A chatty command streams its output in hundreds of deltas;
                // forwarding the whole block on each floods the stream and the
                // stored history, so throttle it. item/completed below still
                // carries the complete text. See `should_emit_output`.
                let now = Instant::now();
                let last = self.output_emits.get(&id).copied();
                let grew = text.len().saturating_sub(last.map_or(0, |(_, len)| len));
                let elapsed = last.map_or(OUTPUT_EMIT_INTERVAL, |(at, _)| {
                    now.saturating_duration_since(at)
                });
                if last.is_none() || should_emit_output(grew, elapsed) {
                    if self.output_emits.len() > 1024 {
                        self.output_emits.clear();
                    }
                    self.output_emits.insert(id.clone(), (now, text.len()));
                    self.core.emit(
                        json!({"type":"tool","id":id,"name":name,"status":"running","text":text}),
                    );
                }
            }
            "item/started" | "item/completed" => self.item(method, &p),
            _ => {}
        }
    }

    pub(super) fn item(&mut self, method: &str, p: &Value) {
        let Some(item) = p.get("item") else {
            return;
        };
        let Some(id) = item.get("id").and_then(Value::as_str).map(str::to_owned) else {
            return;
        };
        let kind = item.get("type").and_then(Value::as_str);
        if matches!(kind, Some("agentMessage" | "plan")) {
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                let text = clip(text, MAX_TEXT);
                self.core.emit(
                    json!({"type":"message","id":id,"role":"assistant","text":text,"delta":false}),
                );
            }
            return;
        }
        if matches!(kind, Some("userMessage" | "reasoning")) {
            return;
        }
        let mut name = [item.get("tool"), item.get("type")]
            .into_iter()
            .find(|v| truthy(*v))
            .flatten()
            .cloned()
            .unwrap_or_else(|| json!("Native tool"));
        let text = match kind {
            Some("commandExecution") => {
                name = json!("Command");
                [item.get("command"), item.get("aggregatedOutput")]
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            Some("fileChange") => {
                name = json!("File changes");
                let changes = match item.get("changes") {
                    None | Some(Value::Null) => Vec::new(),
                    Some(Value::Array(changes)) => changes.clone(),
                    // The bridge's `.map` throws on anything else, and a throw
                    // in a handler breaks the stream.
                    Some(_) => return self.failure("Invalid structured message received."),
                };
                let mut parts = Vec::new();
                for change in &changes {
                    if !change.is_object() {
                        return self.failure("Invalid structured message received.");
                    }
                    let part = |key: &str| {
                        change
                            .get(key)
                            .filter(|v| !v.is_null())
                            .map(js_string)
                            .unwrap_or_default()
                    };
                    parts.push(format!("{}\n{}", part("path"), part("diff")));
                }
                parts.join("\n")
            }
            Some("mcpToolCall") => {
                name = json!(
                    [item.get("server"), item.get("tool")]
                        .into_iter()
                        .filter(|v| truthy(*v))
                        .flatten()
                        .map(js_string)
                        .collect::<Vec<_>>()
                        .join(" / ")
                );
                stringify(
                    item.get("arguments")
                        .filter(|v| !v.is_null())
                        .unwrap_or(&json!({})),
                )
            }
            _ => item
                .get("query")
                .and_then(Value::as_str)
                .or_else(|| item.get("path").and_then(Value::as_str))
                .unwrap_or("")
                .to_owned(),
        };
        let text = clip(&text, MAX_TEXT);
        let name = clip_value(Some(&name), 256);
        self.tools.insert(id.clone(), (name.clone(), text.clone()));
        if self.tools.len() > 512 {
            self.tools.shift_remove_index(0);
        }
        let status = if method == "item/started" {
            "running"
        } else {
            // The command is done streaming: forget its output throttle.
            self.output_emits.remove(&id);
            item_status(item)
        };
        self.core
            .emit(json!({"type":"tool","id":id,"name":name,"status":status,"text":text}));
    }
}
