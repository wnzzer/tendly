//! The agent tools that change Tendly: endpoint profiles, new-session
//! defaults, workspaces.
//!
//! Each change is sorted by what it would cost the person if it were wrong.
//! Anything touching an endpoint -- where requests go, which key they carry,
//! how freely tools run -- is asked in the browser first (activity.rs), and a
//! key is typed there, never passed through the agent. Defaults and a new
//! workspace are made at once and reported, with an undo where there is one.
//! Every change goes through the same checks the web app's own routes use.
use crate::{
    ApiError, AppState,
    activity::{Answer, KeyField, Undo},
    agent::{Caller, endpoint_view, text},
    db,
};
use serde_json::{Map, Value, json};
use tendly_domain::{EndpointProfile, ProviderKind};
use uuid::Uuid;

fn object(properties: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": properties, "required": required, "additionalProperties": false })
}

pub fn tools() -> Vec<Value> {
    vec![
        crate::agent::tool(
            "tendly_endpoint",
            "Create, change, delete, test or import Tendly endpoint profiles (where a Claude Code or Codex session sends requests, with which key and model). Create, update, delete and import are confirmed by the person in the Tendly window, which is also where they type any API key: never ask for a key in the conversation. test lists the models an existing profile can reach.",
            object(
                json!({
                    "action": { "type": "string", "enum": ["create", "update", "delete", "test", "host_configurations", "import_host"] },
                    "id": { "type": "string", "description": "Profile id, for update, delete and test." },
                    "name": { "type": "string" },
                    "provider": { "type": "string", "enum": ProviderKind::AGENTS.map(|kind| kind.as_str()), "description": "For create." },
                    "endpoint_url": { "type": ["string", "null"], "description": "API base URL; empty or null for the official endpoint." },
                    "model": { "type": ["string", "null"] },
                    "effort": { "type": ["string", "null"], "description": "Default reasoning effort, e.g. low, medium, high." },
                    "proxy_url": { "type": ["string", "null"] },
                    "permission_mode": { "type": "string", "enum": ["native", "interactive", "trusted", "plan", "blocked"] },
                    "key": { "type": "string", "enum": ["ask", "none", "keep", "replace"], "description": "create: ask (default when endpoint_url is set) has the person enter a key; none for an endpoint without one. update: keep (default) or replace." },
                    "source_id": { "type": "string", "description": "For import_host: one of the ids host_configurations returns." }
                }),
                &["action"],
            ),
        ),
        crate::agent::tool(
            "tendly_preferences",
            "Read or change Tendly's settings: what new sessions start with, per client (default endpoint profile, reasoning effort, permission mode), and how the interface and its terminals look. Session defaults apply to sessions started afterwards; the appearance changes every open window at once. Changes are shown to the person with an undo; making tools run without asking is confirmed first.",
            object(
                json!({
                    "action": { "type": "string", "enum": ["get", "set"] },
                    "provider": { "type": "string", "enum": ProviderKind::AGENTS.map(|kind| kind.as_str()) },
                    "endpoint_id": { "type": ["string", "null"], "description": "Profile id, or null for the last used." },
                    "effort": { "type": ["string", "null"] },
                    "permission": { "type": ["string", "null"], "enum": ["ask", "plan", "accept_edits", "danger", null] },
                    "appearance": { "type": "string", "enum": ["system", "light", "dark"], "description": "The interface theme; system follows the operating system. Needs no provider." },
                    "terminal_appearance": { "type": "string", "enum": ["interface", "light", "dark"], "description": "The terminals' theme; interface follows the interface theme. Needs no provider." }
                }),
                &["action"],
            ),
        ),
        crate::agent::tool(
            "tendly_workspace",
            "Add a directory on this machine as a Tendly workspace, so sessions can be opened in it. It must be inside the roots the server allows.",
            object(
                json!({
                    "action": { "type": "string", "enum": ["add"] },
                    "path": { "type": "string", "description": "Absolute directory path." },
                    "name": { "type": "string", "description": "Defaults to the directory's name." }
                }),
                &["action", "path"],
            ),
        ),
    ]
}

