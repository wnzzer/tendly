//! The `/api/accounts` routes.

use super::*;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/accounts", get(list).post(create))
        .route("/api/accounts/import-native", post(import_native))
        .route("/api/accounts/{id}", get(read))
        .route("/api/accounts/{id}/rename", post(rename))
        .route("/api/accounts/{id}/remove", post(remove))
        .route("/api/accounts/{id}/proxy", post(set_proxy))
        .route("/api/accounts/{id}/refresh", post(refresh))
        .route("/api/accounts/{id}/usage", post(usage))
        .route("/api/accounts/{id}/login", post(login))
        .route("/api/accounts/{id}/cancel-login", post(cancel_login))
        .route("/api/accounts/{id}/logout", post(logout))
        .route("/api/accounts/{id}/reset-quota", post(reset_quota))
        .layer(axum::middleware::from_fn(
            |request: axum::extract::Request, next: axum::middleware::Next| async move {
                let mut response = next.run(request).await;
                response.headers_mut().insert(
                    axum::http::header::CACHE_CONTROL,
                    axum::http::HeaderValue::from_static("no-store"),
                );
                response
            },
        ))
}

pub(crate) async fn list(State(state): State<AppState>) -> Result<Json<Vec<AccountView>>> {
    tokio::task::spawn_blocking(move || {
        let _guard = state
            .accounts
            .metadata
            .lock()
            .expect("account metadata lock");
        let root = match root_directory(&state, false) {
            Ok(path) => path,
            Err(error) if error.status == axum::http::StatusCode::NOT_FOUND => {
                return Ok(Json(Vec::new()));
            }
            Err(error) => return Err(error),
        };
        let mut result = Vec::new();
        for (index, entry) in fs::read_dir(root).map_err(ApiError::internal)?.enumerate() {
            if index >= 2000 {
                return Err(ApiError::bad("Too many account entries"));
            }
            let entry = entry.map_err(ApiError::internal)?;
            if let Some(id) = entry
                .file_name()
                .to_str()
                .and_then(|id| Uuid::parse_str(id).ok())
            {
                result.push(view(&state, id)?);
            }
        }
        result.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
        Ok(Json(result))
    })
    .await
    .map_err(ApiError::internal)?
}
pub(super) async fn read(
    State(state): State<AppState>,
    RoutePath(id): RoutePath<Uuid>,
) -> Result<Json<AccountView>> {
    tokio::task::spawn_blocking(move || view(&state, id).map(Json))
        .await
        .map_err(ApiError::internal)?
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateAccount {
    pub(super) name: String,
    pub(super) provider: ProviderKind,
}
pub(super) async fn create(
    State(state): State<AppState>,
    Json(input): Json<CreateAccount>,
) -> Result<Json<AccountView>> {
    tokio::task::spawn_blocking(move || {
        if input.name.trim().is_empty()
            || input.name.trim().len() > 120
            || !input.provider.is_agent()
        {
            return Err(ApiError::bad(
                "Choose an agent client and a name of 1–120 bytes",
            ));
        }
        let _guard = state
            .accounts
            .metadata
            .lock()
            .expect("account metadata lock");
        let record = AccountRecord {
            id: Uuid::new_v4(),
            name: input.name.trim().into(),
            provider: input.provider,
            profile_id: Uuid::new_v4(),
            native_config: None,
            proxy_url: None,
        };
        let directory = protected_directory(
            &root_directory(&state, true)?.join(record.id.to_string()),
            true,
        )?;
        let metadata_path = directory.join("account.json");
        write_metadata_new(
            &metadata_path,
            &serde_json::to_vec(&record).map_err(ApiError::internal)?,
        )?;
        for (name, contents) in crate::adapters::agent(record.provider)?.account_files() {
            write_new(&directory.join(name), contents)?;
        }
        let (_, reference) = pin(&state, &format!("account:{}", record.id))?;
        let profile = EndpointProfile {
            id: record.profile_id,
            name: record.name.clone(),
            provider: record.provider,
            endpoint_url: None,
            model: None,
            permission_mode: "native".into(),
            secret_ref: None,
            proxy_url: None,
            effort: None,
            model_aliases: Default::default(),
            models: Vec::new(),
            api: None,
            key_header: None,
            native_config: Some(reference),
            environment: Default::default(),
            created_at: chrono::Utc::now(),
        };
        if let Err(error) = state.store.create_endpoint_profile(&profile) {
            let _ = fs::remove_file(metadata_path);
            let _ = fs::remove_file(directory.join("config.toml"));
            let _ = fs::remove_dir(directory);
            return Err(ApiError::internal(error));
        }
        initial_view(&state, &record).map(Json)
    })
    .await
    .map_err(ApiError::internal)?
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ImportNativeAccount {
    pub(super) source_id: String,
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) confirmed_shared_config: bool,
}

/// Link an existing Claude/Codex native home as an official account. This is
/// deliberately a reference-only operation: no source files or credentials
/// are opened, copied, or returned to the browser.
pub(super) async fn import_native(
    State(state): State<AppState>,
    Json(input): Json<ImportNativeAccount>,
) -> Result<Json<AccountView>> {
    if !input.confirmed_shared_config {
        return Err(ApiError::bad(
            "Confirm sharing the original client configuration before importing it",
        ));
    }
    tokio::task::spawn_blocking(move || {
        let _guard = state
            .accounts
            .metadata
            .lock()
            .expect("account metadata lock");
        let config_state = state.clone();
        let (provider, reference) = crate::native_config::pin(&config_state, &input.source_id)?;
        if provider == ProviderKind::Terminal {
            return Err(ApiError::bad(
                "Terminal has no native account configuration",
            ));
        }
        // Import is idempotent per native source. Reusing an existing account
        // keeps one endpoint/profile for one host configuration instead of
        // creating duplicate account rows after a repeated click.
        if let Ok(root) = root_directory(&state, false) {
            for entry in fs::read_dir(root).map_err(ApiError::internal)? {
                let entry = entry.map_err(ApiError::internal)?;
                let Some(id) = entry
                    .file_name()
                    .to_str()
                    .and_then(|value| Uuid::parse_str(value).ok())
                else {
                    continue;
                };
                if let Ok(existing) = record(&state, id)
                    && existing
                        .native_config
                        .as_ref()
                        .is_some_and(|current| current.source_id == reference.source_id)
                {
                    return initial_view(&state, &existing).map(Json);
                }
            }
        }
        let imported = format!("Imported {} account", provider.label());
        let name = bounded_account_name(input.name.as_deref().unwrap_or(&imported))?;
        let record = AccountRecord {
            id: Uuid::new_v4(),
            name: name.clone(),
            provider,
            profile_id: Uuid::new_v4(),
            native_config: Some(reference.clone()),
            proxy_url: None,
        };
        let metadata_dir = protected_directory(
            &root_directory(&state, true)?.join(record.id.to_string()),
            true,
        )?;
        let metadata_path = metadata_dir.join("account.json");
        write_metadata_new(
            &metadata_path,
            &serde_json::to_vec(&record).map_err(ApiError::internal)?,
        )?;
        let profile = EndpointProfile {
            id: record.profile_id,
            name,
            provider,
            endpoint_url: None,
            model: None,
            permission_mode: "native".into(),
            secret_ref: None,
            proxy_url: None,
            effort: None,
            model_aliases: Default::default(),
            models: Vec::new(),
            api: None,
            key_header: None,
            native_config: Some(reference),
            environment: Default::default(),
            created_at: chrono::Utc::now(),
        };
        if let Err(error) = state.store.create_endpoint_profile(&profile) {
            let _ = fs::remove_file(metadata_path);
            let _ = fs::remove_dir(metadata_dir);
            return Err(ApiError::internal(error));
        }
        initial_view(&state, &record).map(Json)
    })
    .await
    .map_err(ApiError::internal)?
}

pub(super) fn bounded_account_name(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 120 || value.chars().any(char::is_control) {
        return Err(ApiError::bad("Choose an account name of 1–120 bytes"));
    }
    Ok(value.into())
}

/// Used by the profile API to keep account-linked templates managed from the
/// account screen, including imported references whose source id is not
/// `account:<uuid>`.
pub fn owns_profile(state: &AppState, profile_id: Uuid) -> bool {
    let root = match root_directory(state, false) {
        Ok(root) => root,
        Err(_) => return false,
    };
    let Ok(entries) = fs::read_dir(root) else {
        return false;
    };
    entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .any(|id| {
            Uuid::parse_str(&id)
                .ok()
                .and_then(|id| record(state, id).ok())
                .is_some_and(|record| record.profile_id == profile_id)
        })
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct Refresh {
    #[serde(default)]
    pub(super) refresh_token: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Login {
    pub(super) mode: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Confirm {
    pub(super) confirmed: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResetQuota {
    pub(super) confirmed: bool,
    pub(super) idempotency_key: String,
    pub(super) credit_id: Option<String>,
}

pub(super) async fn refresh(
    State(state): State<AppState>,
    RoutePath(id): RoutePath<Uuid>,
    Json(input): Json<Refresh>,
) -> Result<Json<AccountView>> {
    let operation = state.accounts.operation(id);
    let _guard = operation.lock().await;
    let record = record(&state, id)?;
    if input.refresh_token
        && !crate::adapters::agent(record.provider)?
            .account_operations()
            .managed_login
    {
        return Err(ApiError::bad(format!(
            "{} token refresh is managed by its official CLI; this operation is unsupported",
            record.provider.label()
        )));
    }
    ensure_not_pending(&state, id)?;
    let _maintenance = if input.refresh_token {
        Some(begin_maintenance(&state, id).await?)
    } else {
        None
    };
    run_operation(
        &state,
        record,
        json!({"action":"read","refresh_token":input.refresh_token}),
    )
    .await?;
    Ok(Json(view(&state, id)?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RenameAccount {
    pub(super) name: String,
}
/// Rename the account and the endpoint profile that represents it, so the two
/// never drift apart. Nothing about its sign-in or native reference changes.
pub(super) async fn rename(
    State(state): State<AppState>,
    RoutePath(id): RoutePath<Uuid>,
    Json(input): Json<RenameAccount>,
) -> Result<Json<AccountView>> {
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 120 || name.chars().any(char::is_control) {
        return Err(ApiError::bad("Invalid account name"));
    }
    let operation = state.accounts.operation(id);
    let _guard = operation.lock().await;
    let mut record = record(&state, id)?;
    record.name = name.to_owned();
    let path = directory(&state, id)?.join("account.json");
    {
        let _metadata = state
            .accounts
            .metadata
            .lock()
            .expect("accounts metadata lock");
        write_metadata_replace(
            &path,
            &serde_json::to_vec(&record).map_err(ApiError::internal)?,
        )?;
    }
    if let Some(mut profile) = state
        .store
        .get_endpoint_profile(record.profile_id)
        .map_err(ApiError::internal)?
    {
        profile.name = name.to_owned();
        state
            .store
            .update_endpoint_profile(&profile)
            .map_err(ApiError::internal)?;
    }
    state
        .accounts
        .inner
        .lock()
        .expect("accounts lock")
        .views
        .entry(id)
        .and_modify(|view| view.name = name.to_owned());
    view(&state, id).map(Json)
}

/// Remove an account Tendly created or linked. Only Tendly's own account
/// directory is deleted; a linked native home is left completely untouched.
pub(super) async fn remove(
    State(state): State<AppState>,
    RoutePath(id): RoutePath<Uuid>,
    Json(input): Json<Confirm>,
) -> Result<axum::http::StatusCode> {
    if !input.confirmed {
        return Err(ApiError::bad(
            "Removing an account requires explicit confirmation",
        ));
    }
    let operation = state.accounts.operation(id);
    let _guard = operation.lock().await;
    let record = record(&state, id)?;
    // Reuses the running-session check, so an account still in use cannot be
    // pulled out from under a live client.
    let _maintenance = begin_maintenance(&state, id).await?;
    // A sign-in still in flight for this account must not outlive it.
    let pending = state
        .accounts
        .inner
        .lock()
        .expect("accounts lock")
        .pending
        .remove(&id);
    if let Some(pending) = pending {
        finish_pending(pending).await;
    }
    let directory = directory(&state, id)?;
    // Sessions keep their immutable endpoint snapshot, so removing the template
    // does not rewrite any history.
    state
        .store
        .delete_endpoint_profile(record.profile_id)
        .map_err(ApiError::internal)?;
    // Always Tendly's own account directory, never the referenced native
    // home: an isolated account's sign-in lives here and goes with it, while a
    // linked account only ever stored metadata beside the untouched original.
    fs::remove_dir_all(&directory).map_err(ApiError::internal)?;
    state
        .accounts
        .inner
        .lock()
        .expect("accounts lock")
        .views
        .remove(&id);
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProxyPatch {
    /// Null or empty clears the proxy, returning this account to a direct connection.
    pub(super) proxy_url: Option<String>,
}
/// Set the proxy used for this account's own operations. It takes effect on the
/// next sign-in or usage check; nothing in flight is re-routed.
pub(super) async fn set_proxy(
    State(state): State<AppState>,
    RoutePath(id): RoutePath<Uuid>,
    Json(input): Json<ProxyPatch>,
) -> Result<Json<AccountView>> {
    let operation = state.accounts.operation(id);
    let _guard = operation.lock().await;
    ensure_not_pending(&state, id)?;
    let proxy = validated_proxy(input.proxy_url.as_deref())?;
    let mut record = record(&state, id)?;
    record.proxy_url = proxy.clone();
    let path = directory(&state, id)?.join("account.json");
    let _metadata = state
        .accounts
        .metadata
        .lock()
        .expect("accounts metadata lock");
    write_metadata_replace(
        &path,
        &serde_json::to_vec(&record).map_err(ApiError::internal)?,
    )?;
    // Carry it to the account's endpoint profile so sessions started from this
    // account take the same route, instead of the proxy only covering sign-in.
    if let Some(mut profile) = state
        .store
        .get_endpoint_profile(record.profile_id)
        .map_err(ApiError::internal)?
        && profile.proxy_url != proxy
    {
        profile.proxy_url = proxy.clone();
        state
            .store
            .update_endpoint_profile(&profile)
            .map_err(ApiError::internal)?;
    }
    let mut view = view(&state, id)?;
    view.proxy_url = proxy;
    Ok(Json(view))
}
pub(super) async fn usage(
    State(state): State<AppState>,
    RoutePath(id): RoutePath<Uuid>,
) -> Result<Json<AccountView>> {
    let operation = state.accounts.operation(id);
    let _guard = operation.lock().await;
    let record = record(&state, id)?;
    if !crate::adapters::agent(record.provider)?
        .account_operations()
        .usage
    {
        return Err(ApiError::bad(format!(
            "{} usage comes with the account itself; refresh the account instead",
            record.provider.label()
        )));
    }
    ensure_not_pending(&state, id)?;
    let previous = view(&state, id).ok();
    // An explicit request only. Nothing here polls, caches or infers usage, and
    // no credential or private endpoint is touched.
    run_operation(&state, record, json!({"action":"usage"})).await?;
    let mut current = view(&state, id)?;
    // Distinguish "capture is turned off" from "nothing captured yet" so the
    // page can say which one it is instead of asking for a session that would
    // never record anything.
    if !crate::native_config::usage_capture_enabled()
        && current
            .usage
            .as_ref()
            .is_some_and(|usage| usage.availability == "capture_missing")
        && let Some(usage) = current.usage.as_mut()
    {
        usage.availability = "capture_disabled".into();
    }
    // A reading the user spent an allowance on is persisted before anything
    // else can overwrite it, so it survives refreshes and restarts.
    if current
        .usage
        .as_ref()
        .is_some_and(|usage| usage.availability == "available")
        && let Some(limits) = current.limits.clone()
        && (limits.primary.is_some() || limits.secondary.is_some())
    {
        write_usage_cache(
            &state,
            id,
            &CachedUsage {
                limits,
                checked_at: current
                    .checked_at
                    .clone()
                    .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
                source: current
                    .usage
                    .as_ref()
                    .and_then(|usage| usage.source.clone()),
            },
        );
    }
    retain_previous_usage(&mut current, previous.or_else(|| cached_view(&state, id)));
    Ok(Json(current))
}
/// The stored reading as a stand-in previous view, so a refused query can fall
/// back to it even when this process has never served a successful one.
pub(super) fn cached_view(state: &AppState, id: Uuid) -> Option<AccountView> {
    let cached = read_usage_cache(state, id)?;
    let mut view = view(state, id).ok()?;
    view.checked_at = Some(cached.checked_at);
    view.limits = Some(cached.limits);
    view.usage = Some(UsageView {
        availability: "cached".into(),
        capture_path: None,
        captured_at: None,
        source: cached.source,
        retry_after_seconds: None,
        retained_checked_at: None,
    });
    Some(view)
}
/// The official endpoint rate-limits tightly, so pressing Check usage twice
/// would otherwise wipe figures the user just fetched. On a failed attempt keep
/// the previous ones, stamped with when they were read, rather than blanking
/// the panel or presenting them as current.
pub(super) fn retain_previous_usage(current: &mut AccountView, previous: Option<AccountView>) {
    let has_windows = |limits: &Option<Limits>| {
        limits
            .as_ref()
            .is_some_and(|value| value.primary.is_some() || value.secondary.is_some())
    };
    if !current.usage.as_ref().is_some_and(|usage| {
        matches!(
            usage.availability.as_str(),
            "rate_limited" | "query_failed" | "unauthorized"
        )
    }) || has_windows(&current.limits)
    {
        return;
    }
    let Some(previous) = previous.filter(|previous| has_windows(&previous.limits)) else {
        return;
    };
    current.limits = previous.limits;
    if let Some(usage) = current.usage.as_mut() {
        // Provenance travels with the figures, not with the failed attempt, so
        // retained numbers are never relabelled as a different source.
        let before = previous.usage.as_ref();
        usage.source = before.and_then(|usage| usage.source.clone());
        usage.captured_at = before.and_then(|usage| usage.captured_at);
        usage.retained_checked_at = previous.checked_at;
    }
}
pub(super) async fn logout(
    State(state): State<AppState>,
    RoutePath(id): RoutePath<Uuid>,
    Json(input): Json<Confirm>,
) -> Result<Json<AccountView>> {
    if !input.confirmed {
        return Err(ApiError::bad("Logout requires explicit confirmation"));
    }
    let operation = state.accounts.operation(id);
    let _guard = operation.lock().await;
    let record = record(&state, id)?;
    ensure_managed_login(&record)?;
    ensure_not_pending(&state, id)?;
    let _maintenance = begin_maintenance(&state, id).await?;
    run_operation(&state, record, json!({"action":"logout","confirmed":true})).await?;
    Ok(Json(view(&state, id)?))
}
pub(super) async fn reset_quota(
    State(state): State<AppState>,
    RoutePath(id): RoutePath<Uuid>,
    Json(input): Json<ResetQuota>,
) -> Result<Json<AccountView>> {
    if !input.confirmed
        || input.idempotency_key.trim().is_empty()
        || input.idempotency_key.len() > 200
        || input.idempotency_key.chars().any(char::is_control)
        || input.credit_id.as_ref().is_some_and(|value| {
            value.trim().is_empty() || value.len() > 300 || value.chars().any(char::is_control)
        })
    {
        return Err(ApiError::bad(
            "Quota reset requires confirmation and a valid idempotency key",
        ));
    }
    let operation = state.accounts.operation(id);
    let _guard = operation.lock().await;
    let record = record(&state, id)?;
    ensure_managed_login(&record)?;
    ensure_not_pending(&state, id)?;
    let _maintenance = begin_maintenance(&state, id).await?;
    let retry_authorized = reset_keys(&state, id)?.contains(&input.idempotency_key);
    // The bridge re-reads official credits immediately before consumption; a
    // cached UI number can never authorize a locally manufactured reset.
    run_operation(&state, record, json!({"action":"reset_quota","confirmed":true,"idempotency_key":input.idempotency_key,"credit_id":input.credit_id,"retry_authorized":retry_authorized})).await?;
    Ok(Json(view(&state, id)?))
}
/// Sign-in, token refresh and sign-out run through Tendly only for a
/// client whose own tooling offers them.
pub(super) fn ensure_managed_login(record: &AccountRecord) -> Result<()> {
    if crate::adapters::agent(record.provider)?
        .account_operations()
        .managed_login
    {
        Ok(())
    } else {
        Err(ApiError::bad(format!(
            "This managed account operation is unsupported for {}; use its official CLI directly",
            record.provider.label()
        )))
    }
}
pub(super) fn ensure_not_pending(state: &AppState, id: Uuid) -> Result<()> {
    if state.accounts.pending(id) {
        Err(ApiError::conflict(
            "An account login is already pending; cancel it before another operation",
        ))
    } else {
        Ok(())
    }
}

pub(super) async fn login(
    State(state): State<AppState>,
    RoutePath(id): RoutePath<Uuid>,
    Json(input): Json<Login>,
) -> Result<Json<AccountView>> {
    if !matches!(input.mode.as_str(), "device" | "browser") {
        return Err(ApiError::bad("Choose device or browser login"));
    }
    let operation = state.accounts.operation(id);
    let _guard = operation.lock().await;
    let record = record(&state, id)?;
    ensure_managed_login(&record)?;
    ensure_not_pending(&state, id)?;
    let _maintenance = begin_maintenance(&state, id).await?;
    if state
        .accounts
        .inner
        .lock()
        .expect("accounts lock")
        .pending
        .len()
        >= 4
    {
        return Err(ApiError::conflict("Too many pending account logins"));
    }
    let generation = Uuid::new_v4();
    let mut initial = view(&state, id)?;
    initial.status = "login_pending".into();
    initial.login = None;
    initial.error = None;
    {
        let mut inner = state.accounts.inner.lock().expect("accounts lock");
        inner.generations.insert(id, generation);
        inner.views.insert(id, initial);
    }
    let (cancel, receiver) = oneshot::channel();
    let (ready, first) = oneshot::channel();
    let (start, started) = oneshot::channel();
    let task_state = state.clone();
    let task = tokio::spawn(async move {
        if started.await.is_err() {
            return;
        }
        let _ = bridge(
            &task_state,
            record,
            generation,
            json!({"action":"login","mode":input.mode}),
            Some(receiver),
            Some(ready),
        )
        .await;
        let mut inner = task_state.accounts.inner.lock().expect("accounts lock");
        if inner
            .pending
            .get(&id)
            .is_some_and(|pending| pending.generation == generation)
        {
            inner.pending.remove(&id);
        }
    });
    {
        let mut inner = state.accounts.inner.lock().expect("accounts lock");
        if inner.pending.len() >= 4 {
            task.abort();
            if let Some(view) = inner.views.get_mut(&id) {
                view.status = "unknown".into();
                view.error = Some("Too many pending account logins".into());
            }
            return Err(ApiError::conflict("Too many pending account logins"));
        }
        inner.pending.insert(
            id,
            PendingLogin {
                generation,
                cancel,
                task,
            },
        );
    }
    let _ = start.send(());
    let _ = tokio::time::timeout(Duration::from_secs(25), first).await;
    Ok(Json(view(&state, id)?))
}
pub(super) async fn cancel_login(
    State(state): State<AppState>,
    RoutePath(id): RoutePath<Uuid>,
) -> Result<Json<AccountView>> {
    let operation = state.accounts.operation(id);
    let _guard = operation.lock().await;
    record(&state, id)?;
    let _maintenance = begin_maintenance(&state, id).await?;
    let pending = state
        .accounts
        .inner
        .lock()
        .expect("accounts lock")
        .pending
        .remove(&id);
    if let Some(pending) = pending {
        finish_pending(pending).await;
    }
    let mut current = view(&state, id)?;
    if current.status == "login_pending" {
        current.status = "unknown".into();
        current.login = None;
        current.error = Some("Login cancelled; refresh to check the official account state".into());
        state
            .accounts
            .inner
            .lock()
            .expect("accounts lock")
            .views
            .insert(id, current.clone());
    }
    Ok(Json(current))
}
