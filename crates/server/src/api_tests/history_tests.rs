//! Importing conversations from a client's own history.

use super::*;

#[tokio::test]
async fn native_history_requires_confirmation_and_filters_cross_workspace_imports() {
    let mut f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let workspace = register(&f).await;
    let cwd = dunce::canonicalize(f.path.join("repo")).unwrap();
    fs::create_dir(f.path.join("other-repo")).unwrap();
    let other_cwd = dunce::canonicalize(f.path.join("other-repo")).unwrap();
    let other = f
        .state
        .store
        .create_workspace("other fixture", other_cwd.to_str().unwrap())
        .unwrap();
    let (source_id, config) = f.native_source(ProviderKind::Codex, vec![
        json!({"id":"native-allowed","provider":"codex","title":"Fixture thread","cwd":cwd,"updated_at":"2026-09-09T00:00:00Z"}),
    ]);
    let historical =
        json!({"type":"message","id":"native-user","role":"user","text":"历史内容 🦊"});
    fs::write(config.join("history-fixture.json"), serde_json::to_vec(&json!({
        "items":[{"id":"native-allowed","provider":"codex","title":"Fixture thread","cwd":cwd,"updated_at":"2026-09-09T00:00:00Z"}],
        "truncated":false,"events":[historical.clone()]
    })).unwrap()).unwrap();
    let import_path = format!("/api/workspaces/{workspace}/native-history/import");
    let (status, rejection) = call(
        f.app(),
        "POST",
        &import_path,
        json!({"source_id":source_id,"native_id":"native-allowed","confirmed_original_config":false}),
    ).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(rejection["error"].as_str().unwrap().contains("Confirm"));
    assert!(
        !config.join("bridge-used").exists(),
        "No bridge should run before consent"
    );
    assert_eq!(
        call(
            f.app(),
            "POST",
            &import_path,
            json!({"source_id":source_id,"native_id":"native-allowed"}),
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert!(!config.join("bridge-used").exists());
    assert_eq!(call(
        f.app(), "POST", &import_path,
        json!({"source_id":source_id,"native_id":"../native-allowed","confirmed_original_config":true}),
    ).await.0, StatusCode::BAD_REQUEST);
    assert!(!config.join("bridge-used").exists());

    let (status, rejection) = call(
        f.app(), "POST", &format!("/api/workspaces/{}/native-history/import", other.id),
        json!({"source_id":source_id,"native_id":"native-allowed","confirmed_original_config":true}),
    ).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{rejection}");
    assert!(config.join("bridge-used").exists());
    assert!(f.state.store.list_sessions(None).unwrap().is_empty());

    let (status, imported) = call(
        f.app(), "POST", &import_path,
        json!({"source_id":source_id,"native_id":"native-allowed","confirmed_original_config":true}),
    ).await;
    assert_eq!(status, StatusCode::OK, "{imported}");
    assert_eq!(imported["workspace_id"], workspace);
    assert_eq!(imported["provider_session_id"], "native-allowed");
    assert_eq!(imported["status"], "stopped");
    assert!(imported.get("native_config_dir").is_none());
    let (_, duplicate) = call(
        f.app(), "POST", &import_path,
        json!({"source_id":source_id,"native_id":"native-allowed","confirmed_original_config":true}),
    ).await;
    assert_eq!(duplicate["id"], imported["id"]);
    let (status, snapshot) = call(
        f.app(),
        "GET",
        &format!(
            "/api/sessions/{}/conversation",
            imported["id"].as_str().unwrap()
        ),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let events = snapshot["events"].as_array().unwrap();
    assert_eq!(
        events.len(),
        1,
        "Repeated imports must not duplicate historical messages"
    );
    assert_eq!(events[0]["text"], historical["text"]);

    assert_eq!(f.state.store.list_sessions(None).unwrap().len(), 1);
    assert!(
        f.state
            .runtime
            .get(imported["id"].as_str().unwrap())
            .is_none(),
        "Import must not launch or attach any process"
    );
}

#[tokio::test]
async fn native_history_list_is_metadata_only_and_registry_identity_is_authoritative() {
    let mut f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let workspace = register(&f).await;
    let cwd = dunce::canonicalize(f.path.join("repo")).unwrap();
    fs::create_dir(f.path.join("unrelated-repo")).unwrap();
    let unrelated = dunce::canonicalize(f.path.join("unrelated-repo")).unwrap();
    let (source_id, config) = f.native_source(ProviderKind::ClaudeCode, vec![
        json!({"id":"claude-thread","provider":"claude_code","title":"长".repeat(300),"cwd":cwd,"updated_at":"2026-09-09T00:00:00Z","imported_session_id":"not-authoritative","api_key":"DO_NOT_EXPOSE_FAKE_KEY","messages":[{"content":"DO_NOT_EXPOSE_FAKE_CONTENT"}],"config_dir":"DO_NOT_EXPOSE_FAKE_CONFIG"}),
        json!({"id":"unrelated-thread","provider":"claude_code","title":"other","cwd":unrelated,"updated_at":"2026-09-09T00:00:00Z"}),
        json!({"id":"wrong-provider","provider":"codex","title":"wrong","cwd":cwd,"updated_at":"2026-09-09T00:00:00Z"}),
        json!({"id":"../path-id","provider":"claude_code","title":"invalid","cwd":cwd,"updated_at":"2026-09-09T00:00:00Z"}),
    ]);
    let list_path = format!("/api/workspaces/{workspace}/native-history?source_id={source_id}");
    let (status, listing) = call(f.app(), "GET", &list_path, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{listing}");
    assert_eq!(listing["items"].as_array().unwrap().len(), 1);
    let item = &listing["items"][0];
    assert_eq!(item["id"], "claude-thread");
    assert_eq!(item["title"].as_str().unwrap().chars().count(), 240);
    assert!(item["imported_session_id"].is_null());
    for field in ["api_key", "messages", "config_dir"] {
        assert!(
            item.get(field).is_none(),
            "Unexpected private field: {field}"
        );
    }
    assert!(!listing.to_string().contains("DO_NOT_EXPOSE"));
    let managed = f
        .state
        .store
        .import_native_session(
            workspace.parse().unwrap(),
            ProviderKind::ClaudeCode,
            "Managed",
            &source_id,
            "claude-thread",
            config.to_str().unwrap(),
        )
        .unwrap();
    let (_, listing) = call(f.app(), "GET", &list_path, Value::Null).await;
    assert_eq!(
        listing["items"][0]["imported_session_id"],
        managed.id.to_string()
    );
    let (_, public_session) = call(
        f.app(),
        "GET",
        &format!("/api/sessions/{}", managed.id),
        Value::Null,
    )
    .await;
    assert!(public_session.get("native_config_dir").is_none());
    assert!(
        !public_session
            .to_string()
            .contains(config.to_str().unwrap())
    );

    let missing = format!("/api/workspaces/{workspace}/native-history?source_id=not-registered");
    assert_eq!(
        call(f.app(), "GET", &missing, Value::Null).await.0,
        StatusCode::NOT_FOUND
    );
}

#[test]
fn native_history_resume_preserves_original_configuration_and_rejects_source_changes() {
    let mut f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let cwd = dunce::canonicalize(f.path.join("repo")).unwrap();
    let workspace = f
        .state
        .store
        .create_workspace("resume fixture", cwd.to_str().unwrap())
        .unwrap();
    for (provider, expected_args, config_key) in [
        (
            ProviderKind::Codex,
            vec!["resume", "fixture-id"],
            "CODEX_HOME",
        ),
        (
            ProviderKind::ClaudeCode,
            vec!["--resume", "fixture-id"],
            "CLAUDE_CONFIG_DIR",
        ),
    ] {
        let (source_id, config) = f.native_source(provider, Vec::new());
        let session = f
            .state
            .store
            .import_native_session(
                workspace.id,
                provider,
                "Resume fixture",
                &source_id,
                "fixture-id",
                config.to_str().unwrap(),
            )
            .unwrap();
        let spec = native_history::resume_spec(&f.state, &session, cwd.clone()).unwrap();
        assert_eq!(spec.args, expected_args);
        assert_eq!(spec.cwd, cwd);
        assert_eq!(spec.env[config_key], config.to_string_lossy());
        assert_eq!(
            spec.env.len(),
            1,
            "Do not inject endpoint, account or permission overrides when resuming original history"
        );
        assert!(!spec.args.iter().any(|arg| arg.contains("bypass")
            || arg.contains("dangerously")
            || arg.contains("--last")));
        assert!(f.state.runtime.get(&session.id.to_string()).is_none());
        assert!(
            !config.join("bridge-used").exists(),
            "Constructing resume arguments must not execute a client"
        );

        let mut invalid_id = session.clone();
        invalid_id.provider_session_id = Some("--all".into());
        assert_eq!(
            native_history::resume_spec(&f.state, &invalid_id, cwd.clone())
                .unwrap_err()
                .status,
            StatusCode::BAD_REQUEST
        );
        let mut wrong_provider = session.clone();
        wrong_provider.provider = ProviderKind::Terminal;
        assert_eq!(
            native_history::resume_spec(&f.state, &wrong_provider, cwd.clone())
                .unwrap_err()
                .status,
            StatusCode::BAD_REQUEST
        );

        let replacement = f.path.join(format!("replacement-{source_id}"));
        fs::create_dir(&replacement).unwrap();
        f.state
            .native_sources
            .iter_mut()
            .find(|source| source.id == source_id)
            .unwrap()
            .config_dir = replacement;
        let rejected = native_history::resume_spec(&f.state, &session, cwd.clone()).unwrap_err();
        assert_eq!(rejected.status, StatusCode::BAD_REQUEST);
        assert!(rejected.message.contains("source changed"));
    }
}

#[cfg(unix)]
#[test]
fn native_history_resume_rejects_original_source_retargeted_by_symlink() {
    use std::os::unix::fs::symlink;
    let mut f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let cwd = dunce::canonicalize(f.path.join("repo")).unwrap();
    let workspace = f
        .state
        .store
        .create_workspace("fixture", cwd.to_str().unwrap())
        .unwrap();
    let (source_id, config) = f.native_source(ProviderKind::Codex, Vec::new());
    let session = f
        .state
        .store
        .import_native_session(
            workspace.id,
            ProviderKind::Codex,
            "Original history",
            &source_id,
            "native-id",
            config.to_str().unwrap(),
        )
        .unwrap();
    let original_moved = f.path.join("preserved-original-config");
    fs::rename(&config, &original_moved).unwrap();
    symlink(&original_moved, &config).unwrap();
    let rejection = native_history::resume_spec(&f.state, &session, cwd).unwrap_err();
    assert_eq!(rejection.status, StatusCode::BAD_REQUEST);
    assert!(rejection.message.contains("source changed"));
}
