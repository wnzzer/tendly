//! User-defined balance queries, independent of subscription usage and session snapshots.
mod agent;
mod runner;
#[cfg(test)]
mod tests;
use crate::{ApiError, AppState, db};
pub(crate) use agent::{call, tool};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tendly_domain::EndpointProfile;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Unit {
    Money { currency: String },
    Percentage {},
}
/// Where Tendly puts the endpoint key on each request. The script never sees
/// the key: the server fills it into this header or query parameter.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Auth {
    None {},
    /// `value` holds `{key}` once, e.g. `Authorization: Bearer {key}` or `X-Token: {key}`.
    Header {
        name: String,
        value: String,
    },
    /// `?name=<key>`, replacing any such parameter the script wrote.
    Query {
        name: String,
    },
}
pub(super) const KEY_PLACEHOLDER: &str = "{key}";
/// Headers that describe the connection rather than the account; neither the
/// configuration nor a script may set them.
pub(super) const RESERVED_HEADERS: [&str; 7] = [
    "host",
    "cookie",
    "proxy-authorization",
    "content-length",
    "transfer-encoding",
    "connection",
    "upgrade",
];
fn validate_auth(auth: &Auth) -> Result<(), &'static str> {
    match auth {
        Auth::None {} => Ok(()),
        Auth::Header { name, value } => {
            let token = |b: u8| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b);
            if name.is_empty() || name.len() > 64 || !name.bytes().all(token) {
                return Err("Use a valid HTTP header name for the balance key");
            }
            if RESERVED_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
                return Err("That header cannot carry the balance key");
            }
            if value.len() > 256
                || value.matches(KEY_PLACEHOLDER).count() != 1
                || value.chars().any(char::is_control)
            {
                return Err("The header value must contain {key} exactly once, on one line");
            }
            Ok(())
        }
        Auth::Query { name } => {
            if name.is_empty()
                || name.len() > 64
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
            {
                return Err("Use a query parameter name of letters, digits, _ . or -");
            }
            Ok(())
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub unit: Unit,
    pub query_url: String,
    pub auth: Auth,
    /// None reuses this endpoint's configured key. No key value enters JavaScript.
    #[serde(default)]
    pub secret_ref: Option<String>,
    /// An async function body; ctx.request() returns the response as JSON.
    pub script: String,
    /// Whether an open page queries it again every few minutes. None is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_refresh: Option<bool>,
}

