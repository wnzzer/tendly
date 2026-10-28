//! The host around the workspaces: the tailnet listener, client discovery, preferences, granted files and resource history.

use super::*;

/// Tailscale forwards from 127.0.0.1, so the tailnet listener has to ask for
/// the token even where this machine's own browser is asked for nothing, and a
/// signed-in phone must not get what only this machine may have.
#[tokio::test]
async fn the_tailnet_listener_needs_the_token_where_this_machine_does_not() {
    let mut f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    f.state.security = security::Security::for_test("127.0.0.1:8787".parse().unwrap(), None)
        .with_access_token(Some("fixture-token-long-enough".into()));
    f.state.tailnet = crate::tailscale::Tailnet::new(&f.path, 8790);
    let tailnet = |method: &str, path: &str, token: Option<&str>| {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "127.0.0.1:8790")
            .header("x-tendly-client", "web")
            .header("content-type", "application/json");
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        // Everything Tailscale forwards comes from loopback.
        let app = f
            .app()
            .layer(axum::Extension(security::ViaTailnet))
            .layer(axum::Extension(axum::extract::ConnectInfo::<SocketAddr>(
                "127.0.0.1:50000".parse().unwrap(),
            )));
        let request = request.body(Body::from("{}")).unwrap();
        async move { app.oneshot(request).await.unwrap().status() }
    };
    assert_eq!(
        call(f.app(), "GET", "/api/workspaces", Value::Null).await.0,
        StatusCode::OK
    );
    assert_eq!(
        tailnet("GET", "/api/workspaces", None).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        tailnet("GET", "/api/workspaces", Some("guess")).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        tailnet("GET", "/api/workspaces", Some("fixture-token-long-enough")).await,
        StatusCode::OK
    );
    let (_, auth) = call(f.app(), "GET", "/api/auth", Value::Null).await;
    assert_eq!(
        (auth["required"].clone(), auth["via_tailnet"].clone()),
        (json!(false), json!(false))
    );
    // Resuming hands out a launch plan with resolved keys: this machine only.
    assert_eq!(
        tailnet(
            "POST",
            &format!("/api/sessions/{}/launch-plan", Uuid::new_v4()),
            Some("fixture-token-long-enough")
        )
        .await,
        StatusCode::FORBIDDEN
    );
    std::fs::remove_dir_all(&f.path).ok();
}

