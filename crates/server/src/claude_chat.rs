//! Claude Code stream-json on the shared Rust conversation core.
//!
//! Claude's protocol is its stream-json control channel: user messages and
//! `control_request`s in, a stream of `system`, `assistant`, `user`,
//! `stream_event` and `result` messages out, with `control_response`s answering
//! requests either way. Tool approvals arrive as the client's own
//! `can_use_tool` requests and are answered on the same channel.
use crate::chat_adapter::{Answers, ChatInit, Decision};
use crate::native_chat::{
    Core, LaunchArgs, MAX_TEXT, NativeAdapter, NativeClient, Protocol, approval_scope, clip,
    clip_value, js_string, native_id, safe_integer, stringify, take_resume, truthy, valid_init,
};
use indexmap::{IndexMap, IndexSet};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap};

mod approvals;
mod events;
mod launch;
#[cfg(test)]
mod tests;
mod values;

pub use launch::*;
pub use values::*;

/// Tendly's names for how tools get approved, and Claude Code's.
const PERMISSIONS: [(&str, &str); 4] = [
    ("ask", "default"),
    ("plan", "plan"),
    ("accept_edits", "acceptEdits"),
    ("danger", "bypassPermissions"),
];

fn claude_mode(ours: &str) -> Option<&'static str> {
    PERMISSIONS
        .iter()
        .find(|(name, _)| *name == ours)
        .map(|(_, theirs)| *theirs)
}

/// The client calls the ask-every-time mode `default` on current builds and
/// `manual` on older ones. Both mean the same thing to a person.
fn our_mode(theirs: &str) -> Option<&'static str> {
    if theirs == "manual" {
        return Some("ask");
    }
    PERMISSIONS
        .iter()
        .find(|(_, name)| *name == theirs)
        .map(|(ours, _)| *ours)
}

/// `controlRequest`: Claude Code's stream-json control channel.
fn control_frame(id: &str, method: &str, params: Value) -> Value {
    let mut request = Map::new();
    request.insert("subtype".into(), json!(method));
    if let Value::Object(params) = params {
        request.extend(params);
    }
    json!({"type":"control_request","request_id":id,"request":request})
}

/// The default `knownReply` for a Claude `control_response`.
fn control_reply(message: &Value) -> Option<(Value, Result<Value, Value>)> {
    if message.get("type").and_then(Value::as_str) != Some("control_response") {
        return None;
    }
    let response = message.get("response").filter(|r| !r.is_null());
    let id = response
        .and_then(|r| r.get("request_id"))
        .cloned()
        .unwrap_or(Value::Null);
    if response
        .and_then(|r| r.get("subtype"))
        .and_then(Value::as_str)
        == Some("error")
    {
        let stated = response
            .and_then(|r| r.get("error"))
            .filter(|e| !e.is_null())
            .cloned()
            .unwrap_or(Value::Bool(true));
        return Some((id, Err(stated)));
    }
    let result = response
        .and_then(|r| r.get("response"))
        .filter(|r| !r.is_null())
        .cloned()
        .unwrap_or_else(|| json!({}));
    Some((id, Ok(result)))
}

/// What follows the answer to one of this client's requests.
enum Then {
    Initialize,
    SetModel {
        model: String,
        effort: Option<String>,
    },
    SetPermission {
        mode: &'static str,
    },
    Interrupt {
        turn: usize,
    },
}

/// How a `can_use_tool` request is answered.
#[derive(Clone)]
enum Responder {
    Permission {
        request: Value,
        input: Value,
        suggestions: Vec<Value>,
    },
    Questions {
        request: Value,
        input: Value,
        questions: Vec<Value>,
    },
}

struct Claude {
    core: Core<Then, Responder, ()>,
    launch: Option<Launch>,
    models: Option<Vec<Value>>,
    model: Option<String>,
    effort: Option<String>,
    /// The id of the assistant message being written, as the client gave it.
    assistant_id: Option<Value>,
    /// Whether the turn has a complete assistant reply.
    final_assistant: bool,
    /// Tool call id to its name, for the calls announced as cards.
    tools: IndexMap<String, String>,
    /// Steered message ids Claude Code has not read back yet.
    unconsumed: IndexSet<String>,
    /// Background task id to the tool call that started it.
    tasks: IndexMap<String, String>,
    /// Tool calls whose task runs on after their placeholder result.
    background: IndexSet<String>,
    /// The last progress summary logged per tool call.
    summary_of: HashMap<String, String>,
    /// Background task id to the card it first ran under. A subagent resumed
    /// by `SendMessage` runs again under the same task, while its output still
    /// names the first card as its parent.
    origins: IndexMap<String, String>,
    /// A first card to the card of the run that resumed it, while that runs:
    /// where the resumed subagent's output goes.
    resumed: HashMap<String, String>,
    /// Cards whose task has ended, which late output must not reopen.
    ended: IndexSet<String>,
}

