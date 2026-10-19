use super::*;

/// How a launch argument list is read: `--flag value` or `--flag=value`.
/// Claude Code accepts an empty value; Codex and Pi do not.
#[test]
fn claude_reads_flags_inline_or_separate_and_accepts_empty_values() {
    let launch = claude_launch(&init(&["--model=opus", "--effort", ""])).unwrap();
    assert_eq!(launch.model.as_deref(), Some("opus"));
    assert_eq!(launch.effort.as_deref(), Some(""));
    assert_eq!(
        claude_launch(&init(&["--model="]))
            .unwrap()
            .model
            .as_deref(),
        Some("")
    );
    assert_eq!(
        claude_launch(&init(&["--model"])).err().as_deref(),
        Some("Missing Claude Code launch option value.")
    );
    // A leading `=` is not a flag with a value.
    assert_eq!(
        claude_launch(&init(&["=opus"])).err().as_deref(),
        Some("Unsupported Claude Code launch option in structured mode.")
    );
    assert_eq!(
        claude_launch(&init(&["-r=abc"])).unwrap().resume.as_deref(),
        Some("abc")
    );
    assert_eq!(
        claude_launch(&init(&["--resume", "abc", "-r", "abd"]))
            .err()
            .as_deref(),
        Some("Invalid or conflicting Claude Code resume ID.")
    );
    assert_eq!(
        claude_launch(&init(&["--resume", "not valid"]))
            .err()
            .as_deref(),
        Some("Invalid or conflicting Claude Code resume ID.")
    );
    // An empty resume ID from the session is no resume at all.
    let mut empty = init(&[]);
    empty.resume_id = Some(String::new());
    assert_eq!(claude_launch(&empty).unwrap().resume, None);
}

fn init(args: &[&str]) -> ChatInit {
    ChatInit {
        provider: "claude_code".into(),
        program: "claude".into(),
        args: args.iter().map(|a| a.to_string()).collect(),
        cwd: "/work".into(),
        resume_id: None,
    }
}

#[test]
fn a_launch_keeps_its_options_and_adds_structured_mode() {
    let launch = claude_launch(&init(&[
        "--model",
        "sonnet",
        "--effort=high",
        "--permission-mode",
        "plan",
        "--strict-mcp-config",
        "--print",
        "--output-format",
        "text",
    ]))
    .unwrap();
    assert_eq!(launch.model.as_deref(), Some("sonnet"));
    assert_eq!(launch.effort.as_deref(), Some("high"));
    assert_eq!(
        launch.args,
        [
            "--model",
            "sonnet",
            "--effort",
            "high",
            "--permission-mode",
            "plan",
            "--strict-mcp-config",
            "--allow-dangerously-skip-permissions",
            "--setting-sources",
            "user,project,local",
            "--replay-user-messages",
            "--print",
            "--verbose",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--include-partial-messages",
            "--permission-prompt-tool",
            "stdio",
        ]
    );
}

#[test]
fn a_launch_refuses_what_structured_mode_cannot_honour() {
    let refused = |args: &[&str]| claude_launch(&init(args)).unwrap_err();
    assert_eq!(
        refused(&["--permission-mode", "bypassPermissions"]),
        "Structured mode does not enable permission bypass."
    );
    assert_eq!(
        refused(&["--permission-prompt-tool", "mcp__x"]),
        "Custom permission prompt tools are unsupported in structured mode."
    );
    assert_eq!(
        refused(&["--dangerously-skip-permissions"]),
        "Unsupported Claude Code launch option in structured mode."
    );
    assert_eq!(
        refused(&["--model"]),
        "Missing Claude Code launch option value."
    );
    assert_eq!(
        refused(&["--resume", "a", "--session-id", "b"]),
        "Conflicting Claude Code session identifiers."
    );
    assert_eq!(
        refused(&["--resume", "../x"]),
        "Invalid or conflicting Claude Code resume ID."
    );
    let mut resumed = init(&["--resume", "session-1"]);
    resumed.resume_id = Some("session-2".into());
    assert_eq!(
        claude_launch(&resumed).unwrap_err(),
        "Invalid or conflicting Claude Code resume ID."
    );
}

