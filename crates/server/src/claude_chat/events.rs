//! Turning Claude Code's stream into conversation events: messages, tool
//! calls and their results, subagents, background tasks and the end of a turn.

use super::*;

impl Claude {
    /// One message from Claude Code's stream, in order of precedence:
    ///
    /// - control requests (approvals, questions) and their cancellations go
    ///   to the approval handling;
    /// - `system` messages report the permission mode the client is really
    ///   in, and `init` announces the session, its commands and its model;
    /// - a replayed user message confirms a message reached the model, which
    ///   opens a turn if none is open;
    /// - output carrying a `parent_tool_use_id` belongs to a subagent;
    /// - task messages track background commands;
    /// - everything else is the turn itself: streamed text, assistant and
    ///   user messages with their tool calls and results, tool progress, and
    ///   the `result` that ends the turn.
    ///
    /// Assistant output with no turn open starts one: the client carried on by
    /// itself after a background command finished.
    pub(super) fn on_notification(&mut self, message: Value) {
        if !message.is_object() {
            return;
        }
        let kind = message
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let subtype = message
            .get("subtype")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        if kind == "control_request" {
            self.request(&message);
            return;
        }
        if kind == "control_cancel_request" {
            let key = js_string(message.get("request_id").unwrap_or(&Value::Null));
            let key = if message.get("request_id").is_none() {
                "undefined".to_owned()
            } else {
                key
            };
            self.core.resolve_native(&key);
            return;
        }
        // The client reports the mode it is actually in, including after a
        // change it made itself.
        if kind == "system"
            && let Some(theirs) = message.get("permissionMode").and_then(Value::as_str)
            && let Some(mode) = our_mode(theirs)
            && self.core.permission_mode.as_deref() != Some(mode)
        {
            self.core.permission_mode = Some(mode.to_owned());
            self.report();
        }
        if kind == "system" && subtype == "init" {
            self.core.announce(
                message.get("session_id").and_then(Value::as_str),
                command_names(message.get("slash_commands")),
            );
            // The init message names the resolved model; the entry that still
            // describes it is kept, so the chip stays on "Sonnet".
            if let Some(reported) = message
                .get("model")
                .and_then(Value::as_str)
                .filter(|m| !m.is_empty())
                && !keeps_selection(self.models.as_deref(), self.model.as_deref(), reported)
            {
                self.model = Some(reported.to_owned());
            }
            let (models, model) = (self.models.clone(), self.model.clone());
            self.core
                .settings(models.as_deref(), model.as_deref(), None);
            return;
        }
        // The echo of a message the model just read. A steered one is no
        // longer waiting; one read with no turn open opens one.
        if kind == "user" && message.get("isReplay") == Some(&Value::Bool(true)) {
            if let Some(id) = message
                .get("uuid")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
            {
                self.unconsumed.shift_remove(id);
                if self.core.active.is_none() {
                    let id = if self.core.has_seen(id) {
                        id.to_owned()
                    } else {
                        format!("background-{id}")
                    };
                    self.begin_turn(id);
                }
            }
            return;
        }
        // An API failure arrives as a synthetic assistant message: the
        // endpoint's complaint, not the model's answer. The result that
        // follows repeats it and fails the turn.
        if kind == "assistant" && truthy(message.get("is_api_error_message")) {
            return;
        }
        let parent = message.get("parent_tool_use_id");
        // A turn the client starts by itself: a background command it waited
        // on finished, and it carries on.
        if self.core.active.is_none()
            && (kind == "assistant" || kind == "stream_event")
            && !truthy(parent)
        {
            self.begin_turn(format!("background-{}", uuid::Uuid::new_v4()));
        }
        // A subagent's output belongs to the tool call that started it, and
        // usually arrives after the turn that started it has ended.
        if let Some(parent) = parent.and_then(Value::as_str).filter(|p| !p.is_empty()) {
            let parent = parent.to_owned();
            self.subagent(&message, &parent);
            return;
        }
        if kind == "system"
            && [
                "task_started",
                "task_progress",
                "task_updated",
                "task_notification",
            ]
            .contains(&subtype.as_str())
        {
            self.task(&message, &subtype);
            return;
        }
        if self.core.active.is_none() {
            return;
        }
        match kind.as_str() {
            "stream_event" => {
                let event = message
                    .get("event")
                    .filter(|e| !e.is_null())
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let event_type = event.get("type").and_then(Value::as_str).unwrap_or("");
                if event_type == "message_start" {
                    self.assistant_id = event
                        .get("message")
                        .and_then(|m| m.get("id"))
                        .filter(|id| !id.is_null())
                        .cloned();
                } else if event_type == "content_block_delta"
                    && event
                        .get("delta")
                        .and_then(|d| d.get("type"))
                        .and_then(Value::as_str)
                        == Some("text_delta")
                {
                    let text = clip_value(event.get("delta").and_then(|d| d.get("text")), MAX_TEXT);
                    let id = self.assistant();
                    self.core.emit(json!({"type":"message","id":id,"role":"assistant","text":text,"delta":true}));
                } else if event_type == "content_block_start"
                    && event
                        .get("content_block")
                        .and_then(|b| b.get("type"))
                        .and_then(Value::as_str)
                        == Some("tool_use")
                {
                    let block = event.get("content_block").cloned().unwrap_or(Value::Null);
                    self.tool(&block, "running", None);
                }
            }
            "assistant" => {
                let body = message
                    .get("message")
                    .filter(|b| !b.is_null())
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let blocks: Vec<Value> = body
                    .get("content")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                if let Some(id) = body.get("id").filter(|id| id.is_string()) {
                    self.assistant_id = Some(id.clone());
                }
                let text: String = blocks
                    .iter()
                    .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|b| b.get("text").and_then(Value::as_str))
                    .collect();
                if !text.is_empty() {
                    self.final_assistant = true;
                    let id = self.assistant();
                    self.core.emit(json!({"type":"message","id":id,"role":"assistant","text":clip(&text, MAX_TEXT),"delta":false}));
                }
                for block in &blocks {
                    if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                        self.tool(block, "running", None);
                    }
                }
                let usage = body.get("usage").filter(|u| !u.is_null());
                let context = context_tokens(usage);
                self.core.usage(usage, context.as_ref(), None);
            }
            "user" => {
                if let Some(content) = message
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(Value::as_array)
                    .cloned()
                {
                    for result in &content {
                        if result.get("type").and_then(Value::as_str) == Some("tool_result")
                            && result.get("tool_use_id").is_some_and(Value::is_string)
                        {
                            self.result(result, None);
                        }
                    }
                }
            }
            "tool_progress" => {
                if let Some(id) = message.get("tool_use_id").and_then(Value::as_str) {
                    let name = match message.get("tool_name").filter(|n| !n.is_null()) {
                        Some(name) => clip_value(Some(name), 256),
                        None => "Native tool".to_owned(),
                    };
                    self.core
                        .emit(json!({"type":"tool","id":id,"name":name,"status":"running"}));
                }
            }
            "result" => self.finished(&message),
            _ => {}
        }
    }

    pub(super) fn finished(&mut self, message: &Value) {
        let Some(turn) = self.core.active else {
            return;
        };
        // Claude reports the window per model it used this turn; only a single
        // agreed size is forwarded.
        let mut windows: Vec<i64> = Vec::new();
        let values: Vec<&Value> = match message.get("modelUsage") {
            Some(Value::Object(map)) => map.values().collect(),
            Some(Value::Array(items)) => items.iter().collect(),
            _ => Vec::new(),
        };
        for entry in values {
            if let Some(window) = safe_integer(entry.get("contextWindow")).filter(|w| *w > 0)
                && !windows.contains(&window)
            {
                windows.push(window);
            }
        }
        let window = (windows.len() == 1).then(|| json!(windows[0]));
        let usage = message.get("usage").filter(|u| !u.is_null());
        self.core.usage(usage, None, window.as_ref());
        let failed = truthy(message.get("is_error"))
            || message.get("subtype").and_then(Value::as_str) != Some("success");
        // An empty result is not an answer, and a failed one is the client's
        // complaint, said in the error below.
        if !failed
            && !self.final_assistant
            && let Some(result) = message
                .get("result")
                .and_then(Value::as_str)
                .filter(|r| !r.trim().is_empty())
        {
            let id = self.assistant();
            self.core.emit(json!({"type":"message","id":id,"role":"assistant","text":clip(result, MAX_TEXT),"delta":false}));
        }
        // What the turn cost, after its reply.
        if let Some(spent) = turn_usage(usage) {
            self.core.emit(json!({"type":"usage","turn":spent}));
        }
        let interrupted = self.core.turns[turn].interrupted;
        if failed && !interrupted {
            let errors = message
                .get("errors")
                .filter(|e| !e.is_null())
                .cloned()
                .unwrap_or_else(|| json!([]));
            let reason = if unsupported_authentication(&stringify(&errors)) {
                "Claude Code does not support this authentication mode for this interface. Use its native client or an approved API configuration.".to_owned()
            } else {
                failure_reason(message)
            };
            self.core.error(&reason);
        }
        let status = if interrupted {
            "interrupted"
        } else if failed {
            "failed"
        } else {
            "completed"
        };
        self.core.finish(status);
        if !interrupted {
            self.continue_steered();
        }
    }

    /// Progress only: an activity line names what the subagent is doing; its
    /// own tool calls become cards nested under it.
    ///
    /// A subagent resumed by `SendMessage` still names the card it first ran
    /// under; its output goes to the card of the run it is in now. Output for
    /// a card whose task has ended never marks it running again: that left a
    /// finished subagent in the dock, "no progress" for hours.
    pub(super) fn subagent(&mut self, message: &Value, parent: &str) {
        let parent = self
            .resumed
            .get(parent)
            .map_or(parent, String::as_str)
            .to_owned();
        let parent = parent.as_str();
        let Some(parent_name) = self.tools.get(parent).cloned() else {
            return;
        };
        let mut lines: Vec<String> = Vec::new();
        let blocks: Vec<Value> = message
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        match message.get("type").and_then(Value::as_str) {
            Some("assistant") => {
                for block in &blocks {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            if let Some(text) = block
                                .get("text")
                                .and_then(Value::as_str)
                                .map(str::trim)
                                .filter(|t| !t.is_empty())
                            {
                                lines.push(text.to_owned());
                            }
                        }
                        Some("tool_use") => {
                            let what = call_summary(block.get("input"));
                            let name = match block.get("name").filter(|n| !n.is_null()) {
                                Some(name) => clip_value(Some(name), 120),
                                None => "tool".to_owned(),
                            };
                            lines.push(match what {
                                Some(what) => format!("\u{2192} {name} \u{00b7} {what}"),
                                None => format!("\u{2192} {name}"),
                            });
                            self.tool(block, "running", Some(parent));
                        }
                        _ => {}
                    }
                }
            }
            Some("user") => {
                for result in &blocks {
                    if result.get("type").and_then(Value::as_str) == Some("tool_result")
                        && let Some(id) = result.get("tool_use_id").and_then(Value::as_str)
                        && self.tools.contains_key(id)
                    {
                        self.result(result, Some(parent));
                    }
                }
            }
            _ => {}
        }
        if lines.is_empty() || self.ended.contains(parent) {
            return;
        }
        let name = self.tools.get(parent).cloned().unwrap_or(parent_name);
        self.core.emit(json!({"type":"tool","id":parent,"name":name,"status":"running","activity":clip(&lines.join("\n"), 4096),"at":now_millis()}));
    }

    pub(super) fn result(&mut self, result: &Value, parent: Option<&str>) {
        let id = result
            .get("tool_use_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let text = match result.get("content") {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Array(blocks)) => blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .map(|b| match b.get("text") {
                    None | Some(Value::Null) => String::new(),
                    Some(text) => js_string(text),
                })
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        };
        // A backgrounded task answers at once with a placeholder; the card
        // stays running until its task_notification says how it ended.
        let background = self.background.contains(&id);
        let status = if truthy(result.get("is_error")) {
            "failed"
        } else if background {
            "running"
        } else {
            "completed"
        };
        let tokens = result_tokens(&text);
        let mut event = Map::new();
        event.insert("type".into(), json!("tool"));
        event.insert("id".into(), json!(id));
        event.insert(
            "name".into(),
            json!(self.tools.get(&id).cloned().unwrap_or("Native tool".into())),
        );
        event.insert("status".into(), json!(status));
        event.insert("text".into(), json!(clip(&text, MAX_TEXT)));
        if tokens != 0 {
            event.insert("result_tokens".into(), json!(tokens));
        }
        if background {
            event.insert("background".into(), json!(true));
        }
        if let Some(parent) = parent {
            event.insert("parent".into(), json!(parent));
        }
        self.core.emit(Value::Object(event));
    }

    /// A background task (subagent or command) reports its own start and end.
    pub(super) fn task(&mut self, message: &Value, subtype: &str) {
        let task_id = message.get("task_id").and_then(Value::as_str);
        let id = match message.get("tool_use_id").and_then(Value::as_str) {
            Some(id) => Some(id.to_owned()),
            None => task_id.and_then(|task| self.tasks.get(task).cloned()),
        };
        let Some(id) = id.filter(|id| !id.is_empty() && self.tools.contains_key(id)) else {
            return;
        };
        let usage = message.get("usage").filter(|u| u.is_object());
        let mut stats = Map::new();
        for (key, field) in [
            ("tokens", "total_tokens"),
            ("tools", "tool_uses"),
            ("ms", "duration_ms"),
        ] {
            if let Some(count) = non_negative(usage.and_then(|u| u.get(field))) {
                stats.insert(key.into(), json!(count));
            }
        }
        match subtype {
            "task_started" => {
                if let Some(task) = task_id {
                    remember(&mut self.tasks, task.to_owned(), id.clone());
                    // The same task under another card: a resumed subagent.
                    match self.origins.get(task) {
                        Some(origin) if *origin != id => {
                            self.resumed.insert(origin.clone(), id.clone());
                        }
                        Some(_) => {}
                        None => remember(&mut self.origins, task.to_owned(), id.clone()),
                    }
                }
                if truthy(message.get("is_backgrounded")) {
                    self.background.insert(id.clone());
                    if self.background.len() > 512 {
                        self.background.shift_remove_index(0);
                    }
                }
                // A command's description is the command itself, already the
                // card's input; a subagent's is what it was asked to do.
                let line = if message.get("task_type").and_then(Value::as_str) == Some("local_bash")
                {
                    None
                } else {
                    message
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                };
                self.card(&id, "running", line.as_deref(), &stats);
            }
            "task_updated" => {
                if truthy(message.get("patch").and_then(|p| p.get("is_backgrounded"))) {
                    self.background.insert(id.clone());
                    self.card(&id, "running", None, &stats);
                }
            }
            "task_progress" => {
                // A one-line status that repeats until it changes; logged once.
                let summary = match message.get("summary") {
                    Some(Value::String(summary)) if self.summary_of.get(&id) == Some(summary) => {
                        None
                    }
                    other => other.cloned(),
                };
                if let Some(Value::String(summary)) = &summary {
                    self.summary_of.insert(id.clone(), summary.clone());
                }
                let line = summary.as_ref().and_then(Value::as_str).map(str::to_owned);
                self.card(&id, "running", line.as_deref(), &stats);
            }
            _ => {
                self.background.shift_remove(&id);
                if let Some(task) = task_id {
                    self.tasks.shift_remove(task);
                    if let Some(origin) = self.origins.get(task) {
                        self.resumed.remove(origin);
                    }
                }
                self.ended.insert(id.clone());
                if self.ended.len() > 512 {
                    self.ended.shift_remove_index(0);
                }
                self.summary_of.remove(&id);
                let status = if message.get("status").and_then(Value::as_str) == Some("completed") {
                    "completed"
                } else {
                    "failed"
                };
                let line = message
                    .get("summary")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                self.card(&id, status, line.as_deref(), &stats);
            }
        }
    }

    pub(super) fn card(
        &mut self,
        id: &str,
        status: &str,
        line: Option<&str>,
        stats: &Map<String, Value>,
    ) {
        let mut event = Map::new();
        event.insert("type".into(), json!("tool"));
        event.insert("id".into(), json!(id));
        event.insert(
            "name".into(),
            self.tools.get(id).map_or(Value::Null, |name| json!(name)),
        );
        event.insert("status".into(), json!(status));
        event.insert("at".into(), json!(now_millis()));
        if self.background.contains(id) {
            event.insert("background".into(), json!(true));
        }
        if !stats.is_empty() {
            event.insert("stats".into(), Value::Object(stats.clone()));
        }
        if let Some(line) = line.map(str::trim).filter(|l| !l.is_empty()) {
            event.insert("activity".into(), json!(clip(line, 4096)));
        }
        self.core.emit(Value::Object(event));
    }

    pub(super) fn tool(&mut self, block: &Value, status: &str, parent: Option<&str>) {
        let Some(id) = block.get("id").and_then(Value::as_str).map(str::to_owned) else {
            return;
        };
        let name = match block.get("name").filter(|n| !n.is_null()) {
            Some(name) => clip_value(Some(name), 256),
            None => "Native tool".to_owned(),
        };
        remember(&mut self.tools, id.clone(), name.clone());
        let input = block
            .get("input")
            .filter(|i| !i.is_null())
            .cloned()
            .unwrap_or_else(|| json!({}));
        let mut event = Map::new();
        event.insert("type".into(), json!("tool"));
        event.insert("id".into(), json!(id));
        event.insert("name".into(), json!(name));
        event.insert("status".into(), json!(status));
        event.insert("text".into(), json!(clip(&stringify(&input), MAX_TEXT)));
        if let Some(parent) = parent {
            event.insert("parent".into(), json!(parent));
        }
        self.core.emit(Value::Object(event));
    }
}
