//! Rust client protocol regression tests against synthetic native clients.
use crate::chat_adapter::{ChatCommand, ChatInit, ClientAdapter, dispatch};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf, time::Duration};

/// One thing a scenario does.
enum Step {
    Send(Value),
    /// Wait until an event matches.
    Wait(fn(&Value) -> bool),
    /// Wait until this many events match.
    WaitCount(fn(&Value) -> bool, usize),
    /// Answer the latest approval: the decision, and answers built from it.
    Answer(&'static str, fn(&Value) -> Value),
    Sleep(u64),
}
use Step::*;

fn fixture_package_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packages/native-bridge")
}

/// What the fixture app-server needs in its environment, and a secret the
/// output must never show.
fn environment(log: &std::path::Path) -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "TENDLY_CHAT_TEST_LOG".to_owned(),
            log.to_string_lossy().into_owned(),
        ),
        ("TENDLY_CHAT_TEST_INHERITED".to_owned(), "kept".to_owned()),
        (
            "TENDLY_SECRET_TEST_CREDENTIAL".to_owned(),
            "fixture-secret-value".to_owned(),
        ),
    ])
}

struct Run {
    events: Vec<Value>,
    native: Vec<Value>,
}

async fn drive<A: ClientAdapter>(
    mut adapter: A,
    init: ChatInit,
    steps: &[Step],
    log: &std::path::Path,
) -> Run {
    let mut events = Vec::new();
    adapter.start(&init).await.expect("start");
    let next = async |adapter: &mut A, events: &mut Vec<Value>, wait: Duration| -> bool {
        match tokio::time::timeout(wait, adapter.next_event()).await {
            Ok(Some(Ok(event))) => {
                if let Some(event) = crate::conversations::normalize_event(event) {
                    events.push(event);
                }
                true
            }
            _ => false,
        }
    };
    // Ready, and the models app-server lists, before anything is asked of it.
    while !events
        .iter()
        .any(|e: &Value| e["type"] == "settings" && e.get("models").is_some())
    {
        assert!(
            next(&mut adapter, &mut events, Duration::from_secs(5)).await,
            "no models announced: {events:?}"
        );
    }
    for step in steps {
        match step {
            Send(command) => dispatch(&mut adapter, ChatCommand::parse(command).expect("command"))
                .await
                .expect("sent"),
            Answer(decision, answers) => {
                let approval = events
                    .iter()
                    .rev()
                    .find(|e| e["type"] == "approval")
                    .expect("an approval")
                    .clone();
                let command = json!({"type":"approval","request_id":approval["id"],"decision":decision,"answers":answers(&approval)});
                dispatch(&mut adapter, ChatCommand::parse(&command).expect("answer"))
                    .await
                    .expect("sent");
            }
            Wait(predicate) => {
                while !events.iter().any(predicate) {
                    assert!(
                        next(&mut adapter, &mut events, Duration::from_secs(5)).await,
                        "timed out: {events:?}"
                    );
                }
            }
            WaitCount(predicate, count) => {
                while events.iter().filter(|e| predicate(e)).count() < *count {
                    assert!(
                        next(&mut adapter, &mut events, Duration::from_secs(5)).await,
                        "timed out: {events:?}"
                    );
                }
            }
            Sleep(ms) => {
                let until = tokio::time::Instant::now() + Duration::from_millis(*ms);
                while tokio::time::Instant::now() < until {
                    next(
                        &mut adapter,
                        &mut events,
                        until - tokio::time::Instant::now(),
                    )
                    .await;
                }
            }
        }
    }
    // Whatever follows the last step, so a late event is compared too.
    while next(&mut adapter, &mut events, Duration::from_millis(400)).await {}
    adapter.shutdown().await;
    let native = std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("log line"))
        .collect();
    Run { events, native }
}

/// Approval ids by order of appearance, and the run's directory as a placeholder.
fn comparable(run: Run, dir: &str) -> (Vec<Value>, Vec<Value>) {
    let mut ids: Vec<String> = Vec::new();
    let mut seen = |id: &Value| -> Value {
        let id = id.as_str().unwrap_or_default().to_owned();
        let index = ids
            .iter()
            .position(|known| *known == id)
            .unwrap_or_else(|| {
                ids.push(id);
                ids.len() - 1
            });
        json!(format!("approval#{index}"))
    };
    fn placeholder(value: Value, dir: &str) -> Value {
        match value {
            Value::String(text) => Value::String(untimed(&text.replace(dir, "<dir>"))),
            Value::Array(items) => {
                Value::Array(items.into_iter().map(|v| placeholder(v, dir)).collect())
            }
            Value::Object(object) => Value::Object(
                object
                    .into_iter()
                    .map(|(k, v)| (k, placeholder(v, dir)))
                    .collect(),
            ),
            other => other,
        }
    }
    // A turn Claude Code starts by itself is named with a fresh UUID, and a
    // card's `at` is the moment it was emitted: compared by order and presence.
    let mut background: Vec<String> = Vec::new();
    let events = run
        .events
        .into_iter()
        .map(|mut event| {
            if matches!(
                event["type"].as_str(),
                Some("approval" | "approval_resolved")
            ) {
                event["id"] = seen(&event["id"]);
            }
            if let Some(id) = event["id"]
                .as_str()
                .filter(|id| id.starts_with("background-"))
            {
                let id = id.to_owned();
                let index = background
                    .iter()
                    .position(|known| *known == id)
                    .unwrap_or_else(|| {
                        background.push(id);
                        background.len() - 1
                    });
                event["id"] = json!(format!("background#{index}"));
            }
            if event.get("at").is_some() {
                event["at"] = json!(0);
            }
            placeholder(event, dir)
        })
        .collect();
    let native = run
        .native
        .into_iter()
        .map(|v| placeholder(v, dir))
        .collect();
    (events, native)
}

/// A client under comparison: its provider id, its synthetic stand-in and
/// the adapter that runs it here.
struct Client {
    provider: &'static str,
    fixture: &'static str,
    adapter: fn(&BTreeMap<String, String>, &[String]) -> crate::native_chat::NativeAdapter,
}
const CODEX: Client = Client {
    provider: "codex",
    fixture: "chat-cli.mjs",
    adapter: crate::codex_chat::adapter,
};
const PI: Client = Client {
    provider: "pi",
    fixture: "pi-cli.mjs",
    adapter: crate::pi_chat::adapter,
};
const CLAUDE: Client = Client {
    provider: "claude_code",
    fixture: "claude-cli.mjs",
    adapter: crate::claude_chat::adapter,
};

/// A turn Pi starts by itself is named for the moment it began
/// (`pi-<ms>-<n>`); only the count is compared.
fn untimed(text: &str) -> String {
    let Some(rest) = text.strip_prefix("pi-") else {
        return text.to_owned();
    };
    match rest.split_once('-') {
        Some((millis, count))
            if millis.len() >= 10 && millis.bytes().all(|b| b.is_ascii_digit()) =>
        {
            format!("pi-<t>-{count}")
        }
        _ => text.to_owned(),
    }
}

