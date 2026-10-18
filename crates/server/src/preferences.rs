//! What a new session starts with, per provider: which endpoint profile, how
//! deeply it thinks, and which permission mode it runs in; and how the
//! interface and its terminals look.
//!
//! Stored by the server rather than the browser, so a phone and a laptop open
//! sessions the same way, and an agent can change them like any other setting. Each value is a default, not a rule: the new-session
//! dialog still shows and can change the first two, and the permission chip in
//! a running session changes the third.
use crate::{ApiError, AppState, Result, db, providers};
use axum::{Json, Router, extract::State, routing::get};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tendly_domain::{ProviderKind, Session};
use uuid::Uuid;

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProviderPreference {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint_profile_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Preferences {
    /// Each agent client's defaults, under its name. Any other name fails to
    /// parse, as an unknown field did when the clients were fields.
    #[serde(flatten)]
    pub clients: BTreeMap<ProviderKind, ProviderPreference>,
    /// Whether sessions get Tendly's own tools (agent.rs). On unless
    /// turned off: `None` is the default, not a choice to disable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_tools: Option<bool>,
    /// Official account IDs whose endpoint quota badge the user has hidden.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_account_quotas: Vec<Uuid>,
    /// Whether the host's resources are read at all (resources.rs): the
    /// recorded history, the status bar figures and the system page. On
    /// unless turned off, like the agent tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_monitoring: Option<bool>,
    /// `light` or `dark`; none follows the operating system.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appearance: Option<String>,
    /// `light` or `dark`; none follows the interface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_appearance: Option<String>,
}

impl Default for Preferences {
    fn default() -> Self {
        Preferences {
            clients: BTreeMap::new(),
            agent_tools: None,
            hidden_account_quotas: Vec::new(),
            resource_monitoring: None,
            appearance: None,
            terminal_appearance: None,
        }
        .complete()
    }
}

impl Preferences {
    /// Every agent client listed, an empty entry for one without defaults, so
    /// a reader sees each client whether or not it was ever set.
    pub fn complete(mut self) -> Self {
        for kind in ProviderKind::AGENTS {
            self.clients.entry(kind).or_default();
        }
        self
    }

    pub fn resource_monitoring(&self) -> bool {
        self.resource_monitoring != Some(false)
    }

    pub fn agent_tools(&self) -> bool {
        self.agent_tools != Some(false)
    }
}

/// A client's defaults, empty for one that has none.
pub fn for_provider_owned(
    preferences: &Preferences,
    provider: &ProviderKind,
) -> ProviderPreference {
    preferences
        .for_provider(provider)
        .cloned()
        .unwrap_or_default()
}

impl Preferences {
    pub fn for_provider(&self, provider: &ProviderKind) -> Option<&ProviderPreference> {
        self.clients.get(provider).filter(|_| provider.is_agent())
    }
}

/// The modes each client offers; Codex has no plan or accept-edits mode.
fn permission_modes(provider: &ProviderKind) -> &'static [&'static str] {
    crate::adapters::adapter(*provider).map_or(&[], |client| client.permission_modes())
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/preferences", get(read).put(write))
}

/// For a launch already on a blocking thread (providers::build).
pub fn load_blocking(state: &AppState) -> Preferences {
    state
        .store
        .preferences()
        .ok()
        .and_then(|stored| serde_json::from_value::<Preferences>(stored).ok())
        .map(Preferences::complete)
        .unwrap_or_default()
}

pub async fn load(state: &AppState) -> Result<Preferences> {
    let stored = db(state, |s| s.preferences()).await?;
    // A document that no longer parses -- written by a later version, say --
    // is treated as no preferences rather than failing every new session.
    Ok(serde_json::from_value::<Preferences>(stored)
        .map(Preferences::complete)
        .unwrap_or_default())
}

async fn read(State(state): State<AppState>) -> Result<Json<Preferences>> {
    Ok(Json(load(&state).await?))
}

async fn write(
    State(state): State<AppState>,
    Json(input): Json<Preferences>,
) -> Result<Json<Preferences>> {
    let input = input.complete();
    save(&state, &input).await?;
    Ok(Json(input))
}

/// Check and store a whole preferences document; the agent tools save
/// through here too, so both follow the same rules.
pub async fn save(state: &AppState, input: &Preferences) -> Result<()> {
    for scheme in [&input.appearance, &input.terminal_appearance] {
        if scheme
            .as_deref()
            .is_some_and(|value| value != "light" && value != "dark")
        {
            return Err(ApiError::bad(
                "An appearance is light, dark, or none to follow.",
            ));
        }
    }
    for (&provider, preference) in &input.clients {
        if !provider.is_agent() {
            return Err(ApiError::bad("Only an agent client has session defaults."));
        }
        providers::validate_effort(preference.effort.as_deref())?;
        if let Some(mode) = preference.permission.as_deref()
            && !permission_modes(&provider).contains(&mode)
        {
            return Err(ApiError::bad("Unsupported permission mode."));
        }
        if let Some(id) = preference.endpoint_profile_id {
            let profile = db(state, move |s| s.get_endpoint_profile(id))
                .await?
                .ok_or_else(|| ApiError::missing("Profile"))?;
            if profile.provider != provider {
                return Err(ApiError::bad("Profile and preference providers must match"));
            }
        }
    }
    let value = serde_json::to_value(input).map_err(ApiError::internal)?;
    db(state, move |s| s.set_preferences(&value)).await?;
    Ok(())
}

/// The permission mode a session's client should switch to when it starts.
/// The clients do not carry a mode across a restart -- a resumed Claude Code
/// comes back asking -- so the preference applies at every start, not only
/// the first; without one, the client's own default stands.
pub async fn initial_permission(state: &AppState, session: &Session) -> Result<Option<String>> {
    let preferences = load(state).await?;
    Ok(preferences
        .for_provider(&session.provider)
        .and_then(|p| p.permission.clone())
        .filter(|mode| permission_modes(&session.provider).contains(&mode.as_str())))
}