#[test]
fn a_launch_resumes_and_keeps_its_own_setting_sources() {
    let launch = claude_launch(&init(&["-r", "session-1", "--setting-sources", "user"])).unwrap();
    assert_eq!(launch.resume.as_deref(), Some("session-1"));
    assert!(
        launch
            .args
            .windows(2)
            .any(|w| w == ["--resume", "session-1"])
    );
    assert!(!launch.args.iter().any(|a| a == "user,project,local"));
}

#[test]
fn a_selection_is_kept_while_it_describes_the_resolved_model() {
    let models = vec![
        json!({"id":"sonnet"}),
        json!({"id":"default"}),
        json!({"id":"opus[1m]"}),
    ];
    let keeps =
        |selected: Option<&str>, reported: &str| keeps_selection(Some(&models), selected, reported);
    assert!(keeps(Some("sonnet"), "claude-sonnet-5"));
    assert!(keeps(Some("default"), "claude-opus-5"));
    assert!(keeps(Some("opus[1m]"), "claude-opus-5-5[1m]"));
    assert!(!keeps(Some("sonnet"), "claude-opus-5"));
    assert!(keeps(Some("claude-x"), "claude-x"));
    assert!(!keeps(Some("unlisted"), "claude-x"));
    assert!(!keeps(None, "claude-x"));
}

#[test]
fn calls_are_summarised_from_their_input() {
    let summary = |input: Value| call_summary(Some(&input));
    assert_eq!(
        summary(json!({"description":"List files","command":"ls"})).as_deref(),
        Some("List files")
    );
    assert_eq!(
        summary(json!({"command":"ls -la\nmore"})).as_deref(),
        Some("ls -la")
    );
    assert_eq!(
        summary(json!({"pattern":"TODO","path":"/a/b/c.rs"})).as_deref(),
        Some("TODO \u{00b7} b/c.rs")
    );
    assert_eq!(
        summary(json!({"file_path":"/a/b/c.rs"})).as_deref(),
        Some("b/c.rs")
    );
    assert_eq!(
        summary(json!({"url":"https://example.com/x"})).as_deref(),
        Some("example.com/x")
    );
    assert_eq!(summary(json!({"query":"rust"})).as_deref(), Some("rust"));
    let long = "x".repeat(100);
    assert_eq!(
        summary(json!({"command":long})).unwrap().chars().count(),
        80
    );
    assert_eq!(summary(json!({})), None);
    assert_eq!(call_summary(Some(&json!("text"))), None);
}

#[test]
fn usage_and_tokens_are_counted_as_the_bridge_counts_them() {
    assert_eq!(
        turn_usage(Some(
            &json!({"input_tokens":10,"cache_read_input_tokens":5,"cache_creation_input_tokens":2,"output_tokens":3})
        )),
        Some(json!({"input":17,"output":3,"cached":5}))
    );
    assert_eq!(turn_usage(Some(&json!({"input_tokens":0}))), None);
    assert_eq!(
        context_tokens(Some(
            &json!({"input_tokens":10,"cache_read_input_tokens":-1})
        )),
        Some(json!(10))
    );
    assert_eq!(context_tokens(Some(&json!({"output_tokens":4}))), None);
    assert_eq!(result_tokens("abcd"), 1);
    assert_eq!(result_tokens("abcdef"), 2);
    assert_eq!(result_tokens("你好"), 2);
    assert_eq!(result_tokens(""), 0);
}

#[test]
fn a_failed_turn_names_unsupported_authentication() {
    assert!(unsupported_authentication("[\"OAuth token rejected\"]"));
    assert!(unsupported_authentication("subscription is not supported"));
    assert!(unsupported_authentication(
        "Authentication mode unsupported"
    ));
    assert!(!unsupported_authentication("[\"rate limited\"]"));
    assert!(!unsupported_authentication("not a subscription"));
}

