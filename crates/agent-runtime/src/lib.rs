//! Process and PTY runtime used by Tendly.
//!
//! The runtime deliberately does not understand Claude Code or Codex output. It
//! owns a child process and exposes its terminal as a reconnectable event stream.
//! All blocking operations (PTY reads, writes, and wait/reap) run on dedicated
//! OS threads; the async API only enqueues bounded commands.

use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread,
    time::Duration,
};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio::sync::{Notify, broadcast, oneshot};

#[cfg(unix)]
pub mod host;
pub mod process;

const DEFAULT_COLS: u16 = 120;
const DEFAULT_ROWS: u16 = 40;
const MAX_COLS: u16 = 1_000;
const MAX_ROWS: u16 = 500;
const MAX_INPUT_BYTES: usize = 64 * 1024;
const INPUT_QUEUE_CAPACITY: usize = 64;
const REPLAY_LIMIT_BYTES: usize = 1024 * 1024;

pub type RuntimeError = Box<dyn std::error::Error + Send + Sync>;
pub type RuntimeResult<T> = Result<T, RuntimeError>;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct SpawnSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub env_remove: Vec<String>,
}

impl std::fmt::Debug for SpawnSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpawnSpec")
            .field("program", &self.program)
            .field("args", &self.args)
            .field("cwd", &self.cwd)
            .field("env_keys", &self.env.keys().collect::<Vec<_>>())
            .field("env_remove", &self.env_remove)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeEvent {
    Output { seq: u64, data: Vec<u8> },
    Exited { seq: u64, code: Option<i32> },
}

struct Replay {
    events: VecDeque<RuntimeEvent>,
    output_bytes: usize,
}

struct RuntimeInner {
    running: AtomicBool,
    next_seq: AtomicU64,
    // Sequence allocation and replay insertion are one critical section. This
    // prevents a waiter thread from publishing Exited ahead of an output chunk
    // whose reader thread reserved a lower sequence number first.
    emit_lock: Mutex<()>,
    replay: Mutex<Replay>,
    events: broadcast::Sender<RuntimeEvent>,
    commands: SyncSender<Control>,
    killer: Mutex<Option<Box<dyn portable_pty::ChildKiller + Send + Sync>>>,
    process_id: Option<u32>,
    exit_code: Mutex<Option<Option<i32>>>,
    exit_notify: Notify,
    reader_done: AtomicBool,
    /// Held by a `pty-host` process rather than this one: the server going
    /// away leaves it running, and a later server attaches to it again.
    hosted: bool,
}

impl RuntimeInner {
    fn new(
        commands: SyncSender<Control>,
        killer: Option<Box<dyn portable_pty::ChildKiller + Send + Sync>>,
        process_id: Option<u32>,
        hosted: bool,
    ) -> Arc<Self> {
        let (events, _) = broadcast::channel(512);
        Arc::new(Self {
            running: AtomicBool::new(true),
            next_seq: AtomicU64::new(0),
            emit_lock: Mutex::new(()),
            replay: Mutex::new(Replay {
                events: VecDeque::new(),
                output_bytes: 0,
            }),
            events,
            commands,
            killer: Mutex::new(killer),
            process_id,
            exit_code: Mutex::new(None),
            exit_notify: Notify::new(),
            reader_done: AtomicBool::new(false),
            hosted,
        })
    }
}

#[derive(Debug)]
enum Control {
    Input(Vec<u8>),
    Resize {
        size: PtySize,
        result: oneshot::Sender<std::result::Result<(), String>>,
    },
    Shutdown,
}

pub struct RuntimeSession {
    id: String,
    /// Names this process rather than the session. A view resuming from a
    /// sequence number has to be talking to the process that numbered it: a
    /// restarted session counts from zero again.
    instance: String,
    inner: Arc<RuntimeInner>,
}

impl std::fmt::Debug for RuntimeSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeSession")
            .field("id", &self.id)
            .field("running", &self.running())
            .finish()
    }
}

