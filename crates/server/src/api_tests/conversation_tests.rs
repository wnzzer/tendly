use super::*;

fn chat_fixture() -> Fixture {
    let mut fixture = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    fixture.state.chat_fixture = Some(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../packages/native-bridge/fixtures/chat-supervisor.mjs"),
    );
    fixture
}
async fn new_chat(fixture: &Fixture) -> Uuid {
    let workspace = fixture
        .state
        .store
        .create_workspace("Chat fixture", fixture.path.join("repo").to_str().unwrap())
        .unwrap();
    let (status,session)=call(fixture.app(),"POST",&format!("/api/workspaces/{}/sessions",workspace.id),json!({"provider":"codex","title":"Fixture chat","interaction_mode":"structured","environment":{"OLD_ACCOUNT_VALUE":{"kind":"literal","value":"not-carried"}}})).await;
    assert_eq!(status, StatusCode::CREATED, "{session}");
    assert_eq!(session["interaction_mode"], "structured");
    Uuid::parse_str(session["id"].as_str().unwrap()).unwrap()
}
async fn wait_event(fixture: &Fixture, id: Uuid, kind: &str, status: Option<&str>) -> Vec<Value> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let events = fixture.state.store.conversation(id).unwrap().events;
            if events
                .iter()
                .any(|e| e["type"] == kind && status.is_none_or(|s| e["status"] == s))
            {
                return events;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("fixture event deadline")
}

