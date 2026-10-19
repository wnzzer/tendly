use super::*;

const NATIVE_ID: &str = "11111111-2222-3333-4444-555555555555";

fn mark_conversation(f: &Fixture, id: SessionId) -> Session {
    f.state
        .store
        .set_interaction_mode(id, InteractionMode::Structured)
        .unwrap();
    f.state
        .store
        .set_native_conversation_id(id, NATIVE_ID)
        .unwrap();
    f.state.store.get_session(id).unwrap().unwrap()
}

async fn reopen(f: &Fixture, source: &Session) -> Session {
    let (status, body) = call(
        f.app(),
        "POST",
        &format!("/api/sessions/{}/terminal", source.id),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let id = body["id"].as_str().unwrap().parse().unwrap();
    f.state.store.get_session(id).unwrap().unwrap()
}

#[tokio::test]
async fn terminal_launch_pins_snapshot_revision_credentials_and_flags_for_every_client() {
    for (provider, config_key, key_variable, model, effort_flag) in [
        (
            ProviderKind::ClaudeCode,
            "CLAUDE_CONFIG_DIR",
            "ANTHROPIC_API_KEY",
            "chosen-model",
            "--effort",
        ),
        (
            ProviderKind::Codex,
            "CODEX_HOME",
            "OPENAI_API_KEY",
            "chosen-model",
            "-c",
        ),
        (
            ProviderKind::Pi,
            "PI_CODING_AGENT_DIR",
            "TENDLY_PI_API_KEY",
            "tendly/chosen-model",
            "--thinking",
        ),
    ] {
        let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
        let workspace = f
            .state
            .store
            .create_workspace("fixture", f.path.join("repo").to_str().unwrap())
            .unwrap();
        let (status, _) = call(
            f.app(),
            "PUT",
            "/api/secrets/TENDLY_SECRET_LAUNCH",
            json!({"value":"synthetic-launch-key"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let mode = match provider {
            ProviderKind::ClaudeCode => "plan",
            ProviderKind::Codex => "interactive",
            _ => "native",
        };
        let (status, body) = call(f.app(), "POST", "/api/endpoint-profiles", json!({
            "name":"launch fixture", "provider":provider, "endpoint_url":"https://original.example.test/v1",
            "secret_ref":"env:TENDLY_SECRET_LAUNCH", "model":"template-model", "effort":"low", "permission_mode":mode,
            "environment":{"LAUNCH_MARKER":{"kind":"literal","value":"original"}}
        })).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let mut profile: EndpointProfile = serde_json::from_value(body).unwrap();
        let created = f
            .state
            .store
            .create_session_with_profile(workspace.id, provider, "source", Some(profile.id))
            .unwrap();
        // A session-level choice is part of the pinned launch, not the template.
        assert!(
            f.state
                .store
                .switch_session_configuration_with_effort(
                    created.id,
                    Some(profile.id),
                    Some("chosen-model".into()),
                    Some("high".into())
                )
                .unwrap()
        );
        let source = mark_conversation(&f, created.id);
        let original = launch::prepare(&f.state, &source).await.unwrap();
        profile.endpoint_url = Some("https://changed.example.test/v1".into());
        profile.model = Some("changed-model".into());
        assert!(f.state.store.update_endpoint_profile(&profile).unwrap());
        let terminal = reopen(&f, &source).await;
        assert_eq!(
            serde_json::to_value(&terminal.endpoint_snapshot).unwrap(),
            serde_json::to_value(&source.endpoint_snapshot).unwrap()
        );
        assert_eq!(terminal.environment, source.environment);
        assert_eq!(terminal.resume_configuration_revision, Some(1));
        // Moving the source to its next context must not move this terminal.
        assert!(
            f.state
                .store
                .switch_session_configuration(source.id, Some(profile.id), None)
                .unwrap()
        );
        let spec = launch::prepare(&f.state, &terminal).await.unwrap();
        assert_eq!(spec.env[config_key], original.env[config_key]);
        assert_eq!(spec.env[key_variable], "synthetic-launch-key");
        assert_eq!(spec.env["LAUNCH_MARKER"], "original");
        let args = without_tendly(&spec.args);
        assert!(
            args.windows(2).any(|pair| pair == ["--model", model]),
            "{args:?}"
        );
        assert!(args.iter().any(|arg| arg == effort_flag), "{args:?}");
        assert!(
            args.iter()
                .any(|arg| arg == "high" || arg == "model_reasoning_effort=\"high\""),
            "{args:?}"
        );
        let resume = crate::adapters::agent(provider)
            .unwrap()
            .resume_args(NATIVE_ID);
        assert!(args.ends_with(&resume), "{args:?}");
        assert!(
            !args.iter().any(|arg| arg == "--name"),
            "continuation keeps its original native title"
        );
        match provider {
            ProviderKind::ClaudeCode => {
                // Claude Code adds /v1 itself.
                assert_eq!(
                    spec.env["ANTHROPIC_BASE_URL"],
                    "https://original.example.test"
                );
                assert!(
                    args.windows(2)
                        .any(|pair| pair == ["--permission-mode", "plan"])
                );
            }
            ProviderKind::Codex => {
                let config =
                    fs::read_to_string(PathBuf::from(&spec.env[config_key]).join("config.toml"))
                        .unwrap();
                assert!(config.contains("https://original.example.test/v1"));
                assert!(
                    args.iter()
                        .any(|arg| arg == "approval_policy=\"on-request\"")
                );
            }
            ProviderKind::Pi => {
                let config =
                    fs::read_to_string(PathBuf::from(&spec.env[config_key]).join("models.json"))
                        .unwrap();
                assert!(config.contains("https://original.example.test/v1"));
                assert!(!config.contains("synthetic-launch-key"));
            }
            _ => unreachable!(),
        }
        // Continuing a continuation resolves to the same original home, not a
        // new directory under the intermediate terminal's Tendly id.
        let nested = reopen(&f, &terminal).await;
        assert_eq!(nested.resume_source_id, Some(source.id));
        assert_eq!(nested.resume_configuration_revision, Some(1));
        let nested_spec = launch::prepare(&f.state, &nested).await.unwrap();
        assert_eq!(nested_spec.env[config_key], original.env[config_key]);
    }
}

#[tokio::test]
async fn terminal_launch_keeps_the_sources_worktree() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let repo = f.path.join("repo");
    assert!(
        Command::new("git")
            .args(["commit", "--allow-empty", "-qm", "fixture"])
            .current_dir(&repo)
            .status()
            .unwrap()
            .success()
    );
    let checkout = f.path.join("continuation-worktree");
    assert!(
        Command::new("git")
            .args(["worktree", "add", "-qb", "continuation-fixture"])
            .arg(&checkout)
            .current_dir(&repo)
            .status()
            .unwrap()
            .success()
    );
    let checkout = dunce::canonicalize(checkout).unwrap();
    let workspace = f
        .state
        .store
        .create_workspace("fixture", repo.to_str().unwrap())
        .unwrap();
    let created = f
        .state
        .store
        .create_session(workspace.id, ProviderKind::Codex, "worktree source")
        .unwrap();
    assert!(
        f.state
            .store
            .set_session_checkout(
                created.id,
                checkout.to_str(),
                Some("continuation-fixture"),
                "fixture"
            )
            .unwrap()
    );
    let source = mark_conversation(&f, created.id);
    let terminal = reopen(&f, &source).await;
    assert_eq!(terminal.checkout_path, source.checkout_path);
    assert_eq!(terminal.checkout_branch, source.checkout_branch);
    assert!(
        f.state
            .store
            .set_session_checkout(source.id, None, None, "workspace")
            .unwrap()
    );
    assert_eq!(
        launch::prepare(&f.state, &terminal).await.unwrap().cwd,
        checkout
    );
}

#[tokio::test]
async fn terminal_launch_failure_leaves_no_session_record() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let workspace = f
        .state
        .store
        .create_workspace("fixture", f.path.join("repo").to_str().unwrap())
        .unwrap();
    let (_, body) = call(
        f.app(),
        "POST",
        "/api/endpoint-profiles",
        json!({"name":"blocked", "provider":"codex", "permission_mode":"blocked"}),
    )
    .await;
    let profile: EndpointProfile = serde_json::from_value(body).unwrap();
    let created = f
        .state
        .store
        .create_session_with_profile(
            workspace.id,
            ProviderKind::Codex,
            "blocked source",
            Some(profile.id),
        )
        .unwrap();
    let source = mark_conversation(&f, created.id);
    let (status, _) = call(
        f.app(),
        "POST",
        &format!("/api/sessions/{}/terminal", source.id),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(f.state.store.list_sessions(None).unwrap().len(), 1);
    assert!(f.state.runtime.get(&source.id.to_string()).is_none());
}

#[tokio::test]
async fn terminal_launch_of_imported_history_keeps_the_original_home_and_one_resume_selector() {
    let mut f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let workspace = f
        .state
        .store
        .create_workspace("fixture", f.path.join("repo").to_str().unwrap())
        .unwrap();
    for provider in ProviderKind::AGENTS {
        let (source_id, directory) = f.native_source(provider, Vec::new());
        let source = f
            .state
            .store
            .import_native_session(
                workspace.id,
                provider,
                "imported",
                &source_id,
                NATIVE_ID,
                directory.to_str().unwrap(),
            )
            .unwrap();
        let terminal = reopen(&f, &source).await;
        assert_eq!(terminal.native_source_id, source.native_source_id);
        assert_eq!(terminal.native_config_dir, source.native_config_dir);
        let spec = launch::prepare(&f.state, &terminal).await.unwrap();
        let client = crate::adapters::agent(provider).unwrap();
        assert_eq!(spec.env[client.config_key()], directory.to_string_lossy());
        assert_eq!(without_tendly(&spec.args), client.resume_args(NATIVE_ID));
        let again = f
            .state
            .store
            .import_native_session(
                workspace.id,
                provider,
                "imported again",
                &source_id,
                NATIVE_ID,
                directory.to_str().unwrap(),
            )
            .unwrap();
        assert_eq!(
            again.id, source.id,
            "reimport finds the owner, never a terminal view"
        );
        let second = reopen(&f, &source).await;
        assert_ne!(second.id, terminal.id);
    }
}

#[tokio::test]
async fn choosing_a_new_context_detaches_a_terminal_from_its_original_home() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let workspace = f
        .state
        .store
        .create_workspace("fixture", f.path.join("repo").to_str().unwrap())
        .unwrap();
    let created = f
        .state
        .store
        .create_session(workspace.id, ProviderKind::Codex, "source")
        .unwrap();
    let source = mark_conversation(&f, created.id);
    for change_endpoint in [true, false] {
        let terminal = reopen(&f, &source).await;
        let original = launch::prepare(&f.state, &terminal).await.unwrap();
        if change_endpoint {
            assert!(
                f.state
                    .store
                    .switch_session_configuration(terminal.id, None, Some("different-model".into()))
                    .unwrap()
            );
        } else {
            assert!(
                f.state
                    .store
                    .set_session_checkout(terminal.id, None, None, "workspace")
                    .unwrap()
            );
        }
        let changed = f.state.store.get_session(terminal.id).unwrap().unwrap();
        assert_eq!(changed.resume_source_id, None);
        assert_eq!(changed.resume_configuration_revision, None);
        assert_eq!(changed.provider_session_id, None);
        let plan = launch::prepare(&f.state, &changed).await.unwrap();
        assert_ne!(plan.env["CODEX_HOME"], original.env["CODEX_HOME"]);
        assert!(!plan.args.iter().any(|arg| arg == "resume"));
    }
}

async fn launch_plan(
    f: &Fixture,
    id: SessionId,
    peer: Option<&str>,
    query: &str,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method("POST")
        .uri(format!("/api/sessions/{id}/launch-plan{query}"))
        .header("host", "127.0.0.1:8787")
        .header("x-tendly-client", "cli")
        .body(Body::empty())
        .unwrap();
    if let Some(peer) = peer {
        request
            .extensions_mut()
            .insert(axum::extract::ConnectInfo::<SocketAddr>(
                peer.parse().unwrap(),
            ));
    }
    let response = f.app().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn launch_plan_continues_the_conversation_for_this_machine_only_and_records_nothing() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let workspace = f
        .state
        .store
        .create_workspace("fixture", f.path.join("repo").to_str().unwrap())
        .unwrap();
    let created = f
        .state
        .store
        .create_session(workspace.id, ProviderKind::Codex, "cli source")
        .unwrap();
    let source = mark_conversation(&f, created.id);
    let before = f.state.store.list_sessions(None).unwrap().len();

    let (status, plan) = launch_plan(&f, source.id, Some("127.0.0.1:50000"), "").await;
    assert_eq!(status, StatusCode::OK, "{plan}");
    let args: Vec<&str> = plan["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|arg| arg.as_str().unwrap())
        .collect();
    assert!(
        args.windows(2).any(|pair| pair == ["resume", NATIVE_ID]),
        "{args:?}"
    );
    let home = plan["env"]["CODEX_HOME"].as_str().unwrap();
    assert!(home.contains(&source.id.to_string()), "{home}");
    assert_eq!(plan["provider_session_id"], NATIVE_ID);
    assert_eq!(f.state.store.list_sessions(None).unwrap().len(), before);

    for peer in [None, Some("192.168.0.9:50000")] {
        let (status, body) = launch_plan(&f, source.id, peer, "").await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert!(body.get("env").is_none());
    }
}

#[tokio::test]
async fn launch_plan_refuses_a_running_session_unless_forced_and_needs_a_conversation() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let workspace = f
        .state
        .store
        .create_workspace("fixture", f.path.join("repo").to_str().unwrap())
        .unwrap();
    let fresh = f
        .state
        .store
        .create_session(
            workspace.id,
            ProviderKind::ClaudeCode,
            "no conversation yet",
        )
        .unwrap();
    let (status, _) = launch_plan(&f, fresh.id, Some("127.0.0.1:50000"), "").await;
    assert_eq!(status, StatusCode::CONFLICT);

    let source = mark_conversation(&f, fresh.id);
    f.state
        .store
        .set_session_status(source.id, SessionStatus::Running)
        .unwrap();
    let (status, body) = launch_plan(&f, source.id, Some("127.0.0.1:50000"), "").await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body["error"].as_str().unwrap().contains("--force"));
    let (status, plan) = launch_plan(&f, source.id, Some("[::1]:50000"), "?force=true").await;
    assert_eq!(status, StatusCode::OK, "{plan}");
    assert!(
        plan["args"]
            .as_array()
            .unwrap()
            .iter()
            .any(|arg| arg == "--resume")
    );
}
