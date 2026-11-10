use super::*;

#[tokio::test]
async fn balance_config_query_trial_validation_and_deletion() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let (_, profile) = call(
        f.app(),
        "POST",
        "/api/endpoint-profiles",
        json!({"name":"balance","provider":"codex"}),
    )
    .await;
    let id = profile["id"].as_str().unwrap();
    let path = format!("/api/endpoint-profiles/{id}/balance");
    let config = json!({"unit":{"kind":"money","currency":"CNY"},"query_url":"https://example.invalid/balance","auth":{"kind":"none"},"secret_ref":null,"script":"return {value:'123.4500'};"});
    let (status, view) = call(f.app(), "PUT", &path, config.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert!(view["reading"].is_null());
    let (status, view) = call(f.app(), "POST", &format!("{path}/query"), Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(view["reading"]["value"], "123.4500");
    assert_eq!(view["reading"]["unit"]["currency"], "CNY");
    let (_, again) = call(f.app(), "POST", &format!("{path}/query"), Value::Null).await;
    assert_eq!(
        view["reading"]["checked_at"], again["reading"]["checked_at"],
        "cooldown reuses the saved reading"
    );
    let mut draft = config.clone();
    draft["unit"] = json!({"kind":"percentage"});
    draft["script"] = json!("return {value:101};");
    let (status, _) = call(f.app(), "POST", &format!("{path}/test"), draft.clone()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    draft["script"] = json!("return {value:72};");
    let (status, trial) = call(f.app(), "POST", &format!("{path}/test"), draft.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(trial["value"], "72");
    let (_, saved) = call(f.app(), "GET", &path, Value::Null).await;
    assert_eq!(saved["config"], config);
    assert_eq!(
        saved["reading"]["value"], "123.4500",
        "trial does not replace the saved reading"
    );
    let (_, saved) = call(f.app(), "PUT", &path, draft).await;
    assert!(
        saved["reading"].is_null(),
        "changing units clears old readings"
    );
    let (_, list) = call(f.app(), "GET", "/api/endpoint-balances", Value::Null).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert!(list[0].get("config").is_none());
    call(
        f.app(),
        "DELETE",
        &format!("/api/endpoint-profiles/{id}"),
        Value::Null,
    )
    .await;
    assert!(
        f.state
            .store
            .endpoint_balance(id.parse().unwrap())
            .unwrap()
            .is_none(),
        "deleting a profile cascades its balance data"
    );
}

#[tokio::test]
async fn failed_balance_query_keeps_last_success_and_config_changes_win_races() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let (_, profile) = call(
        f.app(),
        "POST",
        "/api/endpoint-profiles",
        json!({"name":"balance","provider":"codex"}),
    )
    .await;
    let id: Uuid = profile["id"].as_str().unwrap().parse().unwrap();
    let path = format!("/api/endpoint-profiles/{id}/balance");
    let document = json!({"config":{"unit":{"kind":"percentage"},"query_url":"https://example.invalid/balance","auth":{"kind":"none"},"script":"throw Error('provider-secret-value')"},
        "reading":{"value":"42","unit":{"kind":"percentage"},"checked_at":chrono::Utc::now()},"attempted_at":null,"error":null});
    let before = document.to_string();
    f.state.store.set_endpoint_balance(id, &before).unwrap();
    let (status, view) = call(f.app(), "POST", &format!("{path}/query"), Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(view["reading"]["value"], "42");
    assert_eq!(view["stale"], true);
    assert!(
        !view["error"]
            .as_str()
            .unwrap()
            .contains("provider-secret-value")
    );
    assert!(
        !f.state
            .store
            .update_endpoint_balance_result(id, &before, "{}")
            .unwrap(),
        "stale writes do not replace newer state"
    );
}

#[tokio::test]
async fn balance_mcp_requires_configuration_review_and_only_tests_saved_scripts() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let (_, profile) = call(
        f.app(),
        "POST",
        "/api/endpoint-profiles",
        json!({"name":"balance","provider":"codex"}),
    )
    .await;
    let args = json!({"id":profile["id"],"action":"configure","config":{"unit":{"kind":"percentage"},"query_url":"https://example.invalid/balance","auth":{"kind":"none"},"script":"return {value:50}"}});
    let err = balances::call(&f.state, agent::Caller::External, &args)
        .await
        .unwrap_err();
    assert!(err.message.contains("confirm"));
    let err = balances::call(
        &f.state,
        agent::Caller::External,
        &json!({"id":profile["id"],"action":"test"}),
    )
    .await
    .unwrap_err();
    assert!(err.message.contains("Save"));
}

#[tokio::test]
async fn balance_mcp_saves_only_after_browser_approval_then_can_query() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let (_, profile) = call(
        f.app(),
        "POST",
        "/api/endpoint-profiles",
        json!({"name":"review balance","provider":"codex"}),
    )
    .await;
    let config = json!({"unit":{"kind":"percentage"},"query_url":"https://example.invalid/balance","auth":{"kind":"none"},"script":"return { value: 62 };"});
    let args = json!({"id":profile["id"],"action":"configure","config":config});
    let mut events = f.state.activity.subscribe();
    let state = f.state.clone();
    let pending =
        tokio::spawn(async move { balances::call(&state, agent::Caller::External, &args).await });
    let request = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(request["type"], "request");
    assert!(
        request["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["label"] == "JavaScript" && row["value"] == config["script"])
    );
    let id: Uuid = profile["id"].as_str().unwrap().parse().unwrap();
    assert!(f.state.store.endpoint_balance(id).unwrap().is_none());
    let request_id = request["id"].as_str().unwrap();
    let (status, _) = call(
        f.app(),
        "POST",
        &format!("/api/agent-activity/requests/{request_id}"),
        json!({"approve":true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(pending.await.unwrap().is_ok());
    let result = balances::call(
        &f.state,
        agent::Caller::External,
        &json!({"id":id,"action":"query"}),
    )
    .await
    .unwrap();
    assert_eq!(result["reading"]["value"], "62");
}

#[tokio::test]
async fn switching_auto_refresh_keeps_the_reading_and_shows_in_the_list() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let (_, profile) = call(
        f.app(),
        "POST",
        "/api/endpoint-profiles",
        json!({"name":"relay","provider":"codex"}),
    )
    .await;
    let id = profile["id"].as_str().unwrap();
    let path = format!("/api/endpoint-profiles/{id}/balance");
    let mut config = json!({"unit":{"kind":"money","currency":"CNY"},"query_url":"https://example.invalid/balance","auth":{"kind":"none"},"secret_ref":null,"script":"return {value:'8.00'};"});
    call(f.app(), "PUT", &path, config.clone()).await;
    let (_, queried) = call(f.app(), "POST", &format!("{path}/query"), Value::Null).await;
    assert_eq!(queried["reading"]["value"], "8.00");
    let (_, rows) = call(f.app(), "GET", "/api/endpoint-balances", Value::Null).await;
    assert!(rows[0]["auto_refresh"].is_null(), "unset means on");

    // When to ask is not what a reading means: switching it keeps the figure.
    config["auto_refresh"] = json!(false);
    let (status, view) = call(f.app(), "PUT", &path, config.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(view["config"]["auto_refresh"], false);
    assert_eq!(view["reading"]["value"], "8.00");
    let (_, rows) = call(f.app(), "GET", "/api/endpoint-balances", Value::Null).await;
    assert_eq!(rows[0]["auto_refresh"], false);

    // A different query still starts afresh.
    config["script"] = json!("return {value:'9.00'};");
    let (_, view) = call(f.app(), "PUT", &path, config).await;
    assert!(view["reading"].is_null());
    assert_eq!(view["config"]["auto_refresh"], false);
}