/// A fresh directory with the client's fixture in it.
fn scenario_dir(name: &str) -> PathBuf {
    scenario_dir_for(&CODEX, name)
}
fn scenario_dir_for(client: &Client, name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tendly-parity-{name}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let program = dir.join(client.fixture);
    std::fs::copy(
        fixture_package_dir().join("fixtures").join(client.fixture),
        &program,
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    dir
}

fn init(dir: &std::path::Path, args: &[&str]) -> ChatInit {
    init_for(&CODEX, dir, args)
}
fn init_for(client: &Client, dir: &std::path::Path, args: &[&str]) -> ChatInit {
    ChatInit {
        provider: client.provider.into(),
        program: dir.join(client.fixture).to_string_lossy().into_owned(),
        args: args.iter().map(|a| a.to_string()).collect(),
        cwd: dir.to_string_lossy().into_owned(),
        resume_id: None,
    }
}

async fn scenario(name: &str, args: &[&str], steps: &[Step]) -> Vec<Value> {
    scenario_for(&CODEX, name, args, steps).await
}
/// Verify normalized events and native protocol requests.
async fn scenario_for(client: &Client, name: &str, args: &[&str], steps: &[Step]) -> Vec<Value> {
    let rust = {
        let dir = scenario_dir_for(client, &format!("{name}-rust"));
        let log = dir.join("native.jsonl");
        let adapter = (client.adapter)(&environment(&log), &[]);
        let run = drive(adapter, init_for(client, &dir, args), steps, &log).await;
        let result = comparable(run, &dir.to_string_lossy());
        let _ = std::fs::remove_dir_all(&dir);
        result
    };
    let secret = |events: &[Value]| {
        events
            .iter()
            .any(|e| e.to_string().contains("fixture-secret-value"))
    };
    assert!(!secret(&rust.0), "{name}: a secret reached the output");
    assert!(!rust.0.is_empty());
    if name == "permission-running" {
        let turns: Vec<_> = rust
            .1
            .iter()
            .filter(|e| e["method"] == "turn/start")
            .collect();
        assert_eq!(turns.len(), 2);
        assert!(turns[0]["params"].get("approvalPolicy").is_none());
        assert_eq!(turns[1]["params"]["approvalPolicy"], "never");
        assert_eq!(
            turns[1]["params"]["sandboxPolicy"]["type"],
            "dangerFullAccess"
        );
    }
    if name == "sandbox" {
        let turns: Vec<_> = rust
            .1
            .iter()
            .filter(|e| e["method"] == "turn/start")
            .collect();
        assert_eq!(
            turns[0]["params"]["sandboxPolicy"]["type"],
            "dangerFullAccess"
        );
        assert_eq!(turns[1]["params"]["sandboxPolicy"]["type"], "readOnly");
    }
    rust.0
}

fn turn(event: &Value, id: &str, status: &str) -> bool {
    event["type"] == "turn" && event["id"] == id && event["status"] == status
}
fn none(_: &Value) -> Value {
    json!({})
}
fn brief(approval: &Value) -> Value {
    json!({ approval["questions"][0]["id"].as_str().unwrap(): ["Brief"] })
}

#[tokio::test]
async fn a_simple_turn() {
    scenario(
        "simple",
        &[],
        &[
            Send(json!({"type":"message","id":"one","content":"simple"})),
            Wait(|e| turn(e, "one", "completed")),
        ],
    )
    .await;
}

#[tokio::test]
async fn declining_an_approval() {
    scenario(
        "decline",
        &[],
        &[
            Send(json!({"type":"message","id":"approve","content":"approve"})),
            Wait(|e| e["type"] == "approval"),
            Answer("decline", none),
            Wait(|e| turn(e, "approve", "completed")),
        ],
    )
    .await;
}

#[tokio::test]
async fn accepting_then_cancelling() {
    scenario(
        "accept-cancel",
        &[],
        &[
            Send(json!({"type":"message","id":"accept","content":"approve"})),
            Wait(|e| e["type"] == "approval"),
            Answer("accept", none),
            Wait(|e| turn(e, "accept", "completed")),
            Send(json!({"type":"message","id":"cancel","content":"approve"})),
            WaitCount(|e| e["type"] == "approval", 2),
            Answer("cancel", none),
            Wait(|e| turn(e, "cancel", "interrupted")),
        ],
    )
    .await;
}

#[tokio::test]
async fn questions_and_their_answers() {
    scenario(
        "ask",
        &[],
        &[
            Send(json!({"type":"message","id":"ask","content":"ask"})),
            Wait(|e| e["type"] == "approval"),
            Answer("accept", none),
            Wait(|e| e["type"] == "error"),
            Answer("accept", brief),
            Wait(|e| turn(e, "ask", "completed")),
        ],
    )
    .await;
}

#[tokio::test]
async fn permissions_on_a_resumed_thread() {
    scenario(
        "permissions",
        &["resume", "saved-thread"],
        &[
            Send(json!({"type":"message","id":"permissions","content":"permissions"})),
            Wait(|e| e["type"] == "approval"),
            Answer("decline", none),
            Wait(|e| turn(e, "permissions", "completed")),
        ],
    )
    .await;
}

#[tokio::test]
async fn interrupting_a_turn_and_a_repeated_message() {
    scenario(
        "interrupt",
        &[],
        &[
            Send(json!({"type":"message","id":"hold","content":"hold"})),
            Wait(|e| e["type"] == "tool" && e["id"] == "hold-1"),
            Send(json!({"type":"message","id":"hold","content":"hold"})),
            Send(json!({"type":"interrupt"})),
            Wait(|e| turn(e, "hold", "interrupted")),
            Send(json!({"type":"message","id":"after","content":"simple"})),
            Wait(|e| turn(e, "after", "completed")),
        ],
    )
    .await;
}

#[tokio::test]
async fn an_unsupported_request_fails_closed() {
    scenario(
        "unknown",
        &[],
        &[
            Send(json!({"type":"message","id":"unknown","content":"unknown"})),
            Wait(|e| e["type"] == "turn" && e["status"] == "failed"),
        ],
    )
    .await;
}

#[tokio::test]
async fn codex_permission_can_change_while_a_turn_is_running() {
    scenario(
        "permission-running",
        &[],
        &[
            Send(json!({"type":"message","id":"hold","content":"hold"})),
            Wait(|e| e["type"] == "tool" && e["id"] == "hold-1"),
            Send(json!({"type":"permission","mode":"danger"})),
            Wait(|e| e["type"] == "settings" && e["permission_mode"] == "danger"),
            Send(json!({"type":"interrupt"})),
            Wait(|e| turn(e, "hold", "interrupted")),
            Send(json!({"type":"message","id":"next","content":"simple"})),
            Wait(|e| turn(e, "next", "completed")),
        ],
    )
    .await;
}

#[tokio::test]
async fn leaving_danger_restores_the_sandbox() {
    scenario(
        "sandbox",
        &[],
        &[
            Send(json!({"type":"permission","mode":"danger"})),
            Wait(|e| e["type"] == "settings" && e["permission_mode"] == "danger"),
            Send(json!({"type":"message","id":"free","content":"simple"})),
            Wait(|e| turn(e, "free", "completed")),
            Send(json!({"type":"permission","mode":"ask"})),
            Wait(|e| e["type"] == "settings" && e["permission_mode"] == "ask"),
            Send(json!({"type":"message","id":"guarded","content":"simple"})),
            Wait(|e| turn(e, "guarded", "completed")),
            // Unsupported permission modes are refused.
            Send(json!({"type":"permission","mode":"plan"})),
            Wait(|e| e["type"] == "error"),
        ],
    )
    .await;
}

#[tokio::test]
async fn file_changes_and_mcp_tools() {
    scenario(
        "tools",
        &[],
        &[
            Send(json!({"type":"message","id":"tools","content":"tools"})),
            Wait(|e| turn(e, "tools", "completed")),
        ],
    )
    .await;
}

#[tokio::test]
async fn choosing_a_model_and_a_depth() {
    scenario(
        "model",
        &[],
        &[
            Send(json!({"type":"message","id":"before","content":"simple"})),
            Wait(|e| turn(e, "before", "completed")),
            Send(json!({"type":"model","model":"gpt-plain","effort":"high"})),
            Wait(|e| e["type"] == "settings" && e["model"] == "gpt-plain"),
            Send(json!({"type":"message","id":"after","content":"simple"})),
            Wait(|e| turn(e, "after", "completed")),
            Send(json!({"type":"message","id":"hold","content":"hold"})),
            Wait(|e| e["type"] == "tool" && e["id"] == "hold-3"),
            Send(json!({"type":"model","model":"gpt-fixture"})),
            Wait(|e| e["type"] == "error"),
            Send(json!({"type":"interrupt"})),
            Wait(|e| turn(e, "hold", "interrupted")),
        ],
    )
    .await;
}

#[tokio::test]
async fn steering_the_running_turn() {
    scenario(
        "steer",
        &[],
        &[
            Send(json!({"type":"message","id":"steer-a","content":"hold"})),
            Wait(|e| turn(e, "steer-a", "running")),
            Sleep(100),
            Send(json!({"type":"steer","id":"steer-b","content":"also this"})),
            Wait(|e| turn(e, "steer-a", "completed")),
        ],
    )
    .await;
}

#[tokio::test]
async fn a_turn_that_refuses_steering_passes_it_on() {
    scenario(
        "steer-refused",
        &[],
        &[
            Send(json!({"type":"message","id":"steer-a","content":"review"})),
            Wait(|e| turn(e, "steer-a", "running")),
            Sleep(100),
            Send(json!({"type":"steer","id":"steer-b","content":"afterwards"})),
            Wait(|e| turn(e, "steer-b", "completed")),
        ],
    )
    .await;
}

#[tokio::test]
async fn steering_with_no_turn_running_is_a_message() {
    scenario(
        "steer-idle",
        &[],
        &[
            Send(json!({"type":"steer","id":"lone","content":"simple"})),
            Wait(|e| turn(e, "lone", "completed")),
        ],
    )
    .await;
}

/// Everything an adapter says when its client cannot even be launched.
async fn unlaunchable<A: ClientAdapter>(mut adapter: A, init: ChatInit) -> Vec<Value> {
    adapter.start(&init).await.expect("start");
    let mut events = Vec::new();
    while let Ok(Some(Ok(event))) =
        tokio::time::timeout(Duration::from_secs(5), adapter.next_event()).await
    {
        let done = event["type"] == "exit";
        events.extend(crate::conversations::normalize_event(event));
        if done {
            break;
        }
    }
    adapter.shutdown().await;
    events
}

#[tokio::test]
async fn a_client_that_cannot_start() {
    let missing = |dir: &std::path::Path| {
        let mut init = init(dir, &[]);
        init.program = dir.join("no-such-program").to_string_lossy().into_owned();
        init
    };
    let dir = scenario_dir("missing");
    let rust = unlaunchable(
        crate::codex_chat::adapter(&BTreeMap::new(), &[]),
        missing(&dir),
    )
    .await;
    assert!(
        rust.iter()
            .any(|e| e["message"] == "Could not launch the configured native client."),
        "{rust:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn an_unusable_launch_option() {
    let dir = scenario_dir("option");
    let rust = unlaunchable(
        crate::codex_chat::adapter(&BTreeMap::new(), &[]),
        init(&dir, &["--full-auto"]),
    )
    .await;
    assert!(rust.iter().any(|e| e["type"] == "error"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A steer sent before Codex has said which turn is running waits for the id
/// (`early`), then joins that turn.
#[tokio::test]
async fn steering_before_the_turn_has_an_id() {
    scenario(
        "steer-early",
        &[],
        &[
            Send(json!({"type":"message","id":"slow-a","content":"slow"})),
            Send(json!({"type":"steer","id":"slow-b","content":"also this"})),
            Wait(|e| turn(e, "slow-a", "completed")),
        ],
    )
    .await;
}

/// An interrupt sent before the turn has an id is held until it has one, and
/// then stops that turn.
/// A steer written before the turn has an id is not lost when that turn
/// never takes it: Codex refusing the turn, or finishing it before answering
/// turn/start. It runs as the next turn, like a steer a turn refused.
#[tokio::test]
async fn an_early_steer_survives_a_turn_that_fails_to_start() {
    scenario(
        "steer-early-failed",
        &[],
        &[
            Send(json!({"type":"message","id":"first","content":"slowfail"})),
            Send(json!({"type":"steer","id":"steer","content":"simple"})),
            Wait(|e| turn(e, "first", "failed")),
            Wait(|e| turn(e, "steer", "completed")),
        ],
    )
    .await;
}

#[tokio::test]
async fn an_early_steer_survives_a_turn_that_ends_before_its_id() {
    scenario(
        "steer-early-raced",
        &[],
        &[
            Send(json!({"type":"message","id":"first","content":"racing"})),
            Send(json!({"type":"steer","id":"steer","content":"simple"})),
            Wait(|e| turn(e, "first", "completed")),
            Wait(|e| turn(e, "steer", "completed")),
        ],
    )
    .await;
}

#[tokio::test]
async fn interrupting_before_the_turn_has_an_id() {
    scenario(
        "interrupt-early",
        &[],
        &[
            Send(json!({"type":"message","id":"slow-a","content":"slow"})),
            Send(json!({"type":"interrupt"})),
            Wait(|e| turn(e, "slow-a", "interrupted")),
            Send(json!({"type":"message","id":"after","content":"simple"})),
            Wait(|e| turn(e, "after", "completed")),
        ],
    )
    .await;
}

/// An adapter dropped without `shutdown` still ends its conversation and the
/// app-server it started.
#[cfg(unix)]
#[tokio::test]
async fn a_dropped_adapter_stops_its_app_server() {
    let dir = scenario_dir("dropped-rust");
    let log = dir.join("native.jsonl");
    let mut adapter = crate::codex_chat::adapter(&environment(&log), &[]);
    adapter.start(&init(&dir, &[])).await.expect("start");
    let mut pid = None;
    for _ in 0..50 {
        pid = adapter.pid();
        if pid.is_some() {
            break;
        }
        let _ = tokio::time::timeout(Duration::from_millis(100), adapter.next_event()).await;
    }
    let pid = pid.expect("a running app-server") as libc::pid_t;
    drop(adapter);
    let alive = || unsafe { libc::kill(pid, 0) } == 0;
    for _ in 0..50 {
        if !alive() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!alive(), "the app-server outlived its dropped adapter");
}

/// Codex has no `/clear`: Tendly opens a new thread for it, so it clears as
/// Claude Code's does, and a restart resumes the new thread.
#[tokio::test]
async fn codex_clear_moves_to_a_new_thread() {
    let events = scenario(
        "codex-clear",
        &[],
        &[
            // Before any thread there is only the view to drop.
            Send(json!({"type":"message","id":"early","content":"/clear"})),
            Wait(|e| turn(e, "early", "completed")),
            Send(json!({"type":"message","id":"one","content":"hello"})),
            Wait(|e| turn(e, "one", "completed")),
            Send(json!({"type":"message","id":"clear","content":"/clear"})),
            Wait(|e| turn(e, "clear", "completed")),
            Send(json!({"type":"message","id":"two","content":"hello"})),
            Wait(|e| turn(e, "two", "completed")),
        ],
    )
    .await;
    assert!(
        events
            .iter()
            .any(|e| e["type"] == "ready" && e["commands"] == json!(["clear"]))
    );
    assert_eq!(events.iter().filter(|e| e["type"] == "cleared").count(), 2);
    let after = events
        .iter()
        .position(|e| turn(e, "clear", "completed"))
        .unwrap();
    let cleared = events.iter().rposition(|e| e["type"] == "cleared").unwrap();
    assert!(cleared < after);
    assert!(
        events[cleared..]
            .iter()
            .any(|e| e["type"] == "ready" && e["native_session_id"] == "fixture-thread-2")
    );
}

// --- Pi ----------------------------------------------------------------------

#[tokio::test]
async fn pi_a_message_with_its_reply_tools_and_usage() {
    scenario_for(
        &PI,
        "pi-simple",
        &[],
        &[
            Send(json!({"type":"message","id":"one","content":"hello"})),
            Wait(|e| turn(e, "one", "completed")),
        ],
    )
    .await;
}

#[tokio::test]
async fn pi_confirmations_accepted_and_declined() {
    scenario_for(
        &PI,
        "pi-confirm",
        &[],
        &[
            Send(json!({"type":"message","id":"yes","content":"confirm"})),
            Wait(|e| e["type"] == "approval"),
            Answer("accept", none),
            Wait(|e| turn(e, "yes", "completed")),
            Send(json!({"type":"message","id":"no","content":"confirm"})),
            WaitCount(|e| e["type"] == "approval", 2),
            Answer("decline", none),
            Wait(|e| turn(e, "no", "completed")),
        ],
    )
    .await;
}

#[tokio::test]
async fn pi_models_by_provider_and_depths() {
    scenario_for(
        &PI,
        "pi-model",
        &[],
        &[
            Send(json!({"type":"model","model":"kunlun/zai-org/GLM-5.3","effort":"high"})),
            Wait(|e| {
                e["type"] == "settings"
                    && e["model"] == "kunlun/zai-org/GLM-5.3"
                    && e["effort"] == "high"
            }),
            // A name without a provider, and approval modes Pi does not have.
            Send(json!({"type":"model","model":"plain"})),
            Wait(|e| e["type"] == "error"),
            Send(json!({"type":"permission","mode":"ask"})),
            WaitCount(|e| e["type"] == "error", 2),
            Send(json!({"type":"message","id":"after","content":"hello"})),
            Wait(|e| turn(e, "after", "completed")),
        ],
    )
    .await;
}

#[tokio::test]
async fn pi_interrupting_and_steering_a_running_turn() {
    scenario_for(
        &PI,
        "pi-interrupt",
        &[],
        &[
            Send(json!({"type":"message","id":"hold","content":"hold"})),
            Wait(|e| turn(e, "hold", "running")),
            Sleep(100),
            Send(json!({"type":"steer","id":"more","content":"also this"})),
            Send(json!({"type":"model","model":"tendly/qwen-local"})),
            Wait(|e| e["type"] == "error"),
            Send(json!({"type":"interrupt"})),
            Wait(|e| turn(e, "hold", "interrupted")),
            // Steering with no turn running is a message of its own.
            Send(json!({"type":"steer","id":"lone","content":"hello"})),
            Wait(|e| turn(e, "lone", "completed")),
        ],
    )
    .await;
}

/// Pi's `/new` is its terminal's; over RPC `/clear` is `new_session`.
#[tokio::test]
async fn pi_clear_starts_a_new_session() {
    let events = scenario_for(
        &PI,
        "pi-clear",
        &[],
        &[
            Send(json!({"type":"message","id":"one","content":"hello"})),
            Wait(|e| turn(e, "one", "completed")),
            Send(json!({"type":"message","id":"clear","content":"/clear"})),
            Wait(|e| turn(e, "clear", "completed")),
        ],
    )
    .await;
    assert!(events.iter().any(|e| e["type"] == "ready"
        && e["commands"] == json!(["skill:review", "clear"])
        && e["native_session_id"] == "pi-session-1"));
    let cleared = events
        .iter()
        .position(|e| e["type"] == "cleared")
        .expect("cleared");
    assert!(
        events[cleared..]
            .iter()
            .any(|e| e["type"] == "ready" && e["native_session_id"] == "pi-session-2")
    );
}

#[tokio::test]
async fn pi_a_resumed_session_and_an_unusable_option() {
    scenario_for(
        &PI,
        "pi-resume",
        &["--session", "pi-session-1", "--thinking", "high"],
        &[
            Send(json!({"type":"message","id":"one","content":"hello"})),
            Wait(|e| turn(e, "one", "completed")),
        ],
    )
    .await;
}

/// Pi's conversation ends with its dropped adapter too: the shared core does it.
#[cfg(unix)]
#[tokio::test]
async fn pi_a_dropped_adapter_stops_pi() {
    let dir = scenario_dir_for(&PI, "pi-dropped-rust");
    let log = dir.join("native.jsonl");
    let mut adapter = crate::pi_chat::adapter(&environment(&log), &[]);
    adapter
        .start(&init_for(&PI, &dir, &[]))
        .await
        .expect("start");
    let mut pid = None;
    for _ in 0..50 {
        pid = adapter.pid();
        if pid.is_some() {
            break;
        }
        let _ = tokio::time::timeout(Duration::from_millis(100), adapter.next_event()).await;
    }
    let pid = pid.expect("a running Pi") as libc::pid_t;
    drop(adapter);
    let alive = || unsafe { libc::kill(pid, 0) } == 0;
    for _ in 0..50 {
        if !alive() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!alive(), "Pi outlived its dropped adapter");
}

/// An extension command is run by the extension: Pi answers the prompt and
/// starts no run, so no agent_settled ever comes. The turn must still end, or
/// every later message is refused as "a turn is already running".
#[tokio::test]
async fn pi_an_extension_command_ends_its_turn() {
    scenario_for(
        &PI,
        "pi-extension-command",
        &[],
        &[
            Send(json!({"type":"message","id":"ext","content":"/skill:review"})),
            Wait(|e| turn(e, "ext", "completed")),
            Send(json!({"type":"message","id":"after","content":"hello"})),
            Wait(|e| turn(e, "after", "completed")),
        ],
    )
    .await;
}

/// Interrupting before Pi has started the run (its preflight) stops the run
/// once it starts, instead of letting it run to the end as "interrupted".
#[tokio::test]
async fn pi_interrupting_before_the_run_starts() {
    scenario_for(
        &PI,
        "pi-interrupt-early",
        &[],
        &[
            Send(json!({"type":"message","id":"slow","content":"slowstart"})),
            Send(json!({"type":"interrupt"})),
            Wait(|e| turn(e, "slow", "interrupted")),
            Send(json!({"type":"message","id":"after","content":"hello"})),
            Wait(|e| turn(e, "after", "completed")),
        ],
    )
    .await;
}

/// Pi clamps a depth its model cannot use, and a model change re-applies the
/// level that model allows; the depth shown is the one Pi runs, not the one asked.
#[tokio::test]
async fn pi_the_depth_shown_is_the_one_pi_runs() {
    let events = scenario_for(
        &PI,
        "pi-depth",
        &[],
        &[
            // qwen-local does not reason: Pi runs it at `off`.
            Send(json!({"type":"model","effort":"high"})),
            Sleep(300),
            Send(json!({"type":"model","model":"kunlun/zai-org/GLM-5.3","effort":"high"})),
            Wait(|e| {
                e["type"] == "settings"
                    && e["model"] == "kunlun/zai-org/GLM-5.3"
                    && e["effort"] == "high"
            }),
            Send(json!({"type":"model","model":"tendly/qwen-local"})),
            Wait(|e| e["type"] == "settings" && e["model"] == "tendly/qwen-local"),
            Sleep(300),
        ],
    )
    .await;
    let settings: Vec<&Value> = events.iter().filter(|e| e["type"] == "settings").collect();
    assert!(
        !settings
            .iter()
            .any(|e| e["model"] == "tendly/qwen-local" && e.get("effort").is_some()),
        "a depth was shown for a model Pi runs without one: {settings:?}"
    );
    assert_eq!(
        settings.last().map(|e| e["model"].clone()),
        Some(json!("tendly/qwen-local"))
    );
}

// --- Claude Code -------------------------------------------------------------

/// Message ids are UUIDs, so what Claude Code receives (`uuid`) is the same on
/// both sides; any other id goes out as a fresh one.
fn uuid(n: u8) -> String {
    format!("00000000-0000-4000-8000-0000000000{n:02}")
}
fn send_message(n: u8, content: &str) -> Step {
    Send(json!({"type":"message","id":uuid(n),"content":content}))
}
fn finished(n: u8, status: &'static str) -> Step {
    // Steps take plain functions, so the id and status are matched here.
    match (n, status) {
        (1, "completed") => Wait(|e| turn(e, &uuid(1), "completed")),
        (2, "completed") => Wait(|e| turn(e, &uuid(2), "completed")),
        (3, "completed") => Wait(|e| turn(e, &uuid(3), "completed")),
        (4, "completed") => Wait(|e| turn(e, &uuid(4), "completed")),
        (5, "completed") => Wait(|e| turn(e, &uuid(5), "completed")),
        (6, "completed") => Wait(|e| turn(e, &uuid(6), "completed")),
        (1, "interrupted") => Wait(|e| turn(e, &uuid(1), "interrupted")),
        (2, "interrupted") => Wait(|e| turn(e, &uuid(2), "interrupted")),
        (3, "interrupted") => Wait(|e| turn(e, &uuid(3), "interrupted")),
        (1, "failed") => Wait(|e| turn(e, &uuid(1), "failed")),
        (2, "failed") => Wait(|e| turn(e, &uuid(2), "failed")),
        (4, "failed") => Wait(|e| turn(e, &uuid(4), "failed")),
        _ => unreachable!("add the pair"),
    }
}
fn claude_native(events: &[Value]) -> Vec<Value> {
    events.to_vec()
}

#[tokio::test]
async fn claude_a_turn_with_its_reply_usage_and_settings() {
    let events = scenario_for(
        &CLAUDE,
        "claude-simple",
        &[],
        &[send_message(1, "simple"), finished(1, "completed")],
    )
    .await;
    let events = claude_native(&events);
    // The reply streamed, then whole; the turn's cost after it.
    assert!(
        events
            .iter()
            .any(|e| e["type"] == "message" && e["delta"] == true && e["id"] == "msg-1")
    );
    assert!(
        events
            .iter()
            .any(|e| e["type"] == "message" && e["delta"] == false && e["text"] == "Hello 🦊")
    );
    assert!(
        events.iter().any(
            |e| e["type"] == "usage" && e["turn"] == json!({"input":42,"output":4,"cached":30})
        )
    );
    // The models it listed, and `default` as the starting point.
    assert!(
        events
            .iter()
            .any(|e| e["type"] == "settings" && e["model"] == "default")
    );
}

#[tokio::test]
async fn claude_tools_and_their_results() {
    let events = scenario_for(
        &CLAUDE,
        "claude-tools",
        &[],
        &[send_message(1, "tools"), finished(1, "completed")],
    )
    .await;
    let failed = events
        .iter()
        .find(|e| e["type"] == "tool" && e["id"] == "bash-1" && e["status"] == "failed")
        .expect("a failed tool result");
    assert_eq!(failed["text"], "ls: denied");
    assert!(events.iter().any(|e| e["type"] == "tool"
        && e["id"] == "read-1"
        && e["status"] == "completed"
        && e["result_tokens"] == 3));
}

#[tokio::test]
async fn claude_steering_joins_a_turn_or_follows_it() {
    let events = scenario_for(
        &CLAUDE,
        "claude-steer",
        &[],
        &[
            send_message(1, "hold"),
            Wait(|e| e["type"] == "tool" && e["id"] == "hold-1"),
            Send(json!({"type":"steer","id":uuid(2),"content":"join"})),
            finished(1, "completed"),
            send_message(3, "hold"),
            Wait(|e| e["type"] == "tool" && e["id"] == "hold-2"),
            Send(json!({"type":"steer","id":uuid(4),"content":"late"})),
            finished(3, "completed"),
            // Read only after the turn ended: it runs as a turn of its own.
            finished(4, "completed"),
            // Steering with no turn running is a message.
            Send(json!({"type":"steer","id":uuid(5),"content":"simple"})),
            finished(5, "completed"),
        ],
    )
    .await;
    assert!(
        !events.iter().any(|e| turn(e, &uuid(2), "running")),
        "a steer that joined opened a turn"
    );
}

#[tokio::test]
async fn claude_interrupting_a_turn() {
    scenario_for(
        &CLAUDE,
        "claude-interrupt",
        &[],
        &[
            send_message(1, "hold"),
            Wait(|e| e["type"] == "tool" && e["id"] == "hold-1"),
            Send(json!({"type":"interrupt"})),
            finished(1, "interrupted"),
            send_message(2, "simple"),
            finished(2, "completed"),
        ],
    )
    .await;
}

#[tokio::test]
async fn claude_approvals_accepted_for_the_session_declined_and_cancelled() {
    let events = scenario_for(
        &CLAUDE,
        "claude-approve",
        &[],
        &[
            send_message(1, "approve"),
            Wait(|e| e["type"] == "approval"),
            Answer("accept_session", none),
            finished(1, "completed"),
            send_message(2, "write"),
            WaitCount(|e| e["type"] == "approval", 2),
            Answer("decline", none),
            finished(2, "completed"),
            send_message(3, "approve"),
            WaitCount(|e| e["type"] == "approval", 3),
            Answer("cancel", none),
            finished(3, "interrupted"),
        ],
    )
    .await;
    let approval = events.iter().find(|e| e["type"] == "approval").unwrap();
    assert_eq!(approval["title"], "Allow Bash to remove output?");
    assert_eq!(
        approval["choices"],
        json!(["accept", "accept_session", "decline", "cancel"])
    );
    assert!(
        approval["scope"]["outside"]
            .as_array()
            .is_some_and(|o| !o.is_empty()),
        "{approval}"
    );
    // A tool with no suggestions offers no session-wide choice.
    let write = events
        .iter()
        .filter(|e| e["type"] == "approval")
        .nth(1)
        .unwrap();
    assert_eq!(write["choices"], json!(["accept", "decline", "cancel"]));
}

/// The "don't ask again" rule goes back to the client kept to this session,
/// and only the updates that are updates.
#[tokio::test]
async fn claude_a_session_wide_approval_stays_in_the_session() {
    let dir = scenario_dir_for(&CLAUDE, "claude-session-rule");
    let log = dir.join("native.jsonl");
    let run = drive(
        crate::claude_chat::adapter(&environment(&log), &[]),
        init_for(&CLAUDE, &dir, &[]),
        &[
            send_message(1, "approve"),
            Wait(|e| e["type"] == "approval"),
            Answer("accept_session", none),
            finished(1, "completed"),
        ],
        &log,
    )
    .await;
    let _ = std::fs::remove_dir_all(&dir);
    let answer = run
        .native
        .iter()
        .find(|m| m["type"] == "control_response")
        .expect("an answer");
    let result = &answer["response"]["response"];
    assert_eq!(result["behavior"], "allow");
    assert_eq!(result["toolUseID"], "tool-1");
    assert_eq!(
        result["updatedPermissions"],
        json!([{"type":"addRules","rules":[{"toolName":"Bash"}],"behavior":"allow","destination":"session"}])
    );
}

#[tokio::test]
async fn claude_questions_answered_refused_and_declined() {
    let events = scenario_for(
        &CLAUDE,
        "claude-ask",
        &[],
        &[
            send_message(1, "ask"),
            Wait(|e| e["type"] == "approval"),
            // Two answers to a single-choice question are refused...
            Answer(
                "accept",
                |_| json!({"question-0":["Brief","Long"],"question-1":["API"]}),
            ),
            Wait(|e| e["type"] == "error"),
            // ...and the question stays open for a valid answer.
            Answer(
                "accept",
                |_| json!({"question-0":["Brief"],"question-1":["API","UI"]}),
            ),
            finished(1, "completed"),
            send_message(2, "ask"),
            WaitCount(|e| e["type"] == "approval", 2),
            Answer("decline", none),
            finished(2, "completed"),
        ],
    )
    .await;
    let approval = events.iter().find(|e| e["type"] == "approval").unwrap();
    assert_eq!(
        approval["questions"][0]["options"][0],
        json!({"label":"Brief","description":"One line"})
    );
    assert_eq!(approval["questions"][1]["multiSelect"], true);
}

#[tokio::test]
async fn claude_requests_it_cannot_take() {
    let events = scenario_for(
        &CLAUDE,
        "claude-refused",
        &[],
        &[
            send_message(1, "ask-invalid"),
            finished(1, "completed"),
            send_message(2, "interaction"),
            finished(2, "completed"),
            send_message(3, "unsupported"),
            finished(3, "completed"),
        ],
    )
    .await;
    assert_eq!(events.iter().filter(|e| e["type"] == "error").count(), 3);
    assert!(!events.iter().any(|e| e["type"] == "approval"));
}

/// A native dialog must be handled in Claude Code itself: the turn is stopped
/// rather than the dialog answered.
#[tokio::test]
async fn claude_a_native_dialog_stops_the_turn() {
    scenario_for(
        &CLAUDE,
        "claude-dialog",
        &[],
        &[send_message(1, "dialog"), finished(1, "interrupted")],
    )
    .await;
}

#[tokio::test]
async fn claude_a_request_the_client_withdraws() {
    let events = scenario_for(
        &CLAUDE,
        "claude-withdrawn",
        &[],
        &[send_message(1, "withdrawn"), finished(1, "completed")],
    )
    .await;
    assert!(events.iter().any(|e| e["type"] == "approval_resolved"));
}

#[tokio::test]
async fn claude_subagents_and_background_tasks() {
    let events = scenario_for(
        &CLAUDE,
        "claude-subagent",
        &[],
        &[
            send_message(1, "subagent"),
            finished(1, "completed"),
            Wait(|e| e["type"] == "tool" && e["id"] == "task-1" && e["status"] == "failed"),
        ],
    )
    .await;
    let cards: Vec<&Value> = events.iter().filter(|e| e["id"] == "task-1").collect();
    // What the subagent said and ran, nested under its card.
    assert!(
        cards.iter().any(|e| e["activity"]
            == "Reading the tree\n\u{2192} Grep \u{00b7} TODO \u{00b7} src/lib.rs")
    );
    assert!(
        events
            .iter()
            .any(|e| e["id"] == "grep-1" && e["parent"] == "task-1" && e["status"] == "completed")
    );
    // A backgrounded call stays running after its placeholder result.
    assert!(
        cards.iter().any(|e| e["status"] == "running"
            && e["background"] == true
            && e.get("text").is_some())
    );
    // A repeated progress line is logged once.
    assert_eq!(
        cards
            .iter()
            .filter(|e| e["activity"] == "Searching")
            .count(),
        1
    );
    assert!(cards.iter().any(|e| e["status"] == "completed"
        && e["activity"] == "Found 1 TODO"
        && e["stats"] == json!({"tokens":2000,"tools":3,"ms":5000})));
}

/// A subagent resumed by SendMessage: its output went to the first card and
/// marked it running again, while the end went to the new card, so the first
/// sat in the dock as running, "no progress" for hours.
#[tokio::test]
async fn claude_a_resumed_subagent_ends_where_it_runs() {
    let events = scenario_for(
        &CLAUDE,
        "claude-resume",
        &[],
        &[
            send_message(1, "resume"),
            finished(1, "completed"),
            Wait(|e| e["id"] == "send-1" && e["status"] == "completed"),
            Sleep(200),
        ],
    )
    .await;
    let first: Vec<&Value> = events.iter().filter(|e| e["id"] == "task-1").collect();
    let ended = first
        .iter()
        .position(|e| e["status"] == "completed")
        .expect("the first run ends");
    assert!(
        first[ended..].iter().all(|e| e["status"] != "running"),
        "{first:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| e["id"] == "send-1" && e["activity"] == "Fixing again")
    );
    assert!(!events.iter().any(|e| e["activity"] == "Late line"));
}

#[tokio::test]
async fn claude_a_turn_the_client_starts_by_itself() {
    let events = scenario_for(
        &CLAUDE,
        "claude-background",
        &[],
        &[
            send_message(1, "background"),
            finished(1, "completed"),
            Wait(|e| {
                e["type"] == "turn"
                    && e["id"]
                        .as_str()
                        .is_some_and(|id| id.starts_with("background-"))
                    && e["status"] == "completed"
            }),
        ],
    )
    .await;
    assert!(
        events
            .iter()
            .any(|e| e["type"] == "message" && e["text"] == "Carried on")
    );
}

#[tokio::test]
async fn claude_failed_turns_say_why() {
    let events = scenario_for(
        &CLAUDE,
        "claude-fail",
        &[],
        &[
            send_message(1, "fail-auth"),
            finished(1, "failed"),
            send_message(2, "fail"),
            finished(2, "failed"),
            send_message(3, "result-only"),
            finished(3, "completed"),
            send_message(4, "fail-401"),
            finished(4, "failed"),
        ],
    )
    .await;
    let errors: Vec<&Value> = events.iter().filter(|e| e["type"] == "error").collect();
    assert!(
        errors[0]["message"]
            .as_str()
            .unwrap()
            .contains("authentication mode")
    );
    assert!(
        errors[1]["message"]
            .as_str()
            .unwrap()
            .contains("could not complete the turn")
    );
    // A result with no assistant message is the reply.
    assert!(
        events
            .iter()
            .any(|e| e["type"] == "message" && e["id"] == format!("assistant-{}", uuid(3)))
    );
    // A refused request says what the endpoint said, and its complaint is
    // not shown as the model's reply.
    let refused = errors[2]["message"].as_str().unwrap();
    assert!(
        refused.contains("(HTTP 401)") && refused.contains("401 bad key"),
        "{refused}"
    );
    assert!(!events.iter().any(|e| {
        e["type"] == "message"
            && e["text"]
                .as_str()
                .is_some_and(|t| t.contains("Failed to authenticate"))
    }));
}

#[tokio::test]
async fn claude_models_depths_and_permission_modes() {
    let events = scenario_for(
        &CLAUDE,
        "claude-settings",
        &[],
        &[
            Send(json!({"type":"model","model":"sonnet","effort":"high"})),
            Wait(|e| e["type"] == "settings" && e["model"] == "sonnet" && e["effort"] == "high"),
            // A depth-only change keeps the model.
            Send(json!({"type":"model","effort":"low"})),
            Wait(|e| e["type"] == "settings" && e["model"] == "sonnet" && e["effort"] == "low"),
            Send(json!({"type":"model","model":"refused-model"})),
            Wait(|e| e["type"] == "error"),
            Send(json!({"type":"permission","mode":"plan"})),
            Wait(|e| e["type"] == "settings" && e["permission_mode"] == "plan"),
            // The client changes its own mode; the interface follows it.
            send_message(1, "mode-change"),
            finished(1, "completed"),
            Wait(|e| e["type"] == "settings" && e["permission_mode"] == "accept_edits"),
            // The init message names the resolved model; the listed choice stays.
            send_message(2, "simple"),
            finished(2, "completed"),
            Send(json!({"type":"permission","mode":"danger"})),
            Wait(|e| e["type"] == "settings" && e["permission_mode"] == "danger"),
        ],
    )
    .await;
    assert!(
        !events
            .iter()
            .any(|e| e["type"] == "settings" && e["model"] == "claude-sonnet-5-fixture"),
        "the listed choice was replaced by the resolved id"
    );
    let refused = events.iter().find(|e| e["type"] == "error").unwrap();
    assert!(
        refused["message"]
            .as_str()
            .unwrap()
            .contains("availability probe refused")
    );
}

#[tokio::test]
async fn claude_a_resumed_session_with_its_model_and_depth() {
    let events = scenario_for(
        &CLAUDE,
        "claude-resume",
        &[
            "--resume",
            "claude-fixture-session",
            "--model",
            "opus",
            "--effort",
            "high",
        ],
        &[send_message(1, "simple"), finished(1, "completed")],
    )
    .await;
    assert!(
        events
            .iter()
            .any(|e| e["type"] == "settings" && e["model"] == "opus" && e["effort"] == "high")
    );
}

#[tokio::test]
async fn claude_a_launch_option_structured_mode_refuses() {
    let dir = scenario_dir_for(&CLAUDE, "claude-unusable-rust");
    let log = dir.join("native.jsonl");
    let rust = unlaunchable(
        crate::claude_chat::adapter(&environment(&log), &[]),
        init_for(&CLAUDE, &dir, &["--permission-mode", "bypassPermissions"]),
    )
    .await;
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        rust.iter().any(|e| e["type"] == "error"
            && e["message"] == "Structured mode does not enable permission bypass."),
        "{rust:?}"
    );
    assert!(rust.iter().any(|e| e["type"] == "exit"));
}

/// Claude's conversation ends with its dropped adapter too.
#[cfg(unix)]
#[tokio::test]
async fn claude_a_dropped_adapter_stops_claude() {
    let dir = scenario_dir_for(&CLAUDE, "claude-dropped-rust");
    let log = dir.join("native.jsonl");
    let mut adapter = crate::claude_chat::adapter(&environment(&log), &[]);
    adapter
        .start(&init_for(&CLAUDE, &dir, &[]))
        .await
        .expect("start");
    let mut pid = None;
    for _ in 0..50 {
        pid = adapter.pid();
        if pid.is_some() {
            break;
        }
        let _ = tokio::time::timeout(Duration::from_millis(100), adapter.next_event()).await;
    }
    let pid = pid.expect("a running client") as libc::pid_t;
    drop(adapter);
    let alive = || unsafe { libc::kill(pid, 0) } == 0;
    for _ in 0..50 {
        if !alive() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!alive(), "claude outlived its dropped adapter");
}

/// The real Claude Code, not the fixture: a short turn and an interrupted one,
/// through the Rust adapter. It uses the host's own
/// `claude` and sign-in and spends a little of its quota on the cheapest
/// model, so it only runs when asked: `cargo test -- --ignored real_claude`.
#[tokio::test]
#[ignore]
async fn real_claude_a_turn_and_an_interrupt() {
    let program = std::env::var("TENDLY_REAL_CLAUDE").unwrap_or_else(|_| "claude".into());
    async fn next<A: ClientAdapter>(adapter: &mut A, events: &mut Vec<Value>, wait: u64) -> bool {
        match tokio::time::timeout(Duration::from_secs(wait), adapter.next_event()).await {
            Ok(Some(Ok(event))) => {
                events.extend(crate::conversations::normalize_event(event));
                true
            }
            _ => false,
        }
    }
    async fn until<A: ClientAdapter>(
        adapter: &mut A,
        events: &mut Vec<Value>,
        what: &str,
        done: impl Fn(&Value) -> bool,
    ) {
        let deadline = std::time::Instant::now() + Duration::from_secs(120);
        while !events.iter().any(&done) {
            assert!(
                std::time::Instant::now() < deadline && next(adapter, events, 120).await,
                "{what}: {events:#?}"
            );
        }
    }
    async fn run<A: ClientAdapter>(mut adapter: A, program: &str, layer: &str) -> Vec<Value> {
        let dir = std::env::temp_dir().join(format!(
            "tendly-real-claude-{layer}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let init = ChatInit {
            provider: "claude_code".into(),
            program: program.into(),
            args: std::env::var("TENDLY_REAL_CLAUDE_ARGS").map_or_else(
                |_| vec!["--model".into(), "haiku".into()],
                |args| args.split_whitespace().map(str::to_owned).collect(),
            ),
            cwd: dir.to_string_lossy().into_owned(),
            resume_id: None,
        };
        adapter.start(&init).await.expect("start");
        let mut events = Vec::new();
        until(&mut adapter, &mut events, "models", |e| {
            e["type"] == "settings" && e.get("models").is_some()
        })
        .await;
        let one = uuid(1);
        dispatch(&mut adapter, ChatCommand::parse(&json!({"type":"message","id":one,"content":"Reply with exactly the word pong and nothing else."})).unwrap()).await.unwrap();
        until(&mut adapter, &mut events, "first turn", |e| {
            e["type"] == "turn" && e["id"] == uuid(1) && e["status"] != "running"
        })
        .await;
        let two = uuid(2);
        dispatch(&mut adapter, ChatCommand::parse(&json!({"type":"message","id":two,"content":"Write the numbers from 1 to 400, one per line, with no other text."})).unwrap()).await.unwrap();
        until(&mut adapter, &mut events, "second reply", |e| {
            e["type"] == "message" && e["delta"] == true && events_after_two(e)
        })
        .await;
        dispatch(
            &mut adapter,
            ChatCommand::parse(&json!({"type":"interrupt"})).unwrap(),
        )
        .await
        .unwrap();
        until(&mut adapter, &mut events, "interrupt", |e| {
            e["type"] == "turn" && e["id"] == uuid(2) && e["status"] != "running"
        })
        .await;
        // A tool call: approved here if the host's settings ask for it.
        let three = uuid(3);
        dispatch(&mut adapter, ChatCommand::parse(&json!({"type":"message","id":three,"content":"Use the Bash tool to run exactly this command: touch tendly-smoke.txt && echo tendly-smoke. Then reply done."})).unwrap()).await.unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(180);
        let mut answered = 0;
        loop {
            if events
                .iter()
                .any(|e| e["type"] == "turn" && e["id"] == uuid(3) && e["status"] != "running")
            {
                break;
            }
            let open: Vec<Value> = events
                .iter()
                .filter(|e| e["type"] == "approval")
                .skip(answered)
                .cloned()
                .collect();
            for approval in open {
                answered += 1;
                dispatch(&mut adapter, ChatCommand::parse(&json!({"type":"approval","request_id":approval["id"],"decision":"accept","answers":{}})).unwrap()).await.unwrap();
            }
            assert!(
                std::time::Instant::now() < deadline && next(&mut adapter, &mut events, 120).await,
                "tool turn: {events:#?}"
            );
        }
        adapter.shutdown().await;
        let _ = std::fs::remove_dir_all(&dir);
        events
    }
    // A delta of any assistant message: the second turn is the only one running.
    fn events_after_two(_: &Value) -> bool {
        true
    }
    let rust = run(
        crate::claude_chat::adapter(&BTreeMap::new(), &[]),
        &program,
        "rust",
    )
    .await;
    // The real client's approval, as each layer shows it, apart from its id
    // and the run's directory.
    let shown = |events: &[Value]| -> Vec<Value> {
        events
            .iter()
            .filter(|e| e["type"] == "approval")
            .map(|e| {
                let mut e = e.clone();
                e["id"] = json!("approval");
                let text = e.to_string();
                let text = regex_free_dirs(&text);
                serde_json::from_str(&text).unwrap()
            })
            .collect()
    };
    fn regex_free_dirs(text: &str) -> String {
        // Each layer ran in its own temporary directory.
        let mut out = String::new();
        let mut rest = text;
        while let Some(at) = rest.find("tendly-real-claude-") {
            out.push_str(&rest[..at]);
            let tail = &rest[at..];
            let end = tail.find(['"', '/', '\\']).unwrap_or(tail.len());
            out.push_str("<dir>");
            rest = &tail[end..];
        }
        out.push_str(rest);
        out
    }
    eprintln!("approval: {:#}", json!(shown(&rust)));
    {
        let (layer, events) = ("rust", &rust);
        let status = |id: &str| {
            events
                .iter()
                .rev()
                .find(|e| e["type"] == "turn" && e["id"] == id)
                .map(|e| e["status"].clone())
        };
        assert!(
            events.iter().any(|e| e["type"] == "ready"),
            "{layer}: not ready"
        );
        assert_eq!(
            status(&uuid(1)),
            Some(json!("completed")),
            "{layer}: {events:#?}"
        );
        let reply: String = events
            .iter()
            .filter(|e| e["type"] == "message" && e["delta"] == false && e["role"] == "assistant")
            .filter_map(|e| e["text"].as_str())
            .collect();
        assert!(reply.to_lowercase().contains("pong"), "{layer}: {reply}");
        assert!(
            events
                .iter()
                .any(|e| e["type"] == "usage" && e.get("turn").is_some()),
            "{layer}: no turn usage"
        );
        assert_eq!(
            status(&uuid(2)),
            Some(json!("interrupted")),
            "{layer}: {events:#?}"
        );
        assert_eq!(
            status(&uuid(3)),
            Some(json!("completed")),
            "{layer}: {events:#?}"
        );
        let ran = events.iter().any(|e| {
            e["type"] == "tool"
                && e["status"] == "completed"
                && e["text"]
                    .as_str()
                    .is_some_and(|t| t.contains("tendly-smoke"))
        });
        assert!(
            ran,
            "{layer}: the command's output never showed: {events:#?}"
        );
        eprintln!(
            "{layer}: approvals {}",
            events.iter().filter(|e| e["type"] == "approval").count()
        );
        eprintln!(
            "{layer}: {} events, kinds {:?}",
            events.len(),
            events
                .iter()
                .filter_map(|e| e["type"].as_str())
                .collect::<std::collections::BTreeSet<_>>()
        );
    }
}