#[tokio::test]
async fn structured_conversation_open_is_prompt_free_and_messages_are_durable_and_idempotent() {
    let fixture = chat_fixture();
    let id = new_chat(&fixture).await;
    let (status, snapshot) = call(
        fixture.app(),
        "GET",
        &format!("/api/sessions/{id}/conversation"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(snapshot["mode"], "structured");
    assert_eq!(snapshot["running"], false);
    assert_eq!(snapshot["events"], json!([]));
    let (status, session) = call(
        fixture.app(),
        "POST",
        &format!("/api/sessions/{id}/start"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{session}");
    assert_eq!(session["status"], "running");
    let initial = fixture.state.store.conversation(id).unwrap().events;
    assert!(initial.iter().all(|e| e["type"] == "ready"));
    let message_id = Uuid::new_v4().to_string();
    let request = json!({"id":message_id,"content":"HELLO"});
    assert_eq!(
        call(
            fixture.app(),
            "POST",
            &format!("/api/sessions/{id}/conversation/message"),
            request.clone()
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    let events = wait_event(&fixture, id, "turn", Some("completed")).await;
    assert_eq!(
        events
            .iter()
            .filter(|e| e["type"] == "message" && e["role"] == "user")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e["type"] == "message" && e["role"] == "assistant")
            .map(|e| e["text"].as_str().unwrap())
            .collect::<String>(),
        "fixture reply"
    );
    assert_eq!(
        call(
            fixture.app(),
            "POST",
            &format!("/api/sessions/{id}/conversation/message"),
            request
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    assert_eq!(
        fixture.state.store.conversation(id).unwrap().events.len(),
        events.len()
    );
    assert_eq!(
        call(
            fixture.app(),
            "POST",
            &format!("/api/sessions/{id}/conversation/message"),
            json!({"id":message_id,"content":"different"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            fixture.app(),
            "POST",
            &format!("/api/sessions/{id}/stop"),
            json!({})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(!fixture.state.chats.get(id).unwrap().running());
    assert_eq!(
        fixture
            .state
            .store
            .get_session(id)
            .unwrap()
            .unwrap()
            .provider_session_id
            .as_deref(),
        Some("fixture-native-thread")
    );
    assert!(
        !fixture
            .state
            .store
            .conversation(id)
            .unwrap()
            .events
            .is_empty()
    );
}

#[tokio::test]
async fn native_approvals_require_explicit_supported_decisions_and_block_endpoint_switches() {
    let fixture = chat_fixture();
    let id = new_chat(&fixture).await;
    let base = format!("/api/sessions/{id}");
    assert_eq!(
        call(
            fixture.app(),
            "POST",
            &(base.clone() + "/conversation/message"),
            json!({"id":Uuid::new_v4(),"content":"APPROVAL"})
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    wait_event(&fixture, id, "approval", None).await;
    assert_eq!(
        call(
            fixture.app(),
            "PATCH",
            &(base.clone() + "/configuration"),
            json!({"endpoint_profile_id":null,"confirmed":true})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            fixture.app(),
            "POST",
            &(base.clone() + "/conversation/approval"),
            json!({"request_id":"fixture-approval","decision":"acceptForSession"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            fixture.app(),
            "POST",
            &(base.clone() + "/conversation/approval"),
            json!({"request_id":"other-session-request","decision":"accept"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            fixture.app(),
            "POST",
            &(base.clone() + "/conversation/approval"),
            json!({"request_id":"fixture-approval","decision":"decline"})
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    let events = wait_event(&fixture, id, "turn", Some("completed")).await;
    assert!(
        events
            .iter()
            .any(|e| e["type"] == "tool" && e["status"] == "failed" && e["text"] == "decline")
    );
    assert_eq!(
        call(
            fixture.app(),
            "POST",
            &(base.clone() + "/conversation/approval"),
            json!({"request_id":"fixture-approval","decision":"accept"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    fixture.state.chats.stop(id).await.unwrap();
}

#[tokio::test]
async fn interrupt_cancels_a_turn_not_the_session_and_allows_a_confirmed_fresh_endpoint_context() {
    let fixture = chat_fixture();
    let id = new_chat(&fixture).await;
    let base = format!("/api/sessions/{id}");
    let (_,profile)=call(fixture.app(),"POST","/api/endpoint-profiles",json!({"name":"Next account","provider":"codex","endpoint_url":"https://fixture.invalid/v1","environment":{"NEW_ACCOUNT_VALUE":{"kind":"literal","value":"new"}}})).await;
    assert_eq!(
        call(
            fixture.app(),
            "POST",
            &(base.clone() + "/conversation/message"),
            json!({"id":Uuid::new_v4(),"content":"WAIT"})
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    wait_event(&fixture, id, "turn", Some("running")).await;
    assert_eq!(
        call(
            fixture.app(),
            "POST",
            &(base.clone() + "/conversation/message"),
            json!({"id":Uuid::new_v4(),"content":"must-not-queue"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            fixture.app(),
            "POST",
            &(base.clone() + "/conversation/interrupt"),
            json!({})
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    wait_event(&fixture, id, "turn", Some("interrupted")).await;
    assert!(fixture.state.chats.get(id).unwrap().running());
    assert_eq!(
        call(
            fixture.app(),
            "PATCH",
            &(base.clone() + "/configuration"),
            json!({"endpoint_profile_id":profile["id"],"confirmed":false})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (status, changed) = call(
        fixture.app(),
        "PATCH",
        &(base.clone() + "/configuration"),
        json!({"endpoint_profile_id":profile["id"],"confirmed":true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{changed}");
    assert_eq!(changed["status"], "stopped");
    assert_eq!(changed["configuration_revision"], 1);
    assert_eq!(changed["provider_session_id"], Value::Null);
    assert_eq!(
        changed["environment"],
        json!({"NEW_ACCOUNT_VALUE":{"kind":"literal","value":"new"}})
    );
    assert!(!fixture.state.chats.get(id).unwrap().running());
    assert!(
        fixture
            .state
            .store
            .conversation(id)
            .unwrap()
            .events
            .iter()
            .any(|e| e["type"] == "configuration")
    );
    assert_eq!(
        call(
            fixture.app(),
            "POST",
            &(base.clone() + "/conversation/message"),
            json!({"id":Uuid::new_v4(),"content":"HELLO"})
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    assert!(
        fixture
            .state
            .state_dir
            .join("sessions")
            .join(id.to_string())
            .join("configurations/1/config.toml")
            .exists()
    );
    fixture.state.chats.stop(id).await.unwrap();
}

#[tokio::test]
async fn chat_conversion_and_configuration_do_not_take_over_a_live_pty_or_change_cli() {
    let fixture = chat_fixture();
    let workspace = fixture
        .state
        .store
        .create_workspace("Fixture", fixture.path.join("repo").to_str().unwrap())
        .unwrap();
    let record = fixture
        .state
        .store
        .create_session(workspace.id, ProviderKind::Codex, "Existing PTY")
        .unwrap();
    let id = record.id;
    let spec = tendly_runtime::SpawnSpec {
        program: waiting_process(&["-c", "read line"]).0,
        args: waiting_process(&["-c", "read line"]).1,
        cwd: fixture.path.join("repo"),
        env: Default::default(),
        env_remove: vec![],
    };
    fixture
        .state
        .runtime
        .start(id.to_string(), spec)
        .await
        .unwrap(); // DB deliberately still stopped: live-runtime check must win.
    assert_eq!(
        call(
            fixture.app(),
            "POST",
            &format!("/api/sessions/{id}/conversation/open"),
            json!({})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            fixture.app(),
            "PATCH",
            &format!("/api/sessions/{id}/configuration"),
            json!({"endpoint_profile_id":null,"confirmed":true})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert!(
        fixture
            .state
            .runtime
            .get(&id.to_string())
            .unwrap()
            .running()
    );
    fixture.state.runtime.shutdown().await;
    let (_, foreign) = call(
        fixture.app(),
        "POST",
        "/api/endpoint-profiles",
        json!({"name":"Other CLI","provider":"claude_code"}),
    )
    .await;
    assert_eq!(
        call(
            fixture.app(),
            "PATCH",
            &format!("/api/sessions/{id}/configuration"),
            json!({"endpoint_profile_id":foreign["id"],"confirmed":true})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        fixture
            .state
            .store
            .get_session(id)
            .unwrap()
            .unwrap()
            .interaction_mode,
        InteractionMode::Pty
    );
}

#[tokio::test]
async fn chat_and_configuration_routes_require_existing_authentication() {
    let fixture = Fixture::new(
        "127.0.0.1:8787".parse().unwrap(),
        Some("fixture-auth-token-long-enough"),
    );
    let id = Uuid::new_v4();
    for (method, suffix, body) in [
        ("GET", "conversation", Value::Null),
        ("POST", "conversation/open", json!({})),
        (
            "POST",
            "conversation/message",
            json!({"id":Uuid::new_v4(),"content":"not sent"}),
        ),
        (
            "PATCH",
            "configuration",
            json!({"confirmed":true,"endpoint_profile_id":null}),
        ),
    ] {
        assert_eq!(
            call(
                fixture.app(),
                method,
                &format!("/api/sessions/{id}/{suffix}"),
                body
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
}

#[tokio::test]
async fn structured_websocket_only_attaches_and_disconnection_keeps_native_process_alive() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut fixture = chat_fixture();
    fixture.state.security = security::Security::for_test(address, None);
    let workspace = fixture
        .state
        .store
        .create_workspace(
            "Socket fixture",
            fixture.path.join("repo").to_str().unwrap(),
        )
        .unwrap();
    let session = fixture
        .state
        .store
        .create_session(workspace.id, ProviderKind::Codex, "Socket fixture")
        .unwrap();
    let id = session.id;
    fixture
        .state
        .store
        .set_interaction_mode(id, InteractionMode::Structured)
        .unwrap();
    let app = fixture.app();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let url = format!("ws://{address}/api/sessions/{id}/chat/ws");
    let (mut ws, _) = connect_async(&url).await.unwrap();
    let first = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let snapshot: Value = serde_json::from_str(first.to_text().unwrap()).unwrap();
    assert_eq!(snapshot["running"], false);
    assert!(fixture.state.chats.get(id).is_none());
    // A browser cannot send a WebSocket ping, so the view probes with its own.
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"type":"ping"}).to_string().into(),
    ))
    .await
    .unwrap();
    let pong = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(pong.to_text().unwrap(), r#"{"type":"pong"}"#);
    ws.close(None).await.unwrap();
    {
        let _guard = fixture.state.operations.lock().await;
        conversations::start_locked(
            &fixture.state,
            &fixture.state.store.get_session(id).unwrap().unwrap(),
        )
        .await
        .unwrap();
    }
    let (mut attached, _) = connect_async(&url).await.unwrap();
    let first = tokio::time::timeout(Duration::from_secs(2), attached.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let snapshot: Value = serde_json::from_str(first.to_text().unwrap()).unwrap();
    assert_eq!(snapshot["running"], true);
    assert_eq!(snapshot["events"][0]["type"], "ready");
    attached.close(None).await.unwrap();
    assert!(fixture.state.chats.get(id).unwrap().running());
    let (mut again, _) = connect_async(&url).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), again.next())
            .await
            .unwrap()
            .unwrap()
            .is_ok()
    );
    again.close(None).await.unwrap();
    fixture.state.chats.stop(id).await.unwrap();
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn managed_account_profiles_cannot_be_orphaned_by_generic_profile_deletion() {
    let fixture = chat_fixture();
    fs::create_dir_all(&fixture.state.state_dir).unwrap();
    let (status, account) = call(
        fixture.app(),
        "POST",
        "/api/accounts",
        json!({"name":"Fixture managed account","provider":"codex"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{account}");
    let profile = account["profile_id"].as_str().unwrap();
    assert_eq!(
        call(
            fixture.app(),
            "DELETE",
            &format!("/api/endpoint-profiles/{profile}"),
            Value::Null
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert!(
        fixture
            .state
            .store
            .get_endpoint_profile(Uuid::parse_str(profile).unwrap())
            .unwrap()
            .is_some()
    );
    assert_eq!(account["status"], "unknown"); // Creation is not a login or account probe.
}

#[tokio::test]
async fn typed_command_receipts_are_readable_and_stale_configuration_cannot_dispatch_a_message() {
    let fixture = chat_fixture();
    let id = new_chat(&fixture).await;
    let path = format!("/api/sessions/{id}/conversation/message");
    fixture
        .state
        .store
        .switch_session_configuration(id, None, None)
        .unwrap();
    let message_id = Uuid::new_v4();
    let (status, _) = call(
        fixture.app(),
        "POST",
        &path,
        json!({"id":message_id,"content":"not another account","configuration_revision":0}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(fixture.state.chats.get(id).is_none());
    let input = json!({"id":message_id,"content":"intended account","configuration_revision":1});
    let (status, receipt) = call(fixture.app(), "POST", &path, input.clone()).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(receipt["accepted"], true);
    assert_eq!(receipt["id"], message_id.to_string());
    assert_eq!(receipt["duplicate"], false);
    let (_, repeat) = call(fixture.app(), "POST", &path, input).await;
    assert_eq!(repeat["duplicate"], true);
    let (_, interrupt) = call(
        fixture.app(),
        "POST",
        &format!("/api/sessions/{id}/conversation/interrupt"),
        json!({}),
    )
    .await;
    assert_eq!(interrupt["accepted"], true);
    fixture.state.chats.stop(id).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn bridge_exit_closes_its_owned_group_and_records_only_one_exit() {
    let mut fixture = chat_fixture();
    let bridge = fixture.path.join("owned-group-fixture.mjs");
    fs::write(&bridge,r#"import {spawn} from 'node:child_process';
import {writeFileSync} from 'node:fs';
import {join} from 'node:path';
import {createInterface} from 'node:readline';
process.on('SIGTERM',()=>process.exit(0));
for await (const line of createInterface({input:process.stdin})) {
  const job=JSON.parse(line);
  if(job.type==='init') {
    const child=spawn('/bin/sh',['-c','trap "" TERM; printf ready; while :; do sleep 10; done'],{stdio:['ignore','pipe','ignore']});
    writeFileSync(join(job.cwd,'owned-child.pid'),String(child.pid));
    child.stdout.once('data',()=>process.stdout.write('{"type":"ready"}\n'));
  }
  if(job.type==='message') process.stdout.write('{"type":"exit"}\n');
  if(job.type==='shutdown') process.exit(0);
}
"#).unwrap();
    fixture.state.chat_fixture = Some(bridge);
    let id = new_chat(&fixture).await;
    assert_eq!(
        call(
            fixture.app(),
            "POST",
            &format!("/api/sessions/{id}/conversation/message"),
            json!({"id":Uuid::new_v4(),"content":"synthetic exit"})
        )
        .await
        .0,
        StatusCode::ACCEPTED
    );
    let events = wait_event(&fixture, id, "exit", None).await;
    assert_eq!(
        events
            .iter()
            .filter(|event| event["type"] == "exit")
            .count(),
        1
    );
    assert!(!fixture.state.chats.get(id).unwrap().running());
    let pid = fs::read_to_string(fixture.path.join("repo/owned-child.pid")).unwrap();
    let output = Command::new("ps")
        .args(["-o", "stat=", "-p", pid.trim()])
        .output()
        .unwrap();
    let status = String::from_utf8_lossy(&output.stdout);
    assert!(
        status.trim().is_empty() || status.trim().starts_with('Z'),
        "owned descendant still executing: {status}"
    );
}

/// A running structured session and a token that speaks for it.
async fn agent_chat(fixture: &Fixture) -> (Uuid, String) {
    let id = new_chat(fixture).await;
    let (status, _) = call(
        fixture.app(),
        "POST",
        &format!("/api/sessions/{id}/start"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let session = fixture.state.store.get_session(id).unwrap().unwrap();
    // A fresh launch spec reissues the session's token; the chat is running,
    // so the new one identifies it.
    let token = launch::build(&fixture.state, &session, fixture.path.join("repo"))
        .unwrap()
        .env["TENDLY_AGENT_TOKEN"]
        .clone();
    (id, token)
}

async fn tool(fixture: &Fixture, token: &str, name: &str, arguments: Value) -> Value {
    agent_request(
        fixture.app(),
        "POST",
        "/api/agent/call",
        Some(token),
        Some("agent"),
        json!({"name":name,"arguments":arguments}),
    )
    .await
    .1
}

#[tokio::test]
async fn a_session_starts_another_follows_its_reply_and_only_its_own() {
    let fixture = chat_fixture();
    let (parent, token) = agent_chat(&fixture).await;
    let mut events = fixture.state.activity.subscribe();

    // The first start is the person's call.
    let app = fixture.app();
    let spawn = {
        let token = token.clone();
        tokio::spawn(async move {
            agent_request(app, "POST", "/api/agent/call", Some(&token), Some("agent"), json!({"name":"tendly_spawn","arguments":{"provider":"codex","prompt":"Try the other approach","title":"Other approach"}})).await.1
        })
    };
    let request = next_event(&mut events, "request").await;
    assert_eq!(request["title"], "Start a {client} session");
    assert_eq!(request["who"]["session_id"], parent.to_string());
    call(
        fixture.app(),
        "POST",
        &format!(
            "/api/agent-activity/requests/{}",
            request["id"].as_str().unwrap()
        ),
        json!({"approve":true}),
    )
    .await;
    let started = spawn.await.unwrap();
    assert_eq!(started["isError"], false, "{started}");
    let child: Uuid = tool_text(&started)["started"]["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        fixture.state.store.agent_parent(child).unwrap(),
        Some(parent)
    );
    let session = fixture.state.store.get_session(child).unwrap().unwrap();
    assert!(
        session.ephemeral,
        "a started session is temporary unless kept"
    );
    let notice = next_event(&mut events, "notice").await;
    assert_eq!(notice["open"]["session_id"], child.to_string());

    let waited = tool_text(
        &tool(
            &fixture,
            &token,
            "tendly_session",
            json!({"action":"wait","id":child,"seconds":5}),
        )
        .await,
    );
    assert_eq!(waited["finished_waiting"], true, "{waited}");
    assert_eq!(waited["last_reply"], "fixture reply");
    assert_eq!(waited["last_turn"], "completed");

    let sent = tool(
        &fixture,
        &token,
        "tendly_session",
        json!({"action":"message","id":child,"text":"And now?"}),
    )
    .await;
    assert_eq!(sent["isError"], false, "{sent}");
    let listed =
        tool_text(&tool(&fixture, &token, "tendly_session", json!({"action":"list"})).await);
    assert_eq!(listed[0]["id"], child.to_string());

    // After the first, no more questions.
    let again = tool(
        &fixture,
        &token,
        "tendly_spawn",
        json!({"provider":"codex","prompt":"A third look"}),
    )
    .await;
    assert_eq!(again["isError"], false, "{again}");

    // A started session cannot start more, nor read what it did not start.
    let child_session = fixture.state.store.get_session(child).unwrap().unwrap();
    let child_token = launch::build(&fixture.state, &child_session, fixture.path.join("repo"))
        .unwrap()
        .env["TENDLY_AGENT_TOKEN"]
        .clone();
    let refused = tool(
        &fixture,
        &child_token,
        "tendly_spawn",
        json!({"provider":"codex","prompt":"Grandchild"}),
    )
    .await;
    assert_eq!(refused["isError"], true);
    let prying = tool(
        &fixture,
        &child_token,
        "tendly_session",
        json!({"action":"result","id":parent}),
    )
    .await;
    assert_eq!(prying["isError"], true);
    assert!(
        prying["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("not a session you started")
    );

    // Pointing at a file reaches the window; the usage of a native sign-in is unknown.
    let shown = tool(
        &fixture,
        &token,
        "tendly_show",
        json!({"file":"README.md","line":3}),
    )
    .await;
    assert_eq!(shown["isError"], false, "{shown}");
    let show = next_event(&mut events, "show").await;
    assert_eq!(show["target"]["kind"], "file");
    assert_eq!(show["target"]["line"], 3);
    let usage = tool_text(&tool(&fixture, &token, "tendly_usage", json!({})).await);
    assert_eq!(usage["known"], false);

    let renamed = tool(
        &fixture,
        &token,
        "tendly_session",
        json!({"action":"rename","title":"Lead"}),
    )
    .await;
    assert_eq!(renamed["isError"], false);
    assert_eq!(
        fixture
            .state
            .store
            .get_session(parent)
            .unwrap()
            .unwrap()
            .title,
        "Lead"
    );
    fixture.state.chats.shutdown().await;
}

#[tokio::test]
async fn an_isolated_child_works_in_a_worktree_of_its_own() {
    let fixture = chat_fixture();
    let repo = fixture.path.join("repo");
    for args in [
        vec!["checkout", "-qb", "main"],
        vec!["commit", "-q", "--allow-empty", "-m", "start"],
    ] {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(&repo)
                .status()
                .unwrap()
                .success()
        );
    }
    let (parent, token) = agent_chat(&fixture).await;
    let mut events = fixture.state.activity.subscribe();
    let app = fixture.app();
    let spawn = {
        let token = token.clone();
        tokio::spawn(async move {
            agent_request(app, "POST", "/api/agent/call", Some(&token), Some("agent"), json!({"name":"tendly_spawn","arguments":{"provider":"codex","prompt":"Fix it on the side","title":"Side fix","isolated":true}})).await.1
        })
    };
    let request = next_event(&mut events, "request").await;
    assert!(
        request["rows"].to_string().contains("Worktree"),
        "the person sees that it gets a worktree: {request}"
    );
    call(
        fixture.app(),
        "POST",
        &format!(
            "/api/agent-activity/requests/{}",
            request["id"].as_str().unwrap()
        ),
        json!({"approve":true}),
    )
    .await;
    let started = spawn.await.unwrap();
    assert_eq!(started["isError"], false, "{started}");
    let started = tool_text(&started)["started"].clone();
    assert_eq!(started["branch"], "tendly/side-fix");
    assert_eq!(started["base"], "main");
    let child: Uuid = started["id"].as_str().unwrap().parse().unwrap();
    let session = fixture.state.store.get_session(child).unwrap().unwrap();
    assert_eq!(session.checkout_base.as_deref(), Some("main"));
    assert!(
        std::path::Path::new(session.checkout_path.as_deref().unwrap())
            .ends_with("repo.worktrees/tendly-side-fix")
    );

    // Each side knows where it works and with whom.
    let child_token = launch::build(
        &fixture.state,
        &session,
        PathBuf::from(session.checkout_path.clone().unwrap()),
    )
    .unwrap()
    .env["TENDLY_AGENT_TOKEN"]
        .clone();
    let theirs = tool_text(&tool(&fixture, &child_token, "tendly_status", json!({})).await);
    assert_eq!(
        theirs["you"]["checkout"]["isolated_from"], "main",
        "{theirs}"
    );
    assert_eq!(theirs["you"]["checkout"]["branch"], "tendly/side-fix");
    let mine = tool_text(&tool(&fixture, &token, "tendly_status", json!({})).await);
    assert_eq!(mine["you"]["checkout"]["branch"], "main", "{mine}");
    assert_eq!(mine["you"]["checkout"]["worktree"], false);
    assert!(mine["you"]["checkout"]["isolated_from"].is_null());
    let _ = parent;
    fixture.state.chats.shutdown().await;
}

/// A client that is ready at once and takes the adapter's full shutdown
/// allowance (NativeAdapter waits up to 6 s for its task) to stop.
struct SlowToStop {
    ready: bool,
}
impl crate::chat_adapter::ClientAdapter for SlowToStop {
    fn pid(&self) -> Option<u32> {
        None
    }
    async fn start(
        &mut self,
        _: &crate::chat_adapter::ChatInit,
    ) -> std::result::Result<(), crate::chat_adapter::Failure> {
        Ok(())
    }
    async fn message(
        &mut self,
        _: String,
        _: String,
    ) -> std::result::Result<(), crate::chat_adapter::Failure> {
        Ok(())
    }
    async fn steer(
        &mut self,
        _: String,
        _: String,
    ) -> std::result::Result<(), crate::chat_adapter::Failure> {
        Ok(())
    }
    async fn approve(
        &mut self,
        _: String,
        _: crate::chat_adapter::Decision,
        _: Option<crate::chat_adapter::Answers>,
    ) -> std::result::Result<(), crate::chat_adapter::Failure> {
        Ok(())
    }
    async fn interrupt(&mut self) -> std::result::Result<(), crate::chat_adapter::Failure> {
        Ok(())
    }
    async fn set_model(
        &mut self,
        _: Option<String>,
        _: Option<String>,
    ) -> std::result::Result<(), crate::chat_adapter::Failure> {
        Ok(())
    }
    async fn set_permission_mode(
        &mut self,
        _: String,
    ) -> std::result::Result<(), crate::chat_adapter::Failure> {
        Ok(())
    }
    async fn next_event(
        &mut self,
    ) -> Option<std::result::Result<Value, crate::chat_adapter::Failure>> {
        if !self.ready {
            self.ready = true;
            return Some(Ok(json!({"type":"ready"})));
        }
        std::future::pending().await
    }
    async fn shutdown(self) {
        tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    }
}

/// Stopping a conversation waits for a client that uses the whole time its
/// adapter allows to shut down, instead of answering "still shutting down"
/// while it is still on its way out -- which made a restart fail.
#[tokio::test]
async fn stopping_waits_for_a_client_that_takes_its_full_shutdown_time() {
    let fixture = chat_fixture();
    let id = new_chat(&fixture).await;
    let session = session_record(&fixture.state, id).await.unwrap();
    let init = crate::chat_adapter::ChatInit {
        provider: "codex".into(),
        program: "codex".into(),
        args: Vec::new(),
        cwd: fixture.path.to_string_lossy().into_owned(),
        resume_id: None,
    };
    crate::conversations::run_adapter(&fixture.state, &session, SlowToStop { ready: false }, init)
        .await
        .expect("started");
    let started = std::time::Instant::now();
    let stopped = fixture.state.chats.stop(session.id).await;
    assert!(
        stopped.is_ok(),
        "stop gave up after {:?}",
        started.elapsed()
    );
}

/// History is read in pages: the latest with the state it needs, older ones
/// by cursor; a socket resumes after the last event a view holds, or sends
/// the latest page to a view too far behind or holding an event past the end.
#[tokio::test]
async fn conversations_read_in_pages_and_sockets_resume_where_a_view_left_off() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut fixture = chat_fixture();
    let workspace = fixture
        .state
        .store
        .create_workspace("Pages", fixture.path.join("repo").to_str().unwrap())
        .unwrap();
    let id = fixture
        .state
        .store
        .create_session(workspace.id, ProviderKind::Codex, "Pages")
        .unwrap()
        .id;
    fixture
        .state
        .store
        .set_interaction_mode(id, InteractionMode::Structured)
        .unwrap();
    let store = &fixture.state.store;
    store
        .append_conversation_event(
            id,
            json!({"type":"settings","model":"m","models":[{"id":"m"}]}),
        )
        .unwrap();
    for n in 0..7000 {
        store
            .append_conversation_event(
                id,
                json!({"type":"tool","id":format!("t{n}"),"name":"Bash","status":"completed"}),
            )
            .unwrap();
    }
    let (status, latest) = call(
        fixture.app(),
        "GET",
        &format!("/api/sessions/{id}/conversation"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{latest}");
    let events = latest["events"].as_array().unwrap();
    assert_eq!(
        events.len(),
        1001,
        "the latest thousand and the settings they need"
    );
    assert_eq!(events[0]["type"], "settings");
    assert_eq!(latest["older"], 6002);
    let (_, older) = call(
        fixture.app(),
        "GET",
        &format!("/api/sessions/{id}/conversation?before=6002&limit=50"),
        Value::Null,
    )
    .await;
    assert_eq!(older["type"], "page");
    assert_eq!(older["events"].as_array().unwrap().len(), 50);
    assert_eq!(older["events"][0]["seq"], 5952);
    assert_eq!(older["older"], 5952);
    let (status, _) = call(
        fixture.app(),
        "GET",
        &format!("/api/sessions/{id}/conversation?before=x"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    fixture.state.security = security::Security::for_test(address, None);
    let app = fixture.app();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let greet = |after: Option<u64>| async move {
        let query = after.map_or(String::new(), |a| format!("?after={a}"));
        let (mut ws, _) = connect_async(format!("ws://{address}/api/sessions/{id}/chat/ws{query}"))
            .await
            .unwrap();
        let first = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let _ = ws.close(None).await;
        serde_json::from_str::<Value>(first.to_text().unwrap()).unwrap()
    };
    let resumed = greet(Some(6900)).await;
    assert_eq!(resumed["after"], 6900);
    assert_eq!(resumed["events"].as_array().unwrap().len(), 101);
    assert_eq!(resumed["running"], false);
    // 4,900 behind is still resumed, not reloaded.
    assert_eq!(
        greet(Some(2101)).await["events"].as_array().unwrap().len(),
        4900
    );
    // Too far behind, past the end, or a fresh view: the latest page.
    for after in [Some(100), Some(9999), None] {
        let full = greet(after).await;
        assert!(full.get("after").is_none(), "{after:?}");
        assert_eq!(full["events"].as_array().unwrap().len(), 1001, "{after:?}");
    }
    server.abort();
}

/// Search covers the whole stored history, newest first, and refuses an
/// oversized query.
#[tokio::test]
async fn a_conversation_is_searched_through_its_whole_history() {
    let fixture = chat_fixture();
    let workspace = fixture
        .state
        .store
        .create_workspace("Search", fixture.path.join("repo").to_str().unwrap())
        .unwrap();
    let id = fixture
        .state
        .store
        .create_session(workspace.id, ProviderKind::Codex, "Search")
        .unwrap()
        .id;
    let store = &fixture.state.store;
    store
        .append_conversation_event(
            id,
            json!({"type":"message","id":"old","role":"user","text":"the needle, long ago"}),
        )
        .unwrap();
    for n in 0..3000 {
        store
            .append_conversation_event(id, json!({"type":"tool","id":format!("t{n}"),"name":"Bash","status":"completed","text":"hay"}))
            .unwrap();
    }
    store
        .append_conversation_event(
            id,
            json!({"type":"message","id":"new","role":"assistant","text":"another Needle"}),
        )
        .unwrap();
    let (status, found) = call(
        fixture.app(),
        "GET",
        &format!("/api/sessions/{id}/conversation/search?q=needle"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{found}");
    let ids: Vec<&str> = found["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["new", "old"]);
    assert_eq!(found["hits"][1]["seq"], 1);
    assert_eq!(found["hits"][1]["snippet"], "the needle, long ago");
    assert_eq!(found["more"], false);
    let (_, limited) = call(
        fixture.app(),
        "GET",
        &format!("/api/sessions/{id}/conversation/search?q=hay&limit=5"),
        Value::Null,
    )
    .await;
    assert_eq!(limited["hits"].as_array().unwrap().len(), 5);
    assert_eq!(limited["more"], true);
    let long = "x".repeat(201);
    let (status, _) = call(
        fixture.app(),
        "GET",
        &format!("/api/sessions/{id}/conversation/search?q={long}"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
