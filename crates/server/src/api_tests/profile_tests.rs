//! Endpoint profiles: secrets, providers, snapshots, and profiles imported from a client's own configuration.

use super::*;

#[tokio::test]
async fn first_custom_endpoint_sets_only_its_clients_unset_default() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let hidden = Uuid::new_v4();
    call(
        f.app(),
        "PUT",
        "/api/preferences",
        json!({"codex":{"effort":"high"},"hidden_account_quotas":[hidden]}),
    )
    .await;
    let (status, first) = call(
        f.app(),
        "POST",
        "/api/endpoint-profiles",
        json!({"name":"first","provider":"codex"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (_, prefs) = call(f.app(), "GET", "/api/preferences", Value::Null).await;
    assert_eq!(prefs["codex"]["endpoint_profile_id"], first["id"]);
    assert_eq!(prefs["codex"]["effort"], "high");
    assert_eq!(prefs["hidden_account_quotas"], json!([hidden]));
    assert_eq!(prefs["claude_code"], json!({}));
    call(
        f.app(),
        "POST",
        "/api/endpoint-profiles",
        json!({"name":"second","provider":"codex"}),
    )
    .await;
    assert_eq!(
        call(f.app(), "GET", "/api/preferences", Value::Null)
            .await
            .1["codex"]["endpoint_profile_id"],
        first["id"]
    );
    // Clearing the default is a user choice; creating another endpoint preserves it.
    call(f.app(), "PUT", "/api/preferences", json!({"codex":{}})).await;
    call(
        f.app(),
        "POST",
        "/api/endpoint-profiles",
        json!({"name":"third","provider":"codex"}),
    )
    .await;
    assert!(
        call(f.app(), "GET", "/api/preferences", Value::Null)
            .await
            .1["codex"]
            .get("endpoint_profile_id")
            .is_none()
    );
}

#[tokio::test]
async fn a_key_saved_in_tendly_reaches_the_client_and_never_comes_back() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let id = register(&f).await;
    let name = "TENDLY_SECRET_API_TEST_STORED";
    let (status, saved) = call(
        f.app(),
        "PUT",
        &format!("/api/secrets/{name}"),
        json!({"value":"sk-stored-value"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved, json!({"name":name,"source":"tendly","stored":true}));
    let (status, listed) = call(f.app(), "GET", "/api/secrets", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        listed
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["name"] == name)
    );
    assert!(!listed.to_string().contains("sk-stored-value"));
    let (status, _) = call(f.app(), "PUT", "/api/secrets/HOME", json!({"value":"x"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (_, profile) = call(f.app(), "POST", "/api/endpoint-profiles", json!({"name":"stored","provider":"codex","endpoint_url":"https://example.test/v1","secret_ref":format!("env:{name}")})).await;
    let (_, created) = call(
        f.app(),
        "POST",
        &format!("/api/workspaces/{id}/sessions"),
        json!({"title":"uses stored key","provider":"codex","endpoint_profile_id":profile["id"]}),
    )
    .await;
    let session: Session = serde_json::from_value(created).unwrap();
    let spec = launch::build(&f.state, &session, f.path.join("repo")).unwrap();
    assert_eq!(
        spec.env.get("OPENAI_API_KEY").map(String::as_str),
        Some("sk-stored-value")
    );

    let (status, removed) = call(
        f.app(),
        "DELETE",
        &format!("/api/secrets/{name}"),
        Value::Null,
    )
    .await;
    assert_eq!((status, removed), (StatusCode::OK, json!({"removed":true})));
    let error = launch::build(&f.state, &session, f.path.join("repo")).unwrap_err();
    assert!(error.message.contains(name), "{}", error.message);
}

#[tokio::test]
async fn a_pi_profile_becomes_a_provider_in_its_own_models_json() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let id = register(&f).await;
    let name = "TENDLY_SECRET_API_TEST_PI";
    call(
        f.app(),
        "PUT",
        &format!("/api/secrets/{name}"),
        json!({"value":"sk-pi-value"}),
    )
    .await;
    let (_, limits) = call(
        f.app(),
        "PUT",
        "/api/models/limits",
        json!({"models":{"zai-org/glm-5.3":{"context_window":40000}}}),
    )
    .await;
    assert!(limits["models"].is_object());
    let (status, profile) = call(f.app(), "POST", "/api/endpoint-profiles", json!({"name":"pi gateway","provider":"pi","endpoint_url":"http://gateway.test/v1","secret_ref":format!("env:{name}"),"model":"qwen-local","effort":"high","models":["zai-org/GLM-5.3"]})).await;
    assert_eq!(status, StatusCode::CREATED, "{profile}");
    let (_, created) = call(
        f.app(),
        "POST",
        &format!("/api/workspaces/{id}/sessions"),
        json!({"title":"pi task","provider":"pi","endpoint_profile_id":profile["id"]}),
    )
    .await;
    let session: Session = serde_json::from_value(created).unwrap();
    let spec = launch::build(&f.state, &session, f.path.join("repo")).unwrap();
    let args = spec.args.join(" ");
    assert!(args.contains("--model tendly/qwen-local"), "{args}");
    assert!(args.contains("--thinking high"), "{args}");
    assert!(args.contains("--name pi task"), "{args}");
    assert!(!args.contains("mcp"), "Pi has no MCP: {args}");
    assert_eq!(
        spec.env.get("TENDLY_PI_API_KEY").map(String::as_str),
        Some("sk-pi-value")
    );
    let home = PathBuf::from(spec.env.get("PI_CODING_AGENT_DIR").expect("its own home"));
    #[cfg(unix)]
    assert_eq!(
        fs::read_link(home.join("bin")).unwrap(),
        f.state.state_dir.join("clients/pi-bin"),
        "the tools Pi downloads are shared by every session"
    );
    let written = fs::read_to_string(home.join("models.json")).unwrap();
    assert!(
        !written.contains("sk-pi-value"),
        "the key stays out of the file"
    );
    let config: Value = serde_json::from_str(&written).unwrap();
    assert_eq!(
        config["providers"]["tendly"],
        json!({"baseUrl":"http://gateway.test/v1","api":"openai-completions","apiKey":"$TENDLY_PI_API_KEY",
            "models":[{"id":"qwen-local"},{"id":"zai-org/GLM-5.3","contextWindow":40000}]})
    );

    // A profile can name the API its endpoint speaks; a client that speaks
    // only one has no API to name.
    let (status, _) = call(
        f.app(),
        "PATCH",
        &format!("/api/endpoint-profiles/{}", profile["id"].as_str().unwrap()),
        json!({"api":"openai-responses"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, created) = call(
        f.app(),
        "POST",
        &format!("/api/workspaces/{id}/sessions"),
        json!({"title":"pi responses","provider":"pi","endpoint_profile_id":profile["id"]}),
    )
    .await;
    let session: Session = serde_json::from_value(created).unwrap();
    let spec = launch::build(&f.state, &session, f.path.join("repo")).unwrap();
    let home = PathBuf::from(spec.env.get("PI_CODING_AGENT_DIR").unwrap());
    let config: Value =
        serde_json::from_str(&fs::read_to_string(home.join("models.json")).unwrap()).unwrap();
    assert_eq!(config["providers"]["tendly"]["api"], "openai-responses");
    for (provider, api) in [("pi", "soap"), ("claude_code", "openai-responses")] {
        let (status, _) = call(f.app(), "POST", "/api/endpoint-profiles", json!({"name":"wrong api","provider":provider,"endpoint_url":"http://gateway.test/v1","api":api})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{provider} {api}");
    }
}

#[tokio::test]
async fn profiles_validate_snapshot_and_session_states() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let id = register(&f).await;
    let (_,p)=call(f.app(),"POST","/api/endpoint-profiles",json!({"name":"work","provider":"codex","endpoint_url":"https://example.test/v1","permission_mode":"interactive"})).await;
    let pid = p["id"].as_str().unwrap();
    let (status, s) = call(
        f.app(),
        "POST",
        &format!("/api/workspaces/{id}/sessions"),
        json!({"title":"one","provider":"codex","endpoint_profile_id":pid}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(s["status"], "stopped");
    assert_eq!(
        call(
            f.app(),
            "POST",
            &format!("/api/workspaces/{id}/sessions"),
            json!({"title":"wrong","provider":"claude_code","endpoint_profile_id":pid})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            f.app(),
            "PATCH",
            &format!("/api/endpoint-profiles/{pid}"),
            json!({"endpoint_url":null})
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, unchanged) = call(
        f.app(),
        "GET",
        &format!("/api/sessions/{}", s["id"].as_str().unwrap()),
        Value::Null,
    )
    .await;
    assert_eq!(
        unchanged["endpoint_snapshot"]["endpoint_url"],
        "https://example.test/v1"
    );
    let record = f
        .state
        .store
        .get_session(s["id"].as_str().unwrap().parse().unwrap())
        .unwrap()
        .unwrap();
    let spec = launch::build(&f.state, &record, f.path.join("repo")).unwrap();
    let config =
        fs::read_to_string(PathBuf::from(&spec.env["CODEX_HOME"]).join("config.toml")).unwrap();
    let parsed: toml::Value = toml::from_str(&config).unwrap();
    assert_eq!(
        parsed["model_providers"]["tendly"]["base_url"].as_str(),
        Some("https://example.test/v1")
    );
    assert!(
        spec.args
            .contains(&"approval_policy=\"on-request\"".to_owned())
    );
    assert!(!config.contains("never"));
    let sid = s["id"].as_str().unwrap();
    assert_eq!(
        call(
            f.app(),
            "POST",
            &format!("/api/sessions/{sid}/stop"),
            Value::Null
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            f.app(),
            "PATCH",
            &format!("/api/sessions/{sid}"),
            json!({"status":"running"})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
async fn claude_plan_mode_is_native_and_codex_does_not_fake_it() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let workspace = register(&f).await;
    let (status, profile) = call(
        f.app(),
        "POST",
        "/api/endpoint-profiles",
        json!({"name":"Claude plan","provider":"claude_code","permission_mode":"plan"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (_, session) = call(
        f.app(),
        "POST",
        &format!("/api/workspaces/{workspace}/sessions"),
        json!({"title":"plan session","provider":"claude_code","endpoint_profile_id":profile["id"]}),
    ).await;
    let record = f
        .state
        .store
        .get_session(session["id"].as_str().unwrap().parse().unwrap())
        .unwrap()
        .unwrap();
    let spec = launch::build(&f.state, &record, f.path.join("repo")).unwrap();
    assert!(
        spec.args
            .windows(2)
            .any(|pair| pair == ["--permission-mode", "plan"])
    );
    assert_eq!(
        call(
            f.app(),
            "POST",
            "/api/endpoint-profiles",
            json!({"name":"Codex plan","provider":"codex","permission_mode":"plan"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn proxy_alias_and_session_model_override_are_snapshotted() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let id = register(&f).await;
    let (status,p)=call(f.app(),"POST","/api/endpoint-profiles",json!({"name":"mapped","provider":"codex","proxy_url":"http://127.0.0.1:7890","model":"fast","effort":"high","model_aliases":{"fast":"actual-model-id"}})).await;
    assert_eq!(status, StatusCode::CREATED);
    let (_,s)=call(f.app(),"POST",&format!("/api/workspaces/{id}/sessions"),json!({"title":"mapped session","provider":"codex","endpoint_profile_id":p["id"],"model":"fast","effort":"low"})).await;
    let record = f
        .state
        .store
        .get_session(s["id"].as_str().unwrap().parse().unwrap())
        .unwrap()
        .unwrap();
    let spec = launch::build(&f.state, &record, f.path.join("repo")).unwrap();
    assert!(
        spec.args
            .windows(2)
            .any(|pair| pair == ["--model", "actual-model-id"])
    );
    assert!(
        spec.args
            .iter()
            .any(|arg| arg == "model_reasoning_effort=\"low\"")
    );
    assert_eq!(spec.env["HTTPS_PROXY"], "http://127.0.0.1:7890");
    assert_eq!(spec.env["https_proxy"], "http://127.0.0.1:7890");
    let pid = p["id"].as_str().unwrap();
    call(
        f.app(),
        "PATCH",
        &format!("/api/endpoint-profiles/{pid}"),
        json!({"proxy_url":null,"model_aliases":{},"effort":"max"}),
    )
    .await;
    let snapshot = record.endpoint_snapshot.as_ref().unwrap();
    assert_eq!(snapshot.model_aliases["fast"], "actual-model-id");
    assert_eq!(snapshot.effort.as_deref(), Some("low"));
    let (_, dirs) = call(f.app(), "GET", "/api/host/directories", Value::Null).await;
    assert_eq!(dirs["root_id"], "0");
    assert_eq!(
        call(
            f.app(),
            "GET",
            "/api/host/directories?root=0&path=../",
            Value::Null
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn native_config_import_requires_confirmation_and_only_exposes_source_metadata() {
    let mut f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let (source_id, config) = f.native_source(ProviderKind::Codex, Vec::new());
    let sentinel = b"fake private credential: never read or copy this fixture";
    fs::write(config.join("auth.json"), sentinel).unwrap();
    let (status, sources) = call(
        f.app(),
        "GET",
        "/api/host/native-configurations",
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(sources[0]["id"], source_id);
    assert_eq!(sources[0]["available"], true);
    assert!(!sources.to_string().contains("private credential"));
    assert!(
        sources[0].get("authenticated").is_none(),
        "Directory presence is not proof of login"
    );
    for body in [
        json!({"source_id":source_id}),
        json!({"source_id":source_id,"confirmed_shared_config":false}),
    ] {
        assert_eq!(
            call(
                f.app(),
                "POST",
                "/api/endpoint-profiles/import-native",
                body
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        call(
            f.app(),
            "POST",
            "/api/endpoint-profiles/import-native",
            json!({"source_id":"../unknown","confirmed_shared_config":true})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            f.app(),
            "POST",
            "/api/endpoint-profiles/import-native",
            json!({"source_id":source_id,"confirmed_shared_config":true,"config_dir":"/arbitrary"})
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(call(f.app(), "POST", "/api/endpoint-profiles", json!({"name":"forged","provider":"codex","native_config":{"source_id":source_id,"config_dir":config}})).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(f.state.store.list_endpoint_profiles().unwrap().is_empty());
    assert!(!config.join("bridge-used").exists());
    assert_eq!(fs::read(config.join("auth.json")).unwrap(), sentinel);
}

#[tokio::test]
async fn imported_native_profiles_create_new_sessions_without_copying_or_overriding_config() {
    let mut f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let workspace = register(&f).await;
    let cwd = dunce::canonicalize(f.path.join("repo")).unwrap();
    for (provider, key, config_file) in [
        (ProviderKind::Codex, "CODEX_HOME", "config.toml"),
        (
            ProviderKind::ClaudeCode,
            "CLAUDE_CONFIG_DIR",
            "settings.json",
        ),
    ] {
        let (source_id, config) = f.native_source(provider, Vec::new());
        let original = b"Fixture config is intentionally not parsed: preserve these exact bytes";
        fs::write(config.join(config_file), original).unwrap();
        #[cfg(unix)]
        let mode_before = fs::metadata(&config).unwrap().permissions();
        let body =
            json!({"source_id":source_id,"name":"Host account","confirmed_shared_config":true});
        let (status, p) = call(
            f.app(),
            "POST",
            "/api/endpoint-profiles/import-native",
            body.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(p["native_config"]["source_id"], source_id);
        assert_eq!(p["native_config"]["config_dir"], config.to_str().unwrap());
        assert_eq!(p["provider"], serde_json::to_value(provider).unwrap());
        assert_eq!(p["permission_mode"], "native");
        let (_, again) = call(
            f.app(),
            "POST",
            "/api/endpoint-profiles/import-native",
            body,
        )
        .await;
        assert_eq!(again["id"], p["id"]);
        for title in ["First", "Second"] {
            let (status, result) = call(
                f.app(),
                "POST",
                &format!("/api/workspaces/{workspace}/sessions"),
                json!({"provider":provider,"title":title,"endpoint_profile_id":p["id"]}),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED);
            let session: Session = serde_json::from_value(result).unwrap();
            assert!(
                session.provider_session_id.is_none(),
                "This opens a new conversation, not resume"
            );
            assert!(
                session.native_source_id.is_none(),
                "History imports keep their separate contract"
            );
            assert!(matches!(session.status, SessionStatus::Stopped));
            let spec = launch::build(&f.state, &session, cwd.clone()).unwrap();
            assert!(
                without_tendly(&spec.args).is_empty(),
                "Do not inject login, endpoint, model, permission or resume flags"
            );
            assert_eq!(spec.cwd, cwd);
            // Only the configuration directory, besides the session's own
            // Tendly identity (agent.rs), which every launch carries.
            let own: Vec<_> = spec
                .env
                .keys()
                .filter(|name| !name.starts_with("TENDLY_"))
                .collect();
            assert_eq!(own, [key]);
            assert_eq!(spec.env[key], config.to_str().unwrap());
            assert!(f.state.runtime.get(&session.id.to_string()).is_none());
            assert!(
                !f.state
                    .state_dir
                    .join("sessions")
                    .join(session.id.to_string())
                    .exists()
            );
        }
        assert!(!config.join("bridge-used").exists());
        assert_eq!(fs::read(config.join(config_file)).unwrap(), original);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&config).unwrap().permissions().mode(),
                mode_before.mode()
            );
        }
    }
    assert_eq!(f.state.store.list_endpoint_profiles().unwrap().len(), 2);
}

#[tokio::test]
async fn native_profile_rename_and_unlink_preserve_existing_session_reference() {
    let mut f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let workspace = register(&f).await;
    let (source_id, config) = f.native_source(ProviderKind::Codex, Vec::new());
    let (_, p) = call(
        f.app(),
        "POST",
        "/api/endpoint-profiles/import-native",
        json!({"source_id":source_id,"name":"Original","confirmed_shared_config":true}),
    )
    .await;
    let profile_path = format!("/api/endpoint-profiles/{}", p["id"].as_str().unwrap());
    let create_path = format!("/api/workspaces/{workspace}/sessions");
    // A session may pick its own model even on a shared native configuration:
    // that is a launch flag for this session, and the imported profile below is
    // still asserted to keep the client's own settings.
    let (status, overridden) = call(f.app(), "POST", &create_path, json!({"title":"override","provider":"codex","endpoint_profile_id":p["id"],"model":"other"})).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(overridden["endpoint_snapshot"]["model"], "other");
    assert_eq!(
        call(f.app(), "GET", &profile_path, Value::Null).await.1["model"],
        Value::Null
    );
    assert_eq!(
        call(
            f.app(),
            "POST",
            &create_path,
            json!({"title":"wrong client","provider":"claude_code","endpoint_profile_id":p["id"]})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    for patch in [
        json!({"endpoint_url":"https://example.test"}),
        json!({"model":null}),
        json!({"proxy_url":null}),
        json!({"permission_mode":"trusted"}),
        json!({"native_config":null}),
        json!({"model_aliases":{}}),
    ] {
        assert_eq!(
            call(f.app(), "PATCH", &profile_path, patch).await.0,
            StatusCode::BAD_REQUEST
        );
    }
    let (_, record) = call(
        f.app(),
        "POST",
        &create_path,
        json!({"title":"unchanged reference","provider":"codex","endpoint_profile_id":p["id"]}),
    )
    .await;
    let session: Session = serde_json::from_value(record).unwrap();
    let (status, renamed) = call(f.app(), "PATCH", &profile_path, json!({"name":"Renamed"})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(renamed["name"], "Renamed");
    let stored = f.state.store.get_session(session.id).unwrap().unwrap();
    assert_eq!(stored.endpoint_snapshot.as_ref().unwrap().name, "Original");
    assert_eq!(
        call(f.app(), "DELETE", &profile_path, Value::Null).await.0,
        StatusCode::NO_CONTENT
    );
    let stored = f.state.store.get_session(session.id).unwrap().unwrap();
    assert!(
        f.state
            .store
            .get_endpoint_profile(session.endpoint_profile_id.unwrap())
            .unwrap()
            .is_none()
    );
    assert!(
        stored
            .endpoint_snapshot
            .as_ref()
            .unwrap()
            .native_config
            .is_some()
    );
    let spec = launch::build(
        &f.state,
        &stored,
        dunce::canonicalize(f.path.join("repo")).unwrap(),
    )
    .unwrap();
    assert_eq!(spec.env["CODEX_HOME"], config.to_str().unwrap());
    assert!(
        config.join("history-fixture.json").exists(),
        "Unlink must not delete native history/config"
    );
}

#[tokio::test]
async fn native_profile_rejects_missing_changed_or_wrong_provider_sources() {
    let mut f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let workspace = register(&f).await;
    let (source_id, config) = f.native_source(ProviderKind::Codex, Vec::new());
    let (_, p) = call(
        f.app(),
        "POST",
        "/api/endpoint-profiles/import-native",
        json!({"source_id":source_id,"confirmed_shared_config":true}),
    )
    .await;
    let (_, record) = call(
        f.app(),
        "POST",
        &format!("/api/workspaces/{workspace}/sessions"),
        json!({"title":"pinned","provider":"codex","endpoint_profile_id":p["id"]}),
    )
    .await;
    let session: Session = serde_json::from_value(record).unwrap();
    let cwd = dunce::canonicalize(f.path.join("repo")).unwrap();
    let replacement = f.path.join("other-native-account");
    fs::create_dir(&replacement).unwrap();
    f.state.native_sources[0].config_dir = replacement;
    assert_eq!(
        launch::build(&f.state, &session, cwd.clone())
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            f.app(),
            "POST",
            &format!("/api/workspaces/{workspace}/sessions"),
            json!({"title":"changed source","provider":"codex","endpoint_profile_id":p["id"]})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    f.state.native_sources[0].config_dir = config.clone();
    f.state.native_sources[0].provider = ProviderKind::ClaudeCode;
    assert_eq!(
        launch::build(&f.state, &session, cwd.clone())
            .unwrap_err()
            .status,
        StatusCode::BAD_REQUEST
    );
    f.state.native_sources[0].provider = ProviderKind::Codex;
    f.state.native_sources[0].config_dir = f.path.join("missing-native-source");
    assert_eq!(
        launch::build(&f.state, &session, cwd.clone())
            .unwrap_err()
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            f.app(),
            "POST",
            "/api/endpoint-profiles/import-native",
            json!({"source_id":source_id,"confirmed_shared_config":true})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    f.state.native_sources.clear();
    assert_eq!(
        launch::build(&f.state, &session, cwd).unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    assert!(config.join("history-fixture.json").exists());
}

#[tokio::test]
async fn native_configuration_sources_are_not_public_without_authentication() {
    let f = Fixture::new(
        "127.0.0.1:8787".parse().unwrap(),
        Some("a-long-enough-fixture-token-12345"),
    );
    assert_eq!(
        call(
            f.app(),
            "GET",
            "/api/host/native-configurations",
            Value::Null
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(
            f.app(),
            "POST",
            "/api/endpoint-profiles/import-native",
            json!({"source_id":"codex-default","confirmed_shared_config":true})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}

#[test]
fn native_config_keeps_default_and_explicit_login_contexts_distinct() {
    for (provider, key) in [
        (ProviderKind::ClaudeCode, "CLAUDE_CONFIG_DIR"),
        (ProviderKind::Codex, "CODEX_HOME"),
    ] {
        let mut f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
        let (source_id, config) = f.native_source(provider, Vec::new());
        let cwd = dunce::canonicalize(f.path.join("repo")).unwrap();
        f.state.native_sources[0].config_env = None;
        let (_, reference) = native_config::pin(&f.state, &source_id).unwrap();
        assert!(reference.config_env.is_none());
        let spec = native_config::build(&f.state, &provider, &reference, cwd.clone()).unwrap();
        assert!(
            !spec.env.contains_key(key),
            "Do not replace an unset default home with an explicit same-path env value"
        );
        assert!(spec.env_remove.iter().any(|removed| removed == key));
        assert!(
            native_config::source_views(&f.state)[0]
                .config_env
                .is_none()
        );
        f.state.native_sources[0].config_env = Some(config.as_os_str().to_owned());
        assert_eq!(
            native_config::validate_reference(&f.state, &provider, &reference)
                .unwrap_err()
                .status,
            StatusCode::CONFLICT
        );
        let (_, explicit) = native_config::pin(&f.state, &source_id).unwrap();
        let spec = native_config::build(&f.state, &provider, &explicit, cwd).unwrap();
        assert_eq!(spec.env[key], config.to_str().unwrap());
        assert!(!spec.env_remove.iter().any(|removed| removed == key));
        f.state.native_sources[0].config_env = Some("relative-native-home".into());
        assert!(
            native_config::pin(&f.state, &source_id)
                .unwrap_err()
                .message
                .contains("relative")
        );
    }
}

#[cfg(unix)]
#[test]
fn native_config_preserves_explicit_directory_spelling_and_rejects_symlink_retarget() {
    use std::os::unix::fs::symlink;
    let mut f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let (source_id, config) = f.native_source(ProviderKind::ClaudeCode, Vec::new());
    let alias = f.path.join("original-native-alias");
    symlink(&config, &alias).unwrap();
    f.state.native_sources[0].config_dir = alias.clone();
    f.state.native_sources[0].config_env = Some(alias.as_os_str().to_owned());
    let (_, reference) = native_config::pin(&f.state, &source_id).unwrap();
    assert_eq!(reference.config_dir, config.to_str().unwrap());
    assert_eq!(reference.config_env.as_deref(), alias.to_str());
    let views = native_config::source_views(&f.state);
    assert_eq!(views[0].path, reference.config_dir);
    assert_eq!(views[0].config_env, reference.config_env);
    let spec = native_config::build(
        &f.state,
        &ProviderKind::ClaudeCode,
        &reference,
        dunce::canonicalize(f.path.join("repo")).unwrap(),
    )
    .unwrap();
    assert_eq!(spec.env["CLAUDE_CONFIG_DIR"], alias.to_str().unwrap());
    let replacement = f.path.join("another-account");
    fs::create_dir(&replacement).unwrap();
    fs::remove_file(&alias).unwrap();
    symlink(replacement, alias).unwrap();
    assert_eq!(
        native_config::validate_reference(&f.state, &ProviderKind::ClaudeCode, &reference)
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
}

/// A model check is a POST the endpoint accepts or refuses per model; the
/// interface then prunes to what worked. The route sees the form's draft
/// profile, so a key pasted but not yet saved still checks the endpoint.
#[tokio::test]
async fn a_model_check_asks_the_endpoint_once_per_model_and_reports_what_runs() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let endpoint = tokio::spawn(async move {
        let mut paths = Vec::new();
        for body in [
            r#"{"id":"resp_1"}"#,
            r#"{"error":{"message":"no such model"}}"#,
        ] {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0; 16 * 1024];
            let n = socket.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).into_owned();
            paths.push(request.lines().next().unwrap_or_default().to_owned());
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(reply.as_bytes()).await.unwrap();
        }
        paths
    });
    let (status, results) = call(
        f.app(),
        "POST",
        "/api/endpoint-profiles/probe-models",
        json!({
            "profile": {"name":"draft","provider":"codex","endpoint_url":format!("http://{address}/v1")},
            "models": ["good-model", "bad-model"],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{results}");
    let rows: Vec<(&str, &str)> = results
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["model"].as_str().unwrap(),
                row["status"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(rows, [("good-model", "ok"), ("bad-model", "failed")]);
    // The protocol travels with each answer: a model can work on one API and
    // fail on another, so a row without it would be ambiguous.
    assert!(
        results
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["api"] == "openai-responses"),
        "Codex speaks the Responses API: {results}"
    );
    let paths = endpoint.await.unwrap();
    assert_eq!(paths.len(), 2);
    assert!(
        paths
            .iter()
            .all(|path| path == "POST /v1/responses HTTP/1.1")
    );
}

/// The form shows which protocol a check would speak before anything is
/// billed, so the plan names the API and the path without sending a request.
#[tokio::test]
async fn a_probe_plan_names_the_protocol_and_path_without_asking_the_endpoint() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    // Nothing listens here: a plan that made a request would fail or hang.
    for (provider, api, path, guessed) in [
        ("claude_code", "anthropic-messages", "/v1/messages", false),
        ("codex", "openai-responses", "/v1/responses", false),
        // Pi speaks whichever API the profile names, or guesses from the URL.
        ("pi", "anthropic-messages", "/v1/messages", true),
        ("pi", "openai-responses", "/v1/responses", false),
        ("pi", "openai-completions", "/v1/chat/completions", false),
        (
            "pi",
            "google-generative-ai",
            "/v1beta/models/model:generateContent",
            false,
        ),
    ] {
        let mut body = json!({
            "name": "draft",
            "provider": provider,
            "endpoint_url": "https://gateway.test",
        });
        if provider == "pi" && !guessed {
            body["api"] = json!(api);
        }
        if provider == "pi" && guessed {
            body["endpoint_url"] = json!("https://api.anthropic.com");
        }
        let (status, plan) = call(f.app(), "POST", "/api/endpoint-profiles/probe-plan", body).await;
        assert_eq!(status, StatusCode::OK, "{plan}");
        assert_eq!(plan["api"], api, "{provider} {plan}");
        assert_eq!(plan["path"], path, "{provider} {plan}");
        assert_eq!(plan["guessed"], guessed, "{provider} {plan}");
    }
    // A client that cannot be checked without starting it says so, rather
    // than naming a protocol it would not speak.
    let (status, body) = call(
        f.app(),
        "POST",
        "/api/endpoint-profiles/probe-plan",
        json!({"name":"terminal","provider":"terminal"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test]
async fn a_model_check_refuses_a_batch_it_cannot_finish_and_names_the_reason() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let (status, body) = call(
        f.app(),
        "POST",
        "/api/endpoint-profiles/probe-models",
        json!({
            "profile": {"name":"draft","provider":"codex","endpoint_url":"https://example.test/v1"},
            "models": [],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("at least one model")
    );
    let many: Vec<String> = (0..25).map(|i| format!("m{i}")).collect();
    let (status, body) = call(
        f.app(),
        "POST",
        "/api/endpoint-profiles/probe-models",
        json!({
            "profile": {"name":"draft","provider":"codex","endpoint_url":"https://example.test/v1"},
            "models": many,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("at most 24"));
}
