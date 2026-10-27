//! Terminals that outlive the server.
//!
//! A hosted session's child and PTY belong to a `pty-host` process of their
//! own, detached with `setsid`, which keeps draining output into its replay
//! while no server is attached. The server holds a mirror: a `RuntimeSession`
//! fed over a Unix socket, numbered with the host's own sequence numbers and
//! named after the host's process, so a view that was watching before a
//! restart resumes where it was rather than being redrawn.
//!
//! The wire is frames of a kind byte, a big-endian `u32` length and the
//! payload. The host opens with `HELLO`, replays what it holds and then
//! streams; the server sends input and resizes. Stopping needs no message: the
//! server signals the child's process group directly, and the host reports the
//! exit like any other. A host whose child has exited stays until a server has
//! seen that exit, or for `LINGER` if none attaches, and then removes its
//! socket and ends.

use std::{
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::{fs::PermissionsExt, net::UnixStream, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

use serde::Deserialize;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{Notify, broadcast::error::RecvError},
};

use super::{
    Control, HostConfig, INPUT_QUEUE_CAPACITY, RuntimeEvent, RuntimeInner, RuntimeResult,
    RuntimeSession, SpawnSpec, process,
};

/// Bumped whenever a frame changes meaning. A server meeting a host of another
/// version cannot drive it, so it ends that host's terminal instead.
const PROTOCOL: u32 = 1;
const HELLO: u8 = 1;
const OUTPUT: u8 = 2;
const EXITED: u8 = 3;
const INPUT: u8 = 10;
const RESIZE: u8 = 11;
const MAX_FRAME: usize = 1024 * 1024;
const LINGER: Duration = Duration::from_secs(30);
/// `sun_path` holds 108 bytes on Linux and 104 on macOS, with a terminator.
const MAX_SOCKET_PATH: usize = 100;

#[derive(Deserialize)]
struct Hello {
    version: u32,
    pid: Option<u32>,
    instance: String,
}

fn frame(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(5 + payload.len());
    frame.push(kind);
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

fn header(header: [u8; 5]) -> io::Result<(u8, usize)> {
    let len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    Ok((header[0], len))
}

fn read_frame(reader: &mut impl Read) -> io::Result<(u8, Vec<u8>)> {
    let mut head = [0; 5];
    reader.read_exact(&mut head)?;
    let (kind, len) = header(head)?;
    let mut payload = vec![0; len];
    reader.read_exact(&mut payload)?;
    Ok((kind, payload))
}

async fn read_frame_async(reader: &mut (impl AsyncReadExt + Unpin)) -> io::Result<(u8, Vec<u8>)> {
    let mut head = [0; 5];
    reader.read_exact(&mut head).await?;
    let (kind, len) = header(head)?;
    let mut payload = vec![0; len];
    reader.read_exact(&mut payload).await?;
    Ok((kind, payload))
}

fn seq_of(event: &RuntimeEvent) -> u64 {
    match event {
        RuntimeEvent::Output { seq, .. } | RuntimeEvent::Exited { seq, .. } => *seq,
    }
}

fn encode(event: &RuntimeEvent) -> Vec<u8> {
    match event {
        RuntimeEvent::Output { seq, data } => {
            let mut payload = seq.to_be_bytes().to_vec();
            payload.extend_from_slice(data);
            frame(OUTPUT, &payload)
        }
        RuntimeEvent::Exited { seq, code } => {
            let mut payload = seq.to_be_bytes().to_vec();
            payload.push(u8::from(code.is_some()));
            payload.extend_from_slice(&code.unwrap_or_default().to_be_bytes());
            frame(EXITED, &payload)
        }
    }
}

fn decode(kind: u8, payload: &[u8]) -> Option<RuntimeEvent> {
    let seq = u64::from_be_bytes(payload.get(..8)?.try_into().ok()?);
    match kind {
        OUTPUT => Some(RuntimeEvent::Output {
            seq,
            data: payload[8..].to_vec(),
        }),
        EXITED => {
            let code = i32::from_be_bytes(payload.get(9..13)?.try_into().ok()?);
            Some(RuntimeEvent::Exited {
                seq,
                code: (*payload.get(8)? == 1).then_some(code),
            })
        }
        _ => None,
    }
}

/// The socket a session's host listens on.
pub fn socket_path(host: &HostConfig, id: &str) -> PathBuf {
    host.dir.join(format!("{id}.sock"))
}

pub(crate) fn hosted(host: &HostConfig) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(&host.dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter_map(|path| {
            let id = path
                .file_name()?
                .to_str()?
                .strip_suffix(".sock")?
                .to_owned();
            Some((id, path))
        })
        .collect()
}

pub(crate) async fn discard(id: String, socket: PathBuf) {
    let attached = tokio::task::spawn_blocking({
        let socket = socket.clone();
        move || attach(&id, &socket)
    })
    .await;
    match attached {
        Ok(Ok(session)) => {
            let _ = session.stop().await;
        }
        _ => {
            let _ = std::fs::remove_file(&socket);
        }
    }
}

/// Start `spec` in a new host and attach to it. Blocks until the host is
/// listening, which is only as long as spawning the child takes.
pub(crate) fn spawn(
    host: &HostConfig,
    id: &str,
    spec: &SpawnSpec,
) -> RuntimeResult<Arc<RuntimeSession>> {
    let socket = socket_path(host, id);
    if socket.as_os_str().len() > MAX_SOCKET_PATH {
        return Err(format!("socket path too long: {}", socket.display()).into());
    }
    // On stdin rather than the command line: the environment carries secrets.
    let spec = serde_json::to_vec(spec)?;
    let mut command = Command::new(&host.program);
    command
        .arg("pty-host")
        .arg(&socket)
        .arg(id)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe. A session of its own is what keeps
    // the host out of the server's process group, and so out of its stop.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        })
    };
    let mut child = command.spawn()?;
    let ready = (|| -> RuntimeResult<()> {
        child
            .stdin
            .take()
            .ok_or("pty host stdin")?
            .write_all(&spec)?;
        let mut line = String::new();
        BufReader::new(child.stdout.take().ok_or("pty host stdout")?).read_line(&mut line)?;
        if line.trim() != "ready" {
            return Err("pty host did not start".into());
        }
        Ok(())
    })();
    let session = ready.and_then(|()| attach(id, &socket));
    if session.is_err() {
        let _ = child.kill();
    }
    // Reap it whenever it ends, so a closed terminal leaves no zombie behind.
    thread::Builder::new()
        .name("tendly-pty-host-reaper".into())
        .spawn(move || {
            let _ = child.wait();
        })?;
    session
}

/// Mirror the session a host is holding.
pub(crate) fn attach(id: &str, socket: &Path) -> RuntimeResult<Arc<RuntimeSession>> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let (kind, payload) = read_frame(&mut stream)?;
    if kind != HELLO {
        return Err("pty host did not greet".into());
    }
    let hello: Hello = serde_json::from_slice(&payload)?;
    if hello.version != PROTOCOL {
        // Nothing here can drive it, so nothing could ever reach its terminal.
        if let Some(pid) = hello.pid {
            process::terminate_group(pid);
        }
        return Err(format!("pty host speaks protocol {}", hello.version).into());
    }
    stream.set_read_timeout(None)?;
    let (commands, command_rx) = mpsc::sync_channel(INPUT_QUEUE_CAPACITY);
    let session = Arc::new(RuntimeSession {
        id: id.to_owned(),
        instance: hello.instance,
        inner: RuntimeInner::new(commands, None, hello.pid, true),
    });
    let writer = stream.try_clone()?;
    thread::Builder::new()
        .name("tendly-pty-host-writer".into())
        .spawn(move || forward(writer, command_rx))?;
    let mirrored = Arc::clone(&session);
    thread::Builder::new()
        .name("tendly-pty-host-reader".into())
        .spawn(move || mirror(stream, mirrored))?;
    Ok(session)
}