impl Config {
    /// Everything that decides what a reading means, without the settings
    /// that only decide when to take one.
    fn query_part(&self) -> Config {
        Config {
            auto_refresh: None,
            ..self.clone()
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Reading {
    pub value: String,
    pub unit: Unit,
    pub checked_at: DateTime<Utc>,
}
#[derive(Clone, Default, Debug, Deserialize, Serialize)]
pub struct Document {
    pub config: Option<Config>,
    pub reading: Option<Reading>,
    pub attempted_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}
#[derive(Serialize)]
pub struct View {
    #[serde(flatten)]
    pub document: Document,
    pub stale: bool,
}
impl From<Document> for View {
    fn from(document: Document) -> Self {
        let stale = document.error.is_some()
            || document
                .reading
                .as_ref()
                .is_some_and(|r| (Utc::now() - r.checked_at).num_seconds() > 300);
        Self { document, stale }
    }
}

pub fn validate(config: &Config) -> Result<(), ApiError> {
    runner::allowed_url(&config.query_url).map_err(ApiError::bad)?;
    validate_auth(&config.auth).map_err(ApiError::bad)?;
    if let Unit::Money { currency } = &config.unit
        && (currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_uppercase()))
    {
        return Err(ApiError::bad(
            "Currency must be a three-letter uppercase code, such as USD or CNY",
        ));
    }
    if config.script.trim().is_empty() || config.script.len() > 32_768 {
        return Err(ApiError::bad("Balance script must contain 1–32768 bytes"));
    }
    if config.secret_ref.as_ref().is_some_and(|s| {
        !s.strip_prefix("env:")
            .is_some_and(crate::secrets::valid_name)
    }) {
        return Err(ApiError::bad(
            "Use a saved TENDLY_SECRET_ reference for the balance key",
        ));
    }
    Ok(())
}
async fn profile(state: &AppState, id: Uuid) -> Result<EndpointProfile, ApiError> {
    db(state, move |s| s.get_endpoint_profile(id))
        .await?
        .ok_or_else(|| ApiError::missing("Profile"))
}
async fn load(state: &AppState, id: Uuid) -> Result<(Document, Option<String>), ApiError> {
    let raw = db(state, move |s| s.endpoint_balance(id)).await?;
    let document = match &raw {
        Some(raw) => serde_json::from_str(raw).map_err(ApiError::internal)?,
        None => Document::default(),
    };
    Ok((document, raw))
}
async fn save_config(state: &AppState, id: Uuid, config: Option<Config>) -> Result<View, ApiError> {
    profile(state, id).await?;
    if let Some(config) = &config {
        validate(config)?;
    }
    let (old, _) = load(state, id).await?;
    if old.config == config {
        return Ok(old.into());
    }
    let same_query =
        old.config.as_ref().map(Config::query_part) == config.as_ref().map(Config::query_part);
    let document = if same_query {
        // Only when to ask changed: the reading still means the same thing.
        Document { config, ..old }
    } else {
        // A different script/unit starts afresh: never label an old USD value as CNY.
        Document {
            config,
            ..Default::default()
        }
    };
    let raw = serde_json::to_string(&document).map_err(ApiError::internal)?;
    db(state, move |s| s.set_endpoint_balance(id, &raw)).await?;
    Ok(document.into())
}
async fn execute(
    state: &AppState,
    profile: EndpointProfile,
    config: Config,
) -> Result<Reading, String> {
    validate(&config).map_err(|e| e.message)?;
    static RUNNING: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
    let permit = RUNNING
        .try_acquire()
        .map_err(|_| "Balance queries are busy; try again shortly".to_owned())?;
    let key = if config.auth == (Auth::None {}) {
        None
    } else {
        let reference = config
            .secret_ref
            .as_ref()
            .or(profile.secret_ref.as_ref())
            .ok_or("Configure a saved key or choose no authentication")?;
        Some(
            crate::secrets::resolve_reference(&state.state_dir, reference)
                .map_err(|_| "Balance key is unavailable".to_owned())?,
        )
    };
    let unit = config.unit.clone();
    let value = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        runner::run(config, key, profile.proxy_url)
    })
    .await
    .map_err(|_| "Balance runner failed".to_owned())??;
    Ok(Reading {
        value,
        unit,
        checked_at: Utc::now(),
    })
}
static QUERYING: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<Uuid>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashSet::new()));
struct QueryGuard(Uuid);
impl Drop for QueryGuard {
    fn drop(&mut self) {
        QUERYING.lock().unwrap().remove(&self.0);
    }
}
async fn query(state: &AppState, id: Uuid) -> Result<View, ApiError> {
    if !QUERYING.lock().unwrap().insert(id) {
        return Err(ApiError::conflict(
            "A balance query is already running for this endpoint",
        ));
    }
    let _guard = QueryGuard(id);
    let profile = profile(state, id).await?;
    let (mut document, before) = load(state, id).await?;
    let config = document
        .config
        .clone()
        .ok_or_else(|| ApiError::bad("Balance query is not configured"))?;
    // Repeated clicks and MCP calls share a short cooldown.
    if document
        .attempted_at
        .is_some_and(|at| (Utc::now() - at).num_seconds() < 15)
    {
        return Ok(document.into());
    }
    match execute(state, profile, config).await {
        Ok(reading) => {
            document.reading = Some(reading);
            document.error = None;
        }
        Err(error) => document.error = Some(error),
    }
    document.attempted_at = Some(Utc::now());
    let after = serde_json::to_string(&document).map_err(ApiError::internal)?;
    let before = before.unwrap_or_default();
    if !db(state, move |s| {
        s.update_endpoint_balance_result(id, &before, &after)
    })
    .await?
    {
        return Err(ApiError::conflict(
            "Balance configuration or result changed during this query; refresh to see it",
        ));
    }
    Ok(document.into())
}
async fn get_config(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<View>, ApiError> {
    profile(&state, id).await?;
    Ok(Json(load(&state, id).await?.0.into()))
}
async fn put_config(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(config): Json<Option<Config>>,
) -> Result<Json<View>, ApiError> {
    Ok(Json(save_config(&state, id, config).await?))
}
async fn refresh(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<View>, ApiError> {
    Ok(Json(query(&state, id).await?))
}
async fn trial(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(config): Json<Config>,
) -> Result<Json<Reading>, ApiError> {
    let profile = profile(&state, id).await?;
    execute(&state, profile, config)
        .await
        .map(Json)
        .map_err(ApiError::bad)
}
async fn list(State(state): State<AppState>) -> Result<Json<Vec<Value>>, ApiError> {
    let profiles = db(&state, |s| s.list_endpoint_profiles()).await?;
    let mut views = Vec::new();
    for profile in profiles {
        let (document, _) = load(&state, profile.id).await?;
        if document.config.is_some() {
            let view: View = document.into();
            let auto_refresh = view.document.config.as_ref().and_then(|c| c.auto_refresh);
            views.push(json!({"id":profile.id,"name":profile.name,"auto_refresh":auto_refresh,"reading":view.document.reading,
                "error":view.document.error,"attempted_at":view.document.attempted_at,"stale":view.stale}));
        }
    }
    Ok(Json(views))
}
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/endpoint-balances", get(list))
        .route(
            "/api/endpoint-profiles/{id}/balance",
            get(get_config).put(put_config),
        )
        .route("/api/endpoint-profiles/{id}/balance/query", post(refresh))
        .route("/api/endpoint-profiles/{id}/balance/test", post(trial))
}