impl RuntimeSession {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn instance(&self) -> &str {
        &self.instance
    }

    /// Subscribe to future events. Callers should take a snapshot as well when
    /// reconnecting; each event has a monotonic sequence number.
    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.inner.events.subscribe()
    }

    pub fn snapshot(&self) -> Vec<RuntimeEvent> {
        let _emit_guard = self
            .inner
            .emit_lock
            .lock()
            .expect("runtime emit mutex poisoned");
        self.inner
            .replay
            .lock()
            .expect("runtime replay mutex poisoned")
            .events
            .iter()
            .cloned()
            .collect()
    }

    /// Queue bytes for the child. The queue is bounded and this method never
    /// waits for a blocking writer thread.
    pub async fn input(&self, input: Vec<u8>) -> RuntimeResult<()> {
        if input.is_empty() {
            return Ok(());
        }
        if input.len() > MAX_INPUT_BYTES {
            return Err("PTY input exceeds 64 KiB limit".into());
        }
        if !self.running() {
            return Err("session is not running".into());
        }
        self.inner
            .commands
            .try_send(Control::Input(input))
            .map_err(control_queue_error)
    }

    pub async fn resize(&self, cols: u16, rows: u16) -> RuntimeResult<()> {
        validate_size(cols, rows)?;
        if !self.running() {
            return Err("session is not running".into());
        }
        let (tx, rx) = oneshot::channel();
        self.inner
            .commands
            .try_send(Control::Resize {
                size: PtySize {
                    rows,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                },
                result: tx,
            })
            .map_err(control_queue_error)?;
        tokio::time::timeout(Duration::from_secs(2), rx)
            .await
            .map_err(|_| -> RuntimeError { "PTY resize timed out".into() })?
            .map_err(|_| -> RuntimeError { "PTY resize worker stopped".into() })?
            .map_err(|error| error.into())
    }

    pub fn running(&self) -> bool {
        self.inner.running.load(Ordering::Acquire)
    }

    /// Whether a `pty-host` process holds this session's terminal, so that it
    /// outlives this server.
    pub fn hosted(&self) -> bool {
        self.inner.hosted
    }

    /// Wait for child reaping without blocking a Tokio worker thread.
    pub async fn wait(&self) -> Option<i32> {
        loop {
            // Register the notification before checking the state so an exit
            // racing this check cannot be missed.
            let notified = self.inner.exit_notify.notified();
            if let Some(code) = *self
                .inner
                .exit_code
                .lock()
                .expect("runtime exit mutex poisoned")
            {
                return code;
            }
            notified.await;
        }
    }

    /// Request termination of this session's process group and wait briefly for
    /// reaping. The process group is created by portable-pty (`setsid` on Unix).
    pub async fn stop(&self) -> RuntimeResult<()> {
        if !self.running() {
            return Ok(());
        }
        let _ = self.inner.commands.try_send(Control::Shutdown);
        signal_group(self.inner.process_id, process::terminate_group);
        if let Some(killer) = self
            .inner
            .killer
            .lock()
            .expect("runtime killer mutex poisoned")
            .as_mut()
        {
            let _ = killer.kill();
        }
        // A hosted session's killer is in its host; this is what it would send.
        #[cfg(unix)]
        if self.inner.hosted {
            signal_group(self.inner.process_id, process::hangup_group);
        }
        if tokio::time::timeout(Duration::from_secs(3), self.wait())
            .await
            .is_err()
        {
            signal_group(self.inner.process_id, process::kill_group);
            let _ = tokio::time::timeout(Duration::from_secs(1), self.wait()).await;
        }
        Ok(())
    }

    fn emit_output(&self, data: Vec<u8>) {
        let _guard = self
            .inner
            .emit_lock
            .lock()
            .expect("runtime emit mutex poisoned");
        let seq = self.inner.next_seq.fetch_add(1, Ordering::AcqRel) + 1;
        self.publish(RuntimeEvent::Output { seq, data });
    }

    fn emit_exited(&self, code: Option<i32>) {
        let _guard = self
            .inner
            .emit_lock
            .lock()
            .expect("runtime emit mutex poisoned");
        let seq = self.inner.next_seq.fetch_add(1, Ordering::AcqRel) + 1;
        self.publish(RuntimeEvent::Exited { seq, code });
    }

    #[cfg(unix)]
    /// Publish an event numbered elsewhere: a hosted session's events keep
    /// the host's sequence numbers, so a view can resume across a restart.
    fn emit_numbered(&self, event: RuntimeEvent) {
        let _guard = self
            .inner
            .emit_lock
            .lock()
            .expect("runtime emit mutex poisoned");
        let (RuntimeEvent::Output { seq, .. } | RuntimeEvent::Exited { seq, .. }) = &event;
        self.inner.next_seq.fetch_max(*seq, Ordering::AcqRel);
        self.publish(event);
    }

    /// Record the exit after its event is published, and wake every waiter.
    fn finish(&self, code: Option<i32>) {
        *self
            .inner
            .exit_code
            .lock()
            .expect("runtime exit mutex poisoned") = Some(code);
        self.inner.exit_notify.notify_waiters();
        // Called on a dedicated thread (never an async worker), so it is safe
        // to wait for the bounded writer queue to drain and guarantee the
        // writer thread is asked to terminate even when input saturated it.
        let _ = self.inner.commands.send(Control::Shutdown);
    }

    fn publish(&self, event: RuntimeEvent) {
        let _ = self.inner.events.send(event.clone());
        let mut replay = self
            .inner
            .replay
            .lock()
            .expect("runtime replay mutex poisoned");
        if let RuntimeEvent::Output { data, .. } = &event {
            replay.output_bytes = replay.output_bytes.saturating_add(data.len());
        }
        replay.events.push_back(event);
        while replay.output_bytes > REPLAY_LIMIT_BYTES {
            let Some(old) = replay.events.pop_front() else {
                break;
            };
            if let RuntimeEvent::Output { data, .. } = old {
                replay.output_bytes = replay.output_bytes.saturating_sub(data.len());
            }
        }
    }
}

