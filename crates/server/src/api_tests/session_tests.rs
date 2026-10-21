//! A session's life: temporary and permanent sessions, restarts, titles, permissions, steering and environment.

use super::*;

#[tokio::test]
async fn a_temporary_session_is_declared_at_creation_and_only_then_can_be_discarded() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let workspace = register(&f).await;
    let sessions_path = format!("/api/workspaces/{workspace}/sessions");
    let create = |body: Value| call(f.app(), "POST", &sessions_path, body);

    let (_, temporary) =
        create(json!({"title":"Scratch","provider":"claude_code","ephemeral":true})).await;
    let (_, permanent) = create(json!({"title":"Keep me","provider":"claude_code"})).await;
    let temporary_id = temporary["id"].as_str().unwrap().to_owned();
    let permanent_id = permanent["id"].as_str().unwrap().to_owned();
    assert_eq!(temporary["ephemeral"], true);
    // Omitting the field is the ordinary, permanent case.
    assert_eq!(permanent["ephemeral"], false);

    // An unarchived session cannot be deleted directly; a permanent one must
    // be archived first.
    assert_eq!(
        call(
            f.app(),
            "DELETE",
            &format!("/api/sessions/{permanent_id}"),
            Value::Null
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert!(
        f.state
            .store
            .get_session(permanent_id.parse().unwrap())
            .unwrap()
            .is_some()
    );

    // Promotion is one-way: once kept, the destructive path closes behind it.
    let (status, kept) = call(
        f.app(),
        "POST",
        &format!("/api/sessions/{temporary_id}/keep"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(kept["ephemeral"], false);
    assert_eq!(kept["id"], temporary_id);
    assert_eq!(
        call(
            f.app(),
            "DELETE",
            &format!("/api/sessions/{temporary_id}"),
            Value::Null
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    // There is deliberately no request shape that marks an existing session
    // temporary again.
    assert_eq!(
        call(
            f.app(),
            "PATCH",
            &format!("/api/sessions/{temporary_id}"),
            json!({"ephemeral":true})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    let (_, throwaway) =
        create(json!({"title":"Second scratch","provider":"terminal","ephemeral":true})).await;
    let throwaway_id = throwaway["id"].as_str().unwrap().to_owned();
    let (status, discarded) = call(
        f.app(),
        "DELETE",
        &format!("/api/sessions/{throwaway_id}"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(discarded["id"], throwaway_id);
    assert!(
        f.state
            .store
            .get_session(throwaway_id.parse().unwrap())
            .unwrap()
            .is_none()
    );
    // A second discard reports the record is gone rather than succeeding twice.
    assert_eq!(
        call(
            f.app(),
            "DELETE",
            &format!("/api/sessions/{throwaway_id}"),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_permanent_session_can_be_deleted_only_once_archived() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let workspace = register(&f).await;
    let (_, permanent) = call(
        f.app(),
        "POST",
        &format!("/api/workspaces/{workspace}/sessions"),
        json!({"title":"Old work","provider":"terminal"}),
    )
    .await;
    let id = permanent["id"].as_str().unwrap().to_owned();
    let path = format!("/api/sessions/{id}");
    assert_eq!(
        call(f.app(), "DELETE", &path, Value::Null).await.0,
        StatusCode::CONFLICT
    );
    let (status, _) = call(
        f.app(),
        "PATCH",
        &format!("/api/sessions/{id}/archive"),
        json!({"archived":true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, deleted) = call(f.app(), "DELETE", &path, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(deleted["id"], id);
    assert!(
        f.state
            .store
            .get_session(id.parse().unwrap())
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn a_restart_clears_temporary_sessions_and_keeps_every_permanent_one() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let workspace = register(&f).await;
    let sessions_path = format!("/api/workspaces/{workspace}/sessions");
    let create = |body: Value| call(f.app(), "POST", &sessions_path, body);
    let (_, temporary) =
        create(json!({"title":"Scratch","provider":"claude_code","ephemeral":true})).await;
    let (_, permanent) = create(json!({"title":"Keep me","provider":"claude_code"})).await;

    f.state.store.reconcile_after_restart().unwrap();

    // A temporary session cannot outlive its process, so it goes rather than
    // becoming another stopped row in the list.
    assert!(
        f.state
            .store
            .get_session(temporary["id"].as_str().unwrap().parse().unwrap())
            .unwrap()
            .is_none()
    );
    let survivor = f
        .state
        .store
        .get_session(permanent["id"].as_str().unwrap().parse().unwrap())
        .unwrap()
        .unwrap();
    assert!(matches!(survivor.status, SessionStatus::Stopped));
    assert!(!survivor.ephemeral);
}

#[tokio::test]
async fn session_title_can_be_renamed_without_changing_identity_or_process_state() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let workspace = register(&f).await;
    let (_, created) = call(
        f.app(),
        "POST",
        &format!("/api/workspaces/{workspace}/sessions"),
        json!({"title":"Initial Claude task","provider":"claude_code"}),
    )
    .await;
    let id = created["id"].as_str().unwrap().to_owned();
    let (status, renamed) = call(
        f.app(),
        "PATCH",
        &format!("/api/sessions/{id}"),
        json!({"title":"Review billing"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(renamed["id"], id);
    assert_eq!(renamed["title"], "Review billing");
    assert_eq!(renamed["status"], "stopped");
    let record = f
        .state
        .store
        .get_session(id.parse().unwrap())
        .unwrap()
        .unwrap();
    let spec = launch::build(&f.state, &record, f.path.join("repo")).unwrap();
    assert!(
        spec.args
            .windows(2)
            .any(|pair| pair == ["--name", "Review billing"])
    );
    assert_eq!(
        call(
            f.app(),
            "PATCH",
            &format!("/api/sessions/{id}"),
            json!({"title":"  "})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            f.app(),
            "PATCH",
            "/api/sessions/not-a-session",
            json!({"title":"Nope"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn a_permission_chosen_while_stopped_waits_for_the_next_start() {
    // A session whose client exited -- after a failed turn, say -- still shows
    // the permission chip. Refusing the choice there left the phone with an
    // error hidden behind its settings sheet and nothing changed.
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let cwd = dunce::canonicalize(f.path.join("repo")).unwrap();
    let workspace = f
        .state
        .store
        .create_workspace("fixture", cwd.to_str().unwrap())
        .unwrap();
    let session = f
        .state
        .store
        .create_session_with_configuration(
            workspace.id,
            ProviderKind::ClaudeCode,
            "Stopped",
            None,
            None,
            None,
            Default::default(),
            false,
        )
        .unwrap();
    let path = format!("/api/sessions/{}/conversation/permission", session.id);
    let (status, body) = call(f.app(), "POST", &path, json!({"mode": "danger"})).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["pending"], true);
    let (status, _) = call(f.app(), "POST", &path, json!({"mode": "everything"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn session_environment_api_merges_templates_and_only_edits_stopped_sessions() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let workspace = register(&f).await;
    let (_,profile) = call(f.app(),"POST","/api/endpoint-profiles",json!({"name":"env","provider":"codex","environment":{"MODE":{"kind":"literal","value":"template"},"KEEP":{"kind":"literal","value":"keep"}}})).await;
    let (status,session) = call(f.app(),"POST",&format!("/api/workspaces/{workspace}/sessions"),json!({"title":"env","provider":"codex","endpoint_profile_id":profile["id"],"environment":{"MODE":{"kind":"unset"}}})).await;
    assert_eq!(status, StatusCode::CREATED, "{session}");
    assert_eq!(session["environment"]["MODE"]["kind"], "unset");
    assert_eq!(session["environment"]["KEEP"]["value"], "keep");
    assert_eq!(
        session["endpoint_snapshot"]["environment"]["MODE"]["value"],
        "template"
    );
    let id: Uuid = session["id"].as_str().unwrap().parse().unwrap();
    let url = format!("/api/sessions/{id}/environment");
    let changes = json!({"environment":{"OPENAI_API_KEY":{"kind":"secret_ref","reference":"env:TENDLY_SECRET_FIXTURE"}}});
    let (status, patched) = call(f.app(), "PATCH", &url, changes.clone()).await;
    assert_eq!(status, StatusCode::OK, "{patched}");
    assert_eq!(patched["environment"], changes["environment"]);
    assert_eq!(patched["endpoint_snapshot"], session["endpoint_snapshot"]);
    f.state
        .store
        .set_session_status(id, SessionStatus::Running)
        .unwrap();
    assert_eq!(
        call(f.app(), "PATCH", &url, json!({"environment":{}}))
            .await
            .0,
        StatusCode::CONFLICT
    );
    f.state
        .store
        .set_session_status(id, SessionStatus::Failed)
        .unwrap();
    assert_eq!(
        call(
            f.app(),
            "PATCH",
            &url,
            json!({"environment":{"HOME":{"kind":"unset"}}})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            f.app(),
            "PATCH",
            &url,
            json!({"environment":{"API_KEY":{"kind":"literal","value":"synthetic-no"}}})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(f.app(), "PATCH", &url, json!({"environment":{}}))
            .await
            .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn session_environment_rejects_live_runtime_even_if_database_says_stopped() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let workspace = f
        .state
        .store
        .create_workspace("fixture", f.path.join("repo").to_str().unwrap())
        .unwrap();
    let session = f
        .state
        .store
        .create_session(workspace.id, ProviderKind::Terminal, "fixture shell")
        .unwrap();
    let runtime = f
        .state
        .runtime
        .start(
            session.id.to_string(),
            tendly_runtime::SpawnSpec {
                program: waiting_process(&["-i"]).0,
                args: waiting_process(&["-i"]).1,
                cwd: f.path.join("repo"),
                env: Default::default(),
                env_remove: vec![],
            },
        )
        .await
        .unwrap();
    let status = call(
        f.app(),
        "PATCH",
        &format!("/api/sessions/{}/environment", session.id),
        json!({"environment":{}}),
    )
    .await
    .0;
    runtime.stop().await.unwrap();
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn native_and_history_build_apply_environment_without_mutating_native_configuration() {
    use tendly_domain::{EnvironmentOverrides, EnvironmentValue};
    let mut f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let (source_id, config) = f.native_source(ProviderKind::Codex, Vec::new());
    let sentinel = config.join("config.toml");
    fs::write(&sentinel, "# immutable fixture\n").unwrap();
    let workspace = f
        .state
        .store
        .create_workspace("fixture", f.path.join("repo").to_str().unwrap())
        .unwrap();
    let (_, reference) = native_config::pin(&f.state, &source_id).unwrap();
    let mut profile = f
        .state
        .store
        .import_native_profile("native", ProviderKind::Codex, reference)
        .unwrap();
    profile.environment = [(
        "NATIVE_TEST_MODE".into(),
        EnvironmentValue::Literal {
            value: "template".into(),
        },
    )]
    .into();
    f.state.store.update_endpoint_profile(&profile).unwrap();
    let mut session = f
        .state
        .store
        .create_session_with_profile(
            workspace.id,
            ProviderKind::Codex,
            "native",
            Some(profile.id),
        )
        .unwrap();
    let spec = launch::build(&f.state, &session, f.path.join("repo")).unwrap();
    assert_eq!(spec.env["NATIVE_TEST_MODE"], "template");
    assert_eq!(spec.env["CODEX_HOME"], config.to_str().unwrap());
    let environment: EnvironmentOverrides = [
        (
            "NATIVE_TEST_MODE".into(),
            EnvironmentValue::Literal {
                value: "history".into(),
            },
        ),
        ("REMOVE_ME".into(), EnvironmentValue::Unset),
    ]
    .into();
    session = f
        .state
        .store
        .import_native_session_with_environment(
            workspace.id,
            ProviderKind::Codex,
            "history",
            &source_id,
            "fixture-thread",
            config.to_str().unwrap(),
            Some(environment),
        )
        .unwrap()
        .unwrap();
    let spec = launch::build(&f.state, &session, f.path.join("repo")).unwrap();
    assert_eq!(spec.env["NATIVE_TEST_MODE"], "history");
    assert!(spec.env_remove.contains(&"REMOVE_ME".into()));
    assert!(!spec.env_remove.contains(&"PATH".into()));
    assert!(!spec.env.contains_key("PATH"));
    assert_eq!(
        fs::read_to_string(sentinel).unwrap(),
        "# immutable fixture\n"
    );
}

#[tokio::test]
async fn history_import_environment_conflicts_without_overwriting_existing_session() {
    let mut f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let workspace = register(&f).await;
    let cwd = dunce::canonicalize(f.path.join("repo")).unwrap();
    let (source_id,_) = f.native_source(ProviderKind::Codex,vec![json!({"id":"fixture-thread","provider":"codex","cwd":cwd,"title":"fixture","updated_at":"2026-09-09T00:00:00Z"})]);
    let url = format!("/api/workspaces/{workspace}/native-history/import");
    let input = json!({"source_id":source_id,"native_id":"fixture-thread","confirmed_original_config":true,"environment":{"MODE":{"kind":"literal","value":"first"}}});
    let (status, session) = call(f.app(), "POST", &url, input.clone()).await;
    assert_eq!(status, StatusCode::OK, "{session}");
    f.state
        .store
        .set_session_status(
            session["id"].as_str().unwrap().parse().unwrap(),
            SessionStatus::Running,
        )
        .unwrap();
    let mut changed = input.clone();
    changed["environment"] = json!({});
    assert_eq!(
        call(f.app(), "POST", &url, changed).await.0,
        StatusCode::CONFLICT
    );
    let mut no_override = input;
    no_override.as_object_mut().unwrap().remove("environment");
    let (_, unchanged) = call(f.app(), "POST", &url, no_override).await;
    assert_eq!(unchanged["environment"], session["environment"]);
    assert_eq!(unchanged["status"], "running");
}

#[tokio::test]
async fn session_environment_api_requires_existing_authentication() {
    let f = Fixture::new(
        "127.0.0.1:8787".parse().unwrap(),
        Some("fixture-token-long-enough"),
    );
    assert_eq!(
        call(
            f.app(),
            "PATCH",
            &format!("/api/sessions/{}/environment", Uuid::new_v4()),
            json!({"environment":{}})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn steering_is_validated_like_a_message() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let workspace = register(&f).await;
    let (_, session) = call(
        f.app(),
        "POST",
        &format!("/api/workspaces/{workspace}/sessions"),
        json!({"title":"steer","provider":"claude_code","interaction_mode":"structured"}),
    )
    .await;
    let path = format!(
        "/api/sessions/{}/conversation/steer",
        session["id"].as_str().unwrap()
    );
    for bad in [
        json!({"id":"not-a-uuid","content":"hello"}),
        json!({"id":Uuid::new_v4(),"content":"   "}),
    ] {
        assert_eq!(
            call(f.app(), "POST", &path, bad).await.0,
            StatusCode::BAD_REQUEST
        );
    }
}

#[tokio::test]
async fn creating_a_cross_client_handoff_persists_the_source_history() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let workspace = register(&f).await;
    let path = format!("/api/workspaces/{workspace}/sessions");
    let (_, source) = call(
        f.app(),
        "POST",
        &path,
        json!({"title":"CC","provider":"claude_code","interaction_mode":"structured"}),
    )
    .await;
    let source_id = source["id"].as_str().unwrap().parse().unwrap();
    f.state
        .store
        .append_conversation_event(
            source_id,
            json!({"type":"message","id":"u","role":"user","text":"Keep the retry fix"}),
        )
        .unwrap();
    let (status, target) = call(f.app(), "POST", &path, json!({"title":"Codex","provider":"codex","interaction_mode":"structured","handoff_source_id":source_id})).await;
    assert_eq!(status, StatusCode::CREATED, "{target}");
    let target_id = target["id"].as_str().unwrap().parse().unwrap();
    assert!(
        f.state
            .store
            .carried_transcript(target_id, 65536)
            .unwrap()
            .unwrap()
            .contains("Keep the retry fix")
    );
    assert_eq!(
        f.state.store.carried_transcript(source_id, 65536).unwrap(),
        None
    );
    let (status, _) = call(
        f.app(),
        "POST",
        &path,
        json!({"title":"Invalid","provider":"codex","handoff_source_id":source_id}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