/// Who the person is told is asking.
pub(crate) async fn who(state: &AppState, caller: Caller) -> Value {
    match caller {
        Caller::External => json!({ "kind": "external" }),
        Caller::Peer(id) => state.peers.who(id),
        Caller::Session(id) => {
            let session = db(state, move |s| s.get_session(id)).await.ok().flatten();
            json!({
                "kind": "session",
                "session_id": id,
                "title": session.as_ref().map(|s| s.title.clone()),
                "provider": session.as_ref().map(|s| s.provider),
            })
        }
    }
}

/// A field the call set: `Some(None)` to clear, `None` when not mentioned.
fn field(arguments: &Value, key: &str) -> Result<Option<Option<String>>, ApiError> {
    match arguments.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(value)) => Ok(Some(crate::optional(Some(value.clone())))),
        Some(_) => Err(ApiError::bad(format!("{key} must be text or null"))),
    }
}

fn provider_of(arguments: &Value) -> Result<ProviderKind, ApiError> {
    text(arguments, "provider")
        .and_then(ProviderKind::parse_agent)
        .ok_or_else(|| ApiError::bad(format!("provider must be {}", ProviderKind::agent_names())))
}

fn provider_label(provider: &ProviderKind) -> &'static str {
    provider.label()
}

const TRUSTED_NOTE: &str = "Trusted: file edits will not ask first.";

fn describe(profile: &EndpointProfile) -> Vec<(&'static str, String)> {
    let mut rows = vec![
        ("Client", provider_label(&profile.provider).to_owned()),
        (
            "Endpoint",
            profile.endpoint_url.clone().unwrap_or_else(|| "—".into()),
        ),
    ];
    if let Some(model) = &profile.model {
        rows.push(("Model", model.clone()));
    }
    if let Some(effort) = &profile.effort {
        rows.push(("Effort", effort.clone()));
    }
    if let Some(proxy) = &profile.proxy_url {
        rows.push(("Proxy", proxy.clone()));
    }
    rows.push(("Permission", profile.permission_mode.clone()));
    rows
}

/// A fresh key name for a profile, taken by no stored key, no environment
/// variable and no other profile's reference.
async fn fresh_secret_name(state: &AppState, label: &str) -> Result<String, ApiError> {
    let profiles = db(state, |s| s.list_endpoint_profiles()).await?;
    let mut taken: std::collections::HashSet<String> = crate::secrets::list(&state.state_dir)
        .into_iter()
        .map(|entry| entry.name)
        .collect();
    taken.extend(profiles.iter().filter_map(|p| {
        p.secret_ref
            .as_deref()
            .and_then(|r| r.strip_prefix("env:"))
            .map(str::to_owned)
    }));
    let mut slug = String::new();
    for c in label.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_uppercase());
        } else if !slug.is_empty() && !slug.ends_with('_') {
            slug.push('_');
        }
    }
    let slug = slug.trim_end_matches('_');
    let base = format!(
        "TENDLY_SECRET_{}",
        if slug.is_empty() { "KEY" } else { slug }
    );
    if !taken.contains(&base) {
        return Ok(base);
    }
    Ok((2..)
        .map(|n| format!("{base}_{n}"))
        .find(|name| !taken.contains(name))
        .expect("a free name"))
}

async fn profile(state: &AppState, arguments: &Value) -> Result<EndpointProfile, ApiError> {
    let id: Uuid = text(arguments, "id")
        .ok_or_else(|| ApiError::bad("id is required"))?
        .parse()
        .map_err(|_| ApiError::bad("id is not a profile id"))?;
    db(state, move |s| s.get_endpoint_profile(id))
        .await?
        .ok_or_else(|| ApiError::missing("Profile"))
}