/// Where hosted terminals live: the program that runs `pty-host`, and the
/// private directory their sockets are made in.
#[derive(Clone, Debug)]
pub struct HostConfig {
    pub program: PathBuf,
    pub dir: PathBuf,
}

#[derive(Clone, Default)]
pub struct RuntimeManager {
    sessions: Arc<RwLock<HashMap<String, Arc<RuntimeSession>>>>,
    host: Option<Arc<HostConfig>>,
}

impl RuntimeManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start sessions in `pty-host` processes, so they outlive this server.
    /// Only Unix has them; elsewhere this is the same as `new`.
    pub fn with_host(host: HostConfig) -> Self {
        Self {
            host: cfg!(unix).then(|| Arc::new(host)),
            ..Self::default()
        }
    }

    pub fn host(&self) -> Option<&HostConfig> {
        self.host.as_deref()
    }

    pub async fn start(&self, id: String, spec: SpawnSpec) -> RuntimeResult<Arc<RuntimeSession>> {
        validate_spawn(&id, &spec)?;
        let host = self.host.clone();
        self.register(id, move |id| {
            #[cfg(unix)]
            if let Some(host) = host {
                match host::spawn(&host, id, &spec) {
                    Ok(session) => return Ok(session),
                    // A terminal that dies with the server beats none at all.
                    Err(error) => tracing::warn!(%error, "pty host failed; running in process"),
                }
            }
            #[cfg(not(unix))]
            let _ = host;
            spawn_runtime(id.to_owned(), spec)
        })
        .await
    }

    /// The sessions hosts are still holding from an earlier server, by id.
    pub fn hosted(&self) -> Vec<(String, PathBuf)> {
        #[cfg(unix)]
        if let Some(host) = &self.host {
            return host::hosted(host);
        }
        Vec::new()
    }

    /// Take over a session a `pty-host` is still holding from an earlier server.
    pub async fn attach(&self, id: String, socket: PathBuf) -> RuntimeResult<Arc<RuntimeSession>> {
        #[cfg(unix)]
        return self.register(id, move |id| host::attach(id, &socket)).await;
        #[cfg(not(unix))]
        {
            let _ = (id, socket);
            Err("terminal hosts need Unix".into())
        }
    }

    /// End a host no session is waiting for, or clear the socket of one that
    /// is already gone.
    pub async fn discard(&self, id: String, socket: PathBuf) {
        #[cfg(unix)]
        host::discard(id, socket).await;
        #[cfg(not(unix))]
        let _ = (id, socket);
    }

    async fn register(
        &self,
        id: String,
        spawn: impl FnOnce(&str) -> RuntimeResult<Arc<RuntimeSession>> + Send + 'static,
    ) -> RuntimeResult<Arc<RuntimeSession>> {
        let registry = self.sessions.clone();
        tokio::task::spawn_blocking(move || {
            let mut sessions=registry.write().expect("runtime sessions lock poisoned");
            if sessions.get(&id).is_some_and(|existing|existing.running()) {
                return Err(format!("session {id:?} is already running").into());
            }
            if sessions.len()>=128 && !sessions.contains_key(&id) {
                return Err("Runtime registry is full (128 sessions). Restart the server to release stopped replay buffers.".into());
            }
            let session=spawn(&id)?;
            sessions.insert(id,session.clone());
            Ok(session)
        }).await.map_err(|error| -> RuntimeError { Box::new(error) })?
    }

    pub fn get(&self, id: &str) -> Option<Arc<RuntimeSession>> {
        self.sessions
            .read()
            .expect("runtime sessions lock poisoned")
            .get(id)
            .cloned()
    }

    /// Process ids of the sessions still running, for resource reporting.
    pub fn process_ids(&self) -> Vec<(String, u32)> {
        self.sessions
            .read()
            .expect("runtime sessions lock poisoned")
            .iter()
            .filter(|(_, session)| session.running())
            .filter_map(|(id, session)| session.inner.process_id.map(|pid| (id.clone(), pid)))
            .collect()
    }

    pub async fn stop(&self, id: &str) -> RuntimeResult<()> {
        let session = self
            .get(id)
            .ok_or_else(|| format!("session {id:?} not found"))?;
        session.stop().await
    }

    /// Stop the sessions this process holds. Hosted ones are left to their
    /// hosts, for the next server to attach to.
    pub async fn shutdown(&self) {
        let sessions: Vec<_> = self
            .sessions
            .read()
            .expect("runtime sessions lock poisoned")
            .values()
            .filter(|session| !session.hosted())
            .cloned()
            .collect();
        for session in sessions {
            let _ = session.stop().await;
        }
    }
}

