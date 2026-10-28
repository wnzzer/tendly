//! The shared canvas: owners, bindings, limits, paths and storage.

use super::*;

#[tokio::test]
async fn shared_canvas_roundtrips_multiple_projects_without_migrating_legacy_layouts() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let first_workspace = register(&f).await;
    let first_id = first_workspace.parse().unwrap();
    // An unavailable project directory is still valid layout ownership. Saving
    // a canvas must not read/stat project files or initialize client credentials.
    let other_path = f.path.join("unavailable-project");
    let second_workspace = f
        .state
        .store
        .create_workspace("Second project", other_path.to_str().unwrap())
        .unwrap();
    let first_session = f
        .state
        .store
        .create_session(first_id, ProviderKind::Codex, "First agent")
        .unwrap();
    let second_session = f
        .state
        .store
        .create_session(
            second_workspace.id,
            ProviderKind::ClaudeCode,
            "Second agent",
        )
        .unwrap();
    let legacy = default_layout();
    f.state
        .store
        .save_layout(first_id, &legacy.to_string())
        .unwrap();
    f.state
        .store
        .save_layout(second_workspace.id, "unparsed legacy layout")
        .unwrap();
    let (status, initial) = call(f.app(), "GET", "/api/canvas/layout", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(initial, json!({"layout":null,"revision":0}));
    let layout = json!({"version":1,"root":{
        "type":"split","id":"multi-project","direction":"horizontal","ratio":0.5,
        "first":{"type":"stack","kind":"stack","id":"first-stack","panes":[
            {"type":"pane","id":"first-agent","kind":"agent_chat","metadata":{"workspace_id":first_workspace,"session_id":first_session.id,"provider":"codex"}},
            {"type":"pane","id":"first-editor","kind":"editor","metadata":{"workspace_id":first_workspace,"path":"src/not-read.rs"}}
        ]},
        "second":{"type":"stack","kind":"stack","id":"second-stack","panes":[
            {"type":"pane","id":"second-agent","kind":"agent_chat","metadata":{"workspace_id":second_workspace.id,"session_id":second_session.id,"provider":"claude_code"}},
            {"type":"pane","id":"second-diff","kind":"git_diff","metadata":{"workspace_id":second_workspace.id}},
            {"type":"pane","id":"second-preview","kind":"file_preview","metadata":{"workspace_id":second_workspace.id,"path":"images/not-read.png"}}
        ]}
    }});
    let (status, saved) = call(
        f.app(),
        "PUT",
        "/api/canvas/layout",
        json!({"layout":layout,"expected_revision":0}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(saved, json!({"revision":1}));
    assert_eq!(
        call(f.app(), "GET", "/api/canvas/layout", Value::Null)
            .await
            .1,
        json!({"layout":layout,"revision":1})
    );
    let mut changed = layout.clone();
    changed["root"]["ratio"] = json!(0.4);
    assert_eq!(
        call(
            f.app(),
            "PUT",
            "/api/canvas/layout",
            json!({"layout":changed,"expected_revision":0})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(f.app(), "GET", "/api/canvas/layout", Value::Null)
            .await
            .1,
        json!({"layout":layout,"revision":1})
    );
    assert_eq!(
        call(
            f.app(),
            "PUT",
            "/api/canvas/layout",
            json!({"layout":changed,"expected_revision":1})
        )
        .await
        .1,
        json!({"revision":2})
    );
    assert_eq!(
        call(
            f.app(),
            "GET",
            &format!("/api/workspaces/{first_workspace}/layout"),
            Value::Null
        )
        .await
        .1,
        legacy
    );
    assert_eq!(
        f.state
            .store
            .get_layout(second_workspace.id)
            .unwrap()
            .as_deref(),
        Some("unparsed legacy layout")
    );
    assert!(matches!(
        f.state
            .store
            .get_session(first_session.id)
            .unwrap()
            .unwrap()
            .status,
        SessionStatus::Stopped
    ));
    assert!(f.state.runtime.get(&first_session.id.to_string()).is_none());
    assert!(!other_path.exists());
}

#[tokio::test]
async fn shared_canvas_rejects_unknown_owners_wrong_session_bindings_and_unsafe_paths() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let workspace = register(&f).await;
    let second = f
        .state
        .store
        .create_workspace("Other", "/fixture/other")
        .unwrap();
    let session = f
        .state
        .store
        .create_session(workspace.parse().unwrap(), ProviderKind::Codex, "bound")
        .unwrap();
    let cases = [
        (json!({"kind":"git_diff"}), StatusCode::BAD_REQUEST),
        (json!({"kind":"file_preview"}), StatusCode::BAD_REQUEST),
        (
            json!({"kind":"editor","metadata":{"path":"src/main.rs"}}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"kind":"agent_chat","metadata":{"session_id":session.id}}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"kind":"agent_chat","metadata":{"workspace_id":"bad-id"}}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"kind":"agent_chat","metadata":{"workspace_id":null}}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"kind":"agent_chat","metadata":{"workspace_id":Uuid::new_v4()}}),
            StatusCode::NOT_FOUND,
        ),
        (
            json!({"kind":"editor","metadata":[]}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"kind":"agent_chat","metadata":{"workspace_id":workspace,"session_id":"bad-session"}}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"kind":"agent_chat","metadata":{"workspace_id":workspace,"session_id":Uuid::new_v4()}}),
            StatusCode::NOT_FOUND,
        ),
        (
            json!({"kind":"agent_chat","metadata":{"workspace_id":second.id,"session_id":session.id}}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"kind":"editor","metadata":{"workspace_id":workspace,"session_id":session.id}}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"kind":"agent_chat","metadata":{"workspace_id":workspace,"session_id":session.id,"provider":"claude_code"}}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"kind":"terminal","metadata":{"workspace_id":workspace,"session_id":session.id,"provider":"codex"}}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"kind":"terminal","metadata":{"workspace_id":workspace,"session_id":session.id,"provider":false}}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"kind":"editor","metadata":{"workspace_id":workspace,"path":"/outside/file"}}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"kind":"editor","metadata":{"workspace_id":workspace,"path":"../outside"}}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"kind":"editor","metadata":{"workspace_id":workspace,"path":"nested/../../outside"}}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"kind":"editor","metadata":{"workspace_id":workspace,"path":"nul\u{0}name"}}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"kind":"editor","metadata":{"workspace_id":workspace,"path":27}}),
            StatusCode::BAD_REQUEST,
        ),
        (json!({"kind":"unknown-pane"}), StatusCode::BAD_REQUEST),
    ];
    for (mut pane, expected) in cases {
        pane["type"] = json!("pane");
        pane["id"] = json!("invalid-pane");
        let (status, response) = call(
            f.app(),
            "PUT",
            "/api/canvas/layout",
            json!({"layout":canvas_document(vec![pane.clone()]),"expected_revision":0}),
        )
        .await;
        assert_eq!(status, expected, "pane={pane}, response={response}");
    }
    assert_eq!(
        call(f.app(), "GET", "/api/canvas/layout", Value::Null)
            .await
            .1,
        json!({"layout":null,"revision":0})
    );
}

