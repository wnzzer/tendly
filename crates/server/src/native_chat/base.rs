//! The conversation's state (`ChatBase`): what reaches it, its turns, the
//! requests and approvals it has open, and the events it emits.
use super::*;
use crate::chat_adapter::{Answers, ChatInit, Decision};
use indexmap::IndexMap;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tendly_runtime::process::OwnedGroup;
use tokio::{sync::mpsc, time::Instant};

/// What reaches a conversation's task.
pub enum Input {
    /// Start the client; the sender learns once the launch has been tried.
    Start(ChatInit, tokio::sync::oneshot::Sender<()>),
    Message(String, String),
    Steer(String, String),
    Approve(String, Decision, Option<Answers>),
    Interrupt,
    Model(Option<String>, Option<String>),
    Permission(String),
    Shutdown,
    /// A message the client wrote, parsed.
    Native(Value),
    /// The connection to the client broke, in the bridge's words.
    Broken(&'static str),
    /// The client's output ended.
    Ended,
}

/// The turn a chat is running, with what a client keeps per turn.
pub struct Turn<X> {
    pub id: String,
    pub interrupted: bool,
    pub extra: X,
}

pub struct Pending<T> {
    pub(super) then: T,
    pub(super) deadline: Instant,
    /// A capability probe, which may simply never be answered.
    pub(super) optional: bool,
    /// An interrupt that times out is not a broken connection.
    pub(super) interrupt: bool,
}

pub struct Approval<R> {
    pub id: String,
    pub responder: R,
    /// When it times out: once, like the bridge's timer. A timeout whose answer
    /// could not be sent leaves the approval open but never fires again.
    pub(super) deadline: Option<Instant>,
    pub choices: Vec<String>,
}

/// A client's wire envelope for one request.
pub type Frame = fn(&str, &str, Value) -> Value;
/// Which waiting request a message answers, and how: the reply's id, then
/// its result or what the client said in refusing. `None` is not a reply.
pub type Reply = fn(&Value) -> Option<(Value, Result<Value, Value>)>;

/// The message that clears a conversation, whatever the client: Claude Code's
/// own command, which Tendly carries out itself for the clients without one.
pub fn is_clear(content: &str) -> bool {
    content.trim() == "/clear"
}
/// What a client that has no `/clear` of its own offers once Tendly does it.
pub fn clear_command() -> Vec<String> {
    vec!["clear".to_owned()]
}

/// `jsonRpcRequest`.
pub fn json_rpc_frame(id: &str, method: &str, params: Value) -> Value {
    json!({"id":id,"method":method,"params":params})
}

/// How a client's protocol frames requests and replies, and names itself.
pub struct Protocol {
    pub frame: Frame,
    pub reply: Reply,
    pub client_name: &'static str,
    /// What the conversation says when the client's output ends.
    pub exited: &'static str,
    pub permission_modes: Option<&'static [&'static str]>,
}

/// What every client driven from this process shares.
pub struct Core<T, R, X> {
    pub(super) events: mpsc::UnboundedSender<Value>,
    pub(super) secrets: Vec<String>,
    pub(super) env: Vec<(String, String)>,
    pub(super) env_remove: Vec<String>,
    pub(super) inputs: mpsc::UnboundedSender<Input>,
    pub(super) writer: Option<mpsc::UnboundedSender<Vec<u8>>>,
    pub(super) queued: Arc<AtomicUsize>,
    pub(super) group: OwnedGroup,
    pub(super) child: Option<tokio::process::Child>,
    pub(super) pid: Arc<std::sync::Mutex<Option<u32>>>,
    pub(super) sequence: u64,
    pub(super) pending: HashMap<String, Pending<T>>,
    pub(super) frame: Frame,
    pub(super) reply: Reply,
    /// The client's name in a refusal: "Codex refused: …".
    pub(super) client_name: &'static str,
    pub(super) seen: HashMap<String, String>,
    pub approvals: IndexMap<String, Approval<R>>,
    pub turns: Vec<Turn<X>>,
    pub active: Option<usize>,
    pub cwd: String,
    pub ready: bool,
    pub closed: bool,
    pub port_closed: bool,
    pub initializing: bool,
    pub initialized: bool,
    pub(super) finished: bool,
    /// An event too long for the wire: the bridge's `emit` called `fatal`.
    pub(super) oversized: bool,
    /// More than 4 MB of events sent and not yet taken: the conversation stops.
    pub(crate) backlogged: bool,
    /// Bytes of events sent and not yet taken by the adapter.
    pub(super) unsent: Arc<AtomicUsize>,
    pub native_session_id: Option<String>,
    pub permission_mode: Option<String>,
    /// The approval modes this client offers, reported with its settings.
    pub permission_modes: Option<&'static [&'static str]>,
}

