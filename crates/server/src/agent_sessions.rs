//! Agent tools about sessions: start one alongside, follow it, point the
//! person at something, look at the quota.
//!
//! A session may start others -- Codex trying another approach on its own
//! branch while Claude keeps going, say -- and then read their replies and send
//! them messages. It may do that only for sessions it started, and a session
//! it started may not start more, so there is never a tree to lose track of.
//! The first start from a session is confirmed by the person; after that it
//! may start more without asking, at most a few running at once. Other
//! sessions' conversations stay out of reach.
use crate::{
    ApiError, AppState,
    activity::Answer,
    agent::{Caller, live, text},
    agent_config::who,
    db,
};
use serde_json::{Value, json};
use std::time::Duration;
use tendly_domain::{ProviderKind, Session, SessionId};

/// Sessions one session may have running at once.
const RUNNING_CHILDREN: usize = 4;
const LONGEST_WAIT: u64 = 120;
const REPLY_CHARS: usize = 4000;

fn object(properties: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": properties, "required": required, "additionalProperties": false })
}

pub fn tools() -> Vec<Value> {
    vec![
        crate::agent::tool(
            "tendly_spawn",
            "Start another Claude Code or Codex session in your workspace, with a first message, optionally on its own Git branch (a separate worktree). It runs on its own; follow it with tendly_session. Use it for independent work worth doing in parallel, not for quick questions. The first start is confirmed by the person; a started session cannot start more.",
            object(
                json!({
                    "provider": { "type": "string", "enum": ProviderKind::AGENTS.map(|kind| kind.as_str()) },
                    "prompt": { "type": "string", "description": "The first message: a complete, self-contained task." },
                    "title": { "type": "string" },
                    "branch": { "type": "string", "description": "Work on this branch in its own worktree, creating it if needed. Omit to share your directory." },
                    "endpoint_id": { "type": "string", "description": "Endpoint profile id; default is the person's default for that client." },
                    "model": { "type": "string" },
                    "keep": { "type": "boolean", "description": "Keep it in the session list afterwards. Default false: a temporary session." }
                }),
                &["provider", "prompt"],
            ),
        ),
        crate::agent::tool(
            "tendly_session",
            "Follow the sessions you started (list, result, wait until its turn ends, message it), or manage your own session (rename, keep). You cannot read sessions you did not start.",
            object(
                json!({
                    "action": { "type": "string", "enum": ["list", "result", "wait", "message", "rename", "keep"] },
                    "id": { "type": "string", "description": "A session you started, for result, wait and message." },
                    "seconds": { "type": "integer", "minimum": 1, "maximum": LONGEST_WAIT, "description": "For wait. Default 60." },
                    "text": { "type": "string", "description": "For message." },
                    "title": { "type": "string", "description": "For rename: your own session's new title." }
                }),
                &["action"],
            ),
        ),
        crate::agent::tool(
            "tendly_show",
            "Show the person something in the Tendly window: a file (at a line), your workspace's Git changes, or a session you started.",
            object(
                json!({
                    "file": { "type": "string", "description": "Path relative to your working directory, or absolute inside it." },
                    "line": { "type": "integer", "minimum": 1 },
                    "changes": { "type": "boolean", "description": "Open the Git changes view." },
                    "session_id": { "type": "string" }
                }),
                &[],
            ),
        ),
        crate::agent::tool(
            "tendly_usage",
            "The quota your session's official account last reported (Claude or ChatGPT sign-in managed in Tendly). Nothing is known for API-key endpoints.",
            object(json!({}), &[]),
        ),
    ]
}

async fn own_session(state: &AppState, caller: Caller, tool: &str) -> Result<Session, ApiError> {
    let Caller::Session(id) = caller else {
        return Err(ApiError::bad(format!(
            "{tool} works only from a session Tendly launched"
        )));
    };
    db(state, move |s| s.get_session(id))
        .await?
        .ok_or_else(|| ApiError::missing("Session"))
}

fn session_id(arguments: &Value) -> Result<SessionId, ApiError> {
    text(arguments, "id")
        .or_else(|| text(arguments, "session_id"))
        .ok_or_else(|| ApiError::bad("id is required"))?
        .parse()
        .map_err(|_| ApiError::bad("id is not a session id"))
}

/// A session the caller started, or an error that says it is not one.
async fn child(state: &AppState, parent: SessionId, id: SessionId) -> Result<Session, ApiError> {
    if db(state, move |s| s.agent_parent(id)).await? != Some(parent) {
        return Err(ApiError::bad("That is not a session you started"));
    }
    db(state, move |s| s.get_session(id))
        .await?
        .ok_or_else(|| ApiError::missing("Session"))
}

fn activity(state: &AppState, id: SessionId) -> &'static str {
    state
        .chats
        .get(id)
        .map_or("stopped", |runtime| runtime.activity())
}

