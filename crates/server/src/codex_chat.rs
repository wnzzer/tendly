//! Codex, driven from this process: `codex app-server` over JSON-RPC, with no
//! Node bridge in between.
//!
//! This is `packages/native-bridge/chat-codex.mjs` (with the parts of
//! `chat-common.mjs` and `chat.mjs` it relies on) in Rust, and it produces the
//! same events: the same fields, the same clipping and redaction, the same
//! error wording. Behaviour that differs from the bridge is a bug here.
//!
//! One task owns the conversation. Everything that can happen to it -- a
//! command from the server, a message or a reply from Codex, a request timing
//! out, an approval expiring, Codex exiting -- arrives on one channel and is
//! handled to the end before the next. A request to Codex records what should
//! follow its answer (a [`Then`]) instead of awaiting it, so no state is held
//! across a wait. The bridge's checks that a reply still belongs to the turn
//! that asked (`this.active === active`) are kept, as turn indexes.
use crate::chat_adapter::{Answers, ChatInit, Decision};
use crate::native_chat::{
    Core, LaunchArgs, MAX_TEXT, NativeAdapter, NativeClient, Protocol, approval_scope, clip,
    clip_value, js_string, json_rpc_frame, native_id, resolve, stringify, take_resume, truthy,
    valid_init,
};
use indexmap::IndexMap;
use serde_json::{Map, Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
    time::Instant,
};

mod approvals;
mod events;
mod launch;
#[cfg(test)]
mod tests;
mod values;

pub use launch::*;
use values::*;

// --- the conversation --------------------------------------------------------

/// A message written into the conversation: its id and content.
#[derive(Debug, Clone)]
struct Written {
    id: String,
    content: String,
}

/// What Codex keeps per turn (`Turn` in chat-common.mjs): steering written
/// before the turn had an id, that id, and whether an interrupt went out.
#[derive(Default)]
struct TurnExtra {
    early: Vec<Written>,
    native_turn: Option<Value>,
    interrupt_sent: bool,
}

/// What follows a request's answer: the code after an `await` in the bridge.
enum Then {
    Initialize,
    ModelList,
    OpenThread {
        turn: usize,
        content: String,
    },
    StartTurn {
        turn: usize,
    },
    Steer {
        turn: usize,
        message: Written,
        then: Vec<Written>,
    },
    Interrupt {
        turn: usize,
    },
}

/// How a native request is answered once the person has decided.
#[derive(Clone)]
enum Responder {
    Decision { id: Value },
    Questions { id: Value, questions: Vec<Value> },
    Permissions { id: Value, permissions: Value },
    Elicitation { id: Value },
}

/// Codex, on the shared core.
struct Codex {
    core: Core<Then, Responder, TurnExtra>,
    program: String,
    launch: Option<Launch>,
    deferred: Vec<Written>,
    tools: IndexMap<String, (String, String)>,
    /// Per streaming command, when its output was last forwarded and how long
    /// it was then, so a flood of small deltas is throttled (see
    /// [`should_emit_output`]).
    output_emits: HashMap<String, (Instant, usize)>,
    models: Option<Vec<Value>>,
    default_model: Option<String>,
    effort: Option<String>,
    approval_policy: Option<String>,
    sandbox_policy: Option<Value>,
    sandbox_overridden: bool,
    base_sandbox: Option<Value>,
}

impl Codex {
    fn restate_settings(&mut self) {
        let models = self.models.clone();
        let model = self.selected_model();
        let effort = self.effort.clone();
        self.core
            .settings(models.as_deref(), model.as_deref(), effort.as_deref());
    }
    fn selected_model(&self) -> Option<String> {
        self.launch
            .as_ref()
            .and_then(|l| l.thread.get("model"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| self.default_model.clone())
    }

    // -- chat.mjs: what to do with each command --

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
        let launch = match codex_launch(&init) {
            Ok(launch) => launch,
            Err(message) => {
                self.core.raw_error(&message);
                self.shutdown();
                return;
            }
        };
        self.core.cwd = init.cwd.clone();
        self.program = init.program.clone();
        let args = launch.args.clone();
        self.launch = Some(launch);
        self.core.initializing = true;
        let program = self.program.clone();
        if self.core.spawn(&program, &args).is_err() {
            // A process that never started fails as the bridge's does: its
            // first request is already waiting when the launch error arrives,
            // and is failed by the close that follows.
            self.core.waiting(Then::Initialize);
            self.failure("Could not launch the configured native client.");
            return;
        }
        self.rpc(
            "initialize",
            json!({"clientInfo":{"name":"tendly","title":"Tendly","version":"0.1.0"}}),
            Then::Initialize,
            false,
        );
    }