/// Inserting into a map capped at 512 entries, oldest out first.
fn remember(map: &mut IndexMap<String, String>, key: String, value: String) {
    map.insert(key, value);
    if map.len() > 512 {
        map.shift_remove_index(0);
    }
}

impl Claude {
    fn report(&mut self) {
        let (models, model, effort) =
            (self.models.clone(), self.model.clone(), self.effort.clone());
        self.core
            .settings(models.as_deref(), model.as_deref(), effort.as_deref());
    }
    fn assistant(&self) -> Value {
        match &self.assistant_id {
            Some(id) => id.clone(),
            None => {
                let turn = self.core.active.map(|t| self.core.turns[t].id.clone());
                json!(format!("assistant-{}", turn.unwrap_or_default()))
            }
        }
    }

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
        let launch = match claude_launch(&init) {
            Ok(launch) => launch,
            Err(message) => {
                self.core.raw_error(&message);
                self.shutdown();
                return;
            }
        };
        self.core.cwd = init.cwd.clone();
        self.core.initializing = true;
        let args = launch.args.clone();
        self.launch = Some(launch);
        if self.core.spawn(&init.program, &args).is_err() {
            self.core.waiting(Then::Initialize);
            self.failure("Could not launch the configured native client.");
            return;
        }
        // Without forwardSubagentText the client sends only a subagent's tool
        // calls; the nested flag adds those of subagents that subagents start.
        self.rpc(
            "initialize",
            json!({"promptSuggestions":false,"forwardSubagentText":true,"forwardNestedSubagentToolCalls":true}),
            Then::Initialize,
            false,
        );
    }

    fn initialized(&mut self, result: Value) {
        let launch = self.launch.clone().unwrap_or(Launch {
            args: Vec::new(),
            resume: None,
            session_id: None,
            model: None,
            effort: None,
        });
        // The client lists its own commands here, before any turn has run.
        let session = launch.resume.clone().or(launch.session_id.clone());
        self.core
            .announce(session.as_deref(), command_names(result.get("commands")));
        self.models = model_entries(result.get("models"));
        // `default` means "whatever this configuration resolves to": a truthful
        // start, when the client listed it. A launch flag is more specific.
        let lists_default = self.models.as_ref().is_some_and(|models| {
            models
                .iter()
                .any(|entry| entry.get("id").and_then(Value::as_str) == Some("default"))
        });
        self.model = launch
            .model
            .clone()
            .or_else(|| lists_default.then(|| "default".to_owned()));
        // The depth it was launched with is the one in force.
        self.effort = launch.effort.clone();
        self.report();
        self.core.initializing = false;
        self.core.initialized = true;
    }

    fn choose_model(&mut self, model: Option<String>, effort: Option<String>) {
        if self.core.active.is_some() {
            self.core
                .raw_error("Wait for the current turn to finish before changing the model.");
            return;
        }
        // A depth-only change keeps the current model: the client has no
        // separate control for it, and guessing one would switch the user's.
        let Some(model) = model.or_else(|| self.model.clone()) else {
            self.core.raw_error("Claude Code has not reported which model this session uses yet. Send a message first, then adjust the thinking depth.");
            return;
        };
        let effort = effort.filter(|e| !e.is_empty());
        let mut params = Map::new();
        params.insert("model".into(), json!(model));
        if let Some(effort) = &effort {
            params.insert("effort".into(), json!(effort));
        }
        self.rpc(
            "set_model",
            Value::Object(params),
            Then::SetModel { model, effort },
            false,
        );
    }

    fn choose_permission(&mut self, mode: String) {
        if self.core.active.is_some() {
            self.core
                .raw_error("Wait for the current turn to finish before changing permissions.");
            return;
        }
        let Some(mode) = claude_mode(&mode) else {
            self.core.raw_error("Unsupported permission mode.");
            return;
        };
        self.rpc(
            "set_permission_mode",
            json!({"mode": mode}),
            Then::SetPermission { mode },
            false,
        );
    }

    fn user_frame(&self, id: &str, content: &str) -> Value {
        json!({
            "type": "user",
            "uuid": wire_id(id),
            "session_id": self.core.native_session_id.clone().unwrap_or_default(),
            "message": {"role": "user", "content": content},
            "parent_tool_use_id": null,
        })
    }

    fn send_message(&mut self, id: String, content: String) {
        if self.core.begin(id.clone(), &content).is_none() {
            return;
        }
        self.assistant_id = None;
        self.final_assistant = false;
        let frame = self.user_frame(&id, &content);
        if self.core.send(frame).is_err() {
            self.core
                .error("Could not send the message to Claude Code.");
            self.core.finish("failed");
        }
    }

    /// Steering needs nothing of Claude Code but the message: written while a
    /// turn runs, the model reads it at its next step. If the turn ends first
    /// it becomes a turn of its own (`continue_steered`).
    fn steer_turn(&mut self, id: String, content: String) {
        if self.core.active.is_none() {
            self.send_message(id, content);
            return;
        }
        if !self.core.accept_steer(&id, &content) {
            return;
        }
        self.unconsumed.insert(id.clone());
        let frame = self.user_frame(&id, &content);
        if self.core.send(frame).is_err() {
            self.unconsumed.shift_remove(&id);
            self.core
                .error("Could not send the message to Claude Code.");
        }
    }

    /// A steered message the turn ended without reading runs next, under its own id.
    fn continue_steered(&mut self) {
        let Some(next) = self.unconsumed.shift_remove_index(0) else {
            return;
        };
        self.assistant_id = None;
        self.final_assistant = false;
        self.core.open_turn(next.clone());
        self.core
            .emit(json!({"type":"turn","id":next,"status":"running"}));
    }

    fn interrupt_turn(&mut self) {
        let Some(turn) = self.core.active else {
            return;
        };
        self.core.turns[turn].interrupted = true;
        self.cancel_approvals();
        self.unconsumed.clear();
        self.rpc("interrupt", json!({}), Then::Interrupt { turn }, false);
    }

    fn begin_turn(&mut self, id: String) {
        self.assistant_id = None;
        self.final_assistant = false;
        self.core.open_turn(id.clone());
        self.core
            .emit(json!({"type":"turn","id":id,"status":"running"}));
    }

    fn after(&mut self, then: Then, result: Result<Value, String>) {
        match then {
            Then::Initialize => match result {
                Ok(result) => self.initialized(result),
                Err(message) => {
                    self.core.initializing = false;
                    self.core.raw_error(&message);
                    self.shutdown();
                }
            },
            Then::SetModel { model, effort } => match result {
                Ok(_) => {
                    self.model = Some(model);
                    if effort.is_some() {
                        self.effort = effort;
                    }
                    self.report();
                }
                Err(message) => self.core.raw_error(&message),
            },
            Then::SetPermission { mode } => match result {
                Ok(result) => {
                    // The answer comes back in the client's vocabulary; one this
                    // bridge cannot name is left to the status message.
                    let theirs = result.get("mode").and_then(Value::as_str).unwrap_or(mode);
                    if let Some(applied) = our_mode(theirs) {
                        self.core.permission_mode = Some(applied.to_owned());
                    }
                    self.report();
                }
                Err(message) => self.core.raw_error(&message),
            },
            Then::Interrupt { turn } => {
                if result.is_err() && self.core.active == Some(turn) {
                    self.core
                        .error("Claude Code could not confirm turn interruption.");
                }
            }
        }
    }
}

/// Claude Code, run in this process (see the module documentation).
pub fn adapter(env: &BTreeMap<String, String>, env_remove: &[String]) -> NativeAdapter {
    NativeAdapter::new(
        env,
        env_remove,
        Protocol {
            frame: control_frame,
            reply: control_reply,
            client_name: "Claude Code",
            exited: "Claude Code exited.",
            permission_modes: Some(&["ask", "plan", "accept_edits", "danger"]),
        },
        |core| Claude {
            core,
            launch: None,
            models: None,
            model: None,
            effort: None,
            assistant_id: None,
            final_assistant: false,
            tools: IndexMap::new(),
            unconsumed: IndexSet::new(),
            tasks: IndexMap::new(),
            background: IndexSet::new(),
            summary_of: HashMap::new(),
            origins: IndexMap::new(),
            resumed: HashMap::new(),
            ended: IndexSet::new(),
        },
    )
}

impl NativeClient for Claude {
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
    fn set_permission_mode(&mut self, mode: String) {
        self.choose_permission(mode);
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