fn spawn_runtime(id: String, spec: SpawnSpec) -> RuntimeResult<Arc<RuntimeSession>> {
    spawn_local(id, spec, false)
}

/// Own the child and its PTY in this process. `hosted` marks the session a
/// `pty-host` keeps for a server, which never matters inside the host itself.
fn spawn_local(id: String, spec: SpawnSpec, hosted: bool) -> RuntimeResult<Arc<RuntimeSession>> {
    let pty = native_pty_system();
    let pair = pty.openpty(PtySize {
        rows: DEFAULT_ROWS,
        cols: DEFAULT_COLS,
        pixel_width: 0,
        pixel_height: 0,
    })?;
    let (program, args) = process::launcher(&spec.program, &spec.args)?;
    let mut command = CommandBuilder::new(&program);
    command.args(&args);
    command.cwd(&spec.cwd);
    command.env("TERM", "xterm-256color");
    for (key, value) in &spec.env {
        command.env(key, value);
    }
    for key in &spec.env_remove {
        command.env_remove(key);
    }
    let child = pair.slave.spawn_command(command)?;
    let process_id = child.process_id();
    drop(pair.slave);
    let reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;
    let master: Arc<Mutex<Box<dyn MasterPty + Send>>> = Arc::new(Mutex::new(pair.master));
    let (commands, command_rx) = mpsc::sync_channel(INPUT_QUEUE_CAPACITY);
    let inner = RuntimeInner::new(commands, Some(child.clone_killer()), process_id, hosted);
    let started = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let session = Arc::new(RuntimeSession {
        id,
        instance: format!("{}-{started:x}", process_id.unwrap_or_default()),
        inner: Arc::clone(&inner),
    });

    let reader_session = Arc::clone(&session);
    thread::Builder::new()
        .name("tendly-pty-reader".into())
        .spawn(move || runtime_read_output(reader, reader_session))?;

    let writer_master = Arc::clone(&master);
    thread::Builder::new()
        .name("tendly-pty-writer".into())
        .spawn(move || runtime_write_loop(writer, writer_master, command_rx))?;

    let wait_session = Arc::clone(&session);
    thread::Builder::new()
        .name("tendly-pty-waiter".into())
        .spawn(move || runtime_wait_loop(child, wait_session))?;

    Ok(session)
}