    // -- CodexChat --

    fn choose_model(&mut self, model: Option<String>, effort: Option<String>) {
        if self.core.active.is_some() {
            self.core
                .raw_error("Wait for the current turn to finish before changing the model.");
            return;
        }
        if let Some(model) = model.filter(|m| !m.is_empty())
            && let Some(launch) = self.launch.as_mut()
        {
            launch.thread.insert("model".into(), json!(model));
        }
        if let Some(effort) = effort.filter(|e| !e.is_empty()) {
            self.effort = Some(effort);
        }
        self.restate_settings();
    }
    fn choose_permission(&mut self, mode: &str) {
        if self.core.active.is_some() {
            self.core
                .raw_error("Wait for the current turn to finish before changing permissions.");
            return;
        }
        let (policy, sandbox) = match mode {
            "ask" => ("on-request", None),
            "danger" => ("never", Some(json!({"type":"dangerFullAccess"}))),
            _ => {
                self.core.raw_error(
                    "Codex approves each command rather than switching between review modes.",
                );
                return;
            }
        };
        self.approval_policy = Some(policy.into());
        // A turn's sandbox stays the thread's for the turns after it, so
        // leaving `danger` has to name the sandbox to return to.
        self.sandbox_policy =
            sandbox.or_else(|| self.sandbox_overridden.then(|| self.return_sandbox()));
        self.core.permission_mode = Some(mode.into());
        self.restate_settings();
    }
    fn return_sandbox(&self) -> Value {
        let chosen = self
            .launch
            .as_ref()
            .is_some_and(|l| l.args.iter().any(|arg| arg.starts_with("sandbox_mode=")));
        match &self.base_sandbox {
            Some(base)
                if base.get("type").and_then(Value::as_str) != Some("dangerFullAccess")
                    || chosen =>
            {
                base.clone()
            }
            _ => json!({"type":"workspaceWrite"}),
        }
    }
    fn send_message(&mut self, id: String, content: String) {
        if let Some(turn) = self.core.begin(id, &content) {
            self.run_turn(turn, content);
        }
    }
    fn run_turn(&mut self, turn: usize, content: String) {
        if self.core.native_session_id.is_none() {
            let launch = self.launch.clone().expect("launched");
            let mut params = launch.thread.clone();
            let method = match &launch.resume {
                Some(resume) => {
                    params.insert("threadId".into(), json!(resume));
                    "thread/resume"
                }
                None => "thread/start",
            };
            self.rpc(
                method,
                Value::Object(params),
                Then::OpenThread { turn, content },
                false,
            );
            return;
        }
        self.start_turn(turn, content);
    }
    fn start_turn(&mut self, turn: usize, content: String) {
        if self.core.active != Some(turn) {
            return;
        }
        if self.core.turns[turn].interrupted {
            self.core.finish("interrupted");
            return;
        }
        if self.models.is_some() {
            self.restate_settings();
        }
        if self.sandbox_policy.is_some() {
            self.sandbox_overridden = true;
        }
        let mut params = Map::new();
        params.insert("threadId".into(), json!(self.core.native_session_id));
        params.insert("input".into(), json!([{"type":"text","text":content}]));
        if let Some(model) = self
            .launch
            .as_ref()
            .and_then(|l| l.thread.get("model"))
            .cloned()
        {
            params.insert("model".into(), model);
        }
        if let Some(effort) = &self.effort {
            params.insert("effort".into(), json!(effort));
        }
        if let Some(policy) = &self.approval_policy {
            params.insert("approvalPolicy".into(), json!(policy));
        }
        if let Some(sandbox) = &self.sandbox_policy {
            params.insert("sandboxPolicy".into(), sandbox.clone());
        }
        self.rpc(
            "turn/start",
            Value::Object(params),
            Then::StartTurn { turn },
            false,
        );
    }
    /// The catch around `run`: a failed request ends the turn it was for.
    fn turn_failed(&mut self, turn: usize, message: &str) {
        if self.core.active == Some(turn) {
            self.core.error(message);
            let status = if self.core.turns[turn].interrupted {
                "interrupted"
            } else {
                "failed"
            };
            self.core.finish(status);
        }
        self.pass_on(turn);
    }
    /// Steering written before a turn had an id, left over when that turn will
    /// never take it -- Codex refused to start it, or finished it before
    /// saying its id -- runs as the next turn, like steering a turn refused.
    fn pass_on(&mut self, turn: usize) {
        if self.core.turns[turn].interrupted || self.core.turns[turn].extra.early.is_empty() {
            return;
        }
        let early = std::mem::take(&mut self.core.turns[turn].extra.early);
        self.deferred.extend(early);
        self.continue_deferred();
    }
    fn steer_turn(&mut self, id: String, content: String) {
        let Some(turn) = self.core.active else {
            self.send_message(id, content);
            return;
        };
        if !self.core.accept_steer(&id, &content) {
            return;
        }
        let message = Written { id, content };
        if self.core.turns[turn].extra.native_turn.is_none() {
            self.core.turns[turn].extra.early.push(message);
            return;
        }
        self.send_steer(turn, message, Vec::new());
    }
    fn send_steer(&mut self, turn: usize, message: Written, then: Vec<Written>) {
        let params = json!({
            "threadId": self.core.native_session_id,
            "expectedTurnId": self.core.turns[turn].extra.native_turn,
            "input": [{"type":"text","text":message.content}],
        });
        self.rpc(
            "turn/steer",
            params,
            Then::Steer {
                turn,
                message,
                then,
            },
            false,
        );
    }
    fn continue_deferred(&mut self) {
        if self.core.active.is_some() || self.deferred.is_empty() {
            return;
        }
        let list: Vec<Written> = std::mem::take(&mut self.deferred);
        let id = list[0].id.clone();
        let turn = self.core.open_turn(id.clone());
        self.core
            .emit(json!({"type":"turn","id":id,"status":"running"}));
        let content = list
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        self.run_turn(turn, content);
    }
    fn send_interrupt(&mut self, turn: usize) {
        let state = &mut self.core.turns[turn].extra;
        if !truthy(state.native_turn.as_ref()) || state.interrupt_sent {
            return;
        }
        state.interrupt_sent = true;
        let native_turn = state.native_turn.clone();
        let params = json!({"threadId": self.core.native_session_id, "turnId": native_turn});
        self.rpc("turn/interrupt", params, Then::Interrupt { turn }, false);
    }
    fn interrupt_turn(&mut self) {
        let Some(turn) = self.core.active else {
            return;
        };
        self.core.turns[turn].interrupted = true;
        self.deferred.clear();
        self.cancel_approvals();
        self.send_interrupt(turn);
    }