fn forward(mut writer: UnixStream, commands: mpsc::Receiver<Control>) {
    while let Ok(command) = commands.recv() {
        let sent = match command {
            Control::Input(input) => writer.write_all(&frame(INPUT, &input)),
            Control::Resize { size, result } => {
                let mut payload = size.cols.to_be_bytes().to_vec();
                payload.extend_from_slice(&size.rows.to_be_bytes());
                let sent = writer.write_all(&frame(RESIZE, &payload));
                let _ = result.send(sent.as_ref().map(|_| ()).map_err(|e| e.to_string()));
                sent
            }
            Control::Shutdown => break,
        };
        if sent.is_err() {
            break;
        }
    }
}

fn mirror(mut stream: UnixStream, session: Arc<RuntimeSession>) {
    loop {
        let Ok((kind, payload)) = read_frame(&mut stream) else {
            // The host went without reporting an exit, and its child went
            // with it: the PTY closed under it.
            session
                .inner
                .running
                .store(false, std::sync::atomic::Ordering::Release);
            session.emit_exited(None);
            session.finish(None);
            return;
        };
        match decode(kind, &payload) {
            Some(event @ RuntimeEvent::Output { .. }) => session.emit_numbered(event),
            Some(event @ RuntimeEvent::Exited { code, .. }) => {
                session
                    .inner
                    .running
                    .store(false, std::sync::atomic::Ordering::Release);
                session.emit_numbered(event);
                session.finish(code);
                return;
            }
            None => {}
        }
    }
}