fn managed(state: &AppState, profile: &EndpointProfile) -> bool {
    profile
        .native_config
        .as_ref()
        .is_some_and(|reference| reference.source_id.starts_with("account:"))
        || crate::accounts::owns_profile(state, profile.id)
}

/// The `endpoint` tool: create, update, test and delete endpoint profiles, and
/// list or import the host's own configurations. A create, update, delete or
/// import is confirmed by the person in the browser first, and any key is
/// typed there; the tool never handles one itself.
pub async fn endpoint(
    state: &AppState,
    caller: Caller,
    arguments: &Value,
) -> Result<Value, ApiError> {
    let who = who(state, caller).await;
    match text(arguments, "action") {
        Some("create") => {
            let provider = provider_of(arguments)?;
            let flat = |key| field(arguments, key).map(Option::flatten);
            let mut draft = EndpointProfile {
                id: Uuid::new_v4(),
                name: crate::bounded_name(text(arguments, "name").unwrap_or(""))?,
                provider,
                endpoint_url: flat("endpoint_url")?,
                model: flat("model")?,
                permission_mode: text(arguments, "permission_mode")
                    .unwrap_or("native")
                    .to_owned(),
                secret_ref: None,
                proxy_url: flat("proxy_url")?,
                effort: flat("effort")?,
                model_aliases: Default::default(),
                models: Vec::new(),
                api: None,
                key_header: None,
                native_config: None,
                environment: Default::default(),
                created_at: chrono::Utc::now(),
            };
            crate::providers::validate_profile(&draft)?;
            let wants_key = match text(arguments, "key") {
                Some("none") => false,
                Some("ask") => true,
                None => draft.endpoint_url.is_some(),
                Some(_) => return Err(ApiError::bad("key must be ask or none when creating")),
            };
            let notes = if draft.permission_mode == "trusted" {
                vec![TRUSTED_NOTE]
            } else {
                vec![]
            };
            let answer = state
                .activity
                .ask(
                    who.clone(),
                    "Create endpoint profile “{name}”",
                    json!({ "name": draft.name }),
                    describe(&draft),
                    notes,
                    wants_key.then(|| KeyField {
                        label: "API key".into(),
                        optional: false,
                    }),
                )
                .await?;
            let Answer::Approved { secret } = answer else {
                return Err(ApiError::conflict(
                    "The person declined. Nothing was changed.",
                ));
            };
            let mut saved_as = None;
            if let Some(secret) = secret {
                let name = fresh_secret_name(state, &draft.name).await?;
                crate::secrets::store(&state.state_dir, &name, &secret)?;
                draft.secret_ref = Some(format!("env:{name}"));
                saved_as = Some(name);
            }
            let save = draft.clone();
            if let Err(error) = db(state, move |s| s.create_endpoint_profile(&save)).await {
                if let Some(name) = &saved_as {
                    let _ = crate::secrets::remove(&state.state_dir, name);
                }
                return Err(error);
            }
            state.activity.notify(
                who,
                "Created endpoint profile “{name}”",
                json!({ "name": draft.name }),
                Some(Undo::DeleteProfile {
                    id: draft.id,
                    secret: saved_as,
                }),
            );
            Ok(json!({ "created": endpoint_view(&state.state_dir, &draft) }))
        }
        Some("update") => {
            let before = profile(state, arguments).await?;
            if before.native_config.is_some() {
                return Err(ApiError::bad(
                    "This profile points at a host configuration; its endpoint, key and model belong to the native client and cannot be edited here",
                ));
            }
            let mut after = before.clone();
            let mut changes = Vec::new();
            if let Some(name) = text(arguments, "name") {
                after.name = crate::bounded_name(name)?;
            }
            for key in ["endpoint_url", "model", "effort", "proxy_url"] {
                if let Some(value) = field(arguments, key)? {
                    match key {
                        "endpoint_url" => after.endpoint_url = value,
                        "model" => after.model = value,
                        "effort" => after.effort = value,
                        _ => after.proxy_url = value,
                    }
                }
            }
            if let Some(mode) = text(arguments, "permission_mode") {
                after.permission_mode = mode.to_owned();
            }
            crate::providers::validate_profile(&after)?;
            let pairs: [(&str, Option<&str>, Option<&str>); 6] = [
                (
                    "Name",
                    Some(before.name.as_str()),
                    Some(after.name.as_str()),
                ),
                (
                    "Endpoint",
                    before.endpoint_url.as_deref(),
                    after.endpoint_url.as_deref(),
                ),
                ("Model", before.model.as_deref(), after.model.as_deref()),
                ("Effort", before.effort.as_deref(), after.effort.as_deref()),
                (
                    "Proxy",
                    before.proxy_url.as_deref(),
                    after.proxy_url.as_deref(),
                ),
                (
                    "Permission",
                    Some(before.permission_mode.as_str()),
                    Some(after.permission_mode.as_str()),
                ),
            ];
            for (label, old, new) in pairs {
                if old != new {
                    changes.push((
                        label,
                        format!("{} → {}", old.unwrap_or("—"), new.unwrap_or("—")),
                    ));
                }
            }
            let replace_key = match text(arguments, "key") {
                None | Some("keep") => false,
                Some("replace") => true,
                Some(_) => return Err(ApiError::bad("key must be keep or replace when updating")),
            };
            if replace_key {
                changes.push(("API key", "↻".into()));
            }
            if changes.is_empty() {
                return Ok(json!({ "unchanged": endpoint_view(&state.state_dir, &before) }));
            }
            let notes = if after.permission_mode == "trusted" && before.permission_mode != "trusted"
            {
                vec![TRUSTED_NOTE]
            } else {
                vec![]
            };
            let answer = state
                .activity
                .ask(
                    who.clone(),
                    "Change endpoint profile “{name}”",
                    json!({ "name": before.name }),
                    changes,
                    notes,
                    replace_key.then(|| KeyField {
                        label: "New API key".into(),
                        optional: false,
                    }),
                )
                .await?;
            let Answer::Approved { secret } = answer else {
                return Err(ApiError::conflict(
                    "The person declined. Nothing was changed.",
                ));
            };
            if let Some(secret) = secret {
                // Always a new name: the old key stays for undo, and for any
                // other profile that happens to share it.
                let name = fresh_secret_name(state, &after.name).await?;
                crate::secrets::store(&state.state_dir, &name, &secret)?;
                after.secret_ref = Some(format!("env:{name}"));
            }
            let save = after.clone();
            db(state, move |s| s.update_endpoint_profile(&save)).await?;
            state.activity.notify(
                who,
                "Changed endpoint profile “{name}”",
                json!({ "name": after.name }),
                Some(Undo::RestoreProfile(Box::new(before))),
            );
            Ok(json!({ "updated": endpoint_view(&state.state_dir, &after) }))
        }
        Some("delete") => {
            let target = profile(state, arguments).await?;
            if managed(state, &target) {
                return Err(ApiError::conflict(
                    "This profile belongs to an official account; it is managed from Official accounts",
                ));
            }
            let answer = state
                .activity
                .ask(
                    who.clone(),
                    "Delete endpoint profile “{name}”",
                    json!({ "name": target.name }),
                    describe(&target),
                    vec!["Existing sessions keep the settings they started with."],
                    None,
                )
                .await?;
            if !matches!(answer, Answer::Approved { .. }) {
                return Err(ApiError::conflict(
                    "The person declined. Nothing was changed.",
                ));
            }
            let id = target.id;
            db(state, move |s| s.delete_endpoint_profile(id)).await?;
            state.activity.notify(
                who,
                "Deleted endpoint profile “{name}”",
                json!({ "name": target.name }),
                Some(Undo::RestoreProfile(Box::new(target))),
            );
            Ok(json!({ "deleted": id }))
        }
        Some("test") => {
            let target = profile(state, arguments).await?;
            let catalog = crate::model_catalog::discover(state, &target).await?;
            Ok(json!({
                "reachable": true,
                "models": catalog.models.iter().take(50).map(|model| &model.id).collect::<Vec<_>>(),
                "count": catalog.models.len(),
                "more_available": catalog.has_more,
            }))
        }
        Some("host_configurations") => {
            let config_state = state.clone();
            let sources = tokio::task::spawn_blocking(move || {
                crate::native_config::source_views(&config_state)
            })
            .await
            .map_err(ApiError::internal)?;
            Ok(json!(sources))
        }
        Some("import_host") => {
            let source_id = text(arguments, "source_id")
                .ok_or_else(|| ApiError::bad("source_id is required; see host_configurations"))?
                .to_owned();
            let config_state = state.clone();
            let lookup = source_id.clone();
            let (provider, reference) = tokio::task::spawn_blocking(move || {
                crate::native_config::pin(&config_state, &lookup)
            })
            .await
            .map_err(ApiError::internal)??;
            let host = format!("Host {} configuration", provider.label());
            let name = crate::bounded_name(text(arguments, "name").unwrap_or(&host))?;
            let answer = state
                .activity
                .ask(
                    who.clone(),
                    "Use this machine's {client} configuration",
                    json!({ "client": provider_label(&provider) }),
                    vec![("Directory", reference.config_dir.clone())],
                    vec!["Sessions on this profile share its sign-in and settings; nothing is copied."],
                    None,
                )
                .await?;
            if !matches!(answer, Answer::Approved { .. }) {
                return Err(ApiError::conflict(
                    "The person declined. Nothing was changed.",
                ));
            }
            let created = db(state, move |s| {
                s.import_native_profile(&name, provider, reference)
            })
            .await?;
            state.activity.notify(
                who,
                "Added endpoint profile “{name}”",
                json!({ "name": created.name }),
                Some(Undo::DeleteProfile {
                    id: created.id,
                    secret: None,
                }),
            );
            Ok(json!({ "created": endpoint_view(&state.state_dir, &created) }))
        }
        _ => Err(ApiError::bad(
            "action must be create, update, delete, test, host_configurations or import_host",
        )),
    }
}