fn runtime_read_output(mut reader: Box<dyn Read + Send>, session: Arc<RuntimeSession>) {
    let mut buffer = [0_u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(size) => {
                session.emit_output(buffer[..size].to_vec());
            }
            Err(error) => {
                tracing::debug!(%error, "PTY reader stopped");
                break;
            }
        }
    }
    session.inner.reader_done.store(true, Ordering::Release);
}

fn runtime_write_loop(
    mut writer: Box<dyn Write + Send>,
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    command_rx: Receiver<Control>,
) {
    while let Ok(command) = command_rx.recv() {
        match command {
            Control::Input(input) => {
                if writer
                    .write_all(&input)
                    .and_then(|_| writer.flush())
                    .is_err()
                {
                    break;
                }
            }
            Control::Resize { size, result } => {
                let resize_result = master
                    .lock()
                    .map_err(|_| "PTY master mutex poisoned".to_owned())
                    .and_then(|master| master.resize(size).map_err(|error| error.to_string()));
                let _ = result.send(resize_result);
            }
            Control::Shutdown => break,
        }
    }
}

fn runtime_wait_loop(mut child: Box<dyn Child + Send + Sync>, session: Arc<RuntimeSession>) {
    let status = child.wait();
    // A Unix PTY reaches end of file once the child's side closes. ConPTY never
    // does while the pseudoconsole is open, and the writer thread holds it, so
    // Exited waited out the whole grace period below and anything ConPTY still
    // flushed arrived after it. Stopping the writer closes the pseudoconsole,
    // which flushes the rest to the reader and then ends its stream.
    #[cfg(windows)]
    let _ = session.inner.commands.try_send(Control::Shutdown);
    for _ in 0..100 {
        if session.inner.reader_done.load(Ordering::Acquire) {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let code = status.ok().and_then(|status| {
        if status.signal().is_some() {
            None
        } else {
            i32::try_from(status.exit_code()).ok()
        }
    });
    session.inner.running.store(false, Ordering::Release);
    session.emit_exited(code);
    session.finish(code);
}

fn control_queue_error(error: TrySendError<Control>) -> RuntimeError {
    match error {
        TrySendError::Full(_) => "PTY command queue is full".into(),
        TrySendError::Disconnected(_) => "PTY command queue is closed".into(),
    }
}

fn validate_size(cols: u16, rows: u16) -> RuntimeResult<()> {
    if cols == 0 || rows == 0 {
        return Err("PTY dimensions must be non-zero".into());
    }
    if cols > MAX_COLS || rows > MAX_ROWS {
        return Err(format!("PTY dimensions exceed {MAX_COLS}x{MAX_ROWS}").into());
    }
    Ok(())
}

fn validate_spawn(id: &str, spec: &SpawnSpec) -> RuntimeResult<()> {
    if id.is_empty() || id.len() > 128 {
        return Err("session id must be 1-128 bytes".into());
    }
    if spec.program.is_empty() || spec.program.len() > 4096 {
        return Err("program must be 1-4096 bytes".into());
    }
    if spec.args.len() > 256 {
        return Err("too many process arguments".into());
    }
    if !spec.cwd.is_dir() {
        return Err(format!(
            "working directory is not a directory: {}",
            spec.cwd.display()
        )
        .into());
    }
    Ok(())
}

fn signal_group(process_id: Option<u32>, signal: fn(u32) -> bool) {
    // portable-pty calls setsid in the child on Unix, making this PID the
    // process group leader; on Windows the tree below it is what is signalled.
    // Either way only this session's processes are touched.
    if let Some(pid) = process_id {
        signal(pid);
    }
}

/// The interactive shell a terminal session opens when none is configured.
/// On Windows `SHELL` is ignored: under Git Bash it holds an MSYS path such as
/// `/usr/bin/bash` that a native process cannot start.
pub fn default_shell() -> String {
    #[cfg(windows)]
    {
        if process::find_program("pwsh").is_some() {
            return "pwsh.exe".to_owned();
        }
        "powershell.exe".to_owned()
    }
    #[cfg(not(windows))]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_owned())
    }
}

// Backwards-compatible ephemeral API. New code should use RuntimeManager.
#[derive(Debug)]
pub enum PtyEvent {
    Output(Vec<u8>),
    Exited,
}

pub struct PtySession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    process_id: Option<u32>,
}