    /// What follows the answer to a request Tendly made of app-server: after
    /// `initialize`, list the models; after the model list, offer it; after a
    /// thread opens, start the turn waiting on it; after a turn, a steer or an
    /// interrupt is answered, carry on or report why not.
    fn after(&mut self, then: Then, result: Result<Value, String>) {
        match then {
            Then::Initialize => match result {
                Ok(_) => {
                    if let Err(message) =
                        self.core.send(json!({"method":"initialized","params":{}}))
                    {
                        self.core.initializing = false;
                        self.core.raw_error(message);
                        self.shutdown();
                        return;
                    }
                    self.core.announce(None, None);
                    let model = self
                        .launch
                        .as_ref()
                        .and_then(|l| l.thread.get("model"))
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    self.core.settings(None, model.as_deref(), None);
                    self.core.initializing = false;
                    self.core.initialized = true;
                    // Asking for models must not hold up the session.
                    self.rpc(
                        "model/list",
                        json!({"limit":100,"includeHidden":false}),
                        Then::ModelList,
                        true,
                    );
                }
                Err(message) => {
                    self.core.initializing = false;
                    self.core.raw_error(&message);
                    self.shutdown();
                }
            },
            Then::ModelList => {
                if let Ok(result) = result {
                    self.models = codex_models(&result);
                    self.default_model = self.models.as_ref().and_then(|models| {
                        models
                            .iter()
                            .find(|m| m.get("isDefault") == Some(&Value::Bool(true)))
                            .and_then(|m| m.get("id"))
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    });
                    self.restate_settings();
                }
            }
            Then::OpenThread { turn, content } => {
                let result = match result {
                    Ok(result) => result,
                    Err(message) => return self.turn_failed(turn, &message),
                };
                let Some(id) = result
                    .get("thread")
                    .and_then(|t| t.get("id"))
                    .and_then(Value::as_str)
                    .filter(|id| native_id(id))
                    .map(str::to_owned)
                else {
                    return self
                        .turn_failed(turn, "Codex did not return a usable native thread ID.");
                };
                if self.base_sandbox.is_none()
                    && let Some(sandbox) = result
                        .get("sandbox")
                        .filter(|s| s.is_object() || s.is_array())
                {
                    self.base_sandbox = Some(sandbox.clone());
                }
                self.core.announce(Some(&id), None);
                self.start_turn(turn, content);
            }
            Then::StartTurn { turn } => match result {
                Ok(result) => {
                    if self.core.active != Some(turn) {
                        self.pass_on(turn);
                        return;
                    }
                    self.core.turns[turn].extra.native_turn =
                        result.get("turn").and_then(|t| t.get("id")).cloned();
                    if self.core.turns[turn].interrupted {
                        self.send_interrupt(turn);
                    } else {
                        // Steering written before the turn had an id waited for it.
                        let mut early = std::mem::take(&mut self.core.turns[turn].extra.early);
                        if !early.is_empty() {
                            let first = early.remove(0);
                            self.send_steer(turn, first, early);
                        }
                    }
                }
                Err(message) => self.turn_failed(turn, &message),
            },
            Then::Steer {
                turn,
                message,
                then,
            } => {
                if result.is_err() && !self.core.turns[turn].interrupted {
                    // A turn that would not take it leaves it for the next one.
                    self.deferred.push(message);
                    if self.core.active != Some(turn) {
                        self.continue_deferred();
                    }
                }
                let mut rest = then;
                if !rest.is_empty() {
                    let next = rest.remove(0);
                    self.send_steer(turn, next, rest);
                }
            }
            Then::Interrupt { turn } => {
                if result.is_err() && self.core.active == Some(turn) {
                    self.core.turns[turn].extra.interrupt_sent = false;
                    self.core
                        .error("Codex could not confirm turn interruption.");
                }
            }
        }
    }
}