/// The `pty-host` process: read the spec from stdin, start it, say `ready` on
/// stdout and serve whichever server attaches, one at a time.
pub async fn serve(socket: PathBuf, id: String) -> RuntimeResult<()> {
    let mut input = Vec::new();
    io::stdin().read_to_end(&mut input)?;
    let spec: SpawnSpec = serde_json::from_slice(&input)?;
    hold(&socket, id, spec, || {
        let mut stdout = io::stdout().lock();
        stdout.write_all(b"ready\n")?;
        stdout.flush()
    })
    .await
}

/// Start `spec`, listen on `socket`, call `ready`, and serve until the child
/// has exited and a server has seen it (or none came for `LINGER`).
async fn hold(
    socket: &Path,
    id: String,
    spec: SpawnSpec,
    ready: impl FnOnce() -> io::Result<()>,
) -> RuntimeResult<()> {
    let session = super::spawn_local(id, spec, false)?;
    let _ = std::fs::remove_file(socket);
    let listener = tokio::net::UnixListener::bind(socket)?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
    ready()?;
    let delivered = Arc::new(Notify::new());
    let mut client: Option<tokio::task::JoinHandle<()>> = None;
    let mut deadline: Option<tokio::time::Instant> = None;
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    // A new server replaces whatever was attached before it.
                    if let Some(old) = client.take() {
                        old.abort();
                    }
                    client = Some(tokio::spawn(serve_client(
                        stream,
                        Arc::clone(&session),
                        Arc::clone(&delivered),
                    )));
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            },
            _ = session.wait(), if deadline.is_none() => {
                deadline = Some(tokio::time::Instant::now() + LINGER);
            }
            _ = delivered.notified() => break,
            _ = tokio::time::sleep_until(deadline.unwrap_or_else(tokio::time::Instant::now)),
                if deadline.is_some() => break,
        }
    }
    let _ = std::fs::remove_file(socket);
    Ok(())
}

async fn serve_client(
    stream: tokio::net::UnixStream,
    session: Arc<RuntimeSession>,
    delivered: Arc<Notify>,
) {
    let (mut reader, mut writer) = stream.into_split();
    let commands = tokio::spawn({
        let session = Arc::clone(&session);
        async move {
            while let Ok((kind, payload)) = read_frame_async(&mut reader).await {
                match kind {
                    INPUT => {
                        let _ = session.input(payload).await;
                    }
                    RESIZE if payload.len() == 4 => {
                        let cols = u16::from_be_bytes([payload[0], payload[1]]);
                        let rows = u16::from_be_bytes([payload[2], payload[3]]);
                        let _ = session.resize(cols, rows).await;
                    }
                    _ => {}
                }
            }
        }
    });
    if let Ok(true) = stream_events(&mut writer, &session).await {
        delivered.notify_one();
    }
    commands.abort();
}

