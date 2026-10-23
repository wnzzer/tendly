//! What a view or an agent asks of a conversation: open it, send or steer a
//! message, interrupt, answer an approval, choose the model, the permission
//! mode or the endpoint.

use super::*;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/sessions/{id}/conversation/open",
            post(open_conversation),
        )
        .route("/api/sessions/{id}/conversation/message", post(message))
        .route("/api/sessions/{id}/conversation/steer", post(steer))
        .route("/api/sessions/{id}/conversation/interrupt", post(interrupt))
        .route("/api/sessions/{id}/conversation/approval", post(approval))
        .route("/api/sessions/{id}/conversation/model", post(select_model))
        .route(
            "/api/sessions/{id}/conversation/permission",
            post(select_permission),
        )
        .route("/api/sessions/{id}/configuration", patch(configuration))
}

/// For the agent tools (agent_sessions.rs): start a session's conversation.
pub(crate) async fn open(state: &AppState, id: SessionId) -> Result<()> {
    open_conversation(State(state.clone()), Path(id))
        .await
        .map(|_| ())
}
/// For the agent tools: send a message, exactly as the composer does.
pub(crate) async fn say(state: &AppState, id: SessionId, content: String) -> Result<()> {
    let input = MessageInput {
        id: uuid::Uuid::new_v4().to_string(),
        content,
        configuration_revision: None,
    };
    message(State(state.clone()), Path(id), Json(input))
        .await
        .map(|_| ())
}
/// For the agent tools: the stored conversation.
pub(crate) async fn stored(state: &AppState, id: SessionId) -> Result<Value> {
    snapshot(state, id).await
}