impl<T, R, X: Default> Core<T, R, X> {
    /// A conversation not yet started: `env` applied over this server's
    /// environment and `env_remove` taken out of it, as the bridge's child
    /// process had.
    pub fn new(
        env: &BTreeMap<String, String>,
        env_remove: &[String],
        protocol: &Protocol,
        events: mpsc::UnboundedSender<Value>,
        inputs: mpsc::UnboundedSender<Input>,
        pid: Arc<std::sync::Mutex<Option<u32>>>,
    ) -> Self {
        let mut environment: IndexMap<String, String> = std::env::vars().collect();
        for (key, value) in env {
            environment.insert(key.clone(), value.clone());
        }
        for key in env_remove
            .iter()
            .cloned()
            .chain(crate::bridge::tendly_secrets())
        {
            environment.shift_remove(&key);
        }
        Core {
            events,
            secrets: secrets_of(environment.into_iter()),
            env: env.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            env_remove: env_remove.to_vec(),
            inputs,
            writer: None,
            queued: Arc::new(AtomicUsize::new(0)),
            group: OwnedGroup::new(None),
            child: None,
            pid,
            sequence: 0,
            pending: HashMap::new(),
            frame: protocol.frame,
            reply: protocol.reply,
            client_name: protocol.client_name,
            seen: HashMap::new(),
            approvals: IndexMap::new(),
            turns: Vec::new(),
            active: None,
            cwd: String::new(),
            ready: false,
            closed: false,
            port_closed: false,
            initializing: false,
            initialized: false,
            finished: false,
            oversized: false,
            backlogged: false,
            unsent: Arc::new(AtomicUsize::new(0)),
            native_session_id: None,
            permission_mode: None,
            permission_modes: protocol.permission_modes,
        }
    }

    // -- emitting --

    /// Emit a redacted event within the line size limit.
    pub fn emit(&mut self, event: Value) {
        if self.finished {
            return;
        }
        let event = scrub(event, &self.secrets);
        if stringify(&event).len() + 1 > MAX_EVENT_LINE {
            // `fatal` in the bridge: the error and the failed turn go out now,
            // so a step that goes on to finish the turn cannot take its place;
            // the loop closes the connection once this step is done.
            if !self.oversized {
                self.oversized = true;
                self.error("Structured event exceeded its wire size limit.");
                self.finish("failed");
            }
            return;
        }
        // The bridge stops when more than 4 MB of its output is unread, rather
        // than buffer without bound; the loop shuts the conversation down.
        if self.backlogged || self.unsent.load(Ordering::Acquire) > MAX_UNSENT {
            self.backlogged = true;
            return;
        }
        self.unsent
            .fetch_add(stringify(&event).len() + 1, Ordering::AcqRel);
        let _ = self.events.send(event);
    }
    /// `ChatBase.error`: clipped to 2 KB.
    pub fn error(&mut self, message: &str) {
        let message = clip(message, 2048);
        self.emit(json!({"type":"error","message":message}));
    }
    /// A conversation error, unclipped.
    pub fn raw_error(&mut self, message: &str) {
        self.emit(json!({"type":"error","message":message}));
    }

    // -- ChatBase --

