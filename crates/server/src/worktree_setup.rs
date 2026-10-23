//! Preparing a worktree Tendly made for a session.
//!
//! Git gives a new worktree the tracked files and nothing else: no installed
//! dependencies, no `.env`. What the person set for the workspace fills that
//! in -- paths copied from the workspace directory (as copy-on-write clones
//! where the file system has them, so a `node_modules` costs next to nothing),
//! then a command run inside the worktree. The setting is the person's, made
//! in Tendly; nothing in the repository decides what runs.
//!
//! It runs in the background and reports into the session's conversation as
//! one `setup` event, updated in place. A session's client does not start in
//! the worktree until it is done ([`ready`]).
use crate::{ApiError, AppState, Result};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tendly_domain::{SessionId, WorktreeSetup};
use tokio::{io::AsyncReadExt, process::Command, sync::watch};

const MAX_COPY_PATHS: usize = 32;
const MAX_COMMAND: usize = 4000;
const COPY_TIMEOUT: Duration = Duration::from_secs(600);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(1200);
/// How much of what the copies and the command said is kept: the end of it.
const LOG_TAIL: usize = 16 * 1024;

/// The setups running now, by worktree directory.
#[derive(Clone, Default)]
pub struct Setups(Arc<Mutex<HashMap<String, watch::Receiver<bool>>>>);

impl Setups {
    fn running(&self, path: &str) -> Option<watch::Receiver<bool>> {
        self.0.lock().expect("setups").get(path).cloned()
    }
}

/// A setting as the person entered it, made tidy: blank entries dropped, and
/// nothing that prepares nothing kept at all.
pub fn validate(setup: WorktreeSetup) -> Result<Option<WorktreeSetup>> {
    let mut copy: Vec<String> = Vec::new();
    for entry in setup.copy {
        let entry = entry.trim().trim_end_matches(['/', '\\']).to_owned();
        if entry.is_empty() || copy.contains(&entry) {
            continue;
        }
        let path = Path::new(&entry);
        if entry.len() > 400
            || !path
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
        {
            return Err(ApiError::bad(format!(
                "{entry}: name a path inside the workspace directory, such as .env or node_modules"
            )));
        }
        copy.push(entry);
    }
    if copy.len() > MAX_COPY_PATHS {
        return Err(ApiError::bad(format!(
            "At most {MAX_COPY_PATHS} paths can be copied"
        )));
    }
    let command = setup
        .command
        .map(|command| command.trim().to_owned())
        .filter(|command| !command.is_empty());
    if command
        .as_ref()
        .is_some_and(|command| command.len() > MAX_COMMAND || command.contains('\0'))
    {
        return Err(ApiError::bad(format!(
            "The setup command is longer than {MAX_COMMAND} bytes"
        )));
    }
    Ok((!copy.is_empty() || command.is_some()).then_some(WorktreeSetup { copy, command }))
}

/// Prepare `worktree`, just made for `session` from the workspace directory
/// `home`, as the workspace's setting says. Returns at once; [`ready`] waits.
pub fn start(
    state: &AppState,
    session: SessionId,
    setup: Option<WorktreeSetup>,
    home: PathBuf,
    worktree: PathBuf,
    branch: String,
) {
    let Some(setup) = setup else {
        return;
    };
    let key = worktree.to_string_lossy().into_owned();
    let (done, receiver) = watch::channel(false);
    state
        .setups
        .0
        .lock()
        .expect("setups")
        .insert(key.clone(), receiver);
    let state = state.clone();
    tokio::spawn(async move {
        run(&state, session, &setup, &home, &worktree, &branch).await;
        state.setups.0.lock().expect("setups").remove(&key);
        let _ = done.send(true);
    });
}

/// Wait for the setup of the worktree a session works in, if one is running.
pub async fn ready(state: &AppState, checkout: Option<&str>) {
    let Some(mut receiver) = checkout.and_then(|path| state.setups.running(path)) else {
        return;
    };
    let _ = receiver.wait_for(|done| *done).await;
}