fn preference_view(
    preferences: &crate::preferences::Preferences,
    profiles: &[EndpointProfile],
) -> Value {
    let one = |p: &crate::preferences::ProviderPreference| {
        json!({
            "endpoint": p.endpoint_profile_id.map(|id| json!({ "id": id, "name": profiles.iter().find(|profile| profile.id == id).map(|profile| profile.name.clone()) })),
            "effort": p.effort,
            "permission": p.permission,
        })
    };
    let mut view: Map<String, Value> = preferences
        .clients
        .iter()
        .map(|(kind, preference)| (kind.as_str().to_owned(), one(preference)))
        .collect();
    view.insert(
        "appearance".to_owned(),
        json!(preferences.appearance.as_deref().unwrap_or("system")),
    );
    view.insert(
        "terminal_appearance".to_owned(),
        json!(
            preferences
                .terminal_appearance
                .as_deref()
                .unwrap_or("interface")
        ),
    );
    Value::Object(view)
}

/// An appearance argument: absent leaves it, the following value (`system`,
/// `interface`) or null clears it, `light` and `dark` pin it.
fn appearance_field(
    arguments: &Value,
    key: &str,
    follow: &str,
) -> Result<Option<Option<String>>, ApiError> {
    Ok(field(arguments, key)?.map(|value| value.filter(|value| value != follow)))
}

