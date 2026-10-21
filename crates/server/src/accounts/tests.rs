use super::*;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use tendly_persistence::Store;
use tendly_runtime::RuntimeManager;
use tower::ServiceExt;

struct Fixture {
    state: AppState,
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = env::temp_dir().join(format!("tendly-account-test-{}", Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let root = dunce::canonicalize(root).unwrap();
        let state = AppState {
            store: Arc::new(Store::open(":memory:").unwrap()),
            runtime: RuntimeManager::new(),
            state_dir: root.clone(),
            browse_roots: vec![root.clone()],
            host_grants: crate::host_grants::HostGrants::default(),
            workspace_roots: vec![root.clone()],
            native_sources: vec![],
            native_bridge: root.join("unused-history.mjs"),
            chat_fixture: None,
            chats: crate::conversations::ChatManager::default(),
            watches: crate::file_watch::FileWatches::default(),
            accounts: AccountManager::default(),
            security: crate::security::Security::for_test("127.0.0.1:8787".parse().unwrap(), None),
            tailnet: crate::tailscale::Tailnet::disabled(),
            claude_manual_mode: true,
            operations: Arc::new(tokio::sync::Mutex::new(())),
            agents: crate::agent::AgentRegistry::default(),
            activity: crate::activity::Activity::default(),
            metrics: None,
        };
        Self { state, root }
    }
    async fn create(&self, provider: ProviderKind) -> AccountView {
        create(
            State(self.state.clone()),
            Json(CreateAccount {
                name: "Fixture account".into(),
                provider,
            }),
        )
        .await
        .unwrap()
        .0
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[tokio::test]
async fn accounts_create_only_protected_metadata_and_link_a_reusable_profile() {
    let f = Fixture::new();
    let account = f.create(ProviderKind::Codex).await;
    assert_eq!(account.status, "unknown");
    assert!(account.email.is_none());
    assert!(account.limits.is_none());
    let path = PathBuf::from(&account.storage_path);
    assert!(
        !path.join("auth.json").exists(),
        "Creating an account must not log in or copy credentials"
    );
    let metadata = fs::read_to_string(path.join("account.json")).unwrap();
    assert!(!metadata.contains("token"));
    let profile = f
        .state
        .store
        .get_endpoint_profile(account.profile_id)
        .unwrap()
        .unwrap();
    let reference = profile.native_config.unwrap();
    assert_eq!(reference.source_id, format!("account:{}", account.id));
    assert_eq!(
        reference.config_env.as_deref(),
        Some(account.storage_path.as_str())
    );
    assert_eq!(pin(&f.state, &reference.source_id).unwrap().1, reference);
    assert_eq!(list(State(f.state.clone())).await.unwrap().0.len(), 1);
    // No token read is needed to use/validate this reference, even if the
    // native client's credential format is opaque to Tendly.
    write_new(
        &path.join("auth.json"),
        b"opaque fixture credentials - intentionally not JSON",
    )
    .unwrap();
    assert_eq!(
        read(State(f.state.clone()), RoutePath(account.id))
            .await
            .unwrap()
            .0
            .status,
        "unknown"
    );
    assert_eq!(
        validate_reference(&f.state, &ProviderKind::Codex, &reference).unwrap(),
        path
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(path.join("account.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(path.join("config.toml"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[tokio::test]
async fn import_native_links_existing_home_without_copying_credentials() {
    let mut f = Fixture::new();
    let native = f.root.join("native-codex");
    fs::create_dir(&native).unwrap();
    f.state
        .native_sources
        .push(crate::native_history::NativeSource {
            id: "fixture-codex".into(),
            provider: ProviderKind::Codex,
            label: "Fixture Codex".into(),
            config_dir: native.clone(),
            config_env: None,
        });
    let account = import_native(
        State(f.state.clone()),
        Json(ImportNativeAccount {
            source_id: "fixture-codex".into(),
            name: Some("Imported fixture".into()),
            confirmed_shared_config: true,
        }),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(account.native_source_id.as_deref(), Some("fixture-codex"));
    // A linked account's sign-in lives in a directory the host owns, so
    // anything else editing it changes this account too. Saying so is the
    // difference between "my account stopped working for no reason" and
    // "the thing I share it with switched it".
    assert!(account.shared_configuration);
    assert_eq!(account.storage_path, native.to_string_lossy());
    assert!(
        !f.root
            .join("accounts")
            .join(account.id.to_string())
            .join("auth.json")
            .exists()
    );
    let profile = f
        .state
        .store
        .get_endpoint_profile(account.profile_id)
        .unwrap()
        .unwrap();
    assert_eq!(profile.native_config.unwrap().source_id, "fixture-codex");
    assert!(owns_profile(&f.state, account.profile_id));
    let duplicate = import_native(
        State(f.state.clone()),
        Json(ImportNativeAccount {
            source_id: "fixture-codex".into(),
            name: Some("Should not duplicate".into()),
            confirmed_shared_config: true,
        }),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(duplicate.id, account.id);
    assert_eq!(f.state.store.list_endpoint_profiles().unwrap().len(), 1);
    assert_eq!(
        import_native(
            State(f.state.clone()),
            Json(ImportNativeAccount {
                source_id: "fixture-codex".into(),
                name: None,
                confirmed_shared_config: false
            })
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::BAD_REQUEST
    );
}
#[tokio::test]
async fn claude_has_native_guidance_without_managed_subscription_oauth_or_quota() {
    let f = Fixture::new();
    let account = f.create(ProviderKind::ClaudeCode).await;
    assert!(
        !account.capabilities.login
            && !account.capabilities.quota
            && !account.capabilities.reset_quota
            && !account.capabilities.refresh_token
    );
    let guidance = account.guidance.clone().unwrap();
    assert!(guidance.contains("claude auth login"));
    // An account Tendly created owns its directory, so nothing else on
    // the host edits its sign-in.
    assert!(!account.shared_configuration);
    // And its sign-in has to land in that directory. Without the override
    // the command writes into the host configuration instead, leaving this
    // account empty while it still appears in the list.
    assert!(
        guidance.contains(&format!("CLAUDE_CONFIG_DIR='{}'", account.storage_path)),
        "an owned account signs in into its own directory: {guidance}"
    );
    assert_eq!(
        login(
            State(f.state.clone()),
            RoutePath(account.id),
            Json(Login {
                mode: "device".into()
            })
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        refresh(
            State(f.state.clone()),
            RoutePath(account.id),
            Json(Refresh {
                refresh_token: true
            })
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::BAD_REQUEST
    );
    assert!(!f.state.accounts.pending(account.id));
}
#[tokio::test]
async fn usage_is_an_explicit_claude_only_check_that_never_reports_quota_capability() {
    let f = Fixture::new();
    let claude = f.create(ProviderKind::ClaudeCode).await;
    // Usage exists as an on-demand check; it is not managed quota, so it
    // must not imply a reset credit or a quota interface.
    assert!(claude.capabilities.usage);
    assert!(!claude.capabilities.quota && !claude.capabilities.reset_quota);
    // Nothing is reported until the user asks, so a fresh account shows no
    // usage snapshot at all rather than an assumed zero.
    assert!(claude.usage.is_none() && claude.limits.is_none());
    let codex = f.create(ProviderKind::Codex).await;
    assert!(!codex.capabilities.usage);
    assert_eq!(
        usage(State(f.state.clone()), RoutePath(codex.id))
            .await
            .unwrap_err()
            .status,
        StatusCode::BAD_REQUEST
    );
    assert!(!f.state.accounts.pending(claude.id));
}
#[tokio::test]
async fn a_failed_usage_query_keeps_previous_figures_without_calling_them_current() {
    let f = Fixture::new();
    let base = f.create(ProviderKind::ClaudeCode).await;
    let window = |percent: f64| LimitWindow {
        used_percent: percent,
        window_minutes: Some(300.0),
        window_kind: Some(WindowKind::FiveHour),
        mapping_basis: Some(WindowMappingBasis::OAuthUsage),
        resets_at: None,
    };
    let succeeded = AccountView {
        checked_at: Some("2026-09-15T02:00:00Z".into()),
        limits: Some(Limits {
            primary: Some(window(36.0)),
            ..Default::default()
        }),
        usage: Some(UsageView {
            availability: "available".into(),
            source: Some("claude_oauth_usage".into()),
            ..empty_usage()
        }),
        ..base.clone()
    };
    let failed = || AccountView {
        checked_at: Some("2026-09-15T02:05:00Z".into()),
        limits: None,
        usage: Some(UsageView {
            availability: "rate_limited".into(),
            retry_after_seconds: Some(30),
            ..empty_usage()
        }),
        ..base.clone()
    };

    let mut current = failed();
    retain_previous_usage(&mut current, Some(succeeded.clone()));
    let usage = current.usage.as_ref().unwrap();
    // The figures survive, but they are stamped as the earlier reading and
    // the attempt still reports why it failed.
    assert_eq!(current.limits.unwrap().primary.unwrap().used_percent, 36.0);
    assert_eq!(usage.availability, "rate_limited");
    assert_eq!(usage.retry_after_seconds, Some(30));
    assert_eq!(
        usage.retained_checked_at.as_deref(),
        Some("2026-09-15T02:00:00Z")
    );
    // Provenance follows the figures, never the failed attempt.
    assert_eq!(usage.source.as_deref(), Some("claude_oauth_usage"));

    // With nothing worth keeping, the failure stands on its own rather than
    // inventing a retained reading.
    let mut current = failed();
    retain_previous_usage(&mut current, Some(base.clone()));
    assert!(current.limits.is_none());
    assert!(current.usage.unwrap().retained_checked_at.is_none());

    // A successful query always wins over the previous figures.
    let mut current = AccountView {
        limits: Some(Limits {
            primary: Some(window(12.0)),
            ..Default::default()
        }),
        usage: Some(UsageView {
            availability: "available".into(),
            ..empty_usage()
        }),
        ..base.clone()
    };
    retain_previous_usage(&mut current, Some(succeeded));
    assert_eq!(current.limits.unwrap().primary.unwrap().used_percent, 12.0);
    assert!(current.usage.unwrap().retained_checked_at.is_none());
}
#[tokio::test]
async fn a_paid_for_usage_reading_survives_refreshes_and_restarts() {
    let f = Fixture::new();
    let account = f.create(ProviderKind::ClaudeCode).await;
    let cached = CachedUsage {
        limits: Limits {
            primary: Some(LimitWindow {
                used_percent: 30.0,
                window_minutes: Some(300.0),
                window_kind: Some(WindowKind::FiveHour),
                mapping_basis: Some(WindowMappingBasis::OAuthUsage),
                resets_at: None,
            }),
            ..Default::default()
        },
        checked_at: "2026-09-15T03:58:00Z".into(),
        source: Some("claude_oauth_usage".into()),
    };
    write_usage_cache(&f.state, account.id, &cached);

    // A fresh view — the state a restart or a plain status refresh leaves
    // behind — still shows the figures instead of an empty panel.
    let restored = view(&f.state, account.id).unwrap();
    let limits = restored.limits.as_ref().unwrap();
    assert_eq!(limits.primary.as_ref().unwrap().used_percent, 30.0);
    let usage = restored.usage.as_ref().unwrap();
    // They are labelled with when they were read, never presented as live.
    assert_eq!(
        usage.retained_checked_at.as_deref(),
        Some("2026-09-15T03:58:00Z")
    );
    assert_eq!(usage.source.as_deref(), Some("claude_oauth_usage"));
    assert_eq!(usage.availability, "cached");

    // The cache lives in Tendly's own directory, never the native client's.
    let path = usage_cache_path(&f.state, account.id).unwrap();
    assert!(path.starts_with(&f.root) && path.ends_with("usage.json"));

    // A corrupt or oversized cache is ignored rather than failing the page.
    fs::write(&path, b"{not json").unwrap();
    assert!(read_usage_cache(&f.state, account.id).is_none());
    assert!(view(&f.state, account.id).is_ok());
}
#[tokio::test]
async fn an_account_proxy_applies_to_its_own_operations_and_never_carries_credentials() {
    let f = Fixture::new();
    let account = f.create(ProviderKind::ClaudeCode).await;
    let patch = |value: Option<&str>| {
        let body = ProxyPatch {
            proxy_url: value.map(str::to_owned),
        };
        set_proxy(State(f.state.clone()), RoutePath(account.id), Json(body))
    };

    assert_eq!(
        patch(Some("http://127.0.0.1:7890"))
            .await
            .unwrap()
            .0
            .proxy_url
            .as_deref(),
        Some("http://127.0.0.1:7890")
    );
    // It is durable, so a sign-in after a restart still uses it.
    assert_eq!(
        record(&f.state, account.id).unwrap().proxy_url.as_deref(),
        Some("http://127.0.0.1:7890")
    );
    assert_eq!(
        view(&f.state, account.id).unwrap().proxy_url.as_deref(),
        Some("http://127.0.0.1:7890")
    );
    assert_eq!(
        patch(Some("https://proxy.internal:8443"))
            .await
            .unwrap()
            .0
            .proxy_url
            .as_deref(),
        Some("https://proxy.internal:8443")
    );

    // Credentials in the URL are refused: the account page displays it.
    for rejected in [
        "http://user:secret@127.0.0.1:7890",
        "http://user@127.0.0.1:7890",
        "ftp://127.0.0.1:21",
        // SOCKS is refused because the native Claude client cannot use it;
        // offering it would be an option that silently does nothing.
        "socks5h://10.0.0.1:1080",
        "not a url",
        "http://",
    ] {
        assert_eq!(
            patch(Some(rejected)).await.unwrap_err().status,
            StatusCode::BAD_REQUEST,
            "{rejected} must be refused"
        );
    }
    // A refused value leaves the previous one in place.
    assert_eq!(
        record(&f.state, account.id).unwrap().proxy_url.as_deref(),
        Some("https://proxy.internal:8443")
    );

    // Clearing returns the account to a direct connection.
    for cleared in [None, Some("   ")] {
        assert_eq!(patch(cleared).await.unwrap().0.proxy_url, None);
    }

    // Both cases are exported: Node's --use-env-proxy reads the lowercase
    // names while the native CLIs read the uppercase ones.
    let exported = proxy_environment("http://127.0.0.1:7890");
    let keys: Vec<_> = exported.iter().map(|(key, _)| *key).collect();
    assert!(keys.contains(&"https_proxy") && keys.contains(&"HTTPS_PROXY"));
    assert!(
        exported
            .iter()
            .all(|(_, value)| value == "http://127.0.0.1:7890")
    );
}
#[tokio::test]
async fn accounts_can_be_renamed_and_removed_without_touching_a_linked_native_home() {
    let mut f = Fixture::new();
    let account = f.create(ProviderKind::ClaudeCode).await;
    let storage = PathBuf::from(&account.storage_path);

    // The name is shared with the endpoint profile, so the two cannot drift.
    let renamed = rename(
        State(f.state.clone()),
        RoutePath(account.id),
        Json(RenameAccount {
            name: "  Work Claude  ".into(),
        }),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(renamed.name, "Work Claude");
    assert_eq!(record(&f.state, account.id).unwrap().name, "Work Claude");
    assert_eq!(
        f.state
            .store
            .get_endpoint_profile(account.profile_id)
            .unwrap()
            .unwrap()
            .name,
        "Work Claude"
    );
    for bad in ["", "   ", "bad\nname"] {
        assert_eq!(
            rename(
                State(f.state.clone()),
                RoutePath(account.id),
                Json(RenameAccount { name: bad.into() })
            )
            .await
            .unwrap_err()
            .status,
            StatusCode::BAD_REQUEST
        );
    }

    // Removal is deliberate, never implicit.
    assert_eq!(
        remove(
            State(f.state.clone()),
            RoutePath(account.id),
            Json(Confirm { confirmed: false })
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::BAD_REQUEST
    );
    assert!(storage.exists());
    remove(
        State(f.state.clone()),
        RoutePath(account.id),
        Json(Confirm { confirmed: true }),
    )
    .await
    .unwrap();
    assert!(!storage.exists());
    assert!(
        f.state
            .store
            .get_endpoint_profile(account.profile_id)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        record(&f.state, account.id).unwrap_err().status,
        StatusCode::NOT_FOUND
    );

    // A linked account owns only its metadata: the referenced native home
    // and everything the user keeps there must survive removal untouched.
    let config = f.root.join("native-codex-home");
    fs::create_dir(&config).unwrap();
    fs::write(config.join("auth.json"), "user credential").unwrap();
    f.state
        .native_sources
        .push(crate::native_history::NativeSource {
            id: "fixture-codex".into(),
            provider: ProviderKind::Codex,
            label: "Fixture Codex".into(),
            config_dir: config.clone(),
            config_env: None,
        });
    let linked = import_native(
        State(f.state.clone()),
        Json(ImportNativeAccount {
            source_id: "fixture-codex".into(),
            name: None,
            confirmed_shared_config: true,
        }),
    )
    .await
    .unwrap()
    .0;
    remove(
        State(f.state.clone()),
        RoutePath(linked.id),
        Json(Confirm { confirmed: true }),
    )
    .await
    .unwrap();
    assert!(
        config.join("auth.json").exists(),
        "the native home must be left alone"
    );
    assert_eq!(
        fs::read_to_string(config.join("auth.json")).unwrap(),
        "user credential"
    );
}
fn empty_usage() -> UsageView {
    UsageView {
        availability: String::new(),
        capture_path: None,
        captured_at: None,
        source: None,
        retry_after_seconds: None,
        retained_checked_at: None,
    }
}
#[tokio::test]
async fn account_maintenance_blocks_bound_sessions_and_requires_explicit_consent() {
    let f = Fixture::new();
    let account = f.create(ProviderKind::Codex).await;
    let workspace = f
        .state
        .store
        .create_workspace("fixture", f.root.to_str().unwrap())
        .unwrap();
    let session = f
        .state
        .store
        .create_session_with_profile(
            workspace.id,
            ProviderKind::Codex,
            "fixture",
            Some(account.profile_id),
        )
        .unwrap();
    f.state
        .store
        .set_session_status(session.id, SessionStatus::Running)
        .unwrap();
    assert_eq!(
        logout(
            State(f.state.clone()),
            RoutePath(account.id),
            Json(Confirm { confirmed: true })
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        login(
            State(f.state.clone()),
            RoutePath(account.id),
            Json(Login {
                mode: "device".into()
            })
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::CONFLICT
    );
    assert_eq!(
        logout(
            State(f.state.clone()),
            RoutePath(account.id),
            Json(Confirm { confirmed: false })
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        reset_quota(
            State(f.state.clone()),
            RoutePath(account.id),
            Json(ResetQuota {
                confirmed: false,
                idempotency_key: "fixture".into(),
                credit_id: None
            })
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::BAD_REQUEST
    );
    f.state
        .store
        .set_session_status(session.id, SessionStatus::Stopped)
        .unwrap();
    let guard = begin_maintenance(&f.state, account.id).await.unwrap();
    let reference = pin(&f.state, &format!("account:{}", account.id)).unwrap().1;
    assert_eq!(
        build(&f.state, &ProviderKind::Codex, &reference, f.root.clone())
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    drop(guard);
    assert!(
        !f.state
            .accounts
            .inner
            .lock()
            .unwrap()
            .maintenance
            .contains(&account.id)
    );
}
#[cfg(unix)]
#[tokio::test]
async fn account_metadata_and_credentials_symlinks_are_rejected_without_reading_targets() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let account = f.create(ProviderKind::Codex).await;
    let outside = f.root.join("outside-fixture");
    write_new(&outside, b"unchanged fixture").unwrap();
    let auth = PathBuf::from(&account.storage_path).join("auth.json");
    symlink(&outside, &auth).unwrap();
    assert_eq!(
        pin(&f.state, &format!("account:{}", account.id))
            .unwrap_err()
            .status,
        StatusCode::BAD_REQUEST
    );
    fs::remove_file(auth).unwrap();
    let metadata = PathBuf::from(&account.storage_path).join("account.json");
    fs::remove_file(&metadata).unwrap();
    symlink(&outside, &metadata).unwrap();
    assert_eq!(
        record(&f.state, account.id).unwrap_err().status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(fs::read(outside).unwrap(), b"unchanged fixture");
}
#[tokio::test]
async fn account_routes_keep_management_authentication() {
    let mut f = Fixture::new();
    f.state.security = crate::security::Security::for_test(
        "127.0.0.1:8787".parse().unwrap(),
        Some("fixture-token-long-enough"),
    );
    let request = Request::builder()
        .uri("/api/accounts")
        .header("host", "127.0.0.1:8787")
        .body(Body::empty())
        .unwrap();
    let response = crate::router(f.state.clone())
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let request = Request::builder()
        .uri("/api/accounts")
        .header("host", "127.0.0.1:8787")
        .header("authorization", "Bearer fixture-token-long-enough")
        .body(Body::empty())
        .unwrap();
    let response = crate::router(f.state.clone())
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(to_bytes(response.into_body(), 4096).await.unwrap(), "[]");
}

#[tokio::test]
async fn reset_retry_authorization_is_private_bounded_and_survives_manager_restart() {
    let mut f = Fixture::new();
    let account = f.create(ProviderKind::Codex).await;
    let generation = Uuid::new_v4();
    f.state
        .accounts
        .inner
        .lock()
        .unwrap()
        .generations
        .insert(account.id, generation);
    let metadata = record(&f.state, account.id).unwrap();
    apply_event(
        &f.state,
        &metadata,
        generation,
        json!({"type":"reset_authorized","idempotency_key":"original-retry-key"}),
    )
    .unwrap();
    f.state.accounts = AccountManager::default();
    assert_eq!(
        reset_keys(&f.state, account.id).unwrap(),
        vec!["original-retry-key"]
    );
    assert!(
        apply_event(
            &f.state,
            &metadata,
            generation,
            json!({"type":"reset_authorized","idempotency_key":"stale-attempt"})
        )
        .is_err()
    );
    for index in 0..66 {
        persist_reset_key(&f.state, account.id, &format!("attempt-{index}")).unwrap();
    }
    let keys = reset_keys(&f.state, account.id).unwrap();
    assert_eq!(keys.len(), 64);
    assert_eq!(keys[0], "attempt-2");
    assert!(!Path::new(&account.storage_path).join("auth.json").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = Path::new(&account.storage_path).join("quota-reset-attempts.json");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(f.root.join("outside-retry-file"), path).unwrap();
        assert!(reset_keys(&f.state, account.id).is_err());
    }
}

#[cfg(unix)]
fn fixture_process_live(pid: u32) -> bool {
    let output = std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let status = String::from_utf8_lossy(&output.stdout);
    !status.trim().is_empty() && !status.trim().starts_with('Z')
}
#[cfg(unix)]
async fn process_group_fixture() -> (tokio::process::Child, u32) {
    let mut child = tokio::process::Command::new("/bin/sh")
        .args(["-c", "trap '' TERM; sleep 60 & echo $!; wait"])
        .process_group(0)
        .kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap()).lines();
    let descendant = tokio::time::timeout(Duration::from_secs(3), reader.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .parse()
        .unwrap();
    (child, descendant)
}
#[cfg(unix)]
async fn assert_fixture_exited(pid: u32) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while fixture_process_live(pid) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("owned fixture descendant must not remain running");
}
#[cfg(unix)]
#[tokio::test]
async fn account_group_cleanup_kills_term_ignoring_descendants() {
    let (mut child, descendant) = process_group_fixture().await;
    let mut group = OwnedGroup::new(child.id());
    close_group(&mut group, &mut child).await;
    assert_fixture_exited(descendant).await;
    for pid in [None, Some(0), Some(1)] {
        assert!(OwnedGroup::new(pid).pid().is_none());
    }
}
#[cfg(unix)]
#[tokio::test]
async fn aborting_account_actor_also_drops_its_owned_native_process_group() {
    let (child, descendant) = process_group_fixture().await;
    let (ready, sent) = oneshot::channel();
    let task = tokio::spawn(async move {
        let _child = child;
        let _group = OwnedGroup::new(_child.id());
        let _ = ready.send(());
        std::future::pending::<()>().await;
    });
    sent.await.unwrap();
    task.abort();
    let _ = task.await;
    assert_fixture_exited(descendant).await;
}

#[test]
fn login_urls_must_be_official_https_without_credentials() {
    let official = |raw: &str| is_official_login_url(&url::Url::parse(raw).unwrap());
    assert!(official("https://auth.openai.com/oauth/authorize?x=1"));
    assert!(official("https://chatgpt.com/login"));
    assert!(!official("http://auth.openai.com/oauth"), "not HTTPS");
    assert!(!official("https://evil-openai.com/"), "a lookalike domain");
    assert!(
        !official("https://openai.com.evil.example/"),
        "the domain as a prefix"
    );
    assert!(
        !official("https://user:pass@auth.openai.com/"),
        "embedded credentials"
    );
    assert!(
        !official("https://user@auth.openai.com/"),
        "an embedded username"
    );
}

#[test]
fn proxies_are_plain_http_urls_and_reach_every_proxy_variable() {
    assert_eq!(validated_proxy(None).unwrap(), None);
    assert_eq!(
        validated_proxy(Some("   ")).unwrap(),
        None,
        "blank means none"
    );
    assert_eq!(
        validated_proxy(Some(" http://127.0.0.1:7890 "))
            .unwrap()
            .as_deref(),
        Some("http://127.0.0.1:7890")
    );
    for bad in [
        "socks5://127.0.0.1:1080",
        "http://u:p@host:1",
        "http://host:1/path",
        "http://a b",
    ] {
        assert!(validated_proxy(Some(bad)).is_err(), "{bad}");
    }
    assert!(validated_proxy(Some(&format!("http://{}.example", "a".repeat(400)))).is_err());
    let names: Vec<_> = proxy_environment("http://p:1")
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    for name in [
        "http_proxy",
        "https_proxy",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "all_proxy",
    ] {
        assert!(names.contains(&name), "{name}");
    }
}

#[test]
fn account_names_are_trimmed_and_bounded() {
    assert_eq!(bounded_account_name("  Work  ").unwrap(), "Work");
    assert!(bounded_account_name("   ").is_err());
    assert!(bounded_account_name(&"x".repeat(121)).is_err());
    assert_eq!(bounded_account_name(&"x".repeat(120)).unwrap().len(), 120);
    assert!(bounded_account_name("tab\there").is_err());
}