/// Replay, then stream. True once the exit has been sent.
async fn stream_events(
    writer: &mut (impl AsyncWriteExt + Unpin),
    session: &RuntimeSession,
) -> io::Result<bool> {
    let hello = serde_json::json!({
        "version": PROTOCOL,
        "pid": session.inner.process_id,
        "instance": session.instance(),
    });
    writer
        .write_all(&frame(HELLO, hello.to_string().as_bytes()))
        .await?;
    // Subscribe before the snapshot so nothing falls between the two.
    let mut events = session.subscribe();
    let mut pending = session.snapshot();
    let mut last = 0;
    loop {
        for event in pending.drain(..) {
            if seq_of(&event) <= last {
                continue;
            }
            last = seq_of(&event);
            writer.write_all(&encode(&event)).await?;
            if matches!(event, RuntimeEvent::Exited { .. }) {
                return Ok(true);
            }
        }
        match events.recv().await {
            Ok(event) => pending.push(event),
            // Fell behind the live stream: catch up from the replay instead.
            Err(RecvError::Lagged(_)) => pending = session.snapshot(),
            Err(RecvError::Closed) => return Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    async fn output_until(session: &RuntimeSession, needle: &str) -> bool {
        let mut events = session.subscribe();
        let seen = |events: &[RuntimeEvent]| {
            events.iter().any(|event| {
                matches!(event, RuntimeEvent::Output { data, .. }
                    if String::from_utf8_lossy(data).contains(needle))
            })
        };
        if seen(&session.snapshot()) {
            return true;
        }
        let mut text = String::new();
        tokio::time::timeout(Duration::from_secs(5), async {
            while let Ok(event) = events.recv().await {
                if let RuntimeEvent::Output { data, .. } = event {
                    text.push_str(&String::from_utf8_lossy(&data));
                    if text.contains(needle) {
                        return;
                    }
                }
            }
        })
        .await
        .is_ok()
    }

    /// A second server attaching finds the same shell, with what the first
    /// one typed into it, under the same process name; stopping it ends the
    /// host and takes its socket away.
    #[tokio::test]
    async fn a_new_server_attaches_to_the_same_shell() {
        let dir = std::env::temp_dir().join(format!("tendly-host-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("s.sock");
        let spec = SpawnSpec {
            program: "/bin/bash".into(),
            args: vec!["--noprofile".into(), "--norc".into(), "-i".into()],
            cwd: std::env::temp_dir(),
            env: BTreeMap::new(),
            env_remove: vec!["BASH_ENV".into()],
        };
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let host = tokio::spawn({
            let socket = socket.clone();
            async move {
                hold(&socket, "t".into(), spec, move || {
                    let _ = ready_tx.send(());
                    Ok(())
                })
                .await
                .map_err(|error| error.to_string())
            }
        });
        ready_rx.await.unwrap();
        let attach_to = |socket: PathBuf| {
            tokio::task::spawn_blocking(move || attach("t", &socket).map_err(|e| e.to_string()))
        };

        let first = attach_to(socket.clone()).await.unwrap().unwrap();
        first
            .input(b"export KEPT=$((40+2))\n".to_vec())
            .await
            .unwrap();
        first.input(b"echo first-$KEPT\n".to_vec()).await.unwrap();
        assert!(output_until(&first, "first-42").await);

        let second = attach_to(socket.clone()).await.unwrap().unwrap();
        assert_eq!(second.instance(), first.instance());
        assert!(
            output_until(&second, "first-42").await,
            "replay carried over"
        );
        second.input(b"echo second-$KEPT\n".to_vec()).await.unwrap();
        assert!(
            output_until(&second, "second-42").await,
            "the shell kept its state"
        );

        second.stop().await.unwrap();
        assert!(!second.running());
        tokio::time::timeout(Duration::from_secs(5), host)
            .await
            .expect("host ends once its exit is seen")
            .unwrap()
            .unwrap();
        assert!(!socket.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn events_survive_the_wire() {
        for event in [
            RuntimeEvent::Output {
                seq: 7,
                data: b"hello".to_vec(),
            },
            RuntimeEvent::Exited {
                seq: 8,
                code: Some(-3),
            },
            RuntimeEvent::Exited { seq: 9, code: None },
        ] {
            let bytes = encode(&event);
            let (kind, payload) = read_frame(&mut bytes.as_slice()).unwrap();
            assert_eq!(decode(kind, &payload), Some(event));
        }
    }
}