#[test]
fn a_failed_turn_says_what_the_endpoint_said() {
    assert_eq!(
        failure_reason(
            &json!({"api_error_status":401,"result":" Failed to authenticate. API Error: 401 bad key "})
        ),
        "Claude Code could not complete the turn (HTTP 401). Check the key, and that the endpoint takes it as an x-api-key header. Claude Code said: Failed to authenticate. API Error: 401 bad key"
    );
    assert!(
        failure_reason(&json!({"api_error_status":404}))
            .contains("(HTTP 404). Check the model name")
    );
    assert!(failure_reason(&json!({"api_error_status":429})).contains("limiting requests"));
    assert!(failure_reason(&json!({"api_error_status":529})).contains("The endpoint failed"));
    // No status, or none that is one, and nothing said: the old advice alone.
    for message in [
        json!({"result":""}),
        json!({"api_error_status":"401"}),
        json!({"api_error_status":7}),
    ] {
        assert_eq!(
            failure_reason(&message),
            "Claude Code could not complete the turn. Check its native account, endpoint and permission settings."
        );
    }
}

#[test]
fn replies_are_control_responses_matched_by_request_id() {
    assert_eq!(
        control_reply(
            &json!({"type":"control_response","response":{"subtype":"success","request_id":"a","response":{"x":1}}})
        ),
        Some((json!("a"), Ok(json!({"x":1}))))
    );
    assert_eq!(
        control_reply(
            &json!({"type":"control_response","response":{"subtype":"success","request_id":"a"}})
        ),
        Some((json!("a"), Ok(json!({}))))
    );
    assert_eq!(
        control_reply(
            &json!({"type":"control_response","response":{"subtype":"error","request_id":"a","error":"no"}})
        ),
        Some((json!("a"), Err(json!("no"))))
    );
    assert_eq!(
        control_reply(
            &json!({"type":"control_response","response":{"subtype":"error","request_id":"a"}})
        ),
        Some((json!("a"), Err(json!(true))))
    );
    assert_eq!(control_reply(&json!({"type":"assistant"})), None);
    assert_eq!(
        control_frame("a", "set_model", json!({"model":"opus"})),
        json!({"type":"control_request","request_id":"a","request":{"subtype":"set_model","model":"opus"}})
    );
}

#[test]
fn message_ids_go_out_as_uuids() {
    let id = "0f8fad5b-d9cb-469f-a165-70867728950e";
    assert_eq!(wire_id(id), id);
    assert!(is_uuid(&wire_id("not-a-uuid")));
    assert_ne!(wire_id("not-a-uuid"), "not-a-uuid");
}

#[test]
fn permission_modes_map_both_ways_and_manual_means_ask() {
    for (ours, theirs) in PERMISSIONS {
        assert_eq!(claude_mode(ours), Some(theirs));
        assert_eq!(our_mode(theirs), Some(ours));
    }
    assert_eq!(
        our_mode("manual"),
        Some("ask"),
        "older builds call it manual"
    );
    assert_eq!(claude_mode("manual"), None, "and it is never sent");
    assert_eq!(our_mode("auto"), None);
}

#[test]
fn model_entries_keep_what_the_picker_shows() {
    let models = model_entries(Some(&json!([
        {"value": "sonnet", "displayName": "Sonnet", "description": "Fast", "supportsEffort": true, "supportedEffortLevels": ["low", "high"]},
        {"value": "opus", "supportsEffort": false, "supportedEffortLevels": ["high"]},
        {"value": ""},
        {"displayName": "no id"}
    ])))
    .unwrap();
    assert_eq!(
        models,
        [
            json!({"id": "sonnet", "name": "Sonnet", "description": "Fast", "efforts": ["low", "high"]}),
            json!({"id": "opus", "name": "opus"}),
        ]
    );
    assert_eq!(model_entries(Some(&json!("not a list"))), None);
    assert_eq!(model_entries(None), None);
}

#[test]
fn slash_commands_are_named_either_way_the_client_lists_them() {
    assert_eq!(
        command_names(Some(
            &json!(["clear", {"name": "compact"}, {"label": "x"}, 3])
        )),
        Some(vec!["clear".to_owned(), "compact".to_owned()])
    );
    assert_eq!(command_names(None), None);
}

#[test]
fn a_long_first_line_is_cut_at_eighty_utf16_units() {
    assert_eq!(first_line_short("short\nsecond"), "short");
    let long = "x".repeat(81);
    let cut = first_line_short(&long);
    assert_eq!(cut.encode_utf16().count(), 80);
    assert!(cut.ends_with('\u{2026}'));
    assert_eq!(first_line_short(&"x".repeat(80)), "x".repeat(80));
}
