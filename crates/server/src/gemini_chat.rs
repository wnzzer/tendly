//! Gemini over the Agent Client Protocol (`agy acp`) on the shared Rust
//! conversation core.
//!
//! ACP is JSON-RPC 2.0 both ways. Tendly asks (`initialize`, `session/new`,
//! `session/resume`, `session/set_config_option`, `session/set_mode`); agy
//! streams `session/update` notifications and asks back with
//! `session/request_permission`. A turn is one `session/prompt` whose answer
//! comes only when the turn ends, minutes later, so it is not one of the
//! core's timed requests: its id is kept here and its answer read as it
//! arrives (`prompt`).
//!
//! A conversation opens with the first message, as Codex's thread does, so a
//! chat left empty leaves nothing in agy's own history. Messages sent while a
//! turn runs wait for it and go as the next one: ACP has no steering.
use crate::adapters::gemini::GEMINI_DIR;
use crate::chat_adapter::{Answers, ChatInit, Decision};
use crate::native_chat::{
    Core, LaunchArgs, MAX_TEXT, NativeAdapter, NativeClient, Protocol, approval_scope,
    clear_command, clip, count, is_clear, native_id, stringify, take_resume, truthy, valid_init,
};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap};

// --- launching ---------------------------------------------------------------

/// What a session's launch arguments ask of a chat: agy's ACP server, and the
/// model, depth and mode its terminal would have started with.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Launch {
    pub args: Vec<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// agy's own mode name: `accept-edits`, `auto` or `plan`.
    pub mode: Option<String>,
    pub resume: Option<String>,
}

/// `agy acp`, in the home the session's launch names, from the flags a
/// terminal run of agy takes.
pub fn gemini_launch(init: &ChatInit, gemini_dir: Option<&str>) -> Result<Launch, String> {
    let mut launch = Launch {
        args: vec!["acp".to_owned()],
        resume: init.resume_id.clone(),
        ..Launch::default()
    };
    if let Some(dir) = gemini_dir.filter(|dir| !dir.is_empty()) {
        launch
            .args
            .extend(["--gemini-dir".to_owned(), dir.to_owned()]);
    }
    let mut flags = LaunchArgs::new(&init.args, "Gemini", false);
    while let Some(arg) = flags.next_flag() {
        let flag = arg.name;
        let mut value = || flags.value(&arg);
        match flag {
            "--model" => launch.model = Some(value()?),
            "--effort" => launch.effort = Some(value()?),
            "--mode" => launch.mode = Some(value()?),
            "--dangerously-skip-permissions" => {
                launch.args.push(flag.to_owned());
            }
            "--conversation" | "--resume" | "-r" => {
                if !take_resume(&mut launch.resume, value()?) {
                    return Err("Invalid or conflicting Gemini conversation ID.".into());
                }
            }
            _ => return Err("Unsupported Gemini launch option in structured mode.".into()),
        }
    }
    Ok(launch)
}

/// A JSON-RPC 2.0 request.
fn acp_frame(id: &str, method: &str, params: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
}

/// A message with an id and no method answers the request with that id.
fn acp_reply(message: &Value) -> Option<(Value, Result<Value, Value>)> {
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

/// Tendly's permission modes as agy names them. `danger` runs as `auto`, and
/// the chat approves whatever agy still asks.
fn agy_mode(mode: &str) -> Option<&'static str> {
    Some(match mode {
        "ask" => "default",
        "plan" => "plan",
        "accept_edits" => "accept-edits",
        "danger" => "auto",
        _ => return None,
    })
}
fn tendly_mode(mode: &str) -> Option<&'static str> {
    Some(match mode {
        "default" => "ask",
        "plan" => "plan",
        "accept-edits" => "accept_edits",
        _ => return None,
    })
}