#[tokio::test]
async fn shared_canvas_supports_unbound_placeholders_but_enforces_layout_limits() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let placeholders = canvas_document(vec![
        json!({"type":"pane","id":"agent","kind":"agent_chat"}),
        json!({"type":"pane","id":"editor","kind":"editor"}),
        json!({"type":"pane","id":"terminal","kind":"terminal"}),
    ]);
    assert_eq!(
        call(
            f.app(),
            "PUT",
            "/api/canvas/layout",
            json!({"layout":placeholders,"expected_revision":0})
        )
        .await
        .0,
        StatusCode::OK
    );
    let mut collapsed = placeholders.clone();
    collapsed["collapsed"] = json!([{"node":{"type":"pane","id":"hidden","kind":"git_diff"}}]);
    let mut oversized = placeholders.clone();
    oversized["root"]["panes"][0]["title"] = json!("x".repeat(256 * 1024));
    let mut duplicate = placeholders.clone();
    duplicate["root"]["panes"][1]["id"] = json!("agent");
    let many = canvas_document(
        (0..513)
            .map(|index| json!({"type":"pane","id":format!("pane-{index}"),"kind":"editor"}))
            .collect(),
    );
    let mut deep = json!({"type":"pane","id":"deep-leaf","kind":"editor"});
    for depth in 0..34 {
        deep = json!({"type":"split","id":format!("split-{depth}"),"direction":"horizontal","ratio":0.5,"first":deep,"second":{"type":"pane","id":format!("other-{depth}"),"kind":"editor"}});
    }
    for layout in [
        collapsed,
        oversized,
        duplicate,
        many,
        json!({"version":1,"root":deep}),
        Value::Null,
    ] {
        assert_eq!(
            call(
                f.app(),
                "PUT",
                "/api/canvas/layout",
                json!({"layout":layout,"expected_revision":1})
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    for revision in [json!(-1), json!(0.5), json!("1")] {
        assert_eq!(
            call(
                f.app(),
                "PUT",
                "/api/canvas/layout",
                json!({"layout":placeholders,"expected_revision":revision})
            )
            .await
            .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    assert_eq!(
        call(f.app(), "GET", "/api/canvas/layout", Value::Null)
            .await
            .1,
        json!({"layout":placeholders,"revision":1})
    );
}

#[cfg(unix)]
#[tokio::test]
async fn shared_canvas_keeps_legal_unix_backslashes_in_relative_filenames() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let workspace = register(&f).await;
    let layout = canvas_document(vec![
        json!({"type":"pane","id":"backslashes","kind":"editor","metadata":{"workspace_id":workspace,"path":"literal\\filename.txt"}}),
    ]);
    let (status, response) = call(
        f.app(),
        "PUT",
        "/api/canvas/layout",
        json!({"layout":layout,"expected_revision":0}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(
        call(f.app(), "GET", "/api/canvas/layout", Value::Null)
            .await
            .1,
        json!({"layout":layout,"revision":1})
    );
}

#[tokio::test]
async fn shared_canvas_uses_existing_authentication_and_health_capabilities() {
    let f = Fixture::new(
        "127.0.0.1:8787".parse().unwrap(),
        Some("fixture-token-long-enough"),
    );
    assert_eq!(
        call(f.app(), "GET", "/api/canvas/layout", Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            f.app(),
            "PUT",
            "/api/canvas/layout",
            json!({"layout":canvas_document(Vec::new()),"expected_revision":0})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    for (method, path) in [
        ("GET", "/api/secrets"),
        ("PUT", "/api/secrets/TENDLY_SECRET_X"),
        ("GET", "/api/update"),
        ("POST", "/api/update"),
        ("POST", "/api/update/restart"),
        ("GET", "/api/tailnet"),
        ("PUT", "/api/tailnet"),
        ("GET", "/api/tailnet/login-code"),
        ("GET", "/api/peers"),
        ("POST", "/api/peers"),
        ("POST", "/api/peers/invite"),
        ("GET", "/api/peers/discover"),
        ("GET", "/api/peers/sessions"),
        ("GET", "/api/host/roots"),
        ("POST", "/api/host/roots"),
        ("DELETE", "/api/host/roots"),
    ] {
        assert_eq!(
            call(f.app(), method, path, json!({"value":"x"})).await.0,
            StatusCode::UNAUTHORIZED,
            "{method} {path}"
        );
    }
    let (status, health) = call(f.app(), "GET", "/api/health", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(health["api_version"], 2);
    assert_eq!(
        health["capabilities"],
        json!([
            "shared_canvas",
            "native_configurations",
            "native_history",
            "host_directories",
            "endpoint_models",
            "model_probe",
            "model_probe_plan",
            "session_environment",
            "structured_chat",
            "official_accounts",
            "session_configuration",
            "session_archive",
            "account_import_native",
            "agent_clients",
            "ephemeral_sessions",
            "session_model",
            "workspace_file_search",
            "session_terminal_escape",
            "stored_secrets",
            "custom_balances",
            "agent_tools",
            "self_update",
            "resource_monitoring",
            "tailnet",
            "peers",
            "host_roots"
        ])
    );
    let allowed = Request::builder()
        .uri("/api/canvas/layout")
        .header("host", "127.0.0.1:8787")
        .header("authorization", "Bearer fixture-token-long-enough")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        f.app().oneshot(allowed).await.unwrap().status(),
        StatusCode::OK
    );
    let attack = Request::builder()
        .method("PUT")
        .uri("/api/canvas/layout")
        .header("host", "127.0.0.1:8787")
        .header("authorization", "Bearer fixture-token-long-enough")
        .header("x-tendly-client", "web")
        .header("origin", "https://attacker.invalid")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"layout":canvas_document(Vec::new()),"expected_revision":0}).to_string(),
        ))
        .unwrap();
    assert_eq!(
        f.app().oneshot(attack).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    let missing_client_header = Request::builder()
        .method("PUT")
        .uri("/api/canvas/layout")
        .header("host", "127.0.0.1:8787")
        .header("authorization", "Bearer fixture-token-long-enough")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"layout":canvas_document(Vec::new()),"expected_revision":0}).to_string(),
        ))
        .unwrap();
    assert_eq!(
        f.app()
            .oneshot(missing_client_header)
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        f.state.store.get_shared_canvas_layout().unwrap().revision,
        0
    );
}

#[tokio::test]
async fn shared_canvas_corrupt_storage_never_silently_falls_back_to_a_legacy_layout() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    f.state
        .store
        .save_shared_canvas_layout("invalid json", 0)
        .unwrap();
    assert_eq!(
        call(f.app(), "GET", "/api/canvas/layout", Value::Null)
            .await
            .0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        f.state
            .store
            .get_shared_canvas_layout()
            .unwrap()
            .layout
            .as_deref(),
        Some("invalid json")
    );
}