/// Codex, run in this process (see the module documentation).
pub fn adapter(env: &BTreeMap<String, String>, env_remove: &[String]) -> NativeAdapter {
    NativeAdapter::new(
        env,
        env_remove,
        Protocol {
            frame: json_rpc_frame,
            reply: json_rpc_reply,
            client_name: "Codex",
            exited: "Codex app-server exited.",
            // Codex approves per command, so only the two ends of the range exist.
            permission_modes: Some(&["ask", "danger"]),
        },
        |core| Codex {
            core,
            program: String::new(),
            launch: None,
            deferred: Vec::new(),
            tools: IndexMap::new(),
            output_emits: HashMap::new(),
            models: None,
            default_model: None,
            effort: None,
            approval_policy: None,
            sandbox_policy: None,
            sandbox_overridden: false,
            base_sandbox: None,
        },
    )
}

/// `knownReply` for JSON-RPC: a message with an id and no method answers the
/// request with that id.
fn json_rpc_reply(message: &Value) -> Option<(Value, Result<Value, Value>)> {
    if truthy(message.get("method")) {
        return None;
    }
    let id = message.get("id")?.clone();
    Some(match message.get("error") {
        Some(error) if truthy(Some(error)) => {
            let stated = error
                .get("message")
                .filter(|m| !m.is_null())
                .unwrap_or(error);
            (id, Err(stated.clone()))
        }
        _ => (
            id,
            Ok(message.get("result").cloned().unwrap_or(Value::Null)),
        ),
    })
}

impl NativeClient for Codex {
    type Then = Then;
    type Responder = Responder;
    type Turn = TurnExtra;
    fn core(&mut self) -> &mut Core<Then, Responder, TurnExtra> {
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
        self.choose_permission(&mode);
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