pub async fn preferences(
    state: &AppState,
    caller: Caller,
    arguments: &Value,
) -> Result<Value, ApiError> {
    let current = crate::preferences::load(state).await?;
    let profiles = db(state, |s| s.list_endpoint_profiles()).await?;
    match text(arguments, "action") {
        Some("get") => Ok(preference_view(&current, &profiles)),
        Some("set") => {
            let mut next = current.clone();
            if let Some(value) = appearance_field(arguments, "appearance", "system")? {
                next.appearance = value;
            }
            if let Some(value) = appearance_field(arguments, "terminal_appearance", "interface")? {
                next.terminal_appearance = value;
            }
            let client_fields = ["provider", "endpoint_id", "effort", "permission"]
                .iter()
                .any(|key| arguments.get(key).is_some());
            if !client_fields {
                if next == current {
                    return Ok(json!({ "unchanged": preference_view(&current, &profiles) }));
                }
                crate::preferences::save(state, &next).await?;
                let previous = serde_json::to_value(&current).map_err(ApiError::internal)?;
                state.activity.notify(
                    who(state, caller).await,
                    "Changed the appearance",
                    json!({}),
                    Some(Undo::Preferences(previous)),
                );
                return Ok(json!({ "saved": preference_view(&next, &profiles) }));
            }
            let provider = provider_of(arguments)?;
            let entry = next.clients.entry(provider).or_default();
            if let Some(value) = field(arguments, "endpoint_id")? {
                entry.endpoint_profile_id = value
                    .map(|id| {
                        id.parse()
                            .map_err(|_| ApiError::bad("endpoint_id is not a profile id"))
                    })
                    .transpose()?;
            }
            if let Some(value) = field(arguments, "effort")? {
                entry.effort = value;
            }
            if let Some(value) = field(arguments, "permission")? {
                entry.permission = value;
            }
            if next == current {
                return Ok(json!({ "unchanged": preference_view(&current, &profiles) }));
            }
            let who = who(state, caller).await;
            let before = crate::preferences::for_provider_owned(&current, &provider);
            let now = crate::preferences::for_provider_owned(&next, &provider);
            if now.permission.as_deref() == Some("danger")
                && before.permission.as_deref() != Some("danger")
            {
                let answer = state
                    .activity
                    .ask(
                        who.clone(),
                        "New {client} sessions run tools without asking",
                        json!({ "client": provider_label(&provider) }),
                        vec![],
                        vec!["Every tool call in new sessions will run without your confirmation."],
                        None,
                    )
                    .await?;
                if !matches!(answer, Answer::Approved { .. }) {
                    return Err(ApiError::conflict(
                        "The person declined. Nothing was changed.",
                    ));
                }
            }
            crate::preferences::save(state, &next).await?;
            let previous = serde_json::to_value(&current).map_err(ApiError::internal)?;
            state.activity.notify(
                who,
                "Changed the defaults for new {client} sessions",
                json!({ "client": provider_label(&provider) }),
                Some(Undo::Preferences(previous)),
            );
            Ok(json!({ "saved": preference_view(&next, &profiles) }))
        }
        _ => Err(ApiError::bad("action must be get or set")),
    }
}

pub async fn workspace(
    state: &AppState,
    caller: Caller,
    arguments: &Value,
) -> Result<Value, ApiError> {
    if text(arguments, "action") != Some("add") {
        return Err(ApiError::bad("action must be add"));
    }
    let path = text(arguments, "path").ok_or_else(|| ApiError::bad("path is required"))?;
    let name = text(arguments, "name")
        .map(str::to_owned)
        .unwrap_or_else(|| {
            std::path::Path::new(path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_owned())
        });
    let input: crate::CreateWorkspace =
        serde_json::from_value(json!({ "name": name, "root_path": path }))
            .map_err(ApiError::internal)?;
    let (_, axum::Json(created)) =
        crate::create_workspace(axum::extract::State(state.clone()), axum::Json(input)).await?;
    let who = who(state, caller).await;
    state.activity.notify(
        who,
        "Added workspace “{name}”",
        json!({ "name": created.name }),
        None,
    );
    let mut view = Map::new();
    view.insert("id".into(), json!(created.id));
    view.insert("name".into(), json!(created.name));
    view.insert("root_path".into(), json!(created.root_path));
    Ok(json!({ "added": view }))
}