/// Wait for the setup of the worktree session `id` works in. Called before
/// taking the operations lock, which a start holds: a setup may take minutes.
pub async fn ready_for(state: &AppState, id: SessionId) {
    if let Ok(session) = crate::session_record(state, id).await {
        ready(state, session.checkout_path.as_deref()).await;
    }
}

/// Whether the worktree at `path` is still being prepared.
pub fn preparing(state: &AppState, path: &str) -> bool {
    state.setups.running(path).is_some()
}

/// What the copies and the command said, keeping only the end.
#[derive(Default)]
struct Log(String);

impl Log {
    fn line(&mut self, text: impl AsRef<str>) {
        self.push(text.as_ref());
        self.push("\n");
    }
    fn push(&mut self, text: &str) {
        self.0.push_str(text);
        if self.0.len() > LOG_TAIL * 2 {
            let mut cut = self.0.len() - LOG_TAIL;
            while !self.0.is_char_boundary(cut) {
                cut += 1;
            }
            self.0.drain(..cut);
        }
    }
    fn tail(&self) -> String {
        if self.0.len() <= LOG_TAIL {
            return self.0.clone();
        }
        let mut cut = self.0.len() - LOG_TAIL;
        while !self.0.is_char_boundary(cut) {
            cut += 1;
        }
        format!("…{}", &self.0[cut..])
    }
}

async fn run(
    state: &AppState,
    session: SessionId,
    setup: &WorktreeSetup,
    home: &Path,
    worktree: &Path,
    branch: &str,
) {
    let id = uuid::Uuid::new_v4().to_string();
    let event = |status: &str, log: Option<String>| -> Value {
        let mut event = json!({
            "type": "setup",
            "id": id,
            "status": status,
            "copy": setup.copy,
            "command": setup.command,
        });
        if let Some(log) = log {
            event["log"] = Value::String(log);
        }
        event
    };
    let _ = crate::conversations::persist(state, session, event("running", None)).await;
    let mut log = Log::default();
    let mut ok = true;
    for entry in &setup.copy {
        let (from, to) = (home.join(entry), worktree.join(entry));
        if tokio::fs::symlink_metadata(&from).await.is_err() {
            log.line(format!("Skipped {entry}: not in the workspace directory."));
            continue;
        }
        if tokio::fs::symlink_metadata(&to).await.is_ok() {
            log.line(format!("Kept {entry}: the worktree already has it."));
            continue;
        }
        match copy(&from, &to).await {
            Ok(()) => log.line(format!("Copied {entry}.")),
            Err(error) => {
                ok = false;
                log.line(format!("Could not copy {entry}: {error}"));
            }
        }
    }
    if let Some(command) = &setup.command {
        log.line(format!("$ {command}"));
        let shared = Arc::new(Mutex::new(log));
        let outcome = run_command(command, home, worktree, branch, shared.clone()).await;
        log = Arc::try_unwrap(shared)
            .map(|log| log.into_inner().expect("setup log"))
            .unwrap_or_default();
        match outcome {
            Ok(true) => {}
            Ok(false) => ok = false,
            Err(error) => {
                ok = false;
                log.line(error);
            }
        }
    }
    let status = if ok { "completed" } else { "failed" };
    let _ = crate::conversations::persist(state, session, event(status, Some(log.tail()))).await;
}

/// Copy a file or a directory tree, as a clone where the file system can.
async fn copy(from: &Path, to: &Path) -> std::result::Result<(), String> {
    if let Some(parent) = to.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| error.to_string())?;
    }
    #[cfg(unix)]
    {
        // A clone first: APFS (`-c`) and reflink file systems copy nothing
        // until a file changes. Anywhere that refuses, a plain copy.
        let clone: &[&str] = if cfg!(target_os = "macos") {
            &["-cRP"]
        } else {
            &["-RP", "--reflink=auto"]
        };
        if cp(clone, from, to).await.is_ok() {
            return Ok(());
        }
        remove(to).await;
        cp(&["-RP"], from, to).await
    }
    #[cfg(not(unix))]
    {
        let (from, to) = (from.to_owned(), to.to_owned());
        tokio::task::spawn_blocking(move || copy_tree(&from, &to))
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())
    }
}

