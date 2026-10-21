//! [`NativeClient`]: a client's protocol on top of the [`Core`], and the
//! parts of `ChatBase` every client shares for answering it.
use super::*;
use crate::chat_adapter::{Answers, ChatInit, Decision};
use serde_json::{Map, Value};
use tokio::time::Instant;

/// The decision an approval falls back to when nobody answers it.
fn fallback(choices: &[String]) -> Decision {
    if choices.iter().any(|c| c == "cancel") {
        Decision::Cancel
    } else {
        Decision::Decline
    }
}

/// A structured client: its protocol on top of the [`Core`].
pub trait NativeClient: Send + 'static {
    /// What follows one of its replies.
    type Then: Send;
    /// How one of its requests is answered once the person has decided.
    type Responder: Send;
    /// What it keeps per turn.
    type Turn: Default + Send;
    fn core(&mut self) -> &mut Core<Self::Then, Self::Responder, Self::Turn>;
    /// Start the client (`initialize`).
    fn start(&mut self, init: ChatInit);
    fn message(&mut self, id: String, content: String);
    fn steer(&mut self, id: String, content: String);
    fn interrupt(&mut self);
    fn select_model(&mut self, model: Option<String>, effort: Option<String>);
    fn set_permission_mode(&mut self, mode: String);
    /// A message from the client that answers nothing it asked.
    fn notification(&mut self, message: Value);
    /// What follows a reply, or a request that failed.
    fn settle(&mut self, then: Self::Then, result: Result<Value, String>);
    /// Answer a request; an `Err` is the bridge's throw inside its answer.
    fn respond(
        &mut self,
        responder: &Self::Responder,
        decision: Decision,
        answers: &Answers,
    ) -> Result<(), ()>;
    /// The responder of the approval at `key`, to answer it with.
    fn responder(&mut self, key: &str) -> Option<Self::Responder>;

    // -- provided: the parts of ChatBase that answer the client --

    /// `NativeProcess.rpc`.
    fn rpc(&mut self, method: &str, params: Value, then: Self::Then, optional: bool) {
        if let Err((then, message)) = self.core().request(method, params, then, optional) {
            self.settle(then, Err(message));
        }
    }
    /// `ChatBase.approval`.
    fn approval(
        &mut self,
        key: String,
        description: Map<String, Value>,
        responder: Self::Responder,
    ) {
        if self.core().approvals.contains_key(&key) {
            return;
        }
        let choices: Vec<String> = description
            .get("choices")
            .and_then(Value::as_array)
            .map(|c| {
                c.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        if self.core().approvals.len() >= 32 {
            let _ = self.respond(&responder, fallback(&choices), &Map::new());
            self.core().error("Too many pending native approvals.");
            return;
        }
        self.core().register(key, description, responder, choices);
    }
    /// `ChatBase.resolveApproval`.
    fn resolve_approval(&mut self, key: &str, decision: Decision, answers: &Answers) {
        let Some(responder) = self.responder(key) else {
            return;
        };
        if self.respond(&responder, decision, answers).is_err() {
            self.core()
                .error("Invalid or unavailable native approval response.");
            return;
        }
        self.core().resolve_native(key);
    }
    /// `ChatBase.answer`.
    fn answer(&mut self, request_id: &str, decision: Decision, answers: Option<Answers>) {
        let core = self.core();
        let Some((key, pending)) = core.approvals.iter().find(|(_, p)| p.id == request_id) else {
            core.error("This approval is no longer pending.");
            return;
        };
        if !pending.choices.iter().any(|c| c == decision.as_str()) {
            core.error("This decision is not offered by the native request.");
            return;
        }
        let key = key.clone();
        self.resolve_approval(&key, decision, &answers.unwrap_or_default());
    }
    /// `ChatBase.clearApprovals(true)`: every open request answered with its
    /// fallback.
    fn cancel_approvals(&mut self) {
        let keys: Vec<(String, Decision)> = self
            .core()
            .approvals
            .iter()
            .map(|(key, a)| (key.clone(), fallback(&a.choices)))
            .collect();
        for (key, decision) in keys {
            self.resolve_approval(&key, decision, &Map::new());
        }
    }
    /// `NativeProcess.close`, with what was waiting failed.
    fn close_port(&mut self) {
        for then in self.core().close_port() {
            self.settle(then, Err(CANCELLED.to_owned()));
        }
    }
    /// `ChatBase.shutdown`.
    fn shutdown(&mut self) {
        if self.core().closed {
            return;
        }
        self.core().closed = true;
        self.cancel_approvals();
        self.core().finish("interrupted");
        self.close_port();
    }
    /// `ChatBase.fatal`.
    fn fatal(&mut self, message: &str) {
        if self.core().closed {
            return;
        }
        self.core().error(message);
        self.core().finish("failed");
        self.shutdown();
    }
    /// The connection's own failure, which `NativeProcess` hands to `fatal`.
    fn failure(&mut self, message: &str) {
        if !self.core().port_closed {
            self.fatal(message);
        }
    }
    /// What arrived from the client: a reply to a request, or anything else.
    fn native(&mut self, message: Value) {
        if self.core().port_closed {
            return;
        }
        if let Some((then, result)) = self.core().answered(&message) {
            self.settle(then, result);
            return;
        }
        self.notification(message);
    }
    /// Requests and approvals whose time is up.
    fn expire(&mut self, now: Instant) {
        for (then, breaks) in self.core().expired_requests(now) {
            // A timed-out start may have executed: the connection is not reused.
            if breaks {
                self.failure(
                    "Native control request timed out; the structured connection was stopped.",
                );
            }
            self.settle(then, Err("Native control request timed out.".into()));
        }
        for key in self.core().expired_approvals(now) {
            let decision = self
                .core()
                .approvals
                .get(&key)
                .map_or(Decision::Decline, |a| fallback(&a.choices));
            self.resolve_approval(&key, decision, &Map::new());
            self.core()
                .error("Native approval timed out and was cancelled.");
        }
    }
    /// Handle conversation commands.
    fn command(&mut self, input: Input) {
        if self.core().closed {
            return;
        }
        if self.core().initializing {
            self.core()
                .raw_error("Chat initialization is still in progress.");
            return;
        }
        if !self.core().initialized {
            self.core().raw_error("Invalid native chat initialization.");
            self.shutdown();
            return;
        }
        match input {
            Input::Message(id, content) => self.message(id, content),
            Input::Steer(id, content) => self.steer(id, content),
            Input::Approve(request_id, decision, answers) => {
                self.answer(&request_id, decision, answers)
            }
            Input::Interrupt => self.interrupt(),
            Input::Model(model, effort) => self.select_model(model, effort),
            Input::Permission(mode) => self.set_permission_mode(mode),
            _ => {}
        }
    }
}