    /// `ChatBase.begin`: a message that opens a turn, if it may.
    pub fn begin(&mut self, id: String, content: &str) -> Option<usize> {
        if !self.ready || self.closed {
            self.error("The native client is not ready.");
            return None;
        }
        let digest = hex(Sha256::digest(content.as_bytes()).as_slice());
        if let Some(previous) = self.seen.get(&id) {
            if *previous != digest {
                self.error("Message ID was already used for different content.");
            }
            return None;
        }
        if self.active.is_some() {
            self.error(
                "A turn is already running. Interrupt it or wait before sending another message.",
            );
            return None;
        }
        if self.seen.len() >= 4096 {
            self.error("This bridge reached its message limit. Reopen the saved native session.");
            self.emit(json!({"type":"turn","id":id,"status":"failed"}));
            return None;
        }
        self.seen.insert(id.clone(), digest);
        let turn = self.open_turn(id.clone());
        let text = clip(content, MAX_TEXT);
        self.emit(json!({"type":"message","id":id,"role":"user","text":text,"delta":false}));
        self.emit(json!({"type":"turn","id":id,"status":"running"}));
        Some(turn)
    }
    /// A turn without the user message `begin` writes.
    /// Whether a message id was already taken (`seen` in the bridge).
    pub fn has_seen(&self, id: &str) -> bool {
        self.seen.contains_key(id)
    }
    pub fn open_turn(&mut self, id: String) -> usize {
        self.turns.push(Turn {
            id,
            interrupted: false,
            extra: X::default(),
        });
        let turn = self.turns.len() - 1;
        self.active = Some(turn);
        turn
    }
    /// `ChatBase.acceptSteer`.
    pub fn accept_steer(&mut self, id: &str, content: &str) -> bool {
        if !self.ready || self.closed {
            self.error("The native client is not ready.");
            return false;
        }
        let digest = hex(Sha256::digest(content.as_bytes()).as_slice());
        if let Some(previous) = self.seen.get(id) {
            if *previous != digest {
                self.error("Message ID was already used for different content.");
            }
            return false;
        }
        if self.seen.len() >= 4096 {
            self.error("This bridge reached its message limit. Reopen the saved native session.");
            return false;
        }
        self.seen.insert(id.to_owned(), digest);
        true
    }
    /// `ChatBase.finish`: the turn ends, and its open requests with it.
    pub fn finish(&mut self, status: &str) {
        let Some(turn) = self.active.take() else {
            return;
        };
        let id = self.turns[turn].id.clone();
        let keys: Vec<String> = self.approvals.keys().cloned().collect();
        for key in keys {
            self.resolve_native(&key);
        }
        self.emit(json!({"type":"turn","id":id,"status":status}));
    }
    /// A new native conversation took the old one's place: the view drops
    /// what the client can no longer refer to, and the new id is the one a
    /// restart resumes. Claude Code does this by itself on `/clear`; Codex and
    /// Pi have no such command, so their adapters open a new conversation and
    /// say so here, and `/clear` reads the same in all three.
    pub fn replaced(&mut self, id: Option<&str>, commands: Option<Vec<String>>) {
        self.native_session_id = None;
        self.emit(json!({"type":"cleared"}));
        self.announce(id, commands);
    }
    /// `ChatBase.announce`: ready, with the session's id and the commands the
    /// client itself advertised.
    pub fn announce(&mut self, id: Option<&str>, commands: Option<Vec<String>>) {
        let valid = id.filter(|id| native_id(id));
        if let (Some(id), Some(current)) = (valid, &self.native_session_id)
            && id != current
        {
            self.emit(json!({"type":"cleared"}));
        }
        if let Some(id) = valid {
            self.native_session_id = Some(id.to_owned());
        }
        self.ready = true;
        let names: Vec<String> = commands
            .map(|commands| {
                let mut unique: Vec<String> = Vec::new();
                for name in commands {
                    let mut chars = name.chars();
                    let valid = chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
                        && name.chars().count() <= 64
                        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '-'));
                    if valid && !unique.contains(&name) {
                        unique.push(name);
                    }
                }
                unique.truncate(400);
                unique
            })
            .unwrap_or_default();
        let mut event = json!({"type":"ready"});
        if let Some(id) = &self.native_session_id {
            event["native_session_id"] = json!(id);
        }
        if !names.is_empty() {
            event["commands"] = json!(names);
        }
        self.emit(event);
    }
    /// `ChatBase.settings`.
    pub fn settings(
        &mut self,
        models: Option<&[Value]>,
        model: Option<&str>,
        effort: Option<&str>,
    ) {
        const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
        let list: Option<Vec<Value>> = models.map(|models| {
            models
                .iter()
                .filter_map(|entry| {
                    let id = entry
                        .get("id")?
                        .as_str()
                        .filter(|id| !id.is_empty() && id.encode_utf16().count() <= 128)?;
                    let mut row = Map::new();
                    row.insert("id".into(), json!(id));
                    let name = entry
                        .get("name")
                        .and_then(Value::as_str)
                        .filter(|n| !n.is_empty())
                        .unwrap_or(id);
                    row.insert("name".into(), json!(clip(name, 128)));
                    if let Some(description) = entry
                        .get("description")
                        .and_then(Value::as_str)
                        .filter(|d| !d.is_empty())
                    {
                        row.insert("description".into(), json!(clip(description, 256)));
                    }
                    if entry.get("isDefault") == Some(&Value::Bool(true)) {
                        row.insert("isDefault".into(), json!(true));
                    }
                    if let Some(efforts) = entry.get("efforts").and_then(Value::as_array) {
                        let efforts: Vec<Value> = efforts
                            .iter()
                            .filter(|e| e.as_str().is_some_and(|e| EFFORTS.contains(&e)))
                            .take(16)
                            .cloned()
                            .collect();
                        if !efforts.is_empty() {
                            row.insert("efforts".into(), Value::Array(efforts));
                        }
                    }
                    Some(Value::Object(row))
                })
                .take(64)
                .collect()
        });
        let selected = model.filter(|m| !m.is_empty() && m.encode_utf16().count() <= 128);
        let depth = effort
            .filter(|e| (1..=16).contains(&e.len()) && e.bytes().all(|b| b.is_ascii_lowercase()));
        let permission = self.permission_mode.clone().filter(|m| {
            (1..=24).contains(&m.len()) && m.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
        });
        let list = list.filter(|list| !list.is_empty());
        // The modes alone are worth saying: without them a client whose model
        // list never came had no permission chip either.
        let modes = self.permission_modes.filter(|m| !m.is_empty());
        if list.is_none()
            && selected.is_none()
            && depth.is_none()
            && permission.is_none()
            && modes.is_none()
        {
            return;
        }
        let mut event = Map::new();
        event.insert("type".into(), json!("settings"));
        if let Some(model) = selected {
            event.insert("model".into(), json!(model));
        }
        if let Some(effort) = depth {
            event.insert("effort".into(), json!(effort));
        }
        if let Some(mode) = permission {
            event.insert("permission_mode".into(), json!(mode));
        }
        if let Some(modes) = modes {
            event.insert("permission_modes".into(), json!(modes));
        }
        if let Some(list) = list {
            event.insert("models".into(), Value::Array(list));
        }
        self.emit(Value::Object(event));
    }
    /// `ChatBase.usage`.
    pub fn usage(
        &mut self,
        usage: Option<&Value>,
        context: Option<&Value>,
        window: Option<&Value>,
    ) {
        let pick = |a: &str, b: &str| {
            let value = usage.and_then(|u| u.get(a)).filter(|v| !v.is_null());
            safe_integer(value.or_else(|| usage.and_then(|u| u.get(b))))
        };
        let mut event = Map::new();
        event.insert("type".into(), json!("usage"));
        for (key, value) in [
            ("input_tokens", pick("inputTokens", "input_tokens")),
            ("output_tokens", pick("outputTokens", "output_tokens")),
            ("context_tokens", safe_integer(context)),
            ("context_window", safe_integer(window)),
        ] {
            if let Some(value) = value.filter(|v| {
                if key == "context_window" {
                    *v > 0
                } else {
                    *v >= 0
                }
            }) {
                event.insert(key.into(), json!(value));
            }
        }
        if event.len() > 1 {
            self.emit(Value::Object(event));
        }
    }
    /// `ChatBase.approval`, once room for it is known: the card the person sees.
    pub(super) fn register(
        &mut self,
        key: String,
        description: Map<String, Value>,
        responder: R,
        choices: Vec<String>,
    ) {
        let id = uuid::Uuid::new_v4().to_string();
        self.approvals.insert(
            key,
            Approval {
                id: id.clone(),
                responder,
                deadline: Some(Instant::now() + APPROVAL_TIMEOUT),
                choices,
            },
        );
        let mut event = Map::new();
        event.insert("type".into(), json!("approval"));
        event.insert("id".into(), json!(id));
        event.extend(description);
        self.emit(Value::Object(event));
    }
    /// `ChatBase.resolveNative`: the client settled it itself.
    pub fn resolve_native(&mut self, key: &str) {
        if let Some(pending) = self.approvals.shift_remove(key) {
            self.emit(json!({"type":"approval_resolved","id":pending.id}));
        }
    }
}
