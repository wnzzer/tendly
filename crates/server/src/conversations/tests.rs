use super::*;

// The whitelist strips fields it does not name, and a stripped field is not
// an error anywhere — the bridge emits it, the client never sees it, and
// nothing says so. This already cost one silently dropped model list.
#[test]
fn subagent_progress_survives_normalisation() {
    let event = normalize_event(json!({
        "type": "tool", "id": "task-1", "name": "Task",
        "status": "running", "activity": "Scanning the bridge."
    }))
    .expect("a tool event carrying subagent progress must be accepted");
    assert_eq!(event["activity"], "Scanning the bridge.");
}

#[test]
fn nested_cards_and_task_counts_survive_normalisation() {
    let event = normalize_event(json!({
        "type": "tool", "id": "read-1", "name": "Read", "status": "running",
        "parent": "agent-1", "at": 1_760_000_000_000u64,
        "stats": {"tokens": 3400, "tools": 12, "ms": 80000}
    }))
    .expect("a nested card with counts must be accepted");
    assert_eq!(event["parent"], "agent-1");
    assert_eq!(event["stats"]["tools"], 12);
    let background = normalize_event(json!({
        "type": "tool", "id": "bash-1", "name": "Bash", "status": "running", "background": true
    }))
    .expect("a background command is a tool card");
    assert_eq!(background["background"], true);
    let sized = normalize_event(json!({
        "type": "tool", "id": "read-1", "name": "Read", "status": "completed", "result_tokens": 12000
    }))
    .expect("a call's result size is kept");
    assert_eq!(sized["result_tokens"], 12000);
    let spent = normalize_event(json!({
        "type": "usage", "turn": {"input": 82200, "output": 900, "cached": 81000}
    }))
    .expect("a turn's cost is a usage event");
    assert_eq!(spent["turn"]["cached"], 81000);
    assert!(normalize_event(json!({"type":"usage","turn":{"cost":1}})).is_none());
    let reply = normalize_event(json!({
        "type": "usage", "message_id": "pi-assistant-1", "turn": {"input": 42, "output": 4, "cached": 30}
    }))
    .expect("a reply's cost names its reply");
    assert_eq!(reply["message_id"], "pi-assistant-1");
    assert!(normalize_event(json!({"type":"usage","message_id":"","turn":{"input":1}})).is_none());
    for bad in [
        json!({"type":"tool","id":"t","name":"R","status":"running","parent":""}),
        json!({"type":"tool","id":"t","name":"R","status":"running","stats":{"cost":1}}),
        json!({"type":"tool","id":"t","name":"R","status":"running","stats":{"tokens":-1}}),
        json!({"type":"tool","id":"t","name":"R","status":"running","at":"now"}),
        json!({"type":"tool","id":"t","name":"R","status":"running","background":"yes"}),
    ] {
        assert!(normalize_event(bad.clone()).is_none(), "{bad}");
    }
}

#[test]
fn progress_is_optional_and_must_be_text() {
    assert!(
        normalize_event(json!({"type":"tool","id":"t","name":"Read","status":"completed"}))
            .is_some(),
        "a tool card without a subagent is still a valid event"
    );
    assert!(
        normalize_event(json!({
            "type":"tool","id":"t","name":"Read","status":"running","activity":{"nested":1}
        }))
        .is_none(),
        "progress must be text, not an arbitrary structure"
    );
}

#[test]
fn unknown_fields_are_still_refused() {
    // Proving the guard is real: were it not, the test above would pass for
    // a field nobody added to the list.
    assert!(
        normalize_event(json!({
            "type":"tool","id":"t","name":"Read","status":"running","invented":"x"
        }))
        .is_none_or(|event| event.get("invented").is_none()),
        "a field outside the whitelist must not reach the client"
    );
}

#[test]
fn an_unset_field_is_left_out_rather_than_sent_as_null() {
    let command = model_command(Some("opus"), None);
    assert_eq!(command["model"], "opus");
    assert!(
        command.get("effort").is_none(),
        "a null effort is rejected by the bridge as a malformed level"
    );
    assert_eq!(model_command(Some("opus"), Some("high"))["effort"], "high");
    // A depth-only change keeps whichever model the session already runs.
    let depth_only = model_command(None, Some("high"));
    assert!(depth_only.get("model").is_none());
    assert_eq!(depth_only["effort"], "high");
}

#[test]
fn an_acknowledgement_is_accepted_and_says_whether_it_repeated() {
    let (status, Json(body)) = accepted(Some("m-1"), true);
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(
        body,
        json!({"accepted": true, "id": "m-1", "duplicate": true})
    );
    let (_, Json(body)) = accepted(None, false);
    assert_eq!(body["id"], Value::Null);
}

#[test]
fn activity_reports_what_the_client_is_doing() {
    let runtime = ChatRuntime::for_test();
    assert_eq!(runtime.activity(), "idle");
    assert!(runtime.idle());
    runtime.busy.store(true, Ordering::Release);
    assert_eq!(runtime.activity(), "working");
    assert!(!runtime.idle());
    runtime.busy.store(false, Ordering::Release);
    // A pending approval outranks being ready: the session waits on the user.
    runtime
        .approvals
        .lock()
        .expect("approvals")
        .insert("r-1".into(), HashSet::from(["accept".to_owned()]));
    assert_eq!(runtime.activity(), "waiting_for_approval");
    assert!(!runtime.idle());
    runtime.approvals.lock().expect("approvals").clear();
    runtime.ready.store(false, Ordering::Release);
    assert_eq!(runtime.activity(), "starting");
    runtime.running.store(false, Ordering::Release);
    assert_eq!(runtime.activity(), "stopped");
}

#[test]
fn unknown_event_types_and_approval_choices_are_refused() {
    assert!(normalize_event(json!({"type": "invented"})).is_none());
    assert!(normalize_event(json!("not an object")).is_none());
    let approval = |choices: Value| {
        normalize_event(json!({
            "type": "approval", "id": "a-1", "title": "Run", "text": "ls", "choices": choices
        }))
    };
    assert!(approval(json!(["accept", "decline"])).is_some());
    assert!(approval(json!([])).is_none(), "an approval offers a choice");
    assert!(approval(json!(["always"])).is_none());
}
