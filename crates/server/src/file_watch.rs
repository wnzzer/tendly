//! Live workspace changes. One recursive watcher per workspace, shared by every
//! view of it and dropped with the last one, pushes which paths changed so the
//! tree and open files follow edits an agent (or anything else) makes on disk.
use crate::{AppState, CheckoutParam, Result, checkout_root};
use axum::{
    Router,
    extract::{
        Path, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    response::Response,
    routing::get,
};
use futures_util::StreamExt;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, HashMap},
    path::{Component, PathBuf},
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tendly_domain::WorkspaceId;
use tokio::sync::{broadcast, mpsc};

/// Events closer together than this reach the browser as one message.
const BATCH: Duration = Duration::from_millis(200);
/// Past this many paths a batch says "everything" instead of listing them.
const MAX_PATHS: usize = 500;
/// Churn nobody browses: dependency and build output. Changes there are
/// dropped before they cost the browser a request.
const NOISY: &[&str] = &[
    "node_modules",
    "target",
    ".venv",
    "__pycache__",
    ".next",
    ".turbo",
    ".cache",
];

#[derive(Clone, Default)]
pub struct FileWatches {
    /// Keyed by the directory watched: a workspace's, or a worktree of it.
    watches: Arc<Mutex<HashMap<PathBuf, Weak<WorkspaceWatch>>>>,
}
/// Alive while any socket holds it; dropping it stops the watcher, which
/// closes the batching task behind it.
pub struct WorkspaceWatch {
    _watcher: RecommendedWatcher,
    changes: broadcast::Sender<Value>,
}
impl FileWatches {
    /// Blocking: on Linux a recursive watch walks the whole tree.
    fn subscribe(
        &self,
        root: PathBuf,
    ) -> notify::Result<(Arc<WorkspaceWatch>, broadcast::Receiver<Value>)> {
        let mut watches = self.watches.lock().expect("file watches");
        if let Some(watch) = watches.get(&root).and_then(Weak::upgrade) {
            let changes = watch.changes.subscribe();
            return Ok((watch, changes));
        }
        let (raw, events) = mpsc::unbounded_channel();
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = raw.send(event);
        })?;
        watcher.watch(&root, RecursiveMode::Recursive)?;
        let (changes, receiver) = broadcast::channel(64);
        tokio::spawn(batch(root.clone(), events, changes.clone()));
        let watch = Arc::new(WorkspaceWatch {
            _watcher: watcher,
            changes,
        });
        watches.retain(|_, watch| watch.strong_count() > 0);
        watches.insert(root, Arc::downgrade(&watch));
        Ok((watch, receiver))
    }
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/workspaces/{id}/files/ws", get(socket))
}

async fn socket(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    upgrade: WebSocketUpgrade,
) -> Result<Response> {
    let root = checkout_root(&state, id, &checkout).await?;
    Ok(upgrade
        .max_message_size(1024)
        .on_upgrade(move |socket| stream(state, id, root, socket)))
}

