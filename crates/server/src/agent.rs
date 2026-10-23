//! Tools an agent can use to see and configure Tendly.
//!
//! An agent reaches these through `tendly mcp`, a stdio MCP server that
//! only relays (mcp.rs). Everything that matters happens here: which tools
//! exist, who is calling, and what each caller may do. Keeping it on the
//! server means there is one place to check and one place to change.
//!
//! Callers identify themselves with a bearer token. A session Tendly
//! launches gets its own token, issued at launch and good only while that
//! session's process is running, so a call is known to come from it. Any other caller that
//! passes the server's normal authentication -- `tendly mcp` in a terminal
//! outside Tendly -- is `External`. The token says who is asking; it is not
//! what protects anything sensitive. That is the confirmation a person gives
//! in the browser, which no token can skip.
use crate::{ApiError, AppState, db};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, header},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tendly_domain::{Session, SessionId};

/// Who is calling a tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caller {
    Session(SessionId),
    External,
}

#[derive(Clone, Default)]
pub struct AgentRegistry {
    tokens: Arc<Mutex<HashMap<String, SessionId>>>,
    /// Sessions the person has let start other sessions (agent_sessions.rs),
    /// until the server restarts.
    spawners: Arc<Mutex<std::collections::HashSet<SessionId>>>,
}