/// The models a session offers, from its `model` option, each with the depths
/// agy lists it at (`gemini-4-neon-high` is Gemini 4 Neon at high).
fn session_models(options: &[Value], variants: &[String]) -> Option<Vec<Value>> {
    let models = option(options, "model")?.get("options")?.as_array()?;
    let levels: Vec<&str> = option(options, "thought_level")
        .and_then(|o| o.get("options"))
        .and_then(Value::as_array)
        .map(|levels| {
            levels
                .iter()
                .filter_map(|l| l.get("value").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default();
    Some(
        models
            .iter()
            .filter_map(|model| {
                let id = model.get("value")?.as_str()?;
                let name = model.get("name").and_then(Value::as_str).unwrap_or(id);
                let efforts: Vec<&str> = levels
                    .iter()
                    .copied()
                    .filter(|level| variants.iter().any(|v| *v == format!("{id}-{level}")))
                    .collect();
                let mut row = json!({"id": id, "name": name});
                if !efforts.is_empty() {
                    row["efforts"] = json!(efforts);
                }
                Some(row)
            })
            .collect(),
    )
}
fn option<'a>(options: &'a [Value], id: &str) -> Option<&'a Value> {
    options
        .iter()
        .find(|o| o.get("id").and_then(Value::as_str) == Some(id))
}
fn current(options: &[Value], id: &str) -> Option<String> {
    option(options, id)?
        .get("currentValue")?
        .as_str()
        .map(str::to_owned)
}

/// Tendly's own tools, for a launch that carries its identity (agent.rs):
/// this server run as `tendly mcp` with that identity.
fn tendly_tools(env: &BTreeMap<String, String>) -> Vec<Value> {
    const IDENTITY: [&str; 3] = ["TENDLY_URL", "TENDLY_AGENT_TOKEN", "TENDLY_SESSION_ID"];
    let identity: Vec<Value> = IDENTITY
        .iter()
        .filter_map(|key| Some(json!({"name": key, "value": env.get(*key)?})))
        .collect();
    let program = std::env::current_exe().ok();
    match program {
        Some(program) if identity.len() == IDENTITY.len() => vec![json!({
            "name": "tendly",
            "command": program.to_string_lossy(),
            "args": ["mcp"],
            "env": identity,
        })],
        _ => Vec::new(),
    }
}

// --- the conversation --------------------------------------------------------

/// A message that waits for the running turn.
#[derive(Debug, Clone)]
struct Written {
    id: String,
    content: String,
}

/// What follows a request's answer.
enum Then {
    Initialize,
    /// `authenticate` with the Gemini API key in the environment.
    Authenticate,
    /// The conversation opened (new or resumed); then `steps`, then the turn.
    Open {
        turn: usize,
        content: String,
    },
    /// One of the settings a turn waits on; the next, then the turn.
    Step {
        turn: usize,
        content: String,
        rest: Vec<(&'static str, Value)>,
    },
    /// A new conversation for `/clear`.
    Clear {
        turn: usize,
    },
    /// A setting changed between turns.
    Setting,
}

/// How a permission request is answered: agy's request id, and the option
/// it offered for each decision.
#[derive(Clone)]
struct Responder {
    id: Value,
    options: HashMap<&'static str, String>,
}

/// A tool call as it has been shown so far.
#[derive(Clone, Default)]
struct Tool {
    name: String,
    text: String,
}

struct Gemini {
    core: Core<Then, Responder, ()>,
    program: String,
    /// The home the launch names (`--gemini-dir`), else agy's own.
    gemini_dir: Option<String>,
    launch: Launch,
    /// Tendly's tools, given to each conversation agy opens.
    tools_server: Vec<Value>,
    /// Whether a Gemini API key is in the environment, signed in with.
    api_key: bool,
    options: Vec<Value>,
    variants: Vec<String>,
    model: Option<String>,
    effort: Option<String>,
    /// The mode agy runs in, in Tendly's words.
    mode: Option<String>,
    /// Settings chosen before the conversation opened, applied as it does.
    pending_model: bool,
    pending_mode: bool,
    commands: Vec<String>,
    /// The running turn's `session/prompt`: its id and the turn.
    prompt: Option<(String, usize)>,
    prompts: u64,
    deferred: Vec<Written>,
    tools: HashMap<String, Tool>,
    assistant: Option<String>,
    messages: u64,
}

impl Gemini {
    fn session(&self) -> Option<String> {
        self.core.native_session_id.clone()
    }
    fn report(&mut self) {
        let models = session_models(&self.options, &self.variants);
        let (model, effort) = (self.model.clone(), self.effort.clone());
        if let Some(mode) = &self.mode {
            self.core.permission_mode = Some(mode.clone());
        }
        self.core
            .settings(models.as_deref(), model.as_deref(), effort.as_deref());
    }
    /// What agy says a conversation runs with, from a `session/new`,
    /// `session/resume` or a changed option.
    fn adopt(&mut self, result: &Value) {
        if let Some(options) = result.get("configOptions").and_then(Value::as_array) {
            self.options = options.clone();
            // A choice not yet applied stays the one to apply.
            if !self.pending_model {
                self.model = current(options, "model").or(self.model.take());
                self.effort = current(options, "thought_level").or(self.effort.take());
            }
            if let Some(mode) = current(options, "mode").as_deref().and_then(tendly_mode)
                && !self.pending_mode
                && self.mode.as_deref() != Some("danger")
            {
                self.mode = Some(mode.to_owned());
            }
        }
        if let Some(models) = result
            .pointer("/models/availableModels")
            .and_then(Value::as_array)
        {
            self.variants = models
                .iter()
                .filter_map(|m| m.get("modelId").and_then(Value::as_str))
                .map(str::to_owned)
                .collect();
        }
    }
    fn announce(&mut self) {
        let session = self.session();
        let mut commands = self.commands.clone();
        commands.extend(clear_command());
        self.core.announce(session.as_deref(), Some(commands));
    }

    // -- starting --

    fn open(&mut self, init: ChatInit, gemini_dir: Option<String>) {
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
        self.launch = match gemini_launch(&init, gemini_dir.as_deref()) {
            Ok(launch) => launch,
            Err(message) => {
                self.core.raw_error(&message);
                self.shutdown();
                return;
            }
        };
        self.model = self.launch.model.clone();
        self.effort = self.launch.effort.clone();
        self.pending_model = self.model.is_some() || self.effort.is_some();
        if let Some(mode) = self.launch.mode.as_deref().and_then(tendly_mode) {
            self.mode = Some(mode.to_owned());
            self.pending_mode = true;
        } else if self.launch.mode.as_deref() == Some("auto")
            || self
                .launch
                .args
                .iter()
                .any(|a| a == "--dangerously-skip-permissions")
        {
            self.mode = Some("danger".to_owned());
            self.pending_mode = true;
        }
        self.core.cwd = init.cwd.clone();
        self.program = init.program.clone();
        self.core.initializing = true;
        let (program, args) = (self.program.clone(), self.launch.args.clone());
        if self.core.spawn(&program, &args).is_err() {
            self.core.waiting(Then::Initialize);
            self.failure("Could not launch the configured native client.");
            return;
        }
        self.rpc(
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": {"fs": {"readTextFile": false, "writeTextFile": false}, "terminal": false},
                "clientInfo": {"name": "tendly", "title": "Tendly", "version": env!("CARGO_PKG_VERSION")},
            }),
            Then::Initialize,
            false,
        );
    }
    fn started(&mut self) {
        self.core.initializing = false;
        self.core.initialized = true;
        self.announce();
        self.report();
    }

    // -- turns --

    fn send_message(&mut self, id: String, content: String) {
        let clear = is_clear(&content);
        let Some(turn) = self.core.begin(id, &content) else {
            return;
        };
        if clear {
            self.clear(turn);
        } else {
            self.run_turn(turn, content);
        }
    }
    /// `/clear`: a new conversation with the same settings. With none open
    /// yet there is nothing to drop but the view.
    fn clear(&mut self, turn: usize) {
        self.launch.resume = None;
        if self.session().is_none() {
            self.core.replaced(None, Some(clear_command()));
            self.core.finish("completed");
            return;
        }
        let params = self.session_params();
        self.rpc("session/new", params, Then::Clear { turn }, false);
    }
    fn session_params(&self) -> Value {
        json!({"cwd": self.core.cwd, "mcpServers": self.tools_server})
    }
    fn run_turn(&mut self, turn: usize, content: String) {
        if self.session().is_some() {
            return self.prepare(turn, content);
        }
        let mut params = self.session_params();
        let method = match &self.launch.resume {
            Some(resume) => {
                params["sessionId"] = json!(resume);
                "session/resume"
            }
            None => "session/new",
        };
        self.rpc(method, params, Then::Open { turn, content }, false);
    }
    /// The settings chosen since the conversation last heard of them, one
    /// after another, then the turn.
    fn prepare(&mut self, turn: usize, content: String) {
        let mut steps = Vec::new();
        let session = self.session();
        if std::mem::take(&mut self.pending_model) {
            if let Some(model) = &self.model {
                steps.push((
                    "session/set_config_option",
                    json!({"sessionId": session, "configId": "model", "value": model}),
                ));
            }
            if let Some(effort) = &self.effort
                && option(&self.options, "thought_level").is_some()
            {
                steps.push((
                    "session/set_config_option",
                    json!({"sessionId": session, "configId": "thought_level", "value": effort}),
                ));
            }
        }
        if std::mem::take(&mut self.pending_mode)
            && let Some(mode) = self.mode.as_deref().and_then(agy_mode)
        {
            steps.push((
                "session/set_mode",
                json!({"sessionId": session, "modeId": mode}),
            ));
        }
        self.step(turn, content, steps);
    }
    fn step(&mut self, turn: usize, content: String, mut rest: Vec<(&'static str, Value)>) {
        if rest.is_empty() {
            return self.prompt(turn, content);
        }
        let (method, params) = rest.remove(0);
        self.rpc(
            method,
            params,
            Then::Step {
                turn,
                content,
                rest,
            },
            false,
        );
    }
    fn prompt(&mut self, turn: usize, content: String) {
        if self.core.active != Some(turn) {
            return;
        }
        if self.core.turns[turn].interrupted {
            self.core.finish("interrupted");
            return;
        }
        self.prompts += 1;
        let id = format!("gemini-prompt-{}", self.prompts);
        let params = json!({
            "sessionId": self.session(),
            "prompt": [{"type": "text", "text": content}],
        });
        self.assistant = None;
        match self.core.send(acp_frame(&id, "session/prompt", params)) {
            Ok(()) => self.prompt = Some((id, turn)),
            Err(message) => self.turn_failed(turn, message),
        }
    }
    fn turn_failed(&mut self, turn: usize, message: &str) {
        if self.core.active == Some(turn) {
            self.core.error(&explain(message));
            let status = if self.core.turns[turn].interrupted {
                "interrupted"
            } else {
                "failed"
            };
            self.core.finish(status);
        }
        self.continue_deferred();
    }
    /// The turn's `session/prompt` answered: the turn is over.
    fn ended(&mut self, turn: usize, result: Result<Value, String>) {
        self.prompt = None;
        self.assistant = None;
        let result = match result {
            Ok(result) => result,
            Err(message) => return self.turn_failed(turn, &message),
        };
        if self.core.active != Some(turn) {
            return;
        }
        if let Some(usage) = result.get("usage") {
            let input = count(usage.get("inputTokens"));
            let output = count(usage.get("outputTokens")) + count(usage.get("thoughtTokens"));
            let cached = count(usage.get("cachedReadTokens"));
            if input != 0 || output != 0 {
                self.core.emit(
                    json!({"type":"usage","turn":{"input":input,"output":output,"cached":cached}}),
                );
            }
        }
        let status = match result.get("stopReason").and_then(Value::as_str) {
            Some("cancelled") => "interrupted",
            _ if self.core.turns[turn].interrupted => "interrupted",
            Some("refusal") => {
                self.core.error("Gemini declined to answer this message.");
                "failed"
            }
            _ => "completed",
        };
        self.core.finish(status);
        if status == "interrupted" {
            self.deferred.clear();
        } else {
            self.continue_deferred();
        }
    }
    fn steer_turn(&mut self, id: String, content: String) {
        if self.core.active.is_none() {
            return self.send_message(id, content);
        }
        if self.core.accept_steer(&id, &content) {
            self.deferred.push(Written { id, content });
        }
    }
    /// Messages written while a turn ran, together, as the next turn.
    fn continue_deferred(&mut self) {
        if self.core.active.is_some() || self.deferred.is_empty() {
            return;
        }
        let list = std::mem::take(&mut self.deferred);
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
    fn interrupt_turn(&mut self) {
        let Some(turn) = self.core.active else {
            return;
        };
        self.core.turns[turn].interrupted = true;
        self.deferred.clear();
        self.cancel_approvals();
        if self.prompt.is_some()
            && let Some(session) = self.session()
        {
            let cancel =
                json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":session}});
            if self.core.send(cancel).is_err() {
                self.core.error("Gemini could not be asked to stop.");
            }
        }
    }

    // -- settings --

    fn choose_model(&mut self, model: Option<String>, effort: Option<String>) {
        if self.core.active.is_some() {
            self.core
                .raw_error("Wait for the current turn to finish before changing the model.");
            return;
        }
        if let Some(model) = model {
            // A depth belongs to a model: one the new model is not offered
            // at is left to agy.
            self.model = Some(model);
        }
        if let Some(effort) = effort {
            self.effort = Some(effort);
        }
        self.pending_model = true;
        self.apply_now();
        self.report();
    }
    fn choose_permission(&mut self, mode: String) {
        if agy_mode(&mode).is_none() {
            self.core.raw_error("Unsupported permission mode.");
            return;
        }
        self.mode = Some(mode);
        self.pending_mode = true;
        if self.core.active.is_none() {
            self.apply_now();
        }
        self.report();
    }
    /// Settings changed between turns reach an open conversation at once.
    fn apply_now(&mut self) {
        let Some(session) = self.session() else {
            return;
        };
        if std::mem::take(&mut self.pending_model) {
            if let Some(model) = self.model.clone() {
                self.rpc(
                    "session/set_config_option",
                    json!({"sessionId": session, "configId": "model", "value": model}),
                    Then::Setting,
                    false,
                );
            }
            if let Some(effort) = self.effort.clone()
                && option(&self.options, "thought_level").is_some()
            {
                self.rpc(
                    "session/set_config_option",
                    json!({"sessionId": session, "configId": "thought_level", "value": effort}),
                    Then::Setting,
                    false,
                );
            }
        }
        if std::mem::take(&mut self.pending_mode)
            && let Some(mode) = self.mode.as_deref().and_then(agy_mode)
        {
            self.rpc(
                "session/set_mode",
                json!({"sessionId": session, "modeId": mode}),
                Then::Setting,
                false,
            );
        }
    }

    // -- answers --

    fn after(&mut self, then: Then, result: Result<Value, String>) {
        match then {
            Then::Initialize => match result {
                Ok(_) if self.api_key => self.rpc(
                    "authenticate",
                    json!({"methodId": "gemini-api-key"}),
                    Then::Authenticate,
                    false,
                ),
                Ok(_) => self.started(),
                Err(message) => {
                    self.core.initializing = false;
                    self.core.raw_error(&message);
                    self.shutdown();
                }
            },
            Then::Authenticate => match result {
                Ok(_) => self.started(),
                Err(message) => {
                    self.core.initializing = false;
                    self.core
                        .raw_error(&format!("{message} Check the profile's Gemini API key."));
                    self.shutdown();
                }
            },
            Then::Open { turn, content } => {
                let result = match result {
                    Ok(result) => result,
                    Err(message) => return self.turn_failed(turn, &message),
                };
                let id = result
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| self.launch.resume.clone())
                    .filter(|id| native_id(id));
                let Some(id) = id else {
                    return self
                        .turn_failed(turn, "Gemini did not return a usable conversation ID.");
                };
                self.core.native_session_id = Some(id);
                self.adopt(&result);
                self.announce();
                self.report();
                self.prepare(turn, content);
            }
            Then::Step {
                turn,
                content,
                rest,
            } => {
                match result {
                    Ok(result) => self.adopt(&result),
                    Err(message) => self.core.error(&message),
                }
                self.step(turn, content, rest);
            }
            Then::Clear { turn } => {
                let id = match result {
                    Ok(result) => {
                        self.adopt(&result);
                        result
                            .get("sessionId")
                            .and_then(Value::as_str)
                            .filter(|id| native_id(id))
                            .map(str::to_owned)
                    }
                    Err(message) => return self.turn_failed(turn, &message),
                };
                let Some(id) = id else {
                    return self
                        .turn_failed(turn, "Gemini did not return a usable conversation ID.");
                };
                let mut commands = self.commands.clone();
                commands.extend(clear_command());
                self.core.replaced(Some(&id), Some(commands));
                self.pending_model = self.model.is_some();
                self.pending_mode = self.mode.is_some();
                self.apply_now();
                self.report();
                if self.core.active == Some(turn) {
                    self.core.finish("completed");
                }
                self.continue_deferred();
            }
            Then::Setting => match result {
                Ok(result) => {
                    self.adopt(&result);
                    self.report();
                }
                Err(message) => self.core.error(&message),
            },
        }
    }

    fn on_message(&mut self, message: Value) {
        if !message.is_object() {
            return;
        }
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        if !method.is_empty() && message.get("id").is_some() {
            return self.request(&message);
        }
        if method.is_empty() {
            // The running turn's prompt, answered.
            let id = message.get("id").and_then(Value::as_str);
            if let Some((prompt, turn)) = self.prompt.clone()
                && id == Some(prompt.as_str())
            {
                let result = match acp_reply(&message) {
                    Some((_, Ok(result))) => Ok(result),
                    Some((_, Err(stated))) => Err(format!(
                        "Gemini refused: {}",
                        clip(stated.as_str().unwrap_or("the turn failed"), 400)
                    )),
                    None => Ok(Value::Null),
                };
                self.ended(turn, result);
            }
            return;
        }
        if method != "session/update" {
            return;
        }
        let Some(params) = message.get("params") else {
            return;
        };
        if let (Some(from), Some(current)) = (
            params.get("sessionId").and_then(Value::as_str),
            self.core.native_session_id.as_deref(),
        ) && from != current
        {
            return;
        }
        let Some(update) = params.get("update") else {
            return;
        };
        self.update(update);
    }

    fn update(&mut self, update: &Value) {
        match update.get("sessionUpdate").and_then(Value::as_str) {
            Some("available_commands_update") => {
                let names: Vec<String> = update
                    .get("availableCommands")
                    .and_then(Value::as_array)
                    .map(|list| {
                        list.iter()
                            .filter_map(|c| c.get("name").and_then(Value::as_str))
                            .filter(|name| *name != "clear")
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                if names != self.commands {
                    self.commands = names;
                    self.announce();
                }
            }
            Some("config_option_update") => {
                self.adopt(update);
                self.report();
            }
            Some("current_mode_update") => {
                if let Some(mode) = update
                    .get("currentModeId")
                    .and_then(Value::as_str)
                    .and_then(tendly_mode)
                    && self.mode.as_deref() != Some("danger")
                {
                    self.mode = Some(mode.to_owned());
                    self.report();
                }
            }
            Some("usage_update") => {
                self.core
                    .usage(None, update.get("used"), update.get("size"));
            }
            _ if self.core.active.is_none() => {}
            Some("agent_message_chunk") => {
                let Some(text) = update.pointer("/content/text").and_then(Value::as_str) else {
                    return;
                };
                let id = match &self.assistant {
                    Some(id) => id.clone(),
                    None => {
                        self.messages += 1;
                        let id = format!("gemini-assistant-{}", self.messages);
                        self.assistant = Some(id.clone());
                        id
                    }
                };
                let text = clip(text, MAX_TEXT);
                self.core.emit(
                    json!({"type":"message","id":id,"role":"assistant","text":text,"delta":true}),
                );
            }
            Some("tool_call" | "tool_call_update") => self.tool(update),
            Some("plan") => {
                let text = update
                    .get("entries")
                    .and_then(Value::as_array)
                    .map(|entries| {
                        entries
                            .iter()
                            .filter_map(|e| {
                                let done =
                                    e.get("status").and_then(Value::as_str) == Some("completed");
                                let step = e.get("content").and_then(Value::as_str)?;
                                Some(format!("{} {step}", if done { "[x]" } else { "[ ]" }))
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();
                let id = format!("gemini-plan-{}", self.core.turns.len());
                self.core.emit(json!({"type":"tool","id":id,"name":"Plan","status":"completed","text":clip(&text, MAX_TEXT)}));
            }
            _ => {}
        }
    }

    /// A tool call, or news of one: shown with what it was asked to do and,
    /// once done, what came of it.
    fn tool(&mut self, update: &Value) {
        let Some(id) = update.get("toolCallId").and_then(Value::as_str) else {
            return;
        };
        // What follows a tool is a new message, not more of the one before.
        self.assistant = None;
        let mut tool = self.tools.get(id).cloned().unwrap_or_default();
        let kind = match update.get("kind").and_then(Value::as_str) {
            Some("execute") => Some("Command"),
            Some("edit") => Some("Edit"),
            Some("read") => Some("Read"),
            Some("search") => Some("Search"),
            Some("fetch") => Some("Fetch"),
            Some("delete") => Some("Delete"),
            Some("move") => Some("Move"),
            _ => None,
        };
        if let Some(name) = kind.or_else(|| {
            [update.get("title"), update.get("name")]
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .find(|name| !name.trim().is_empty())
        }) {
            tool.name = clip(name, 256);
        }
        if tool.name.is_empty() {
            tool.name = "Gemini tool".into();
        }
        let described = describe(update);
        if !described.is_empty() {
            tool.text = clip(&described, MAX_TEXT);
        }
        let status = match update.get("status").and_then(Value::as_str) {
            Some("completed") => "completed",
            Some("failed") => "failed",
            _ => "running",
        };
        self.core
            .emit(json!({"type":"tool","id":id,"name":tool.name,"status":status,"text":tool.text}));
        if status == "running" {
            self.tools.insert(id.to_owned(), tool);
            if self.tools.len() > 512 {
                self.tools.clear();
            }
        } else {
            self.tools.remove(id);
        }
    }

    /// A request agy makes of Tendly: a permission it needs, shown as an
    /// approval; anything else it cannot be given here.
    fn request(&mut self, message: &Value) {
        let id = message.get("id").cloned().unwrap_or(Value::Null);
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let params = message.get("params").cloned().unwrap_or_default();
        let ours = params.get("sessionId").and_then(Value::as_str)
            == self.core.native_session_id.as_deref();
        if method != "session/request_permission" {
            let reply = json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Tendly does not offer this to Gemini."}});
            let _ = self.core.send(reply);
            return;
        }
        if !ours || self.core.active.is_none() {
            let _ = self.core.send(cancelled(&id));
            return;
        }
        let mut options: HashMap<&'static str, String> = HashMap::new();
        for offered in params
            .get("options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let (Some(kind), Some(option)) = (
                offered.get("kind").and_then(Value::as_str),
                offered.get("optionId").and_then(Value::as_str),
            ) else {
                continue;
            };
            let decision = match kind {
                "allow_once" => "accept",
                "allow_always" => "accept_session",
                "reject_once" => "decline",
                "reject_always" if !options.contains_key("decline") => "decline",
                _ => continue,
            };
            options.entry(decision).or_insert_with(|| option.to_owned());
        }
        if !options.contains_key("accept") && !options.contains_key("accept_session") {
            let _ = self.core.send(cancelled(&id));
            return;
        }
        let responder = Responder { id, options };
        // Everything is approved in `danger`, as agy's own flag would.
        if self.mode.as_deref() == Some("danger") {
            let decision = if responder.options.contains_key("accept_session") {
                Decision::AcceptSession
            } else {
                Decision::Accept
            };
            let _ = self.answer_request(&responder, decision);
            return;
        }
        let call = params.get("toolCall").cloned().unwrap_or_default();
        let kind = call.get("kind").and_then(Value::as_str).unwrap_or("");
        let command = call
            .pointer("/rawInput/CommandLine")
            .or_else(|| call.pointer("/rawInput/command"))
            .cloned();
        let mut paths: Vec<Value> = call
            .get("locations")
            .and_then(Value::as_array)
            .map(|l| l.iter().filter_map(|l| l.get("path").cloned()).collect())
            .unwrap_or_default();
        if let Some(cwd) = call.pointer("/rawInput/Cwd").cloned() {
            paths.push(cwd);
        }
        let access = match kind {
            "execute" => "execute",
            "edit" | "delete" | "move" => "write",
            "fetch" => "network",
            _ => "read",
        };
        let title = match kind {
            "execute" => "Approve native command",
            "edit" | "delete" | "move" => "Approve native file changes",
            _ => "Approve native tool",
        };
        let mut body = Map::new();
        if let Some(name) = call.get("title").filter(|t| t.is_string()) {
            body.insert("tool".into(), name.clone());
        }
        if let Some(input) = call.get("rawInput") {
            body.insert("input".into(), input.clone());
        }
        let text = stringify(&Value::Object(body));
        let mut choices: Vec<&str> = ["accept", "accept_session", "decline"]
            .into_iter()
            .filter(|c| responder.options.contains_key(c))
            .collect();
        choices.push("cancel");
        let mut description = Map::new();
        description.insert("title".into(), json!(title));
        description.insert("text".into(), json!(clip(&text, MAX_TEXT)));
        description.insert("choices".into(), json!(choices));
        let cwd = self.core.cwd.clone();
        if let Some(scope) = approval_scope(&cwd, Some(access), &paths, command.as_ref()) {
            description.insert("scope".into(), scope);
        }
        let key = format!("acp:{}", stringify(&responder.id));
        self.approval(key, description, responder);
    }

    fn answer_request(&mut self, responder: &Responder, decision: Decision) -> Result<(), ()> {
        let chosen = match decision {
            Decision::Accept => responder
                .options
                .get("accept")
                .or_else(|| responder.options.get("accept_session")),
            Decision::AcceptSession => responder
                .options
                .get("accept_session")
                .or_else(|| responder.options.get("accept")),
            Decision::Decline => responder.options.get("decline"),
            Decision::Cancel => None,
        };
        let reply = match chosen {
            Some(option) => {
                json!({"jsonrpc":"2.0","id":responder.id,"result":{"outcome":{"outcome":"selected","optionId":option}}})
            }
            None => cancelled(&responder.id),
        };
        self.core.send(reply).map_err(|_| ())
    }
}

fn cancelled(id: &Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":{"outcome":{"outcome":"cancelled"}}})
}

/// What a tool call is doing, as the chat shows it: the command and its
/// output, the files an edit touches and how, or what it was given.
fn describe(update: &Value) -> String {
    let mut parts: Vec<String> = Vec::new();
    let input = update.get("rawInput");
    if let Some(command) = input
        .and_then(|i| i.get("CommandLine").or_else(|| i.get("command")))
        .and_then(Value::as_str)
    {
        parts.push(format!("$ {command}"));
    }
    for item in update
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match item.get("type").and_then(Value::as_str) {
            Some("diff") => {
                let path = item.get("path").and_then(Value::as_str).unwrap_or("");
                let new = item.get("newText").and_then(Value::as_str).unwrap_or("");
                parts.push(format!("{path}\n{new}"));
            }
            Some("content") => {
                if let Some(text) = item.pointer("/content/text").and_then(Value::as_str) {
                    parts.push(text.to_owned());
                }
            }
            _ => {}
        }
    }
    if let Some(output) = update.pointer("/rawOutput/output").and_then(Value::as_str) {
        parts.push(output.to_owned());
    }
    if parts.is_empty() {
        let paths = update
            .get("locations")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .filter_map(|l| l.get("path").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .filter(|paths| !paths.is_empty());
        let shown = paths.or_else(|| {
            input
                .filter(|i| i.as_object().is_some_and(|o| !o.is_empty()))
                .map(stringify)
        });
        parts.extend(shown);
    }
    parts.join("\n")
}

/// agy's refusals, with what to do about the one that means no sign-in.
fn explain(message: &str) -> String {
    if message.contains("Authentication required") {
        format!(
            "{message}. This Gemini account is not signed in: run the sign-in command shown for it under Settings → Accounts."
        )
    } else {
        message.to_owned()
    }
}

/// Gemini, run in this process (see the module documentation).
pub fn adapter(env: &BTreeMap<String, String>, env_remove: &[String]) -> NativeAdapter {
    // agy acp is told its home with --gemini-dir: the home a terminal launch
    // is given to find it there (adapters/gemini.rs) is not this one's, and
    // the commands it runs keep the person's own.
    let gemini_dir = env.get(GEMINI_DIR).cloned();
    let mut env = env.clone();
    if gemini_dir.is_some() {
        env.remove("HOME");
    }
    let tools_server = tendly_tools(&env);
    let api_key = env.get("GEMINI_API_KEY").is_some_and(|k| !k.is_empty());
    NativeAdapter::new(
        &env,
        env_remove,
        Protocol {
            frame: acp_frame,
            reply: acp_reply,
            client_name: "Gemini",
            exited: "Gemini (agy acp) exited.",
            permission_modes: Some(&["ask", "plan", "accept_edits", "danger"]),
        },
        |core| Gemini {
            core,
            program: String::new(),
            gemini_dir,
            launch: Launch::default(),
            tools_server,
            api_key,
            options: Vec::new(),
            variants: Vec::new(),
            model: None,
            effort: None,
            mode: None,
            pending_model: false,
            pending_mode: false,
            commands: Vec::new(),
            prompt: None,
            prompts: 0,
            deferred: Vec::new(),
            tools: HashMap::new(),
            assistant: None,
            messages: 0,
        },
    )
}

impl NativeClient for Gemini {
    type Then = Then;
    type Responder = Responder;
    type Turn = ();
    fn core(&mut self) -> &mut Core<Then, Responder, ()> {
        &mut self.core
    }
    fn start(&mut self, init: ChatInit) {
        let dir = self.gemini_dir.clone();
        self.open(init, dir);
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
        self.on_message(message);
    }
    fn settle(&mut self, then: Then, result: Result<Value, String>) {
        self.after(then, result);
    }
    fn respond(
        &mut self,
        responder: &Responder,
        decision: Decision,
        _answers: &Answers,
    ) -> Result<(), ()> {
        self.answer_request(responder, decision)
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

    fn init(args: &[&str], resume: Option<&str>) -> ChatInit {
        ChatInit {
            provider: "gemini".into(),
            program: "agy".into(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            cwd: "/work".into(),
            resume_id: resume.map(str::to_owned),
        }
    }

    #[test]
    fn a_launch_becomes_acp_in_its_home_with_its_model_depth_and_mode() {
        let launch = gemini_launch(
            &init(
                &[
                    "--model",
                    "gemini-4-neon",
                    "--effort=low",
                    "--mode",
                    "plan",
                    "--conversation",
                    "abc-1",
                ],
                None,
            ),
            Some("/accounts/one"),
        )
        .unwrap();
        assert_eq!(launch.args, ["acp", "--gemini-dir", "/accounts/one"]);
        assert_eq!(launch.model.as_deref(), Some("gemini-4-neon"));
        assert_eq!(launch.effort.as_deref(), Some("low"));
        assert_eq!(launch.mode.as_deref(), Some("plan"));
        assert_eq!(launch.resume.as_deref(), Some("abc-1"));
        assert!(gemini_launch(&init(&["--sandbox"], None), None).is_err());
        assert!(gemini_launch(&init(&["--conversation", "b"], Some("a")), None).is_err());
    }

    #[test]
    fn a_model_offers_the_depths_agy_lists_it_at() {
        let options = vec![
            json!({"id":"model","currentValue":"gemini-4-neon","options":[
                {"value":"gemini-4-neon","name":"Gemini 4 Neon"},
                {"value":"gpt-oss-120b-medium","name":"GPT-OSS 120B (Medium)"}]}),
            json!({"id":"thought_level","currentValue":"high","options":[
                {"value":"low"},{"value":"medium"},{"value":"high"},{"value":"xhigh"}]}),
        ];
        let variants: Vec<String> = [
            "gemini-4-neon-high",
            "gemini-4-neon-low",
            "gpt-oss-120b-medium",
        ]
        .map(str::to_owned)
        .to_vec();
        let models = session_models(&options, &variants).unwrap();
        assert_eq!(
            models[0],
            json!({"id":"gemini-4-neon","name":"Gemini 4 Neon","efforts":["low","high"]})
        );
        assert_eq!(
            models[1],
            json!({"id":"gpt-oss-120b-medium","name":"GPT-OSS 120B (Medium)"})
        );
        assert_eq!(current(&options, "thought_level").as_deref(), Some("high"));
    }

    #[test]
    fn modes_translate_both_ways() {
        for mode in ["ask", "plan", "accept_edits"] {
            assert_eq!(tendly_mode(agy_mode(mode).unwrap()), Some(mode));
        }
        assert_eq!(agy_mode("danger"), Some("auto"));
        assert_eq!(agy_mode("bypass"), None);
    }

    #[test]
    fn a_tool_shows_its_command_and_output_or_its_diff() {
        let command = json!({"rawInput":{"CommandLine":"ls"},"rawOutput":{"output":"hello.txt"}});
        assert_eq!(describe(&command), "$ ls\nhello.txt");
        let edit = json!({"rawInput":{"Path":"/w/a"},"content":[{"type":"diff","path":"/w/a","newText":"hi\n"}]});
        assert_eq!(describe(&edit), "/w/a\nhi\n");
        let read = json!({"rawInput":{},"locations":[{"path":"/w/b"}]});
        assert_eq!(describe(&read), "/w/b");
    }

    #[test]
    fn tools_reach_a_chat_only_with_the_whole_identity() {
        let mut env = BTreeMap::new();
        env.insert("TENDLY_URL".into(), "http://127.0.0.1:1".into());
        env.insert("TENDLY_AGENT_TOKEN".into(), "t".into());
        assert!(tendly_tools(&env).is_empty());
        env.insert("TENDLY_SESSION_ID".into(), "s".into());
        let tools = tendly_tools(&env);
        assert_eq!(tools[0]["args"], json!(["mcp"]));
        assert_eq!(tools[0]["env"].as_array().unwrap().len(), 3);
    }
}
