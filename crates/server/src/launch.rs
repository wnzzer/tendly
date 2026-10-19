//! Session launch planning shared by structured chat and interactive terminals.
//! Resolve a session's effective configuration into a SpawnSpec; transports own
//! process execution. Client-specific switches and files belong to adapters.
use crate::providers::{private_dir, resolve_model, validate_profile};
use crate::{ApiError, AppState};
use std::{collections::BTreeMap, env, path::PathBuf};
use tendly_domain::{InteractionMode, ProviderKind, Session, SessionStatus};
use tendly_runtime::SpawnSpec;

/// Resolve the recorded checkout and build configuration away from async I/O.
/// This prepares a launch without starting a process.
pub async fn prepare(state: &AppState, session: &Session) -> Result<SpawnSpec, ApiError> {
    let cwd = crate::checkouts::session_cwd(state, session).await?;
    let state = state.clone();
    let session = session.clone();
    tokio::task::spawn_blocking(move || build(&state, &session, cwd))
        .await
        .map_err(ApiError::internal)?
}

/// Pin the source's effective configuration before persisting a terminal view.
/// The caller holds the operations lock, just as for other session mutations.
pub async fn terminal_reopen(state: &AppState, source: &Session) -> Result<Session, ApiError> {
    let session = reopened(state, source)?;
    // Validate the complete plan first: failed configuration creates no record.
    prepare(state, &session).await?;
    let record = session.clone();
    crate::db(state, move |store| store.insert_terminal_reopen(&record)).await?;
    Ok(session)
}

/// The launch a terminal outside Tendly runs to continue a session
/// (`tendly resume`): the plan an escape-hatch terminal gets, on the
/// source's pinned configuration, without recording a session for it. A
/// terminal session simply opens its shell where it ran.
pub async fn continuation(state: &AppState, source: &Session) -> Result<SpawnSpec, ApiError> {
    if source.provider == ProviderKind::Terminal {
        return prepare(state, source).await;
    }
    prepare(state, &reopened(state, source)?).await
}

/// A terminal view of `source`'s conversation: same client, configuration
/// revision, home and native session, as a new unsaved session.
fn reopened(state: &AppState, source: &Session) -> Result<Session, ApiError> {
    if source.provider == ProviderKind::Terminal {
        return Err(ApiError::bad("This session is already a terminal"));
    }
    if !source
        .provider_session_id
        .as_deref()
        .is_some_and(crate::native_history::valid_id)
    {
        return Err(ApiError::conflict(
            "Send a message first; there is no conversation to reopen yet",
        ));
    }
    let mut session = source.clone();
    session.id = uuid::Uuid::new_v4();
    session.title = format!("{} (terminal)", source.title.trim())
        .chars()
        .take(120)
        .collect();
    session.interaction_mode = InteractionMode::Pty;
    session.status = SessionStatus::Stopped;
    session.created_at = chrono::Utc::now();
    session.updated_at = session.created_at;
    session.archived_at = None;
    session.error = None;
    session.ephemeral = true;
    session.resume_source_id = Some(source.resume_source_id.unwrap_or(source.id));
    session.resume_configuration_revision = Some(configuration_revision(state, source)?);
    session.configuration_revision = 0;
    Ok(session)
}

/// No profile/snapshot means native defaults in a fresh, persistent config dir.
pub fn build(state: &AppState, session: &Session, cwd: PathBuf) -> Result<SpawnSpec, ApiError> {
    crate::environment::validate(&session.environment)?;
    let mut spec = build_base(state, session, cwd)?;
    if session.native_source_id.is_none()
        && let Some(args) = resume_args(session)?
    {
        spec.args.extend(args);
    }
    // A client that keeps a native session title gets the Tendly title on
    // every new or reopened process; renaming a live process remains a
    // display-only change until its next explicit reopen.
    else if let Some(client) = crate::adapters::adapter(session.provider)
        && session.native_source_id.is_none()
        && session
            .endpoint_snapshot
            .as_ref()
            .is_none_or(|profile| profile.native_config.is_none())
        && !session.title.trim().is_empty()
    {
        spec.args.extend(client.title_args(&session.title));
    }
    let main_model = session.endpoint_snapshot.as_ref().and_then(resolve_model);
    let main_context = main_model
        .as_deref()
        .and_then(|model| crate::model_limits::context_override(&state.state_dir, model));
    crate::environment::apply(
        &mut spec,
        &session.environment,
        &state.state_dir,
        crate::environment::Launch {
            main_model: main_model.as_deref(),
            main_context,
        },
    )?;
    // Last, so no session environment override can replace the session's own
    // identity with another's.
    crate::agent::equip(state, session, &mut spec)?;
    Ok(spec)
}

/// The flags that reopen an already-existing conversation, for a session that
/// is an escape hatch out of structured mode.
///
/// Structured mode drives the client over a JSON pipe, which cannot serve a
/// genuinely interactive command such as `/config`. The escape hatch is a real
/// terminal process on the *same* native conversation, so nothing is duplicated
/// and the structured pane keeps its history.
///
/// Keep endpoint, model, effort and permission flags. Resume contributes only
/// the client-specific conversation selector, after those launch flags.
fn resume_args(session: &Session) -> Result<Option<Vec<String>>, ApiError> {
    if session.resume_source_id.is_none() {
        return Ok(None);
    }
    let native_id = session
        .provider_session_id
        .as_deref()
        .filter(|id| crate::native_history::valid_id(id))
        .ok_or_else(|| {
            ApiError::bad("The conversation this terminal reopens has no native session ID yet")
        })?;
    let client = crate::adapters::adapter(session.provider)
        .ok_or_else(|| ApiError::bad("Terminal has no structured conversation"))?;
    Ok(Some(client.resume_args(native_id)))
}

