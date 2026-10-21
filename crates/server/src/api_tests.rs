use super::*;
pub(super) use tower::ServiceExt;
mod agent_tools_tests;
mod archive_tests;
mod balance_tests;
mod canvas_tests;
mod conversation_tests;
mod git_ops_tests;
mod history_tests;
mod host_tests;
mod launch_tests;
mod profile_tests;
mod pty_tests;
mod session_tests;
mod workflow_tests;
use axum::{
    body::{Body, to_bytes},
    http::{Request, header},
};
use std::{collections::BTreeMap, fs, process::Command};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message as WsMessage, client::IntoClientRequest},
};

/// A native process that stays alive until it is stopped: `/bin/sh` with
/// `unix_args`, or on Windows an interactive `cmd.exe` waiting for input.
fn waiting_process(unix_args: &[&str]) -> (String, Vec<String>) {
    if cfg!(windows) {
        ("cmd.exe".into(), vec!["/Q".into()])
    } else {
        (
            "/bin/sh".into(),
            unix_args.iter().map(|arg| (*arg).to_owned()).collect(),
        )
    }
}

/// Terminal input that makes a [`waiting_process`] print `{first}_{second}_READY`.
/// The typed text never contains that sentinel, so the terminal's echo of the
/// input cannot satisfy a test. On Windows it first answers the cursor query
/// ConPTY opens with, as xterm.js does in the app.
fn announce(first: &str, second: &str) -> String {
    if cfg!(windows) {
        format!("\x1b[1;1Recho {first}_^{second}_READY\r")
    } else {
        format!("printf '{first}_%s_READY\\n' '{second}'\r")
    }
}

struct Fixture {
    state: AppState,
    path: PathBuf,
}
impl Fixture {
    fn new(address: SocketAddr, token: Option<&str>) -> Self {
        let path = std::env::temp_dir().join(format!("tendly-api-{}", Uuid::new_v4()));
        fs::create_dir_all(path.join("repo")).unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.name", "Tendly test"],
            vec!["config", "user.email", "test@tendly.invalid"],
            vec!["config", "core.autocrlf", "false"],
        ] {
            assert!(
                Command::new("git")
                    .args(args)
                    .current_dir(path.join("repo"))
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        }
        let state = AppState {
            store: Arc::new(Store::open(":memory:").unwrap()),
            runtime: RuntimeManager::new(),
            state_dir: path.join("state"),
            browse_roots: vec![dunce::canonicalize(path.join("repo")).unwrap()],
            host_grants: crate::host_grants::HostGrants::default(),
            workspace_roots: vec![dunce::canonicalize(&path).unwrap()],
            native_sources: Vec::new(),
            native_bridge: PathBuf::from("packages/native-bridge/history.mjs"),
            chat_fixture: Some(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../packages/native-bridge/fixtures/chat-supervisor.mjs"),
            ),
            // Route tests use a synthetic JSONL supervisor.
            chats: conversations::ChatManager::default(),
            watches: file_watch::FileWatches::default(),
            accounts: accounts::AccountManager::default(),
            security: security::Security::for_test(address, token),
            tailnet: crate::tailscale::Tailnet::disabled(),
            claude_manual_mode: true,
            operations: Arc::new(tokio::sync::Mutex::new(())),
            agents: agent::AgentRegistry::default(),
            activity: activity::Activity::default(),
            metrics: None,
        };
        Self { state, path }
    }
    fn app(&self) -> Router {
        router(self.state.clone())
    }

    /// The test bridge reads only this fixture's static JSON. It never imports
    /// a provider SDK, launches a native client, or inspects the user's history.
    fn native_source(&mut self, provider: ProviderKind, items: Vec<Value>) -> (String, PathBuf) {
        let source_id = format!("fixture-native-{}", self.state.native_sources.len());
        let config = self.path.join(&source_id);
        fs::create_dir(&config).unwrap();
        let config = dunce::canonicalize(config).unwrap();
        fs::write(
            config.join("history-fixture.json"),
            serde_json::to_vec(&json!({"items":items,"truncated":false})).unwrap(),
        )
        .unwrap();
        let bridge = self.path.join("fixture-native-history.mjs");
        fs::write(
            &bridge,
            r#"import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
let input = '';
for await (const chunk of process.stdin) input += chunk;
const job = JSON.parse(input);
writeFileSync(join(job.config_dir, 'bridge-used'), 'fixture only');
const items = readFileSync(join(job.config_dir, 'history-fixture.json'), 'utf8');
process.stdout.write(items);
"#,
        )
        .unwrap();
        self.state.native_bridge = bridge;
        self.state
            .native_sources
            .push(native_history::NativeSource {
                id: source_id.clone(),
                label: "Test fixture only".into(),
                provider,
                config_dir: config.clone(),
                config_env: Some(config.as_os_str().to_owned()),
            });
        (source_id, config)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
async fn call(app: Router, method: &str, path: &str, value: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:8787")
        .header("x-tendly-client", "web")
        .header("content-type", "application/json")
        .body(Body::from(value.to_string()))
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn register(f: &Fixture) -> String {
    let (status, w) = call(
        f.app(),
        "POST",
        "/api/workspaces",
        json!({"name":"fixture","root_path":f.path.join("repo")}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    w["id"].as_str().unwrap().to_owned()
}

/// A launch's arguments without the ones that give it Tendly's own tools
/// (agent::equip), for tests about everything else a launch carries.
fn without_tendly(args: &[String]) -> Vec<String> {
    let mut kept = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg.starts_with("--mcp-config=") || arg == "--allowedTools=mcp__tendly" {
            index += 1;
        } else if arg == "-c"
            && args
                .get(index + 1)
                .is_some_and(|value| value.starts_with("mcp_servers.tendly."))
        {
            index += 2;
        } else {
            kept.push(arg.clone());
            index += 1;
        }
    }
    kept
}

/// A request to the agent tools the way `tendly mcp` makes one.
async fn agent_request(
    app: Router,
    method: &str,
    path: &str,
    bearer: Option<&str>,
    client: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:8787")
        .header("content-type", "application/json");
    if let Some(token) = bearer {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    if let Some(client) = client {
        request = request.header("x-tendly-client", client);
    }
    let response = app
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn tool_text(result: &Value) -> Value {
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap_or(Value::Null)
}

/// The next event of one type from the activity stream, as a browser gets it.
async fn next_event(events: &mut tokio::sync::broadcast::Receiver<Value>, kind: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.unwrap();
            if event["type"] == kind {
                return event;
            }
        }
    })
    .await
    .expect("activity event")
}

fn canvas_document(panes: Vec<Value>) -> Value {
    json!({"version":1,"root":{"type":"stack","kind":"stack","id":"shared-root","panes":panes}})
}

fn urlencoding(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn url_escape(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}