#[cfg(unix)]
async fn cp(flags: &[&str], from: &Path, to: &Path) -> std::result::Result<(), String> {
    let mut command = Command::new("cp");
    command
        .args(flags)
        .arg("--")
        .arg(from)
        .arg(to)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(COPY_TIMEOUT, command.output())
        .await
        .map_err(|_| "the copy took longer than ten minutes".to_owned())?
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

#[cfg(unix)]
async fn remove(path: &Path) {
    match tokio::fs::symlink_metadata(path).await {
        Ok(meta) if meta.is_dir() => {
            let _ = tokio::fs::remove_dir_all(path).await;
        }
        Ok(_) => {
            let _ = tokio::fs::remove_file(path).await;
        }
        Err(_) => {}
    }
}

#[cfg(not(unix))]
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::metadata(from)?.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

/// Run the setup command in the worktree through the person's shell, as a
/// terminal there would: a login, interactive shell on Unix, so the `PATH`
/// their profile sets (nvm, pnpm) is the one it sees. Nothing reads its input.
async fn run_command(
    command: &str,
    home: &Path,
    worktree: &Path,
    branch: &str,
    log: Arc<Mutex<Log>>,
) -> std::result::Result<bool, String> {
    let shell = std::env::var("TENDLY_SHELL")
        .ok()
        .filter(|shell| !shell.is_empty())
        .unwrap_or_else(tendly_runtime::default_shell);
    let mut process = Command::new(&shell);
    if cfg!(windows) {
        process.args(["-Command", command]);
    } else {
        let name = Path::new(&shell)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if matches!(name.as_str(), "zsh" | "bash" | "fish") {
            process.args(["-l", "-i", "-c", command]);
        } else {
            process.args(["-c", command]);
        }
    }
    for key in crate::bridge::tendly_secrets()
        .chain(crate::bridge::host_client_session())
        .chain(
            std::env::vars()
                .map(|(key, _)| key)
                .filter(|key| key.starts_with("GIT_")),
        )
    {
        process.env_remove(key);
    }
    process
        .current_dir(worktree)
        .env("TENDLY_WORKSPACE_ROOT", home)
        .env("TENDLY_WORKTREE", worktree)
        .env("TENDLY_BRANCH", branch)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = process
        .spawn()
        .map_err(|error| format!("Could not start {shell}: {error}"))?;
    let read = |stream: Option<Box<dyn tokio::io::AsyncRead + Unpin + Send>>| {
        let log = log.clone();
        async move {
            let Some(mut stream) = stream else {
                return;
            };
            let mut buffer = [0u8; 8192];
            while let Ok(read) = stream.read(&mut buffer).await {
                if read == 0 {
                    break;
                }
                log.lock()
                    .expect("setup log")
                    .push(&String::from_utf8_lossy(&buffer[..read]));
            }
        }
    };
    let stdout = child.stdout.take().map(|s| Box::new(s) as Box<_>);
    let stderr = child.stderr.take().map(|s| Box::new(s) as Box<_>);
    let finished = tokio::time::timeout(COMMAND_TIMEOUT, async {
        tokio::join!(read(stdout), read(stderr));
        child.wait().await
    })
    .await;
    match finished {
        Ok(Ok(status)) if status.success() => Ok(true),
        Ok(Ok(status)) => {
            log.lock()
                .expect("setup log")
                .line(format!("The setup command exited with {status}."));
            Ok(false)
        }
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => {
            let _ = child.kill().await;
            Err("The setup command took longer than twenty minutes and was stopped.".into())
        }
    }
}