/// Client discovery is read-only, and installing is an explicit action that
/// downloads and runs package code.
#[tokio::test]
async fn client_discovery_reports_this_host_and_refuses_an_unconfirmed_install() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let (status, clients) = call(f.app(), "GET", "/api/clients", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    let clients = clients.as_array().unwrap();
    // Terminal is a shell, not an installable agent client.
    assert_eq!(clients.len(), ProviderKind::AGENTS.len());
    for client in clients {
        let provider = client["provider"].as_str().unwrap();
        assert_eq!(
            client["npm_package"],
            match provider {
                "codex" => "@openai/codex",
                "claude_code" => "@anthropic-ai/claude-code",
                "pi" => "@earendil-works/pi-coding-agent",
                other => panic!("an unexpected client {other}"),
            }
        );
        // An unknown client is never reported as installed, and a program name
        // is always present so a failed launch keeps its own error.
        assert!(
            client["program"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
        if client["source"] == "missing" {
            assert_eq!(client["installed"], false);
            assert!(client["version"].is_null());
        }
        // Nothing is installed by looking.
        assert!(!crate::clients::managed_root(&f.state.state_dir).exists());
    }
    for (provider, body, expected) in [
        (
            "codex",
            json!({"confirmed": false}),
            StatusCode::BAD_REQUEST,
        ),
        (
            "terminal",
            json!({"confirmed": true}),
            StatusCode::BAD_REQUEST,
        ),
        (
            "nonsense",
            json!({"confirmed": true}),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let (status, _) = call(
            f.app(),
            "POST",
            &format!("/api/clients/{provider}/install"),
            body,
        )
        .await;
        assert_eq!(status, expected, "{provider}");
    }
    assert!(!crate::clients::managed_root(&f.state.state_dir).exists());
}

#[tokio::test]
async fn preferences_are_validated_per_provider_and_kept() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let (status, empty) = call(f.app(), "GET", "/api/preferences", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(empty, json!({"claude_code":{},"codex":{},"pi":{}}));
    // Codex has no plan mode, and a depth must be one the clients know.
    for bad in [
        json!({"codex":{"permission":"plan"}}),
        json!({"claude_code":{"effort":"extreme"}}),
        json!({"claude_code":{"endpoint_profile_id":Uuid::new_v4()}}),
    ] {
        assert_ne!(
            call(f.app(), "PUT", "/api/preferences", bad).await.0,
            StatusCode::OK
        );
    }
    let (_, profile) = call(
        f.app(),
        "POST",
        "/api/endpoint-profiles",
        json!({"name":"work","provider":"codex","endpoint_url":"https://example.test/v1","permission_mode":"interactive"}),
    )
    .await;
    let pid = profile["id"].as_str().unwrap();
    // A profile belongs to one provider's preferences only.
    assert_eq!(
        call(
            f.app(),
            "PUT",
            "/api/preferences",
            json!({"claude_code":{"endpoint_profile_id":pid}})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let chosen = json!({"claude_code":{"effort":"high","permission":"plan"},"codex":{"endpoint_profile_id":pid,"permission":"ask"}});
    let (status, _) = call(f.app(), "PUT", "/api/preferences", chosen.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        call(f.app(), "GET", "/api/preferences", Value::Null)
            .await
            .1,
        json!({"claude_code":chosen["claude_code"],"codex":chosen["codex"],"pi":{}}),
        "every client is listed, set or not"
    );
    // A new session takes the preferred depth unless it says otherwise; an
    // explicit null is "automatic" and takes none.
    let workspace = register(&f).await;
    let path = format!("/api/workspaces/{workspace}/sessions");
    let (_, defaulted) = call(
        f.app(),
        "POST",
        &path,
        json!({"title":"a","provider":"claude_code"}),
    )
    .await;
    assert_eq!(
        defaulted["endpoint_snapshot"]["effort"], "high",
        "{defaulted}"
    );
    let (_, automatic) = call(
        f.app(),
        "POST",
        &path,
        json!({"title":"b","provider":"claude_code","effort":null}),
    )
    .await;
    assert!(
        automatic["endpoint_snapshot"]["effort"].is_null(),
        "{automatic}"
    );
    let (_, low) = call(
        f.app(),
        "POST",
        &path,
        json!({"title":"c","provider":"claude_code","effort":"low"}),
    )
    .await;
    assert_eq!(low["endpoint_snapshot"]["effort"], "low");
}

#[tokio::test]
async fn a_granted_file_outside_the_roots_can_be_previewed_until_revoked() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let outside = f.path.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("notes.log"), "outside the roots").unwrap();
    let file = dunce::canonicalize(outside.join("notes.log")).unwrap();
    let preview = format!("/api/host/file?path={}", url_escape(file.to_str().unwrap()));
    let raw = format!("/api/host/raw?path={}", url_escape(file.to_str().unwrap()));
    let (status, _) = call(f.app(), "GET", &preview, Value::Null).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "outside the browsing roots");
    let (status, _) = call(f.app(), "GET", &raw, Value::Null).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the raw bytes have the same reach"
    );

    let (status, grants) = call(
        f.app(),
        "POST",
        "/api/host/grants",
        json!({"path": file, "directory": false}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(grants[0]["directory"], false);
    let (status, body) = call(f.app(), "GET", &preview, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["content"], "outside the roots");
    let (status, _) = call(f.app(), "GET", &raw, Value::Null).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an image or video there can be shown too"
    );

    let (status, _) = call(f.app(), "DELETE", "/api/host/grants/all", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = call(f.app(), "GET", &preview, Value::Null).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "revoked");
    let (status, _) = call(f.app(), "GET", &raw, Value::Null).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "revoked for the raw bytes too"
    );
}

#[tokio::test]
async fn resource_history_answers_from_the_metrics_database() {
    use tendly_persistence::metrics::{HostPoint, MetricsStore, SessionPoint};
    let mut f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let (status, _) = call(f.app(), "GET", "/api/host/resources/history", Value::Null).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "no metrics database, no history"
    );

    let store = Arc::new(MetricsStore::open(":memory:").unwrap());
    let now = chrono::Utc::now().timestamp();
    let host = |ts| HostPoint {
        ts,
        cpu_avg: 12.5,
        cpu_max: 12.5,
        memory_used_avg: 100,
        memory_used_max: 100,
        memory_total: 1000,
        fan_rpm_avg: Some(1200),
        fan_rpm_max: Some(1200),
        temperature_avg: Some(48.5),
        temperature_max: Some(48.5),
        cpu_mhz_avg: Some(3204),
        cpu_mhz_max: Some(3504),
        power_avg: Some(21.5),
        power_max: Some(21.5),
        gpu_avg: Some(24.0),
        gpu_max: Some(24.0),
    };
    let session = |ts, id: &str| SessionPoint {
        ts,
        session_id: id.into(),
        cpu_avg: 3.0,
        cpu_max: 3.0,
        memory_avg: 10,
        memory_max: 10,
    };
    store
        .record(
            &[host(now - 30), host(now - 25)],
            &[session(now - 30, "a"), session(now - 30, "b")],
        )
        .unwrap();
    f.state.metrics = Some(store);

    let (status, body) = call(f.app(), "GET", "/api/host/resources/history", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["resolution"], 5);
    assert_eq!(body["host"].as_array().unwrap().len(), 2);
    assert_eq!(body["host"][0]["cpu_avg"], 12.5);
    assert_eq!(body["host"][0]["fan_rpm_max"], 1200);
    assert_eq!(body["host"][0]["temperature_max"], 48.5);
    assert_eq!(body["host"][0]["cpu_mhz_max"], 3504);
    assert_eq!(body["host"][0]["power_max"], 21.5);
    assert_eq!(body["host"][0]["gpu_max"], 24.0);
    assert_eq!(body["sessions"].as_array().unwrap().len(), 2);

    let (_, body) = call(
        f.app(),
        "GET",
        "/api/host/resources/history?session_id=b",
        Value::Null,
    )
    .await;
    assert_eq!(body["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(body["sessions"][0]["session_id"], "b");

    let path = format!("/api/host/resources/history?from={now}&to={}", now - 1);
    let (status, _) = call(f.app(), "GET", &path, Value::Null).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn resource_monitoring_can_be_turned_off_and_history_cleared() {
    use tendly_persistence::metrics::{HostPoint, MetricsStore};
    let mut f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let store = Arc::new(MetricsStore::open(":memory:").unwrap());
    let now = chrono::Utc::now().timestamp();
    store
        .record(
            &[HostPoint {
                ts: now - 30,
                cpu_avg: 1.0,
                cpu_max: 1.0,
                memory_used_avg: 1,
                memory_used_max: 1,
                memory_total: 2,
                fan_rpm_avg: None,
                fan_rpm_max: None,
                temperature_avg: None,
                temperature_max: None,
                cpu_mhz_avg: None,
                cpu_mhz_max: None,
                power_avg: None,
                power_max: None,
                gpu_avg: None,
                gpu_max: None,
            }],
            &[],
        )
        .unwrap();
    f.state.metrics = Some(store);
    let history = "/api/host/resources/history";
    let (_, body) = call(f.app(), "GET", history, Value::Null).await;
    assert_eq!(body["monitoring"], true, "on unless turned off");
    for live in ["/api/host/resources", "/api/host/system"] {
        let (status, _) = call(f.app(), "GET", live, Value::Null).await;
        assert_eq!(status, StatusCode::OK, "{live}");
    }

    let (status, preferences) = call(
        f.app(),
        "PUT",
        "/api/preferences",
        json!({"resource_monitoring": false}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(preferences["resource_monitoring"], false);
    for live in ["/api/host/resources", "/api/host/system"] {
        let (status, _) = call(f.app(), "GET", live, Value::Null).await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "{live} reads nothing while off"
        );
    }
    let (_, body) = call(f.app(), "GET", history, Value::Null).await;
    assert_eq!(body["monitoring"], false);
    assert_eq!(
        body["host"].as_array().unwrap().len(),
        1,
        "turning monitoring off keeps what was recorded"
    );

    let (status, _) = call(f.app(), "DELETE", history, Value::Null).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, body) = call(f.app(), "GET", history, Value::Null).await;
    assert!(body["host"].as_array().unwrap().is_empty());
}

/// A folder added in Settings is where a workspace may then be, and only a
/// browser on this machine may add one: not the network, not the tailnet.
#[tokio::test]
async fn a_folder_is_added_from_this_machine_and_then_takes_a_workspace() {
    let mut f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    // The tailnet always asks for the token; a signed-in phone has it.
    f.state.security = security::Security::for_test("127.0.0.1:8787".parse().unwrap(), None)
        .with_access_token(Some("fixture-token-long-enough".into()));
    let outside = std::env::temp_dir().join(format!("tendly-outside-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&outside).unwrap();
    let outside = dunce::canonicalize(&outside).unwrap();
    let folder = json!({ "path": outside.to_string_lossy() });
    let from = |address: &str, tailnet: bool, method: &str, body: Value| {
        let mut app = f
            .app()
            .layer(axum::Extension(axum::extract::ConnectInfo::<SocketAddr>(
                address.parse().unwrap(),
            )));
        if tailnet {
            app = app.layer(axum::Extension(security::ViaTailnet));
        }
        let request = Request::builder()
            .method(method)
            .uri("/api/host/roots")
            .header("host", "127.0.0.1:8787")
            .header("x-tendly-client", "web")
            .header("authorization", "Bearer fixture-token-long-enough")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        async move {
            let response = app.oneshot(request).await.unwrap();
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            (
                status,
                serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
            )
        }
    };
    let workspace = json!({ "name": "outside", "root_path": outside.to_string_lossy() });
    assert_eq!(
        call(f.app(), "POST", "/api/workspaces", workspace.clone())
            .await
            .0,
        StatusCode::BAD_REQUEST
    );

    let (_, seen) = from("192.168.1.9:50000", false, "GET", Value::Null).await;
    assert_eq!(seen["editable"], json!(false));
    for (address, tailnet) in [("192.168.1.9:50000", false), ("127.0.0.1:50000", true)] {
        assert_eq!(
            from(address, tailnet, "POST", folder.clone()).await.0,
            StatusCode::FORBIDDEN,
            "{address} tailnet={tailnet}"
        );
    }

    let (status, roots) = from("127.0.0.1:50000", false, "POST", folder.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (roots["editable"].clone(), roots["added"].clone()),
        (json!(true), json!([outside.to_string_lossy()]))
    );
    assert_eq!(
        call(f.app(), "POST", "/api/workspaces", workspace).await.0,
        StatusCode::CREATED
    );

    let (_, roots) = from("127.0.0.1:50000", false, "DELETE", folder).await;
    assert_eq!(roots["added"], json!([]));
    std::fs::remove_dir_all(&outside).ok();
    std::fs::remove_dir_all(&f.path).ok();
}
