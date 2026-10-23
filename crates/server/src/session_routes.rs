//! A session's life: creating it, starting and stopping its client, its
//! title, archive, environment and messages, and discarding it.

use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateSession {
    interaction_mode: Option<InteractionMode>,
    handoff_source_id: Option<SessionId>,
    #[serde(default)]
    environment: tendly_domain::EnvironmentOverrides,
    provider: ProviderKind,
    title: String,
    endpoint_profile_id: Option<Uuid>,
    model: Option<String>,
    /// Absent takes the preferred depth for the provider; `null` is an explicit
    /// "automatic" and takes nothing.
    #[serde(default, deserialize_with = "present")]
    effort: Option<Option<String>>,
    /// Declared here or never. There is deliberately no way to mark an existing
    /// session temporary, only to promote a temporary one to permanent.
    #[serde(default)]
    ephemeral: bool,
}
/// Tells a field sent as `null` apart from one not sent at all.
pub(super) fn present<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(deserializer).map(Some)
}
#[derive(Deserialize, Default)]
pub(super) struct SessionQuery {
    workspace_id: Option<WorkspaceId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MessageInput {
    content: String,
}

pub(super) async fn list_sessions(
    State(state): State<AppState>,
    Query(q): Query<SessionQuery>,
) -> Result<Json<Vec<Session>>> {
    if let Some(id) = q.workspace_id {
        workspace(&state, id).await?;
    }
    Ok(Json(
        db(&state, move |s| s.list_sessions(q.workspace_id)).await?,
    ))
}
pub(super) async fn get_session(
    State(state): State<AppState>,
    Path(id): Path<SessionId>,
) -> Result<Json<Session>> {
    Ok(Json(session_record(&state, id).await?))
}
pub(super) async fn create_session(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    Json(input): Json<CreateSession>,
) -> Result<(StatusCode, Json<Session>)> {
    environment::validate(&input.environment)?;
    workspace(&state, id).await?;
    let title = bounded_name(&input.title)?;
    let model = optional(input.model);
    let effort = match input.effort {
        Some(chosen) => optional(chosen),
        None => preferences::load(&state)
            .await?
            .for_provider(&input.provider)
            .and_then(|p| p.effort.clone()),
    };
    providers::validate_effort(effort.as_deref())?;
    let interaction_mode = input.interaction_mode.unwrap_or_default();
    if input.provider == ProviderKind::Terminal && interaction_mode == InteractionMode::Structured {
        return Err(ApiError::bad(
            "Terminals do not support structured conversations",
        ));
    }
    if let Some(source) = input.handoff_source_id {
        let source = session_record(&state, source).await?;
        if interaction_mode != InteractionMode::Structured
            || source.provider == ProviderKind::Terminal
        {
            return Err(ApiError::bad(
                "Handoffs require a conversation and a structured target",
            ));
        }
    }
    if model
        .as_ref()
        .is_some_and(|m| m.len() > 200 || m.chars().any(char::is_control))
    {
        return Err(ApiError::bad("Invalid model identifier"));
    }
    if input.provider == ProviderKind::Terminal && model.is_some() {
        return Err(ApiError::bad("Terminal sessions do not use a model"));
    }
    if let Some(pid) = input.endpoint_profile_id {
        let p = db(&state, move |s| s.get_endpoint_profile(pid))
            .await?
            .ok_or_else(|| ApiError::missing("Profile"))?;
        if p.provider != input.provider || input.provider == ProviderKind::Terminal {
            return Err(ApiError::bad("Profile and session providers must match"));
        }
        let mut effective = p.environment.clone();
        effective.extend(input.environment.clone());
        environment::validate(&effective)?;
        if let Some(reference) = p.native_config.as_ref() {
            // Model and reasoning depth are launch flags for this session only.
            // They select what the client runs and never rewrite the shared
            // native configuration, so they stay available here.
            let config_state = state.clone();
            let provider = p.provider;
            let reference = reference.clone();
            tokio::task::spawn_blocking(move || {
                native_config::validate_reference(&config_state, &provider, &reference)
            })
            .await
            .map_err(ApiError::internal)??;
        }
    }
    Ok((
        StatusCode::CREATED,
        Json(
            db(&state, move |s| {
                let session = s.create_session_with_configuration(
                    id,
                    input.provider,
                    &title,
                    input.endpoint_profile_id,
                    model,
                    effort,
                    input.environment,
                    input.ephemeral,
                )?;
                if interaction_mode == InteractionMode::Structured {
                    s.set_interaction_mode(session.id, interaction_mode)?;
                    if let Some(source) = input.handoff_source_id {
                        s.carry_conversation(source, session.id)?;
                    }
                    s.get_session(session.id)?
                        .ok_or(tendly_persistence::DbError::QueryReturnedNoRows)
                } else {
                    Ok(session)
                }
            })
            .await?,
        ),
    ))
}

/// Open the conversation a structured session owns in a real terminal client.
///
/// Structured mode drives the same client over a JSON pipe, which is precisely
/// what makes an interactive command such as `/config` unusable there. Rather
/// than approximating a TUI over that pipe, this creates a genuine terminal
/// session on the same native conversation: same client, same account, same
/// configuration home, same session id, but a real PTY the client can run any
/// interactive command in.
///
/// It is a separate session on purpose. The structured pane is left untouched,
/// so the escape hatch is an addition rather than a conversion, and closing the
/// terminal returns to structured mode with its history intact.
pub(super) async fn open_session_in_terminal(
    State(state): State<AppState>,
    Path(id): Path<SessionId>,
) -> Result<(StatusCode, Json<Session>)> {
    let _guard = state.operations.lock().await;
    let source = session_record(&state, id).await?;
    let record = launch::terminal_reopen(&state, &source).await?;
    Ok((StatusCode::CREATED, Json(record)))
}

#[derive(Deserialize, Default)]
pub(super) struct LaunchPlanQuery {
    #[serde(default)]
    force: bool,
}

/// What `tendly resume` runs to continue a session in a terminal of its own:
/// program, arguments, directory and environment, from the shared launch plan.
///
/// The environment carries the endpoint's resolved key, as any launch does, so
/// only a caller on this machine gets one: the plan names this host's paths and
/// is of no use elsewhere. A session Tendly is running now is refused unless
/// forced, since two clients writing one conversation interleave it.
pub(super) async fn session_launch_plan(
    State(state): State<AppState>,
    Path(id): Path<SessionId>,
    Query(query): Query<LaunchPlanQuery>,
    request: Request,
) -> Result<Json<Value>> {
    let local = request
        .extensions()
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
        .is_some_and(|peer| peer.0.ip().is_loopback())
        // Tailscale forwards from loopback too.
        && request.extensions().get::<security::ViaTailnet>().is_none();
    if !local {
        return Err(ApiError {
            status: StatusCode::FORBIDDEN,
            message: "A session can only be resumed in a terminal on the machine Tendly runs on"
                .into(),
        });
    }
    let session = session_record(&state, id).await?;
    if !query.force
        && matches!(
            session.status,
            SessionStatus::Starting | SessionStatus::Running | SessionStatus::Waiting
        )
    {
        return Err(ApiError::conflict(
            "This session is running in Tendly. End it there first, or pass --force to open a second client on the same conversation",
        ));
    }
    let spec = launch::continuation(&state, &session).await?;
    Ok(Json(json!({
        "session_id": session.id,
        "title": session.title,
        "provider": session.provider,
        "provider_session_id": session.provider_session_id,
        "program": spec.program,
        "args": spec.args,
        "cwd": spec.cwd,
        "env": spec.env,
        "env_remove": spec.env_remove,
    })))
}

pub(super) async fn start_session(
    State(state): State<AppState>,
    Path(id): Path<SessionId>,
) -> Result<Json<Session>> {
    crate::worktree_setup::ready_for(&state, id).await;
    let _guard = state.operations.lock().await;
    let session = session_record(&state, id).await?;
    if session.interaction_mode == InteractionMode::Structured {
        conversations::start_locked(&state, &session).await?;
        return Ok(Json(session_record(&state, id).await?));
    }
    if state
        .runtime
        .get(&id.to_string())
        .is_some_and(|s| s.running())
    {
        return Ok(Json(session));
    }
    let spec = launch::prepare(&state, &session).await?;
    let runtime = match state.runtime.start(id.to_string(), spec).await {
        Ok(runtime) => runtime,
        Err(_) => {
            db(&state, move |s| {
                s.set_session_result(
                    id,
                    SessionStatus::Failed,
                    Some("CLI could not start. Check executable path and native configuration."),
                )
            })
            .await?;
            return Err(ApiError::conflict(
                "CLI could not start. Check executable installation or TENDLY_*_BIN.",
            ));
        }
    };
    db(&state, move |s| {
        s.set_session_status(id, SessionStatus::Running)
    })
    .await?;
    let watcher = state.clone();
    tokio::spawn(async move {
        let code = runtime.wait().await;
        let _lock = watcher.operations.lock().await;
        if watcher
            .runtime
            .get(&id.to_string())
            .is_some_and(|current| Arc::ptr_eq(&runtime, &current))
        {
            let _ = db(&watcher, move |s| {
                if s.get_session(id)?
                    .is_some_and(|record| matches!(record.status, SessionStatus::Stopped))
                {
                    return Ok(true); // Preserve an explicit stop's clean status.
                }
                if code == Some(0) {
                    s.set_session_result(id, SessionStatus::Stopped, None)
                } else {
                    s.set_session_result(
                        id,
                        SessionStatus::Stopped,
                        Some("Native process exited; restart explicitly or use native resume."),
                    )
                }
            })
            .await;
        }
    });
    Ok(Json(session_record(&state, id).await?))
}
pub(super) async fn stop_session(
    State(state): State<AppState>,
    Path(id): Path<SessionId>,
) -> Result<Json<Session>> {
    let _guard = state.operations.lock().await;
    session_record(&state, id).await?;
    stop_session_locked(&state, id).await?;
    Ok(Json(session_record(&state, id).await?))
}
/// Ends a session's client and process; the caller holds the operations lock.
pub(super) async fn stop_session_locked(state: &AppState, id: SessionId) -> Result<()> {
    state.chats.stop(id).await?;
    if let Some(runtime) = state.runtime.get(&id.to_string()) {
        runtime
            .stop()
            .await
            .map_err(|_| ApiError::conflict("Could not stop process"))?;
    }
    db(state, move |s| {
        s.set_session_status(id, SessionStatus::Stopped)
    })
    .await?;
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SessionArchivePatch {
    archived: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SessionTitlePatch {
    title: String,
}

pub(super) async fn update_session_title(
    State(state): State<AppState>,
    Path(id): Path<SessionId>,
    Json(input): Json<SessionTitlePatch>,
) -> Result<Json<Session>> {
    session_record(&state, id).await?;
    let title = bounded_name(&input.title)?;
    Ok(Json(
        db(&state, move |store| store.update_session_title(id, &title))
            .await?
            .ok_or_else(|| ApiError::missing("Session"))?,
    ))
}

/// Promote a temporary session to a permanent one. Deliberately one-way: an
/// existing session can never be made destructible by closing its window.
pub(super) async fn keep_session(
    State(state): State<AppState>,
    Path(id): Path<SessionId>,
) -> Result<Json<Session>> {
    Ok(Json(
        db(&state, move |store| store.keep_session(id))
            .await?
            .ok_or_else(|| ApiError::missing("Session"))?,
    ))
}

/// Discard a temporary session: stop whatever it is running, then remove the
/// record. Refused for a permanent session, so the only way to reach this is to
/// have opted in when the session was created.
pub(super) async fn discard_session(
    State(state): State<AppState>,
    Path(id): Path<SessionId>,
) -> Result<Json<serde_json::Value>> {
    let _guard = state.operations.lock().await;
    let session = session_record(&state, id).await?;
    if !session.ephemeral && session.archived_at.is_none() {
        return Err(ApiError::conflict(
            "Archive this session before deleting it.",
        ));
    }
    state.chats.stop(id).await?;
    if let Some(runtime) = state.runtime.get(&id.to_string()) {
        runtime
            .stop()
            .await
            .map_err(|_| ApiError::conflict("Could not stop process"))?;
    }
    // Removing the record after the process is down keeps a discarded session
    // from leaving an orphaned client running with nothing bound to it.
    if !db(&state, move |store| store.delete_session(id)).await? {
        return Err(ApiError::missing("Session"));
    }
    // An isolated task's worktree with nothing in it goes with the session.
    crate::tasks::forget(&state, &session).await;
    Ok(Json(serde_json::json!({ "id": id })))
}

pub(super) async fn update_session_archive(
    State(state): State<AppState>,
    Path(id): Path<SessionId>,
    Json(input): Json<SessionArchivePatch>,
) -> Result<Json<Session>> {
    let session = db(&state, move |store| {
        store.set_session_archived(id, input.archived)
    })
    .await?
    .ok_or_else(|| ApiError::missing("Session"))?;
    Ok(Json(session))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SessionEnvironmentPatch {
    environment: tendly_domain::EnvironmentOverrides,
}

pub(super) async fn update_session_environment(
    State(state): State<AppState>,
    Path(id): Path<SessionId>,
    Json(input): Json<SessionEnvironmentPatch>,
) -> Result<Json<Session>> {
    let _guard = state.operations.lock().await;
    let session = session_record(&state, id).await?;
    if !matches!(
        session.status,
        SessionStatus::Stopped | SessionStatus::Failed
    ) || state.chats.get(id).is_some_and(|runtime| runtime.running())
        || state
            .runtime
            .get(&id.to_string())
            .is_some_and(|runtime| runtime.running())
    {
        return Err(ApiError::conflict(
            "Environment can only be edited while the session is stopped or failed",
        ));
    }
    environment::validate(&input.environment)?;
    if !db(&state, move |store| {
        store.update_session_environment(id, &input.environment)
    })
    .await?
    {
        return Err(ApiError::conflict(
            "Session state changed; environment was not updated",
        ));
    }
    Ok(Json(session_record(&state, id).await?))
}
pub(super) async fn send_message(
    State(state): State<AppState>,
    Path(id): Path<SessionId>,
    Json(input): Json<MessageInput>,
) -> Result<StatusCode> {
    if session_record(&state, id).await?.interaction_mode == InteractionMode::Structured {
        return Err(ApiError::bad(
            "Use the structured conversation message endpoint with a request ID",
        ));
    }
    if input.content.is_empty() || input.content.len() > 32 * 1024 {
        return Err(ApiError::bad("Message must be 1–32768 bytes"));
    }
    let runtime = state
        .runtime
        .get(&id.to_string())
        .ok_or_else(|| ApiError::conflict("Start the session first"))?;
    runtime
        .input(format!("{}\r", input.content).into_bytes())
        .await
        .map_err(|e| ApiError::conflict(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

/// The routes this module serves.
pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/workspaces/{id}/sessions", post(create_session))
        .route("/api/sessions", get(list_sessions))
        .route(
            "/api/sessions/{id}",
            get(get_session)
                .patch(update_session_title)
                .delete(discard_session),
        )
        .route("/api/sessions/{id}/keep", post(keep_session))
        .route(
            "/api/sessions/{id}/archive",
            axum::routing::patch(update_session_archive),
        )
        .route("/api/sessions/{id}/start", post(start_session))
        .route(
            "/api/sessions/{id}/terminal",
            post(open_session_in_terminal),
        )
        .route("/api/sessions/{id}/launch-plan", post(session_launch_plan))
        .route("/api/sessions/{id}/stop", post(stop_session))
        .route(
            "/api/sessions/{id}/environment",
            axum::routing::patch(update_session_environment),
        )
        .route("/api/sessions/{id}/message", post(send_message))
}
