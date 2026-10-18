use super::*;

#[test]
fn codex_reads_flags_inline_or_separate_and_refuses_empty_values() {
    let init = |args: &[&str]| ChatInit {
        provider: "codex".into(),
        program: "codex".into(),
        args: args.iter().map(|a| a.to_string()).collect(),
        cwd: "/fixture".into(),
        resume_id: None,
    };
    let launch = codex_launch(&init(&["-m=gpt-5", "--enable", "x"])).unwrap();
    assert_eq!(launch.thread.get("model"), Some(&json!("gpt-5")));
    assert_eq!(launch.args, ["app-server", "--enable", "x"]);
    for args in [&["-m", ""][..], &["-m="][..], &["--model"][..]] {
        assert_eq!(
            codex_launch(&init(args)).err().as_deref(),
            Some("Missing Codex launch option value."),
            "{args:?}"
        );
    }
    assert_eq!(
        codex_launch(&init(&["=x"])).err().as_deref(),
        Some("Unsupported Codex launch option in structured mode.")
    );
    assert_eq!(
        codex_launch(&init(&["resume", "abc", "--resume=abd"]))
            .err()
            .as_deref(),
        Some("Invalid or conflicting Codex resume ID.")
    );
    // Unlike Claude Code, an empty resume ID from the session is kept as given.
    let mut empty = init(&[]);
    empty.resume_id = Some(String::new());
    assert_eq!(codex_launch(&empty).unwrap().resume.as_deref(), Some(""));
}

#[test]
fn launch_arguments_translate_as_the_bridge_translates_them() {
    let init = |args: &[&str]| ChatInit {
        provider: "codex".into(),
        program: "codex".into(),
        args: args.iter().map(|a| a.to_string()).collect(),
        cwd: "/fixture".into(),
        resume_id: None,
    };
    let launch = codex_launch(&init(&[
        "resume",
        "saved-thread",
        "-c",
        "approval_policy=\"on-request\"",
        "-c",
        "model_reasoning_effort=\"high\"",
        "--model",
        "chosen-model",
        "--no-alt-screen",
    ]))
    .unwrap();
    assert_eq!(
        launch.args,
        [
            "app-server",
            "-c",
            "approval_policy=\"on-request\"",
            "-c",
            "model_reasoning_effort=\"high\""
        ]
    );
    assert_eq!(launch.thread.get("model"), Some(&json!("chosen-model")));
    assert_eq!(launch.resume.as_deref(), Some("saved-thread"));
    assert_eq!(
        codex_launch(&init(&["--full-auto"])).unwrap_err(),
        "Unsupported Codex launch option in structured mode."
    );
    assert_eq!(
        codex_launch(&init(&["-a", "never"])).unwrap().args,
        ["app-server", "-c", "approval_policy=\"never\""]
    );
}

// Paths here are resolved as the platform does, as Node's `path.resolve`
// does for the bridge: these expectations are POSIX ones.
#[cfg(unix)]
#[test]
fn scopes_report_access_and_what_lies_outside_the_workspace() {
    let scope = approval_scope(
        "/work",
        Some("write"),
        &[
            json!("/work/a"),
            json!("../etc/passwd"),
            json!("/tmp/x"),
            json!("/tmp/x"),
        ],
        None,
    )
    .unwrap();
    assert_eq!(
        scope,
        json!({"access":"write","outside":["/etc/passwd","/tmp/x"]})
    );
    assert_eq!(approval_scope("/work", Some("teleport"), &[], None), None);
}

#[test]
fn models_are_listed_as_codex_describes_them() {
    let models = codex_models(&json!({"data":[{"id":"a","displayName":"A","isDefault":true,"supportedReasoningEfforts":[{"reasoningEffort":"low"},"high"]},{"id":"b"},{"id":"c","hidden":true}]})).unwrap();
    assert_eq!(
        models,
        vec![
            json!({"id":"a","name":"A","isDefault":true,"efforts":["low","high"]}),
            json!({"id":"b","name":"b"})
        ]
    );
}