/// The structured session an escape-hatch terminal reopens.
///
/// A launch is built on a blocking thread, so this reads the store directly
/// rather than going through the async helper.
fn resume_source(
    state: &AppState,
    id: tendly_domain::SessionId,
) -> Result<tendly_domain::Session, ApiError> {
    state
        .store
        .get_session(id)
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::bad("The conversation this terminal reopens no longer exists"))
}

/// A pinned continuation never consults its source's mutable current revision.
fn configuration_revision(state: &AppState, session: &Session) -> Result<u64, ApiError> {
    match (
        session.resume_source_id,
        session.resume_configuration_revision,
    ) {
        (None, _) => Ok(session.configuration_revision),
        (Some(_), Some(revision)) => Ok(revision),
        (Some(owner), None) => Ok(resume_source(state, owner)?.configuration_revision),
    }
}

fn build_base(state: &AppState, session: &Session, cwd: PathBuf) -> Result<SpawnSpec, ApiError> {
    if session.native_source_id.is_some() {
        return crate::native_history::resume_spec(state, session, cwd);
    }
    if let Some(profile) = session.endpoint_snapshot.as_ref()
        && let Some(reference) = profile.native_config.as_ref()
    {
        validate_profile(profile)?;
        if profile.provider != session.provider {
            return Err(ApiError::bad("Profile and session providers must match"));
        }
        let mut spec = crate::native_config::build(state, &session.provider, reference, cwd)?;
        // An official account may still choose a model and a depth. The client
        // resolves both against its own account; this only states the choice at
        // launch, which is also the one path that works for a model whose live
        // switch the upstream refuses.
        if let Some(model) = resolve_model(profile) {
            spec.args
                .extend(crate::adapters::agent(session.provider)?.model_args(profile, &model));
        }
        if let Some(effort) = profile.effort.as_deref() {
            spec.args
                .extend(crate::adapters::agent(session.provider)?.effort_args(effort));
        }
        return Ok(spec);
    }
    let mut environment = BTreeMap::new();
    let mut args = Vec::new();
    let p = session.endpoint_snapshot.as_ref();
    if let Some(p) = p {
        validate_profile(p)?;
        if p.permission_mode == "blocked" {
            return Err(ApiError::bad("This profile blocks session launch"));
        }
    }
    let mut remove: Vec<String> = crate::bridge::tendly_secrets()
        .chain(crate::bridge::host_client_session())
        .collect();
    let client = crate::adapters::adapter(session.provider);
    let program = match client {
        Some(_) => crate::clients::program(&state.state_dir, &session.provider),
        None => env::var("TENDLY_SHELL")
            .ok()
            .filter(|shell| !shell.is_empty())
            .unwrap_or_else(tendly_runtime::default_shell),
    };
    // POSIX shells need `-i` to stay interactive on a pipe-like PTY; neither
    // PowerShell nor cmd.exe accepts it.
    if client.is_none() && cfg!(unix) {
        args.push("-i".into());
    }
    if let Some(client) = client {
        // An escape-hatch terminal reopens a structured session's conversation,
        // whose transcript lives under that session's own home. Resolving this
        // one's id instead would point `--resume` at an empty directory.
        let owner = session.resume_source_id.unwrap_or(session.id);
        let revision = configuration_revision(state, session)?;
        let base = state.state_dir.join("sessions").join(owner.to_string());
        let path = if revision == 0 {
            base
        } else {
            base.join("configurations").join(revision.to_string())
        };
        let dir = private_dir(&path)?;
        environment.insert(
            client.config_key().into(),
            dir.to_string_lossy().into_owned(),
        );
        // Avoid inherited endpoint/auth overrides from the Tendly host,
        // every client's: a variable meant for one can still steer another.
        remove.extend(
            crate::adapters::all()
                .flat_map(|client| client.inherited_keys())
                .map(|key| (*key).to_owned()),
        );
        let secret = p
            .and_then(|p| p.secret_ref.as_deref())
            .map(|reference| crate::secrets::resolve_reference(&state.state_dir, reference))
            .transpose()?;
        if let Some(p) = p
            && let Some(model) = resolve_model(p)
        {
            args.extend(client.model_args(p, &model));
        }
        if let Some(proxy) = p.and_then(|p| p.proxy_url.as_ref()) {
            // Override both cases so an inherited lowercase variable cannot win.
            for key in [
                "HTTP_PROXY",
                "HTTPS_PROXY",
                "ALL_PROXY",
                "http_proxy",
                "https_proxy",
                "all_proxy",
            ] {
                environment.insert(key.into(), proxy.clone());
            }
            for key in ["NO_PROXY", "no_proxy"] {
                environment.insert(key.into(), String::new());
            }
        }
        client.configure(crate::adapters::EndpointLaunch {
            state,
            profile: p,
            mode: p.map_or("native", |p| p.permission_mode.as_str()),
            secret,
            home: &dir,
            args: &mut args,
            environment: &mut environment,
        })?;
    }
    // Runtime removes inherited vars first, then injects only this session's config.
    remove.retain(|key| !environment.contains_key(key));
    Ok(SpawnSpec {
        program,
        args,
        cwd,
        env: environment,
        env_remove: remove,
    })
}