impl PtySession {
    pub fn spawn_shell(
        cwd: impl AsRef<Path>,
        cols: u16,
        rows: u16,
    ) -> RuntimeResult<(Self, tokio::sync::mpsc::UnboundedReceiver<PtyEvent>)> {
        let shell = default_shell();
        Self::spawn_program(cwd, shell, &[], cols, rows)
    }

    pub fn spawn_program(
        cwd: impl AsRef<Path>,
        program: impl AsRef<str>,
        args: &[String],
        cols: u16,
        rows: u16,
    ) -> RuntimeResult<(Self, tokio::sync::mpsc::UnboundedReceiver<PtyEvent>)> {
        validate_size(cols, rows)?;
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let (program, args) = process::launcher(program.as_ref(), args)?;
        let mut command = CommandBuilder::new(&program);
        command.args(&args);
        command.cwd(cwd.as_ref());
        command.env("TERM", "xterm-256color");
        let child = pair.slave.spawn_command(command)?;
        let process_id = child.process_id();
        drop(pair.slave);
        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        thread::Builder::new()
            .name("tendly-pty-reader-legacy".into())
            .spawn(move || {
                let mut reader = reader;
                let mut buffer = [0_u8; 8192];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(size) => {
                            if event_tx
                                .send(PtyEvent::Output(buffer[..size].to_vec()))
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(error) => {
                            tracing::debug!(%error, "PTY reader stopped");
                            break;
                        }
                    }
                }
                let _ = event_tx.send(PtyEvent::Exited);
            })?;
        Ok((
            Self {
                master: pair.master,
                writer,
                child,
                process_id,
            },
            event_rx,
        ))
    }

    pub fn write_input(&mut self, input: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(input)?;
        self.writer.flush()
    }

    pub fn resize(&self, cols: u16, rows: u16) -> RuntimeResult<()> {
        validate_size(cols, rows)?;
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }

    pub fn kill(&mut self) {
        signal_group(self.process_id, process::terminate_group);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell_spec(args: &[&str]) -> SpawnSpec {
        SpawnSpec {
            program: if cfg!(windows) {
                "cmd.exe"
            } else {
                "/bin/bash"
            }
            .to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            cwd: std::env::temp_dir(),
            env: BTreeMap::new(),
            env_remove: vec!["BASH_ENV".to_owned()],
        }
    }

    fn interactive_shell() -> SpawnSpec {
        if cfg!(windows) {
            shell_spec(&["/Q"])
        } else {
            shell_spec(&["--noprofile", "--norc", "-i"])
        }
    }

    /// ConPTY opens by asking the terminal where its cursor is (`ESC[6n`) and
    /// shows nothing until it hears back. In the app xterm.js answers; here the
    /// test is the terminal.
    async fn answer_cursor_query(session: &RuntimeSession) {
        if cfg!(windows) {
            session
                .input(b"\x1b[1;1R".to_vec())
                .await
                .expect("cursor report should enqueue");
        }
    }

    /// Input that prints the sentinel and exits. The typed text never contains
    /// the sentinel itself, so the terminal echoing the input cannot pass.
    const SENTINEL_INPUT: &[u8] = if cfg!(windows) {
        b"echo tendly_runtime_^sentinel\r\nexit\r\n"
    } else {
        b"printf '\\ntendly_runtime_sentinel\\n'\nexit\n"
    };

    #[tokio::test]
    async fn interactive_shell_emits_output_and_exits() {
        let manager = RuntimeManager::new();
        let session = manager
            .start("runtime-test-shell".to_owned(), interactive_shell())
            .await
            .expect("shell should start");
        let mut events = session.subscribe();
        answer_cursor_query(&session).await;
        session
            .input(SENTINEL_INPUT.to_vec())
            .await
            .expect("input should enqueue");

        let mut output = Vec::new();
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            while let RuntimeEvent::Output { data, .. } = events.recv().await.expect("event stream")
            {
                output.extend(data);
                if output
                    .windows(b"tendly_runtime_sentinel".len())
                    .any(|window| window == b"tendly_runtime_sentinel")
                {
                    break;
                }
            }
        })
        .await;
        assert!(result.is_ok(), "shell output timeout");
        assert!(
            output
                .windows(b"tendly_runtime_sentinel".len())
                .any(|window| window == b"tendly_runtime_sentinel")
        );
        assert!(
            tokio::time::timeout(Duration::from_secs(2), session.wait())
                .await
                .is_ok()
        );
        manager.shutdown().await;
    }

    #[tokio::test]
    async fn exited_output_is_available_to_reconnect_snapshot() {
        let manager = RuntimeManager::new();
        let session = manager
            .start(
                "runtime-test-replay".to_owned(),
                if cfg!(windows) {
                    shell_spec(&["/C", "echo replay_sentinel"])
                } else {
                    shell_spec(&["--noprofile", "--norc", "-c", "printf replay_sentinel"])
                },
            )
            .await
            .expect("shell should start");
        answer_cursor_query(&session).await;
        let _ = session.wait().await;
        let snapshot = session.snapshot();
        assert!(snapshot.iter().any(|event| matches!(
            event,
            RuntimeEvent::Output { data, .. }
                if data.windows(b"replay_sentinel".len())
                    .any(|window| window == b"replay_sentinel")
        )));
        assert!(matches!(snapshot.last(), Some(RuntimeEvent::Exited { .. })));
        let seqs: Vec<_> = snapshot
            .iter()
            .map(|event| match event {
                RuntimeEvent::Output { seq, .. } | RuntimeEvent::Exited { seq, .. } => *seq,
            })
            .collect();
        assert!(seqs.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[tokio::test]
    async fn resize_and_stop_work_for_running_shell() {
        let manager = RuntimeManager::new();
        let session = manager
            .start("runtime-test-resize".to_owned(), interactive_shell())
            .await
            .expect("shell should start");
        session.resize(80, 24).await.expect("resize should work");
        session.stop().await.expect("stop should work");
        assert!(!session.running());
    }

    #[tokio::test]
    async fn invalid_sizes_and_duplicate_ids_are_rejected() {
        let manager = RuntimeManager::new();
        let session = manager
            .start("duplicate".to_owned(), interactive_shell())
            .await
            .expect("session should start");
        let duplicate = manager
            .start("duplicate".to_owned(), interactive_shell())
            .await;
        assert!(duplicate.is_err());
        assert!(session.resize(0, 20).await.is_err());
        assert!(session.resize(2000, 20).await.is_err());
        manager.shutdown().await;
    }
}
