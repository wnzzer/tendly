//! Pi RPC on the shared Rust conversation core.
//!
//! Pi's protocol is a command in (`{id, type, ...}`), a response per command
//! (`{type: "response", id, success, data | error}`) and a stream of agent
//! events out. Pi runs its tools without asking, so it has no approval modes;
//! what it does ask comes from extensions, as `extension_ui_request` dialogs.
use crate::chat_adapter::{Answers, ChatInit, Decision};
use crate::native_chat::{
    Core, LaunchArgs, MAX_TEXT, NativeAdapter, NativeClient, Protocol, clip, count, js_string,
    safe_integer, stringify, take_resume, valid_init,
};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

/// `piLaunch`: RPC mode in front of the flags a terminal run of Pi takes.
pub fn pi_launch(init: &ChatInit) -> Result<Vec<String>, String> {
    let mut args = vec!["--mode".to_owned(), "rpc".to_owned()];
    let mut resume = init.resume_id.clone();
    let mut launch = LaunchArgs::new(&init.args, "Pi", false);
    while let Some(arg) = launch.next_flag() {
        let flag = arg.name;
        let mut value = || launch.value(&arg);
        match flag {
            "--model" | "--provider" | "--thinking" | "--name" | "-n" => {
                let v = value()?;
                args.extend([flag.to_owned(), v]);
            }
            "--session" => {
                if !take_resume(&mut resume, value()?) {
                    return Err("Invalid or conflicting Pi session ID.".into());
                }
            }
            _ => return Err("Unsupported Pi launch option in structured mode.".into()),
        }
    }
    if let Some(resume) = resume {
        args.extend(["--session".to_owned(), resume]);
    }
    Ok(args)
}

/// `piFrame`: a Pi command, `{id, type, ...fields}`.
fn pi_frame(id: &str, method: &str, params: Value) -> Value {
    let mut frame = Map::new();
    frame.insert("id".into(), json!(id));
    frame.insert("type".into(), json!(method));
    if let Value::Object(fields) = params {
        frame.extend(fields);
    }
    Value::Object(frame)
}

/// `piReply`: a response, matched to its command by id.
fn pi_reply(message: &Value) -> Option<(Value, Result<Value, Value>)> {
    if message.get("type").and_then(Value::as_str) != Some("response") {
        return None;
    }
    let id = message.get("id")?.clone();
    if message.get("success") == Some(&Value::Bool(false)) {
        let stated = message.get("error").filter(|e| !e.is_null()).cloned();
        return Some((id, Err(stated.unwrap_or(Value::Bool(true)))));
    }
    let data = message
        .get("data")
        .filter(|d| !d.is_null())
        .cloned()
        .unwrap_or_else(|| json!({}));
    Some((id, Ok(data)))
}

/// `piModels`: its models as `provider/id`, the one name that runs a model of
/// any of its providers. Levels are offered only where the model reasons.
fn pi_models(data: &Value) -> Vec<Value> {
    data.get("models")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let id = row.get("id")?.as_str()?;
                    let provider = row.get("provider")?.as_str()?;
                    let name = row
                        .get("name")
                        .and_then(Value::as_str)
                        .filter(|n| !n.is_empty())
                        .unwrap_or(id);
                    let mut entry = Map::new();
                    entry.insert("id".into(), json!(format!("{provider}/{id}")));
                    entry.insert("name".into(), json!(name));
                    if row.get("reasoning") == Some(&Value::Bool(true)) {
                        entry.insert("efforts".into(), json!(["low", "medium", "high"]));
                    }
                    Some(Value::Object(entry))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The levels every client shares; Pi's `off` and `minimal` read as none.
const SHARED_LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// `modelName`: `provider/id`.
fn model_name(model: Option<&Value>) -> Option<String> {
    let model = model?;
    let provider = model.get("provider")?.as_str()?;
    let id = model.get("id")?.as_str()?;
    Some(format!("{provider}/{id}"))
}

/// `textOf`: a message's text, from a string or its text blocks.
fn text_of(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
            .map(|block| match block.get("text") {
                None | Some(Value::Null) => String::new(),
                Some(text) => js_string(text),
            })
            .collect(),
        _ => String::new(),
    }
}