impl AgentRegistry {
    /// A fresh token for a session about to launch. Any earlier one for the
    /// same session is revoked: only the process being started holds a key.
    pub fn issue(&self, session: SessionId) -> String {
        let token = format!(
            "adk_{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let mut tokens = self.tokens.lock().expect("agent tokens");
        tokens.retain(|_, owner| *owner != session);
        tokens.insert(token.clone(), session);
        token
    }
    pub fn may_spawn(&self, session: SessionId) -> bool {
        self.spawners.lock().expect("spawners").contains(&session)
    }
    pub fn allow_spawning(&self, session: SessionId) {
        self.spawners.lock().expect("spawners").insert(session);
    }
    fn session_for(&self, token: &str) -> Option<SessionId> {
        self.tokens
            .lock()
            .expect("agent tokens")
            .get(token)
            .copied()
    }
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
}

/// Whether a session has a process running now, in either interface. A token
/// outlives nothing: once its process has ended it identifies no one.
pub(crate) fn live(state: &AppState, session: SessionId) -> bool {
    state.chats.get(session).is_some_and(|chat| chat.running())
        || state
            .runtime
            .get(&session.to_string())
            .is_some_and(|runtime| runtime.running())
}

fn session_caller(state: &AppState, headers: &HeaderMap) -> Option<SessionId> {
    bearer(headers)
        .and_then(|token| state.agents.session_for(token))
        .filter(|session| live(state, *session))
}

/// A request carrying a live session's agent token. The guard lets these
/// through to `/api/agent/` only, and nowhere else.
pub fn is_agent_request(state: &AppState, headers: &HeaderMap) -> bool {
    session_caller(state, headers).is_some()
}

fn caller(state: &AppState, headers: &HeaderMap) -> Caller {
    session_caller(state, headers).map_or(Caller::External, Caller::Session)
}

/// What a session's process needs to reach these tools: where the server is
/// and its own token. Set in its environment at every launch.
pub fn launch_environment(state: &AppState, session: SessionId) -> [(String, String); 3] {
    [
        ("TENDLY_URL".into(), state.security.local_url()),
        ("TENDLY_AGENT_TOKEN".into(), state.agents.issue(session)),
        ("TENDLY_SESSION_ID".into(), session.to_string()),
    ]
}

/// Give a launch Tendly's tools, unless the person turned them off.
///
/// Only through this launch's flags: the person's own client configuration
/// (`~/.claude`, `~/.codex`, or a host configuration a profile points at) is
/// never written. Claude reads a small private file named by `--mcp-config`,
/// and the tools are allowed up front, because what needs the person's word
/// is asked in Tendly itself. Codex takes the same as `-c` overrides.
/// A terminal gets the environment only.
pub fn equip(
    state: &AppState,
    session: &Session,
    spec: &mut tendly_runtime::SpawnSpec,
) -> Result<(), ApiError> {
    if !tools_enabled(state, session.id) {
        return Ok(());
    }
    let identity = launch_environment(state, session.id);
    for (key, value) in &identity {
        spec.env_remove.retain(|removed| removed != key);
        spec.env.insert(key.clone(), value.clone());
    }
    if session.provider == tendly_domain::ProviderKind::Terminal {
        return Ok(());
    }
    let program = std::env::current_exe()
        .map_err(ApiError::internal)?
        .to_string_lossy()
        .into_owned();
    if let Some(client) = crate::adapters::adapter(session.provider) {
        client.equip_tools(state, session.id, &program, &identity, &mut spec.args)?;
    }
    Ok(())
}

/// Whether a session's launches carry the tools: its own choice, else the
/// preference for every session.
fn tools_enabled(state: &AppState, session: SessionId) -> bool {
    state
        .store
        .session_agent_tools(session)
        .ok()
        .flatten()
        .unwrap_or_else(|| crate::preferences::load_blocking(state).agent_tools())
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/agent/tools", get(list_tools))
        .route("/api/agent/call", post(call_tool))
        // Deliberately outside /api/agent/: a session's own token cannot
        // switch its tools on or off.
        .route(
            "/api/sessions/{id}/agent-tools",
            get(session_tools).put(set_session_tools),
        )
}

async fn session_tools_view(state: &AppState, id: SessionId) -> Result<Json<Value>, ApiError> {
    let choice = db(state, move |s| s.session_agent_tools(id)).await?;
    let default = crate::preferences::load(state).await?.agent_tools();
    Ok(Json(
        json!({ "choice": choice, "default": default, "enabled": choice.unwrap_or(default) }),
    ))
}

async fn session_tools(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<SessionId>,
) -> Result<Json<Value>, ApiError> {
    db(&state, move |s| s.get_session(id))
        .await?
        .ok_or_else(|| ApiError::missing("Session"))?;
    session_tools_view(&state, id).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionToolsInput {
    /// `true` or `false` for this session, `null` to follow the preference.
    choice: Option<bool>,
}

async fn set_session_tools(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<SessionId>,
    Json(input): Json<SessionToolsInput>,
) -> Result<Json<Value>, ApiError> {
    if !db(&state, move |s| s.set_session_agent_tools(id, input.choice)).await? {
        return Err(ApiError::missing("Session"));
    }
    session_tools_view(&state, id).await
}

/// What the MCP server hands the model at connection time.
pub const INSTRUCTIONS: &str = "\
Tendly is the workspace this agent may be running in: a local web app that runs Claude Code, Codex and terminals side by side, with workspaces, Git worktrees and endpoint profiles. These tools let you inspect it and, where the person allows, configure it.

- Start with tendly_status to learn which session, workspace and endpoint you are.
- tendly_list shows workspaces, sessions (titles and status only, never conversations), endpoint profiles and installed clients.
- tendly_balance configures custom endpoint balances with a reviewed JS script, or reads/tests/refreshes a saved balance. Money keeps its original currency; percentage means remaining.
- tendly_endpoint creates, changes, tests or imports endpoint profiles. To set up an API endpoint, call create with the provider, URL and model: the person confirms in the Tendly window and types the key there. Then call test to check it works.
- Never ask the person to paste an API key into the conversation, and never put one in a tool argument. Tendly collects keys in its own window.
- tendly_preferences sets what new sessions start with, and the light or dark theme of the interface and its terminals; tendly_workspace adds a directory as a workspace.
- tendly_spawn starts another Claude Code or Codex session with a self-contained task, optionally on its own branch, for independent work worth doing in parallel. Follow it with tendly_session (wait, result, message). You can only follow sessions you started, and a session you start cannot start more.
- tendly_show opens a file at a line, your Git changes, or a session in the person's Tendly window: use it to point at what you changed instead of describing where it is.
- tendly_usage reports the quota your official account last reported.
- tendly_canvas lists the tabs open on the canvas and closes some or all of them. Closing a tab never ends a session, except a temporary one.
- Changes that matter are confirmed by the person in the Tendly window, and a call waits up to 5 minutes for that answer. If they decline, do not retry the same change unasked.";

pub(crate) fn tool(name: &str, description: &str, schema: Value) -> Value {
    json!({ "name": name, "description": description, "inputSchema": schema })
}

pub fn tools() -> Vec<Value> {
    let mut tools = vec![
        tool(
            "tendly_status",
            "Who you are in Tendly: the calling session, its workspace, its directory and branch and which other sessions share that directory, its endpoint profile and model, and the Tendly version.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
        tool(
            "tendly_list",
            "List Tendly workspaces, sessions (metadata only), endpoint profiles, or installed agent clients.",
            json!({
                "type": "object",
                "properties": {
                    "what": { "type": "string", "enum": ["workspaces", "sessions", "endpoints", "clients"] },
                    "workspace_id": { "type": "string", "description": "Only with what=sessions: limit to one workspace." },
                    "include_archived": { "type": "boolean", "description": "Only with what=sessions. Default false." }
                },
                "required": ["what"],
                "additionalProperties": false
            }),
        ),
    ];
    tools.push(crate::balances::tool());
    tools.extend(crate::agent_config::tools());
    tools.extend(crate::agent_sessions::tools());
    tools.extend(crate::agent_canvas::tools());
    tools
}

async fn list_tools() -> Json<Value> {
    Json(json!({ "instructions": INSTRUCTIONS, "tools": tools() }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CallInput {
    name: String,
    #[serde(default)]
    arguments: Value,
}

/// The MCP `tools/call` result shape. A tool that fails is still a successful
/// HTTP exchange: the model reads the error and can try something else.
fn result(outcome: Result<Value, ApiError>) -> Json<Value> {
    Json(match outcome {
        Ok(value) => json!({
            "content": [{ "type": "text", "text": serde_json::to_string_pretty(&value).unwrap_or_default() }],
            "isError": false
        }),
        Err(error) => json!({
            "content": [{ "type": "text", "text": error.message }],
            "isError": true
        }),
    })
}

async fn call_tool(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CallInput>,
) -> Json<Value> {
    let caller = caller(&state, &headers);
    let arguments = if input.arguments.is_null() {
        json!({})
    } else {
        input.arguments
    };
    result(match input.name.as_str() {
        "tendly_status" => status(&state, caller).await,
        "tendly_list" => list(&state, caller, &arguments).await,
        "tendly_balance" => crate::balances::call(&state, caller, &arguments).await,
        "tendly_endpoint" => crate::agent_config::endpoint(&state, caller, &arguments).await,
        "tendly_preferences" => crate::agent_config::preferences(&state, caller, &arguments).await,
        "tendly_workspace" => crate::agent_config::workspace(&state, caller, &arguments).await,
        "tendly_spawn" => crate::agent_sessions::spawn(&state, caller, &arguments).await,
        "tendly_session" => crate::agent_sessions::session(&state, caller, &arguments).await,
        "tendly_show" => crate::agent_sessions::show(&state, caller, &arguments).await,
        "tendly_usage" => crate::agent_sessions::usage(&state, caller).await,
        "tendly_canvas" => crate::agent_canvas::canvas(&state, caller, &arguments).await,
        other => Err(ApiError::bad(format!("Unknown Tendly tool {other:?}"))),
    })
}

pub(crate) fn text<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

async fn session_of(state: &AppState, caller: Caller) -> Result<Option<Session>, ApiError> {
    match caller {
        Caller::External => Ok(None),
        Caller::Session(id) => db(state, move |store| store.get_session(id)).await,
    }
}

/// How a profile's key is set, without saying what it is.
fn key_state(state_dir: &std::path::Path, reference: Option<&str>) -> &'static str {
    match reference.and_then(|value| value.strip_prefix("env:")) {
        None => "none",
        Some(name) if std::env::var(name).is_ok_and(|value| !value.is_empty()) => "environment",
        Some(name) if crate::secrets::resolve(state_dir, name).is_some() => "saved",
        Some(_) => "missing",
    }
}

pub(crate) fn endpoint_view(
    state_dir: &std::path::Path,
    profile: &tendly_domain::EndpointProfile,
) -> Value {
    json!({
        "id": profile.id,
        "name": profile.name,
        "provider": profile.provider,
        "endpoint_url": profile.endpoint_url,
        "model": profile.model,
        "effort": profile.effort,
        "permission_mode": profile.permission_mode,
        "proxy_url": profile.proxy_url,
        "key": key_state(state_dir, profile.secret_ref.as_deref()),
        "host_configuration": profile.native_config.is_some(),
    })
}

fn session_view(session: &Session) -> Value {
    json!({
        "id": session.id,
        "title": session.title,
        "provider": session.provider,
        "status": session.status,
        "workspace_id": session.workspace_id,
        "temporary": session.ephemeral,
        "archived": session.archived_at.is_some(),
        "branch": session.checkout_branch,
    })
}

/// Where a session works and who else works there: what it must know before
/// it switches a branch or rewrites history in that directory.
async fn checkout_view(
    state: &AppState,
    session: &Session,
    workspace: &tendly_domain::Workspace,
) -> Result<Value, ApiError> {
    let directory = session
        .checkout_path
        .clone()
        .unwrap_or_else(|| workspace.root_path.clone());
    let branch = match &session.checkout_path {
        Some(_) => session.checkout_branch.clone(),
        None => crate::workspace_io::git_branches(std::path::Path::new(&workspace.root_path))
            .await
            .ok()
            .and_then(|listed| listed.current),
    };
    let (id, place, workspace_id) = (
        session.id,
        session.checkout_path.clone(),
        session.workspace_id,
    );
    let shared: Vec<Value> = db(state, move |store| store.list_sessions(Some(workspace_id)))
        .await?
        .into_iter()
        .filter(|other| {
            other.id != id && other.archived_at.is_none() && other.checkout_path == place
        })
        .map(|other| json!({ "id": other.id, "title": other.title, "status": other.status }))
        .collect();
    let note = match (&session.checkout_base, shared.is_empty()) {
        (Some(base), _) => format!(
            "An isolated task: this worktree and branch are yours alone, started from {base}. Commit your work here; the person reviews it and merges it into {base}."
        ),
        (None, false) => "Other sessions work in this same directory and see every change you make. Do not switch branches, stash or rewrite history here.".to_owned(),
        (None, true) => "No other session works in this directory now.".to_owned(),
    };
    Ok(json!({
        "directory": directory,
        "worktree": session.checkout_path.is_some(),
        "branch": branch,
        "isolated_from": session.checkout_base,
        "shared_with": shared,
        "note": note,
    }))
}

async fn status(state: &AppState, caller: Caller) -> Result<Value, ApiError> {
    let session = session_of(state, caller).await?;
    let you = match &session {
        None => {
            json!({ "kind": "external", "note": "Not a session Tendly launched; session-specific tools are unavailable." })
        }
        Some(session) => {
            let workspace_id = session.workspace_id;
            let workspace = db(state, move |store| store.get_workspace(workspace_id)).await?;
            let checkout = match &workspace {
                Some(workspace) => checkout_view(state, session, workspace).await?,
                None => Value::Null,
            };
            json!({
                "kind": "session",
                "session": session_view(session),
                "workspace": workspace.map(|w| json!({ "id": w.id, "name": w.name, "root_path": w.root_path })),
                "working_directory": session.checkout_path,
                "checkout": checkout,
                "endpoint": session.endpoint_snapshot.as_ref().map(|profile| endpoint_view(&state.state_dir, profile)),
            })
        }
    };
    Ok(json!({ "tendly": { "version": env!("CARGO_PKG_VERSION") }, "you": you }))
}

async fn list(state: &AppState, _caller: Caller, arguments: &Value) -> Result<Value, ApiError> {
    match text(arguments, "what") {
        Some("workspaces") => {
            let workspaces = db(state, |store| store.list_workspaces()).await?;
            Ok(json!(
                workspaces
                    .iter()
                    .map(|w| json!({ "id": w.id, "name": w.name, "root_path": w.root_path }))
                    .collect::<Vec<_>>()
            ))
        }
        Some("sessions") => {
            let workspace = text(arguments, "workspace_id")
                .map(|value| {
                    value
                        .parse::<tendly_domain::WorkspaceId>()
                        .map_err(|_| ApiError::bad("workspace_id is not a workspace id"))
                })
                .transpose()?;
            let archived = arguments
                .get("include_archived")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let sessions = db(state, move |store| store.list_sessions(workspace)).await?;
            Ok(json!(
                sessions
                    .iter()
                    .filter(|session| archived || session.archived_at.is_none())
                    .take(200)
                    .map(session_view)
                    .collect::<Vec<_>>()
            ))
        }
        Some("endpoints") => {
            let profiles = db(state, |store| store.list_endpoint_profiles()).await?;
            Ok(json!(
                profiles
                    .iter()
                    .map(|profile| endpoint_view(&state.state_dir, profile))
                    .collect::<Vec<_>>()
            ))
        }
        Some("clients") => {
            let mut clients = Vec::new();
            for client in crate::adapters::all() {
                if let Some(view) = crate::clients::view(state, client.kind()).await {
                    clients.push(json!({ "provider": view.provider, "installed": view.installed, "version": view.version }));
                }
            }
            Ok(json!(clients))
        }
        _ => Err(ApiError::bad(
            "what must be workspaces, sessions, endpoints or clients",
        )),
    }
}