#[test]
fn replies_are_told_apart_from_requests_and_carry_the_stated_error() {
    assert_eq!(
        json_rpc_reply(&json!({"id": 1, "method": "item/started"})),
        None,
        "a request, not a reply"
    );
    assert_eq!(
        json_rpc_reply(&json!({"result": {}})),
        None,
        "no id to match"
    );
    assert_eq!(
        json_rpc_reply(&json!({"id": 7, "result": {"ok": true}})),
        Some((json!(7), Ok(json!({"ok": true}))))
    );
    assert_eq!(
        json_rpc_reply(&json!({"id": "a"})),
        Some((json!("a"), Ok(Value::Null)))
    );
    assert_eq!(
        json_rpc_reply(&json!({"id": 2, "error": {"code": -1, "message": "denied"}})),
        Some((json!(2), Err(json!("denied"))))
    );
    // An error without a message is passed on whole.
    assert_eq!(
        json_rpc_reply(&json!({"id": 3, "error": {"code": -1}})),
        Some((json!(3), Err(json!({"code": -1}))))
    );
}

#[test]
fn an_item_is_failed_running_or_completed() {
    for (item, status) in [
        (json!({"status": "failed"}), "failed"),
        (json!({"status": "declined"}), "failed"),
        (json!({"status": "cancelled"}), "failed"),
        (json!({"status": "completed", "success": false}), "failed"),
        (json!({"status": "inProgress"}), "running"),
        (json!({"status": "completed"}), "completed"),
        (json!({}), "completed"),
    ] {
        assert_eq!(item_status(&item), status, "{item}");
    }
}

#[test]
fn streamed_command_output_is_forwarded_on_growth_or_a_pause_not_every_delta() {
    use std::time::Duration;
    // Enough new output since the last update, however recent: send it.
    assert!(should_emit_output(OUTPUT_EMIT_BYTES, Duration::ZERO));
    assert!(should_emit_output(
        OUTPUT_EMIT_BYTES + 1,
        Duration::from_millis(1)
    ));
    // A small trickle that just went out waits rather than flooding.
    assert!(!should_emit_output(
        OUTPUT_EMIT_BYTES - 1,
        Duration::from_millis(1)
    ));
    assert!(!should_emit_output(0, Duration::ZERO));
    // But a slow trickle still shows once the interval has passed.
    assert!(should_emit_output(1, OUTPUT_EMIT_INTERVAL));
    assert!(should_emit_output(
        0,
        OUTPUT_EMIT_INTERVAL + Duration::from_millis(1)
    ));
}

#[test]
fn request_keys_keep_one_and_quoted_one_apart() {
    assert_eq!(request_key(&json!(1)), "number:1");
    assert_eq!(request_key(&json!("1")), "string:1");
    assert_ne!(request_key(&json!(1)), request_key(&json!("1")));
    assert_eq!(request_key(&Value::Null), "object:null");
    assert_eq!(request_key(&json!(true)), "boolean:true");
}

#[test]
fn absolute_paths_are_found_anywhere_in_a_request_up_to_a_limit() {
    let mut found = Vec::new();
    paths_in(
        &json!({"cwd": "/work", "changes": [{"path": "C:\\Users\\a.txt"}, {"path": "relative.txt"}], "n": 3}),
        &mut found,
        0,
    );
    assert_eq!(found, [json!("/work"), json!("C:\\Users\\a.txt")]);
    let mut many = Vec::new();
    paths_in(
        &json!((0..40).map(|i| format!("/p{i}")).collect::<Vec<_>>()),
        &mut many,
        0,
    );
    assert_eq!(many.len(), 16, "no more than sixteen are collected");
    let mut deep = Vec::new();
    paths_in(&json!([[[[[[[["/too/deep"]]]]]]]]), &mut deep, 0);
    assert!(deep.is_empty(), "nesting past six levels is not searched");
}