async fn open_conversation(
    State(state): State<AppState>,
    Path(id): Path<SessionId>,
) -> Result<Json<Session>> {
    crate::worktree_setup::ready_for(&state, id).await;
    let _guard = state.operations.lock().await;
    let session = session_record(&state, id).await?;
    if session.provider == ProviderKind::Terminal {
        return Err(ApiError::bad(
            "Terminals do not support structured conversations",
        ));
    }
    if session.interaction_mode != InteractionMode::Structured {
        if state
            .runtime
            .get(&id.to_string())
            .is_some_and(|r| r.running())
            || !matches!(
                session.status,
                SessionStatus::Stopped | SessionStatus::Failed
            )
        {
            return Err(ApiError::conflict(
                "End the original terminal session before enabling chat; it will not be taken over",
            ));
        }
        if !db(&state, move |s| {
            s.set_interaction_mode(id, InteractionMode::Structured)
        })
        .await?
        {
            return Err(ApiError::conflict("Session state changed"));
        }
    }
    start_locked(&state, &session_record(&state, id).await?).await?;
    Ok(Json(session_record(&state, id).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageInput {
    id: String,
    content: String,
    configuration_revision: Option<u64>,
}
/// How much of the earlier conversation goes ahead of the first message after
/// an endpoint or checkout change: the most recent turns, about 16k tokens.
const CARRIED_BYTES: usize = 64 * 1024;
pub(super) type CommandAck = (StatusCode, Json<Value>);
pub(super) fn accepted(id: Option<&str>, duplicate: bool) -> CommandAck {
    (
        StatusCode::ACCEPTED,
        Json(json!({"accepted":true,"id":id,"duplicate":duplicate})),
    )
}
async fn message(
    State(state): State<AppState>,
    Path(id): Path<SessionId>,
    Json(input): Json<MessageInput>,
) -> Result<CommandAck> {
    if uuid::Uuid::parse_str(&input.id).is_err()
        || input.content.trim().is_empty()
        || input.content.len() > 32768
        || input.content.contains('\0')
    {
        return Err(ApiError::bad(
            "A message needs a UUID and 1–32768 bytes of text without NUL",
        ));
    }
    crate::worktree_setup::ready_for(&state, id).await;
    let _guard = state.operations.lock().await;
    let session = session_record(&state, id).await?;
    let mid = input.id.clone();
    let receipt_content = input.content.clone();
    if let Some(matches) = db(&state, move |s| {
        s.chat_submission(id, &mid, &receipt_content)
    })
    .await?
    {
        return if matches {
            Ok(accepted(Some(&input.id), true))
        } else {
            Err(ApiError::conflict(
                "This message ID was already used for different text",
            ))
        };
    }
    if input
        .configuration_revision
        .is_some_and(|expected| expected != session.configuration_revision)
    {
        return Err(ApiError::conflict(
            "The session configuration changed. Refresh the selected endpoint before sending; this message was not dispatched.",
        ));
    }
    let runtime = start_locked(&state, &session).await?;
    if !runtime.idle() {
        return Err(ApiError::conflict(
            "Wait for the active turn and its approvals before sending another message",
        ));
    }
    // Read before the message is stored: storing it is what marks the new
    // context as spoken to.
    let carried = db(&state, move |s| s.carried_transcript(id, CARRIED_BYTES)).await?;
    let mid = input.id.clone();
    let content = input.content.clone();
    match db(&state, move |s| s.submit_chat_message(id, &mid, &content)).await? {
        MessageSubmission::Conflict => {
            return Err(ApiError::conflict(
                "This message ID was already used for different text",
            ));
        }
        MessageSubmission::Duplicate => return Ok(accepted(Some(&input.id), true)),
        MessageSubmission::New(event) => state.chats.publish(id, event),
    }
    runtime.busy.store(true, Ordering::Release);
    if let Err(error) = runtime
        .send(json!({"type":"message","id":input.id,"content":carried.unwrap_or_default() + &input.content}))
        .await
    {
        // try_send failed before the value entered the queue. Keep the durable
        // receipt to prevent replay, but do not leave an imaginary active turn.
        runtime.busy.store(false, Ordering::Release);
        persist(&state,id,json!({"type":"error","message":"Message dispatch failed. Inspect conversation state before resending; the message is never replayed automatically."})).await?;
        persist(
            &state,
            id,
            json!({"type":"turn","id":input.id,"status":"failed"}),
        )
        .await?;
        return Err(error);
    }
    Ok(accepted(Some(&input.id), false))
}
/// A message for the turn that is running: the client reads it at its next
/// step instead of after the turn. With no turn running it is an ordinary
/// message, so a turn that ended while it was being written loses nothing.
async fn steer(
    State(state): State<AppState>,
    Path(id): Path<SessionId>,
    Json(input): Json<MessageInput>,
) -> Result<CommandAck> {
    if uuid::Uuid::parse_str(&input.id).is_err()
        || input.content.trim().is_empty()
        || input.content.len() > 32768
        || input.content.contains('\0')
    {
        return Err(ApiError::bad(
            "A message needs a UUID and 1–32768 bytes of text without NUL",
        ));
    }
    let running = {
        let _guard = state.operations.lock().await;
        session_record(&state, id).await?;
        let runtime = state.chats.get(id).filter(|r| r.running());
        match runtime {
            Some(runtime) if runtime.busy.load(Ordering::Acquire) => {
                let mid = input.id.clone();
                let content = input.content.clone();
                match db(&state, move |s| {
                    s.submit_chat_message_as(id, &mid, &content, true)
                })
                .await?
                {
                    MessageSubmission::Conflict => {
                        return Err(ApiError::conflict(
                            "This message ID was already used for different text",
                        ));
                    }
                    MessageSubmission::Duplicate => return Ok(accepted(Some(&input.id), true)),
                    MessageSubmission::New(event) => state.chats.publish(id, event),
                }
                runtime
                    .send(json!({"type":"steer","id":input.id,"content":input.content}))
                    .await?;
                true
            }
            _ => false,
        }
    };
    if running {
        Ok(accepted(Some(&input.id), false))
    } else {
        message(State(state), Path(id), Json(input)).await
    }
}
async fn interrupt(State(state): State<AppState>, Path(id): Path<SessionId>) -> Result<CommandAck> {
    let _guard = state.operations.lock().await;
    session_record(&state, id).await?;
    if let Some(runtime) = state.chats.get(id).filter(|r| r.running()) {
        runtime.send(json!({"type":"interrupt"})).await?;
    }
    Ok(accepted(None, false))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PermissionInput {
    mode: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelInput {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    effort: Option<String>,
}
/// Choose the model for a running session.
///
/// This is deliberately not an endpoint change: it neither bumps the
/// configuration revision nor restarts the bridge, because both native clients
/// switch models in place and the conversation's context must survive it. The
/// client validates the name and refuses one it cannot serve.
async fn select_model(
    State(state): State<AppState>,
    Path(id): Path<SessionId>,
    Json(input): Json<ModelInput>,
) -> Result<CommandAck> {
    let _guard = state.operations.lock().await;
    session_record(&state, id).await?;
    let model = input
        .model
        .map(|m| m.trim().to_owned())
        .filter(|m| !m.is_empty());
    if let Some(model) = model.as_deref()
        && (model.len() > 128 || model.chars().any(char::is_control))
    {
        return Err(ApiError::bad("Choose a model the client offers."));
    }
    if let Some(effort) = input.effort.as_deref()
        && !providers::EFFORT_LEVELS.contains(&effort)
    {
        return Err(ApiError::bad("Unsupported reasoning effort."));
    }
    if model.is_none() && input.effort.is_none() {
        return Err(ApiError::bad("Choose a model or a thinking depth."));
    }
    let runtime = state
        .chats
        .get(id)
        .filter(|r| r.running())
        .ok_or_else(|| ApiError::conflict("Start the session before choosing a model"))?;
    runtime
        .send(model_command(model.as_deref(), input.effort.as_deref()))
        .await?;
    Ok(accepted(None, false))
}
/// How tools get approved, for a session that is already running.
///
/// The names are Tendly's and each bridge translates them, because the two
/// clients do not model this the same way: Claude Code switches a session-wide
/// mode, Codex decides per command. `danger` is the one that stops asking, and
/// it is named for what it costs rather than for what it saves.
///
/// Which modes a session actually offers comes from the client, in the same
/// settings event as its model list — Codex has no plan or accept-edits, and a
/// control that errors when used would be worse than one that is not there.
const PERMISSION_MODES: [&str; 4] = ["ask", "plan", "accept_edits", "danger"];

async fn select_permission(
    State(state): State<AppState>,
    Path(id): Path<SessionId>,
    Json(input): Json<PermissionInput>,
) -> Result<CommandAck> {
    let _guard = state.operations.lock().await;
    session_record(&state, id).await?;
    if !PERMISSION_MODES.contains(&input.mode.as_str()) {
        return Err(ApiError::bad("Unsupported permission mode."));
    }
    state.chats.choose_permission(id, &input.mode);
    let Some(runtime) = state.chats.get(id).filter(|r| r.running()) else {
        // Stopped, or exited after a failed turn: the next start applies it.
        return Ok((
            StatusCode::ACCEPTED,
            Json(json!({"accepted":true,"id":null,"duplicate":false,"pending":true})),
        ));
    };
    runtime
        .send(json!({"type": "permission", "mode": input.mode}))
        .await?;
    Ok(accepted(None, false))
}

/// Absent means "leave the depth as it is". A `null` would reach the bridge as a
/// present-but-unusable value and be rejected, so the field is omitted entirely.
pub(super) fn model_command(model: Option<&str>, effort: Option<&str>) -> Value {
    let mut command = json!({"type": "model"});
    if let Some(model) = model {
        command["model"] = Value::from(model);
    }
    if let Some(effort) = effort {
        command["effort"] = Value::from(effort);
    }
    command
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalInput {
    request_id: String,
    decision: String,
    #[serde(default)]
    answers: HashMap<String, Vec<String>>,
}
async fn approval(
    State(state): State<AppState>,
    Path(id): Path<SessionId>,
    Json(input): Json<ApprovalInput>,
) -> Result<CommandAck> {
    let _guard = state.operations.lock().await;
    session_record(&state, id).await?;
    let runtime = state
        .chats
        .get(id)
        .filter(|r| r.running())
        .ok_or_else(|| ApiError::conflict("The native request is no longer active"))?;
    if input.answers.len() > 16
        || input.answers.iter().any(|(k, v)| {
            k.len() > 200 || v.len() > 20 || v.iter().any(|s| s.len() > 8192 || s.contains('\0'))
        })
    {
        return Err(ApiError::bad("Approval answers exceed supported limits"));
    }
    if serde_json::to_vec(&input.answers)
        .map_err(ApiError::internal)?
        .len()
        > 128 * 1024
    {
        return Err(ApiError::bad(
            "Approval answers exceed the total size limit",
        ));
    }
    let allowed = runtime
        .approvals
        .lock()
        .expect("approvals")
        .get(&input.request_id)
        .is_some_and(|choices| choices.contains(&input.decision));
    if !allowed {
        return Err(ApiError::conflict(
            "The request expired or does not support this decision",
        ));
    }
    runtime.send(json!({"type":"approval","request_id":input.request_id,"decision":input.decision,"answers":input.answers})).await?;
    Ok(accepted(Some(&input.request_id), false))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigurationInput {
    endpoint_profile_id: Option<uuid::Uuid>,
    model: Option<String>,
    effort: Option<String>,
    #[serde(default)]
    confirmed: bool,
}
async fn configuration(
    State(state): State<AppState>,
    Path(id): Path<SessionId>,
    Json(input): Json<ConfigurationInput>,
) -> Result<Json<Session>> {
    if !input.confirmed {
        return Err(ApiError::bad(
            "Confirm a new native conversation before switching endpoints; displayed history is not forwarded",
        ));
    }
    if input
        .model
        .as_ref()
        .is_some_and(|m| m.len() > 200 || m.chars().any(char::is_control))
    {
        return Err(ApiError::bad("Invalid model identifier"));
    }
    crate::providers::validate_effort(input.effort.as_deref())?;
    let _guard = state.operations.lock().await;
    let session = session_record(&state, id).await?;
    if session.provider == ProviderKind::Terminal {
        return Err(ApiError::bad(
            "Terminal sessions do not use endpoint profiles",
        ));
    }
    let model = input.model.filter(|s| !s.trim().is_empty());
    let effort = input.effort.filter(|s| !s.trim().is_empty());
    if let Some(pid) = input.endpoint_profile_id {
        let profile = db(&state, move |s| s.get_endpoint_profile(pid))
            .await?
            .ok_or_else(|| ApiError::missing("Profile"))?;
        if profile.provider != session.provider {
            return Err(ApiError::bad("Choose an endpoint for the same CLI"));
        }
        crate::providers::validate_profile(&profile)?;
        if let Some(reference) = &profile.native_config {
            // A model and a depth are launch choices the client still resolves
            // against its own account, so an official account accepts them —
            // and for a model whose live switch the upstream refuses, choosing
            // it at launch is the only way through.
            crate::native_config::validate_reference(&state, &session.provider, reference)?;
        }
    }
    if state
        .runtime
        .get(&id.to_string())
        .is_some_and(|r| r.running())
    {
        return Err(ApiError::conflict(
            "End the native terminal session before changing its endpoint",
        ));
    }
    if let Some(runtime) = state.chats.get(id).filter(|r| r.running()) {
        if !runtime.idle() {
            return Err(ApiError::conflict(
                "Finish or interrupt the active turn and resolve approvals before switching",
            ));
        }
        state.chats.stop(id).await?;
        db(&state, move |s| {
            s.set_session_status(id, SessionStatus::Stopped)
        })
        .await?;
    }
    if !db(&state, move |s| {
        s.switch_session_configuration_with_effort(id, input.endpoint_profile_id, model, effort)
    })
    .await?
    {
        return Err(ApiError::conflict(
            "The session must be idle or stopped before changing its endpoint",
        ));
    }
    state.chats.publish(id, snapshot(&state, id).await?);
    Ok(Json(session_record(&state, id).await?))
}

/// Stop a structured session that is idle, before moving it; refuse one mid-turn
/// or waiting on an approval. A session not running needs nothing.
pub async fn stop_if_idle(state: &AppState, id: SessionId) -> Result<()> {
    if let Some(runtime) = state.chats.get(id).filter(|r| r.running()) {
        if !runtime.idle() {
            return Err(ApiError::conflict(
                "Finish or interrupt the active turn and resolve approvals before switching",
            ));
        }
        state.chats.stop(id).await?;
        db(state, move |s| {
            s.set_session_status(id, SessionStatus::Stopped)
        })
        .await?;
    }
    Ok(())
}