/// What follows one of Pi's replies.
enum Then {
    State,
    Commands {
        state: Value,
    },
    Models,
    SetModel {
        named: String,
        effort: Option<String>,
    },
    SetThinking {
        effort: String,
    },
    Prompt {
        turn: usize,
    },
    Steer,
    Abort {
        turn: usize,
    },
    /// Pi's state after a model or depth change: what it actually runs.
    Follow,
    /// Pi's state after it answered a prompt whose run has not started.
    Idle {
        turn: usize,
    },
    /// A request whose answer changes nothing.
    Ignore,
}

/// How an extension's dialog is answered.
#[derive(Clone)]
enum Responder {
    Confirm { id: String },
    Answer { id: String },
}

struct Pi {
    core: Core<Then, Responder, ()>,
    program: String,
    models: Option<Vec<Value>>,
    model: Option<String>,
    effort: Option<String>,
    window: Option<i64>,
    message_count: u64,
    assistant_id: Option<String>,
    failed: bool,
    /// The turn whose run Pi has started (agent_start), if any.
    started: Option<usize>,
}

impl Pi {
    fn open(&mut self, init: ChatInit) {
        if !valid_init(&init) {
            self.core.raw_error("Invalid native chat initialization.");
            self.shutdown();
            return;
        }
        match std::fs::metadata(&init.cwd) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => {
                self.core.raw_error("Chat workspace is not a directory.");
                self.shutdown();
                return;
            }
            Err(_) => {
                let message = format!("ENOENT: no such file or directory, stat '{}'", init.cwd);
                self.core.raw_error(&message);
                self.shutdown();
                return;
            }
        }
        let args = match pi_launch(&init) {
            Ok(args) => args,
            Err(message) => {
                self.core.raw_error(&message);
                self.shutdown();
                return;
            }
        };
        self.core.cwd = init.cwd.clone();
        self.program = init.program.clone();
        self.core.initializing = true;
        let program = self.program.clone();
        if self.core.spawn(&program, &args).is_err() {
            // The first request is already waiting when a launch fails, as in
            // the bridge, and the close that follows fails it.
            self.core.waiting(Then::State);
            self.failure("Could not launch the configured native client.");
            return;
        }
        self.rpc("get_state", json!({}), Then::State, false);
    }
    /// `follow`: what the session is set to now, from Pi's own state.
    fn follow(&mut self, state: &Value) {
        if !state.is_object() {
            return;
        }
        if let Some(model) = model_name(state.get("model")) {
            self.model = Some(model);
        }
        if let Some(window) = safe_integer(state.get("model").and_then(|m| m.get("contextWindow")))
        {
            self.window = Some(window);
        }
        if let Some(level) = state.get("thinkingLevel").and_then(Value::as_str) {
            self.effort = SHARED_LEVELS.contains(&level).then(|| level.to_owned());
        }
        self.report();
    }
    fn report(&mut self) {
        let (models, model, effort) =
            (self.models.clone(), self.model.clone(), self.effort.clone());
        self.core
            .settings(models.as_deref(), model.as_deref(), effort.as_deref());
    }
    fn choose_model(&mut self, model: Option<String>, effort: Option<String>) {
        if self.core.active.is_some() {
            self.core
                .raw_error("Wait for the current turn to finish before changing the model.");
            return;
        }
        if let Some(model) = model {
            let Some(cut) = model.find('/').filter(|cut| *cut >= 1) else {
                self.core.raw_error("Name a Pi model as provider/id.");
                return;
            };
            let params = json!({"provider": &model[..cut], "modelId": &model[cut + 1..]});
            self.rpc(
                "set_model",
                params,
                Then::SetModel {
                    named: model,
                    effort,
                },
                false,
            );
            return;
        }
        self.set_thinking(effort);
    }
    fn set_thinking(&mut self, effort: Option<String>) {
        match effort {
            Some(effort) => {
                let params = json!({"level": effort});
                self.rpc(
                    "set_thinking_level",
                    params,
                    Then::SetThinking { effort },
                    false,
                );
            }
            // Pi clamps a level its model cannot use, and a new model brings
            // the level it allows, both without saying so: what it runs is
            // what get_state says.
            None => self.rpc("get_state", json!({}), Then::Follow, true),
        }
    }
    fn send_message(&mut self, id: String, content: String) {
        if let Some(turn) = self.core.begin(id, &content) {
            self.rpc(
                "prompt",
                json!({"message": content}),
                Then::Prompt { turn },
                false,
            );
        }
    }
    fn steer_turn(&mut self, id: String, content: String) {
        if self.core.active.is_none() {
            self.send_message(id, content);
            return;
        }
        if !self.core.accept_steer(&id, &content) {
            return;
        }
        self.rpc("steer", json!({"message": content}), Then::Steer, false);
    }
    fn interrupt_turn(&mut self) {
        let Some(turn) = self.core.active else {
            return;
        };
        self.core.turns[turn].interrupted = true;
        self.cancel_approvals();
        self.rpc("abort", json!({}), Then::Abort { turn }, false);
    }
    fn next_assistant(&mut self) -> String {
        self.message_count += 1;
        format!("pi-assistant-{}", self.message_count)
    }

    fn after(&mut self, then: Then, result: Result<Value, String>) {
        match then {
            Then::State => match result {
                Ok(state) => self.rpc("get_commands", json!({}), Then::Commands { state }, true),
                Err(message) => {
                    self.core.initializing = false;
                    self.core.raw_error(&message);
                    self.shutdown();
                }
            },
            Then::Commands { state } => {
                // A Pi without get_commands simply offers none.
                let commands = result.ok().and_then(|data| {
                    data.get("commands")
                        .and_then(Value::as_array)
                        .map(|commands| {
                            commands
                                .iter()
                                .filter_map(|c| {
                                    c.get("name").and_then(Value::as_str).map(str::to_owned)
                                })
                                .collect::<Vec<_>>()
                        })
                });
                let session = state
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                self.core.announce(session.as_deref(), commands);
                self.follow(&state);
                self.core.initializing = false;
                self.core.initialized = true;
                // Asking for models must not hold up the session.
                self.rpc("get_available_models", json!({}), Then::Models, true);
            }
            Then::Models => {
                if let Ok(data) = result {
                    self.models = Some(pi_models(&data));
                    self.report();
                }
            }
            Then::SetModel { named, effort } => match result {
                Ok(model) => {
                    self.model = Some(model_name(Some(&model)).unwrap_or(named));
                    if let Some(window) = safe_integer(model.get("contextWindow")) {
                        self.window = Some(window);
                    }
                    self.set_thinking(effort);
                }
                Err(message) => self.core.raw_error(&message),
            },
            Then::SetThinking { effort } => match result {
                Ok(_) => {
                    self.effort = Some(effort);
                    self.set_thinking(None);
                }
                Err(message) => self.core.raw_error(&message),
            },
            Then::Prompt { turn } => {
                if let Err(message) = result {
                    if self.core.active == Some(turn) {
                        self.core.error(&message);
                        self.core.finish("failed");
                    }
                    return;
                }
                // Pi answers a prompt an extension handled -- its command, or
                // its input handler -- and starts no run, so no agent_settled
                // will end this turn. A real run is already under way when Pi
                // answers (isStreaming), so a turn whose run has not started
                // and that Pi says is not streaming is over. Asked after every
                // prompt, so the answer does not hang on whether Pi's
                // agent_start was read before its reply.
                if self.core.active == Some(turn) {
                    self.rpc("get_state", json!({}), Then::Idle { turn }, true);
                }
            }
            Then::Idle { turn } => {
                if let Ok(state) = result
                    && self.core.active == Some(turn)
                    && self.started != Some(turn)
                    && state.get("isStreaming") == Some(&Value::Bool(false))
                {
                    let status = if self.core.turns[turn].interrupted {
                        "interrupted"
                    } else {
                        "completed"
                    };
                    self.core.finish(status);
                }
            }
            Then::Follow => {
                if let Ok(state) = result {
                    self.follow(&state);
                }
                self.report();
            }
            Then::Ignore => {}
            Then::Steer => {
                if let Err(message) = result {
                    self.core.error(&message);
                }
            }
            Then::Abort { turn } => {
                if result.is_err() && self.core.active == Some(turn) {
                    self.core.error("Pi could not confirm turn interruption.");
                }
            }
        }
    }

    /// One event from Pi's stream:
    ///
    /// - `extension_ui_request` is a dialog an extension opens, shown as a question;
    /// - `agent_start` marks the turn Tendly asked for as started (aborting it
    ///   at once if it was interrupted meanwhile), or opens a turn Pi began by
    ///   itself; `agent_settled` ends it as completed, interrupted or failed;
    /// - `message_*` events stream the assistant's text and, at the end of a
    ///   message, its usage and any error it stopped on;
    /// - `tool_execution_*` events become tool calls with their progress and result.
    fn on_notification(&mut self, message: Value) {
        if !message.is_object() {
            return;
        }
        let kind = message.get("type").and_then(Value::as_str).unwrap_or("");
        if kind == "extension_ui_request" {
            self.ask(&message);
            return;
        }
        // A run Pi starts by itself -- a queued follow-up, a retry -- is shown
        // like any other rather than dropped.
        // An interrupt that came before the run started found nothing to
        // stop: stop the run now that it has.
        if kind == "agent_start"
            && let Some(turn) = self.core.active
        {
            self.started = Some(turn);
            if self.core.turns[turn].interrupted {
                self.rpc("abort", json!({}), Then::Ignore, false);
            }
            return;
        }
        if kind == "agent_start" && self.core.active.is_none() {
            self.message_count += 1;
            let millis = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis());
            let id = format!("pi-{millis}-{}", self.message_count);
            self.started = Some(self.core.open_turn(id.clone()));
            self.core
                .emit(json!({"type":"turn","id":id,"status":"running"}));
            return;
        }
        if kind == "agent_settled" {
            if let Some(turn) = self.core.active {
                let status = if self.core.turns[turn].interrupted {
                    "interrupted"
                } else if self.failed {
                    "failed"
                } else {
                    "completed"
                };
                self.core.finish(status);
            }
            self.failed = false;
            return;
        }
        if self.core.active.is_none() {
            return;
        }
        let assistant = message
            .get("message")
            .and_then(|m| m.get("role"))
            .and_then(Value::as_str)
            == Some("assistant");
        match kind {
            "message_start" if assistant => {
                self.assistant_id = Some(self.next_assistant());
            }
            "message_update" => {
                let delta = message.get("assistantMessageEvent");
                if delta.and_then(|d| d.get("type")).and_then(Value::as_str) == Some("text_delta")
                    && let Some(text) = delta.and_then(|d| d.get("delta")).and_then(Value::as_str)
                {
                    if self.assistant_id.is_none() {
                        self.assistant_id = Some(self.next_assistant());
                    }
                    let id = self.assistant_id.clone();
                    let text = clip(text, MAX_TEXT);
                    self.core.emit(json!({"type":"message","id":id,"role":"assistant","text":text,"delta":true}));
                }
            }
            "message_end" if assistant => {
                let body = message.get("message").cloned().unwrap_or(Value::Null);
                let text = text_of(body.get("content"));
                if !text.is_empty() {
                    if self.assistant_id.is_none() {
                        self.assistant_id = Some(self.next_assistant());
                    }
                    let id = self.assistant_id.clone();
                    let text = clip(&text, MAX_TEXT);
                    self.core.emit(json!({"type":"message","id":id,"role":"assistant","text":text,"delta":false}));
                }
                let assistant_id = self.assistant_id.take();
                if let Some(usage) = body.get("usage").filter(|u| u.is_object() || u.is_array()) {
                    let context: i64 = ["input", "cacheRead", "cacheWrite"]
                        .iter()
                        .map(|key| safe_integer(usage.get(*key)).unwrap_or(0))
                        .sum();
                    let tokens = json!({"input_tokens": usage.get("input"), "output_tokens": usage.get("output")});
                    let context = (context != 0).then(|| json!(context));
                    let window = self.window.map(|w| json!(w));
                    self.core
                        .usage(Some(&tokens), context.as_ref(), window.as_ref());
                    // What this reply cost, read and written, under the reply.
                    let cached = count(usage.get("cacheRead"));
                    let input = count(usage.get("input")) + cached + count(usage.get("cacheWrite"));
                    let output = count(usage.get("output"));
                    if let Some(id) = assistant_id
                        && (input != 0 || output != 0)
                    {
                        self.core.emit(json!({"type":"usage","message_id":id,"turn":{"input":input,"output":output,"cached":cached}}));
                    }
                }
                if body.get("stopReason").and_then(Value::as_str) == Some("error") {
                    self.failed = true;
                    let reason = body
                        .get("errorMessage")
                        .and_then(Value::as_str)
                        .filter(|m| !m.is_empty())
                        .unwrap_or("Pi could not complete the turn.");
                    let reason = clip(reason, 2048);
                    self.core.error(&reason);
                }
            }
            "tool_execution_start" | "tool_execution_update" | "tool_execution_end" => {
                let Some(id) = message
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                else {
                    return;
                };
                let name = clip(
                    message
                        .get("toolName")
                        .and_then(Value::as_str)
                        .unwrap_or("Tool"),
                    256,
                );
                let end = kind == "tool_execution_end";
                let output = message.get(if end { "result" } else { "partialResult" });
                let args = stringify(
                    message
                        .get("args")
                        .filter(|a| !a.is_null())
                        .unwrap_or(&json!({})),
                );
                let shown = text_of(output.and_then(|o| o.get("content")));
                let text = clip(
                    &[args, shown]
                        .into_iter()
                        .filter(|part| !part.is_empty())
                        .collect::<Vec<_>>()
                        .join("\n"),
                    MAX_TEXT,
                );
                let status = if !end {
                    "running"
                } else if crate::native_chat::truthy(message.get("isError")) {
                    "failed"
                } else {
                    "completed"
                };
                self.core
                    .emit(json!({"type":"tool","id":id,"name":name,"status":status,"text":text}));
            }
            "auto_retry_end" if message.get("success") == Some(&Value::Bool(false)) => {
                self.failed = true;
                let reason = match message.get("finalError") {
                    None | Some(Value::Null) => "Pi gave up after retrying.".to_owned(),
                    Some(error) => js_string(error),
                };
                let reason = clip(&reason, 2048);
                self.core.error(&reason);
            }
            _ => {}
        }
    }

    /// `ask`: an extension asking the person something. Only the dialogs need
    /// an answer; notices and status lines are Pi's own display.
    fn ask(&mut self, message: &Value) {
        let Some(id) = message
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.encode_utf16().count() <= 256)
            .map(str::to_owned)
        else {
            return;
        };
        let title = clip(
            message
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("Pi needs your input"),
            512,
        );
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        if method == "confirm" {
            let text = clip(
                message.get("message").and_then(Value::as_str).unwrap_or(""),
                MAX_TEXT,
            );
            let mut description = Map::new();
            description.insert("title".into(), json!(title));
            description.insert("text".into(), json!(text));
            description.insert("choices".into(), json!(["accept", "decline", "cancel"]));
            self.approval(id.clone(), description, Responder::Confirm { id });
            return;
        }
        if matches!(method, "select" | "input" | "editor") {
            let options: Vec<Value> = if method == "select" {
                message
                    .get("options")
                    .and_then(Value::as_array)
                    .map(|options| {
                        options
                            .iter()
                            .filter_map(Value::as_str)
                            .take(32)
                            .map(|label| json!({"label": clip(label, 256), "description": ""}))
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            let mut description = Map::new();
            description.insert("title".into(), json!(title));
            description.insert("text".into(), json!(""));
            description.insert("choices".into(), json!(["accept", "cancel"]));
            description.insert(
                "questions".into(),
                json!([{"id":"answer","header":"","question":title,"options":options,"isSecret":false,"isOther":method != "select","multiSelect":false}]),
            );
            self.approval(id.clone(), description, Responder::Answer { id });
        }
    }

    fn answer_request(
        &mut self,
        responder: &Responder,
        decision: Decision,
        answers: &Answers,
    ) -> Result<(), ()> {
        let (id, fields) = match responder {
            Responder::Confirm { id } => (
                id,
                if decision == Decision::Cancel {
                    json!({"cancelled": true})
                } else {
                    json!({"confirmed": decision == Decision::Accept})
                },
            ),
            Responder::Answer { id } => {
                let answer = answers
                    .get("answer")
                    .and_then(|a| a.get(0))
                    .and_then(Value::as_str);
                if decision == Decision::Accept {
                    let Some(answer) = answer else {
                        return Err(());
                    };
                    (id, json!({"value": answer}))
                } else {
                    (id, json!({"cancelled": true}))
                }
            }
        };
        let mut frame = Map::new();
        frame.insert("type".into(), json!("extension_ui_response"));
        frame.insert("id".into(), json!(id));
        if let Value::Object(fields) = fields {
            frame.extend(fields);
        }
        // Pi having gone is not the person's answer being wrong.
        let _ = self.core.send(Value::Object(frame));
        Ok(())
    }
}

/// Pi, run in this process (see the module documentation).
pub fn adapter(env: &BTreeMap<String, String>, env_remove: &[String]) -> NativeAdapter {
    NativeAdapter::new(
        env,
        env_remove,
        Protocol {
            frame: pi_frame,
            reply: pi_reply,
            client_name: "Pi",
            exited: "Pi exited.",
            // Pi runs its tools without asking.
            permission_modes: None,
        },
        |core| Pi {
            core,
            program: String::new(),
            models: None,
            model: None,
            effort: None,
            window: None,
            message_count: 0,
            assistant_id: None,
            failed: false,
            started: None,
        },
    )
}

impl NativeClient for Pi {
    type Then = Then;
    type Responder = Responder;
    type Turn = ();
    fn core(&mut self) -> &mut Core<Then, Responder, ()> {
        &mut self.core
    }
    fn start(&mut self, init: ChatInit) {
        self.open(init);
    }
    fn message(&mut self, id: String, content: String) {
        self.send_message(id, content);
    }
    fn steer(&mut self, id: String, content: String) {
        self.steer_turn(id, content);
    }
    fn interrupt(&mut self) {
        self.interrupt_turn();
    }
    fn select_model(&mut self, model: Option<String>, effort: Option<String>) {
        self.choose_model(model, effort);
    }
    fn set_permission_mode(&mut self, _mode: String) {
        self.core.raw_error(
            "Pi runs its tools without asking; it has no approval modes to switch between.",
        );
    }
    fn notification(&mut self, message: Value) {
        self.on_notification(message);
    }
    fn settle(&mut self, then: Then, result: Result<Value, String>) {
        self.after(then, result);
    }
    fn respond(
        &mut self,
        responder: &Responder,
        decision: Decision,
        answers: &Answers,
    ) -> Result<(), ()> {
        self.answer_request(responder, decision, answers)
    }
    fn responder(&mut self, key: &str) -> Option<Responder> {
        self.core
            .approvals
            .get(key)
            .map(|pending| pending.responder.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_text_is_its_text_blocks_joined() {
        assert_eq!(text_of(Some(&json!("plain"))), "plain");
        assert_eq!(
            text_of(Some(&json!([
                {"type": "text", "text": "Hello, "},
                {"type": "thinking", "text": "hidden"},
                {"type": "text", "text": null},
                {"type": "text", "text": "world"}
            ]))),
            "Hello, world"
        );
        assert_eq!(text_of(Some(&json!(3))), "");
        assert_eq!(text_of(None), "");
    }

    #[test]
    fn a_model_is_named_by_provider_and_id() {
        assert_eq!(
            model_name(Some(&json!({"provider": "anthropic", "id": "sonnet"}))).as_deref(),
            Some("anthropic/sonnet")
        );
        assert_eq!(model_name(Some(&json!({"id": "sonnet"}))), None);
        assert_eq!(model_name(None), None);
    }

    #[test]
    fn pi_reads_flags_inline_or_separate_and_refuses_empty_values() {
        assert_eq!(
            pi_launch(&init(&["--model=m", "-n", "chat"])).unwrap(),
            ["--mode", "rpc", "--model", "m", "-n", "chat"]
        );
        for args in [&["--model", ""][..], &["--model="][..], &["--thinking"][..]] {
            assert_eq!(
                pi_launch(&init(args)).err().as_deref(),
                Some("Missing Pi launch option value."),
                "{args:?}"
            );
        }
        assert_eq!(
            pi_launch(&init(&["=x"])).err().as_deref(),
            Some("Unsupported Pi launch option in structured mode.")
        );
        assert_eq!(
            pi_launch(&init(&["--session", "abc", "--session=abd"]))
                .err()
                .as_deref(),
            Some("Invalid or conflicting Pi session ID.")
        );
        assert_eq!(
            pi_launch(&init(&["--session=abc"])).unwrap(),
            ["--mode", "rpc", "--session", "abc"]
        );
    }

    fn init(args: &[&str]) -> ChatInit {
        ChatInit {
            provider: "pi".into(),
            program: "pi".into(),
            args: args.iter().map(|a| a.to_string()).collect(),
            cwd: "/work".into(),
            resume_id: None,
        }
    }

    #[test]
    fn a_launch_keeps_model_provider_depth_and_name_and_resumes_by_session() {
        assert_eq!(
            pi_launch(&init(&[
                "--model",
                "tendly/zai-org/GLM-5.3",
                "--thinking",
                "high",
                "--name",
                "task",
                "--session",
                "pi-session-1"
            ]))
            .unwrap(),
            [
                "--mode",
                "rpc",
                "--model",
                "tendly/zai-org/GLM-5.3",
                "--thinking",
                "high",
                "--name",
                "task",
                "--session",
                "pi-session-1"
            ]
        );
        assert_eq!(
            pi_launch(&init(&["--mcp-config", "x"])).unwrap_err(),
            "Unsupported Pi launch option in structured mode."
        );
        assert_eq!(
            pi_launch(&init(&["--session", "../escape"])).unwrap_err(),
            "Invalid or conflicting Pi session ID."
        );
    }

    #[test]
    fn models_are_named_by_provider_and_reasoning_ones_get_depths() {
        let models = pi_models(
            &json!({"models":[{"provider":"tendly","id":"qwen"},{"provider":"k","id":"glm","name":"GLM","reasoning":true},{"id":"orphan"}]}),
        );
        assert_eq!(
            models,
            vec![
                json!({"id":"tendly/qwen","name":"qwen"}),
                json!({"id":"k/glm","name":"GLM","efforts":["low","medium","high"]})
            ]
        );
    }

    #[test]
    fn replies_are_responses_matched_by_id() {
        assert_eq!(
            pi_reply(&json!({"type":"response","id":"a","success":true,"data":{"x":1}})),
            Some((json!("a"), Ok(json!({"x":1}))))
        );
        assert_eq!(
            pi_reply(&json!({"type":"response","id":"a","success":false,"error":"no"})),
            Some((json!("a"), Err(json!("no"))))
        );
        assert_eq!(
            pi_reply(&json!({"type":"response","id":"a","success":false})),
            Some((json!("a"), Err(json!(true))))
        );
        assert_eq!(pi_reply(&json!({"type":"agent_start"})), None);
        assert_eq!(
            pi_frame("tendly-1", "prompt", json!({"message":"hi"})),
            json!({"id":"tendly-1","type":"prompt","message":"hi"})
        );
    }
}
