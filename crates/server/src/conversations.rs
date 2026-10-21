//! Structured native-client transport. The CLI owns the Agent loop and tool
//! execution; this module only supervises JSONL, persistence and UI decisions.
//!
//! This file holds the state of each running conversation ([`ChatRuntime`]),
//! the registry of them ([`ChatManager`]) and the routes. `commands.rs` has
//! the handlers that act on a conversation (open, message, steer, interrupt,
//! approvals, model, permission and endpoint), `history.rs` the stored
//! conversation (appending, pages, snapshots and search), `supervisor.rs`
//! starting a client and supervising it, `events.rs` what a client may report
//! and what each event changes, and `socket.rs` the stream a view follows.
use crate::chat_adapter::{ChatCommand, ChatInit, ClientAdapter, dispatch};
use crate::{ApiError, AppState, Result, db, providers, session_record};
use axum::{
    Json, Router,
    extract::{
        Path, Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::StatusCode,
    response::Response,
    routing::{get, patch, post},
};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tendly_domain::{InteractionMode, ProviderKind, Session, SessionId, SessionStatus};
use tendly_persistence::MessageSubmission;
use tokio::sync::{Notify, broadcast, mpsc};

mod commands;
mod events;
mod history;
mod socket;
mod supervisor;
#[cfg(test)]
mod tests;

pub use commands::*;
pub(crate) use events::*;
pub use history::*;
pub use supervisor::*;

#[derive(Clone, Default)]
pub struct ChatManager {
    sessions: Arc<Mutex<HashMap<SessionId, Arc<ChatRuntime>>>>,
    channels: Arc<Mutex<HashMap<SessionId, broadcast::Sender<Value>>>>,
    /// The permission mode last chosen for each session. The clients forget
    /// it on every restart -- and a session whose turns keep failing restarts
    /// on every message -- so it is reapplied at each start, ahead of the
    /// saved preference. A choice made while the client is stopped waits here
    /// instead of being refused.
    permissions: Arc<Mutex<HashMap<SessionId, String>>>,
}
/// How long stopping a conversation waits for it to end. Its supervisor may
/// still be starting the client (an in-process adapter allows 5 s for that)
/// and then stops it (6 s for the client's task), so a shorter wait reports
/// "still shutting down" for a conversation that is ending normally.
const STOP_WAIT: Duration = Duration::from_secs(12);

pub struct ChatRuntime {
    commands: mpsc::Sender<Value>,
    stop: Notify,
    changed: Notify,
    done: Notify,
    running: AtomicBool,
    finished: AtomicBool,
    ready: AtomicBool,
    busy: AtomicBool,
    approvals: Mutex<HashMap<String, HashSet<String>>>,
    /// The process this conversation owns: the bridge, with the native client
    /// beneath it, or the native client itself. Known once it has started.
    pid: Mutex<Option<u32>>,
}
impl ChatRuntime {
    pub fn running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }
    fn idle(&self) -> bool {
        self.ready.load(Ordering::Acquire)
            && !self.busy.load(Ordering::Acquire)
            && self.approvals.lock().expect("approvals").is_empty()
    }
    /// What the client is doing, for an agent following a session it started.
    pub(crate) fn activity(&self) -> &'static str {
        if !self.running() {
            "stopped"
        } else if !self.approvals.lock().expect("approvals").is_empty() {
            "waiting_for_approval"
        } else if !self.ready.load(Ordering::Acquire) {
            "starting"
        } else if self.busy.load(Ordering::Acquire) {
            "working"
        } else {
            "idle"
        }
    }
    async fn ready(&self) -> Result<()> {
        tokio::time::timeout(Duration::from_secs(15),async {
            loop {let changed=self.changed.notified();if !self.running(){return Err(ApiError::conflict("Native chat could not initialize; check the client installation and configuration"));}
                if self.ready.load(Ordering::Acquire){return Ok(());}changed.await;}
        }).await.map_err(|_|ApiError::conflict("Native chat initialization timed out"))?
    }
    async fn send(&self, value: Value) -> Result<()> {
        if !self.running() {
            return Err(ApiError::conflict(
                "Native chat is disconnected; reconnect before sending",
            ));
        }
        self.commands.try_send(value).map_err(|_|ApiError::conflict("Native chat command queue is unavailable; inspect the current state before retrying"))
    }
    async fn stop(&self) {
        let done = self.done.notified();
        if self.finished.load(Ordering::Acquire) {
            return;
        }
        self.stop.notify_one();
        let _ = tokio::time::timeout(STOP_WAIT, done).await;
    }
}
impl ChatManager {
    /// Process ids of the conversations still running, for resource reporting.
    pub fn process_ids(&self) -> Vec<(SessionId, u32)> {
        self.sessions
            .lock()
            .expect("chat sessions")
            .iter()
            .filter(|(_, runtime)| runtime.running())
            .filter_map(|(id, runtime)| runtime.pid.lock().expect("chat pid").map(|pid| (*id, pid)))
            .collect()
    }
    pub fn get(&self, id: SessionId) -> Option<Arc<ChatRuntime>> {
        self.sessions
            .lock()
            .expect("chat sessions")
            .get(&id)
            .cloned()
    }
    fn chosen_permission(&self, id: SessionId) -> Option<String> {
        self.permissions
            .lock()
            .expect("chat permissions")
            .get(&id)
            .cloned()
    }
    fn choose_permission(&self, id: SessionId, mode: &str) {
        self.permissions
            .lock()
            .expect("chat permissions")
            .insert(id, mode.to_owned());
    }
    fn channel(&self, id: SessionId) -> broadcast::Sender<Value> {
        self.channels
            .lock()
            .expect("chat channels")
            .entry(id)
            .or_insert_with(|| broadcast::channel(256).0)
            .clone()
    }
    fn publish(&self, id: SessionId, event: Value) {
        let _ = self.channel(id).send(event);
    }
    pub async fn stop(&self, id: SessionId) -> Result<()> {
        if let Some(runtime) = self.get(id) {
            runtime.stop().await;
            if !runtime.finished.load(Ordering::Acquire) {
                return Err(ApiError::conflict("Native chat is still shutting down"));
            }
        }
        Ok(())
    }
    pub async fn shutdown(&self) {
        let active: Vec<_> = self
            .sessions
            .lock()
            .expect("chat sessions")
            .values()
            .cloned()
            .collect();
        futures_util::future::join_all(active.iter().map(|r| r.stop())).await;
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .merge(history::routes())
        .merge(commands::routes())
        .merge(socket::routes())
}

#[cfg(test)]
impl ChatRuntime {
    /// A running client with nothing behind it.
    pub(crate) fn for_test() -> Self {
        let (commands, _) = mpsc::channel(1);
        ChatRuntime {
            commands,
            stop: Notify::new(),
            changed: Notify::new(),
            done: Notify::new(),
            running: AtomicBool::new(true),
            finished: AtomicBool::new(false),
            ready: AtomicBool::new(true),
            busy: AtomicBool::new(false),
            approvals: Mutex::new(HashMap::new()),
            pid: Mutex::new(None),
        }
    }
}