/// The last thing the session's agent said, stitched from its streamed parts.
fn last_reply(events: &[Value]) -> Option<String> {
    let mut reply: Option<(String, String)> = None;
    for event in events {
        if event["type"] != "message" || event["role"] != "assistant" {
            continue;
        }
        let id = event["id"].as_str().unwrap_or_default().to_owned();
        let text = event["text"].as_str().unwrap_or_default();
        match &mut reply {
            Some((current, body)) if *current == id && event["delta"] == true => {
                body.push_str(text)
            }
            _ => reply = Some((id, text.to_owned())),
        }
    }
    reply.map(|(_, body)| {
        if body.chars().count() > REPLY_CHARS {
            let tail: String = body
                .chars()
                .rev()
                .take(REPLY_CHARS)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            format!("…{tail}")
        } else {
            body
        }
    })
}

async fn summary(state: &AppState, session: &Session) -> Result<Value, ApiError> {
    let conversation = crate::conversations::stored(state, session.id).await?;
    let events = conversation["events"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let turn = events
        .iter()
        .rev()
        .find(|event| event["type"] == "turn")
        .and_then(|event| event["status"].as_str())
        .map(str::to_owned);
    Ok(json!({
        "id": session.id,
        "title": session.title,
        "provider": session.provider,
        "branch": session.checkout_branch,
        "activity": activity(state, session.id),
        "last_turn": turn,
        "last_reply": last_reply(&events),
    }))
}

fn provider_of(arguments: &Value) -> Result<ProviderKind, ApiError> {
    text(arguments, "provider")
        .and_then(ProviderKind::parse_agent)
        .ok_or_else(|| ApiError::bad(format!("provider must be {}", ProviderKind::agent_names())))
}

pub async fn spawn(state: &AppState, caller: Caller, arguments: &Value) -> Result<Value, ApiError> {
    let parent = own_session(state, caller, "tendly_spawn").await?;
    let parent_id = parent.id;
    if db(state, move |s| s.agent_parent(parent_id))
        .await?
        .is_some()
    {
        return Err(ApiError::bad(
            "A session started by another session cannot start more",
        ));
    }
    let running = db(state, move |s| s.agent_children(parent_id))
        .await?
        .into_iter()
        .filter(|id| live(state, *id))
        .count();
    if running >= RUNNING_CHILDREN {
        return Err(ApiError::conflict(format!(
            "{RUNNING_CHILDREN} sessions you started are still running; wait for one to finish"
        )));
    }
    let provider = provider_of(arguments)?;
    let prompt = text(arguments, "prompt")
        .ok_or_else(|| ApiError::bad("prompt is required"))?
        .to_owned();
    if prompt.len() > 32768 {
        return Err(ApiError::bad("The prompt is longer than 32768 bytes"));
    }
    let title = text(arguments, "title")
        .map(str::to_owned)
        .unwrap_or_else(|| {
            let first: String = prompt
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(48)
                .collect();
            format!("↳ {first}")
        });
    let branch = text(arguments, "branch").map(str::to_owned);
    let keep = arguments
        .get("keep")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let who = who(state, caller).await;
    if !state.agents.may_spawn(parent_id) {
        let label = provider.label();
        let excerpt: String = prompt.chars().take(300).collect();
        let mut rows = vec![("Task", excerpt)];
        if let Some(branch) = &branch {
            rows.push(("Branch", branch.clone()));
        }
        let answer = state
            .activity
            .ask(
                who.clone(),
                "Start a {client} session",
                json!({ "client": label }),
                rows,
                vec!["It runs on its own. This session may then start more without asking, read their replies and message them."],
                None,
            )
            .await?;
        if !matches!(answer, Answer::Approved { .. }) {
            return Err(ApiError::conflict(
                "The person declined. No session was started.",
            ));
        }
        state.agents.allow_spawning(parent_id);
    }
    let input: crate::CreateSession = serde_json::from_value(json!({
        "interaction_mode": "structured",
        "provider": provider,
        "title": title,
        "endpoint_profile_id": text(arguments, "endpoint_id"),
        "model": text(arguments, "model"),
        "ephemeral": !keep,
    }))
    .map_err(|error| ApiError::bad(error.to_string()))?;
    let (_, axum::Json(created)) = crate::create_session(
        axum::extract::State(state.clone()),
        axum::extract::Path(parent.workspace_id),
        axum::Json(input),
    )
    .await?;
    let id = created.id;
    db(state, move |s| s.set_agent_parent(id, parent_id)).await?;
    if let Some(branch) = branch.clone() {
        crate::checkouts::move_to_branch(state, id, branch).await?;
    }
    crate::conversations::open(state, id).await?;
    crate::conversations::say(state, id, prompt).await?;
    state.activity.notify_with(
        who,
        "Started session “{title}”",
        json!({ "title": created.title }),
        None,
        Some(json!({ "session_id": id })),
    );
    Ok(json!({
        "started": { "id": id, "title": created.title, "provider": created.provider, "branch": branch, "temporary": !keep },
        "next": "Call tendly_session with action wait or result and this id to follow it.",
    }))
}

pub async fn session(
    state: &AppState,
    caller: Caller,
    arguments: &Value,
) -> Result<Value, ApiError> {
    let me = own_session(state, caller, "tendly_session").await?;
    match text(arguments, "action") {
        Some("list") => {
            let parent = me.id;
            let mut list = Vec::new();
            for id in db(state, move |s| s.agent_children(parent)).await? {
                if let Some(session) = db(state, move |s| s.get_session(id)).await? {
                    list.push(json!({
                        "id": session.id, "title": session.title, "provider": session.provider,
                        "branch": session.checkout_branch, "activity": activity(state, session.id),
                    }));
                }
            }
            Ok(json!(list))
        }
        Some("result") => {
            let target = child(state, me.id, session_id(arguments)?).await?;
            summary(state, &target).await
        }
        Some("wait") => {
            let target = child(state, me.id, session_id(arguments)?).await?;
            let seconds = arguments
                .get("seconds")
                .and_then(Value::as_u64)
                .unwrap_or(60)
                .clamp(1, LONGEST_WAIT);
            let until = tokio::time::Instant::now() + Duration::from_secs(seconds);
            let mut done = false;
            // A turn just sent may not have begun: give it a moment to start
            // before an idle client is taken to mean a finished one.
            tokio::time::sleep(Duration::from_millis(500)).await;
            while tokio::time::Instant::now() < until {
                if matches!(
                    activity(state, target.id),
                    "idle" | "stopped" | "waiting_for_approval"
                ) {
                    done = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(750)).await;
            }
            let mut result = summary(state, &target).await?;
            result["finished_waiting"] = json!(done);
            Ok(result)
        }
        Some("message") => {
            let target = child(state, me.id, session_id(arguments)?).await?;
            let body = text(arguments, "text").ok_or_else(|| ApiError::bad("text is required"))?;
            crate::conversations::say(state, target.id, body.to_owned()).await?;
            Ok(json!({ "sent": true, "id": target.id }))
        }
        Some("rename") => {
            let title = crate::bounded_name(text(arguments, "title").unwrap_or(""))?;
            let id = me.id;
            let saved = title.clone();
            db(state, move |s| s.update_session_title(id, &saved)).await?;
            Ok(json!({ "renamed": title }))
        }
        Some("keep") => {
            let id = me.id;
            db(state, move |s| s.keep_session(id)).await?;
            Ok(json!({ "kept": true }))
        }
        _ => Err(ApiError::bad(
            "action must be list, result, wait, message, rename or keep",
        )),
    }
}

pub async fn show(state: &AppState, caller: Caller, arguments: &Value) -> Result<Value, ApiError> {
    let me = own_session(state, caller, "tendly_show").await?;
    let target = if let Some(file) = text(arguments, "file") {
        let line = arguments.get("line").and_then(Value::as_u64);
        json!({ "kind": "file", "workspace_id": me.workspace_id, "path": file, "line": line, "checkout": me.checkout_path })
    } else if arguments.get("changes").and_then(Value::as_bool) == Some(true) {
        json!({ "kind": "changes", "workspace_id": me.workspace_id, "checkout": me.checkout_path })
    } else if text(arguments, "session_id").is_some() {
        let id = session_id(arguments)?;
        if id != me.id {
            child(state, me.id, id).await?;
        }
        json!({ "kind": "session", "session_id": id })
    } else {
        return Err(ApiError::bad("Give file, changes or session_id"));
    };
    state.activity.show(who(state, caller).await, target)?;
    Ok(json!({ "shown": true }))
}

pub async fn usage(state: &AppState, caller: Caller) -> Result<Value, ApiError> {
    let me = own_session(state, caller, "tendly_usage").await?;
    let Some(profile) = me
        .endpoint_profile_id
        .or(me.endpoint_snapshot.as_ref().map(|p| p.id))
    else {
        return Ok(
            json!({ "known": false, "why": "This session uses the client's own sign-in, which Tendly does not manage." }),
        );
    };
    let axum::Json(accounts) = crate::accounts::list(axum::extract::State(state.clone())).await?;
    let Some(account) = accounts
        .into_iter()
        .find(|account| account.profile_id == profile)
    else {
        return Ok(
            json!({ "known": false, "why": "Quota is known only for official accounts managed in Tendly, not API-key endpoints." }),
        );
    };
    Ok(
        json!({ "known": true, "account": account.name, "plan": account.plan, "checked_at": account.checked_at, "usage": account.usage }),
    )
}