async fn stream(state: AppState, id: WorkspaceId, root: PathBuf, socket: WebSocket) {
    let (mut sender, mut receiver) = socket.split();
    let watches = state.watches.clone();
    let subscribed = tokio::task::spawn_blocking(move || watches.subscribe(root)).await;
    let (_watch, mut changes) = match subscribed {
        Ok(Ok(subscription)) => subscription,
        failure => {
            let message = match failure {
                Ok(Err(error)) => error.to_string(),
                _ => "watch task failed".into(),
            };
            tracing::warn!(workspace=%id, %message, "file watching unavailable");
            let notice = json!({"type":"unavailable","message":message});
            let _ = crate::send_frame(&mut sender, Message::Text(notice.to_string().into())).await;
            return;
        }
    };
    let ready = json!({"type":"ready"}).to_string();
    if crate::send_frame(&mut sender, Message::Text(ready.into()))
        .await
        .is_err()
    {
        return;
    }
    loop {
        tokio::select! {
            change = changes.recv() => {
                let value = match change {
                    Ok(value) => value,
                    // Missed batches could be anywhere, so the view reloads it all.
                    Err(broadcast::error::RecvError::Lagged(_)) => everything(false),
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                if crate::send_frame(&mut sender, Message::Text(value.to_string().into())).await.is_err() { break; }
            }
            incoming = receiver.next() => match incoming {
                Some(Ok(Message::Ping(bytes))) => { if crate::send_frame(&mut sender, Message::Pong(bytes)).await.is_err() { break; } }
                Some(Ok(Message::Pong(_))) => {}
                _ => break,
            },
        }
    }
}

fn everything(git: bool) -> Value {
    json!({"type":"changed","paths":null,"git":git})
}

async fn batch(
    root: PathBuf,
    mut events: mpsc::UnboundedReceiver<notify::Result<notify::Event>>,
    changes: broadcast::Sender<Value>,
) {
    while let Some(first) = events.recv().await {
        let mut batch = Batch::default();
        batch.add(&root, first);
        let window = tokio::time::sleep(BATCH);
        tokio::pin!(window);
        loop {
            tokio::select! {
                _ = &mut window => break,
                next = events.recv() => match next {
                    Some(event) => batch.add(&root, event),
                    None => return,
                },
            }
        }
        if let Some(message) = batch.message() {
            let _ = changes.send(message);
        }
    }
}

#[derive(Default)]
struct Batch {
    paths: BTreeSet<String>,
    git: bool,
    overflow: bool,
}
impl Batch {
    fn add(&mut self, root: &std::path::Path, event: notify::Result<notify::Event>) {
        // A dropped event queue or an error means the watcher lost track.
        let Ok(event) = event else {
            self.overflow = true;
            return;
        };
        if event.need_rescan() {
            self.overflow = true;
        }
        if matches!(event.kind, notify::EventKind::Access(_)) {
            return;
        }
        for path in &event.paths {
            match classify(root, path) {
                Change::Ignored => {}
                Change::Git => self.git = true,
                Change::Path(path) => {
                    self.paths.insert(path);
                }
            }
        }
        if self.paths.len() > MAX_PATHS {
            self.overflow = true;
        }
    }
    fn message(self) -> Option<Value> {
        if self.overflow {
            return Some(everything(true));
        }
        (!self.paths.is_empty() || self.git)
            .then(|| json!({"type":"changed","paths":self.paths,"git":self.git}))
    }
}

#[derive(Debug, PartialEq)]
enum Change {
    Ignored,
    /// Repository metadata moved: status may differ though no file did.
    Git,
    Path(String),
}

fn classify(root: &std::path::Path, path: &std::path::Path) -> Change {
    let Ok(relative) = path.strip_prefix(root) else {
        return Change::Ignored;
    };
    let parts: Vec<_> = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy()),
            _ => None,
        })
        .collect();
    if parts.is_empty() {
        return Change::Ignored;
    }
    if parts[0] == ".git" {
        // Objects and lock files churn on every command; the index, HEAD and
        // refs they lead to are what status reads.
        let noise = parts.get(1).is_some_and(|part| part == "objects")
            || parts.last().is_some_and(|part| part.ends_with(".lock"));
        return if noise { Change::Ignored } else { Change::Git };
    }
    if parts.iter().any(|part| NOISY.contains(&part.as_ref()))
        || crate::workspace_io::is_protected_path(relative)
    {
        return Change::Ignored;
    }
    Change::Path(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn classifies_workspace_paths() {
        let root = Path::new("/w");
        assert_eq!(
            classify(root, Path::new("/w/src/app.ts")),
            Change::Path("src/app.ts".into())
        );
        assert_eq!(classify(root, Path::new("/w/.git/index")), Change::Git);
        assert_eq!(
            classify(root, Path::new("/w/.git/refs/heads/main")),
            Change::Git
        );
        for ignored in [
            "/w",
            "/elsewhere/file",
            "/w/.git/objects/ab/cdef",
            "/w/.git/index.lock",
            "/w/node_modules/pkg/index.js",
            "/w/crates/server/target/debug/x",
            "/w/.ssh/id_ed25519",
            "/w/config/credentials",
        ] {
            assert_eq!(
                classify(root, Path::new(ignored)),
                Change::Ignored,
                "{ignored}"
            );
        }
    }

    #[test]
    fn batches_overflow_into_everything() {
        let root = Path::new("/w");
        let mut batch = Batch::default();
        for index in 0..=MAX_PATHS {
            let event =
                notify::Event::new(notify::EventKind::Any).add_path(root.join(format!("f{index}")));
            batch.add(root, Ok(event));
        }
        assert_eq!(batch.message(), Some(everything(true)));
        assert_eq!(Batch::default().message(), None);
    }
}
