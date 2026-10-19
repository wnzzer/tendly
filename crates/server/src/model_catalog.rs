//! Read-only model discovery. Explicit UI action only; never runs inference.
use crate::{ApiError, providers};
use serde::Serialize;
use serde_json::Value;
use std::{collections::BTreeSet, time::Duration};
use tendly_domain::EndpointProfile;
use url::Url;
const MAX_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct ModelEntry {
    pub id: String,
    pub name: String,
    /// Reasoning levels this model advertised. Empty means the catalog said
    /// nothing, so no effort choice is offered rather than a guessed one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub efforts: Vec<String>,
    /// The model this configuration resolves to when none is named. Reported so
    /// a session that has not started can offer that model's levels rather than
    /// guessing from the first row, or offering none at all.
    #[serde(rename = "isDefault", skip_serializing_if = "std::ops::Not::not")]
    pub is_default: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct ModelCatalog {
    pub models: Vec<ModelEntry>,
    pub source_url: String,
    pub has_more: bool,
}

pub fn list_url(profile: &EndpointProfile) -> Result<Url, ApiError> {
    let client = crate::adapters::adapter(profile.provider)
        .ok_or_else(|| ApiError::bad("Terminal has no model list"))?;
    let mut url = Url::parse(
        profile
            .endpoint_url
            .as_deref()
            .unwrap_or(client.default_endpoint()),
    )
    .map_err(|_| ApiError::bad("Invalid endpoint URL"))?;
    if matches!(
        url.host_str(),
        Some("169.254.169.254" | "metadata.google.internal")
    ) {
        return Err(ApiError::bad(
            "Cloud metadata addresses cannot be used as model endpoints",
        ));
    }
    let path = url.path().trim_end_matches('/');
    let suffix = if path.is_empty() || (client.versioned_api(profile) && !path.ends_with("/v1")) {
        "/v1/models"
    } else {
        "/models"
    };
    url.set_path(&format!("{path}{suffix}"));
    Ok(url)
}
pub(crate) fn parse_models(value: Value, source_url: String) -> Result<ModelCatalog, ApiError> {
    let rows = value
        .get("data")
        .or_else(|| value.get("models"))
        .and_then(Value::as_array)
        .or_else(|| value.as_array())
        .ok_or_else(|| {
            ApiError::bad(
                "The endpoint did not return a supported model list; enter a model ID manually",
            )
        })?;
    let mut seen = BTreeSet::new();
    let mut models = Vec::new();
    for row in rows.iter().take(2000) {
        let Some(id) = row
            .get("id")
            .or_else(|| row.get("model"))
            .or_else(|| row.get("name"))
            .and_then(Value::as_str)
            // Google names a model `models/<id>`; the ID is what runs it.
            .map(|id| id.strip_prefix("models/").unwrap_or(id))
            .filter(|s| !s.is_empty() && s.len() <= 200 && !s.chars().any(char::is_control))
        else {
            continue;
        };
        if !seen.insert(id.to_owned()) {
            continue;
        }
        let name = row
            .get("display_name")
            .or_else(|| row.get("displayName"))
            .and_then(Value::as_str)
            .filter(|s| s.len() <= 300 && !s.chars().any(char::is_control))
            .unwrap_or(id);
        // Codex reports supported levels per model; anything outside the set
        // both clients accept is dropped rather than offered.
        let efforts = row
            .get("supportedReasoningEfforts")
            .and_then(Value::as_array)
            .map(|levels| {
                levels
                    .iter()
                    .filter_map(|level| {
                        level
                            .get("reasoningEffort")
                            .or(Some(level))
                            .and_then(Value::as_str)
                    })
                    .filter(|level| providers::EFFORT_LEVELS.contains(level))
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        models.push(ModelEntry {
            id: id.into(),
            name: name.into(),
            efforts,
            // Codex marks the row its configuration resolves to; a catalog that
            // marks none leaves the field false rather than electing one.
            is_default: row
                .get("isDefault")
                .or_else(|| row.get("is_default"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
    }
    if models.is_empty() {
        return Err(ApiError::bad(
            "No model IDs were returned; enter a model ID manually",
        ));
    }
    models.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(ModelCatalog {
        models,
        source_url,
        has_more: rows.len() > 2000
            || value
                .get("has_more")
                .and_then(Value::as_bool)
                .unwrap_or(false),
    })
}
/// A value read from somewhere slow, kept: a fresh one is returned as it is,
/// a stale one as it is while one refresh runs in the background, and only
/// the first read for a key waits. A failed refresh keeps what was there.
pub struct Revalidating<T> {
    entries: std::sync::Mutex<std::collections::HashMap<String, (tokio::time::Instant, T)>>,
    refreshing: std::sync::Mutex<std::collections::HashSet<String>>,
    fresh: Duration,
    /// An entry not read for this long is forgotten (an edited profile's).
    keep: Duration,
}

impl<T: Clone + Send + 'static> Revalidating<T> {
    pub fn new(fresh: Duration, keep: Duration) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            entries: Default::default(),
            refreshing: Default::default(),
            fresh,
            keep,
        })
    }
    pub async fn get<E, F, Fut>(self: &std::sync::Arc<Self>, key: String, fetch: F) -> Result<T, E>
    where
        E: Send + 'static,
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<T, E>> + Send + 'static,
    {
        let cached = self
            .entries
            .lock()
            .expect("revalidating cache")
            .get(&key)
            .cloned();
        if let Some((at, value)) = cached {
            if at.elapsed() >= self.fresh
                && self
                    .refreshing
                    .lock()
                    .expect("revalidating cache")
                    .insert(key.clone())
            {
                let cache = self.clone();
                tokio::spawn(async move {
                    if let Ok(value) = fetch().await {
                        cache.store(key.clone(), value);
                    }
                    cache
                        .refreshing
                        .lock()
                        .expect("revalidating cache")
                        .remove(&key);
                });
            }
            return Ok(value);
        }
        let value = fetch().await?;
        self.store(key, value.clone());
        Ok(value)
    }
    fn store(&self, key: String, value: T) {
        let mut entries = self.entries.lock().expect("revalidating cache");
        entries.retain(|_, (at, _)| at.elapsed() < self.keep);
        entries.insert(key, (tokio::time::Instant::now(), value));
    }
}

/// How long a profile's model list counts as current. Older, it is still
/// shown at once while it is asked again in the background: for an official
/// account the answer comes from starting the client itself, seconds of work,
/// for a list that changes when the CLI is upgraded or the profile is edited.
const CATALOG_FRESH: Duration = Duration::from_secs(300);

fn catalogs() -> &'static std::sync::Arc<Revalidating<ModelCatalog>> {
    static CATALOGS: std::sync::OnceLock<std::sync::Arc<Revalidating<ModelCatalog>>> =
        std::sync::OnceLock::new();
    CATALOGS.get_or_init(|| Revalidating::new(CATALOG_FRESH, Duration::from_secs(24 * 3600)))
}

/// `discover`, through the cache: the key is the whole profile, so editing it
/// asks again at once; a list once read is never waited for again.
pub async fn discover_cached(
    state: &crate::AppState,
    profile: &EndpointProfile,
) -> Result<ModelCatalog, ApiError> {
    let key = serde_json::to_string(profile).unwrap_or_default();
    let (state, profile) = (state.clone(), profile.clone());
    catalogs()
        .get(key, move || async move { discover(&state, &profile).await })
        .await
}

/// Reads every profile's model list in the background as the server starts,
/// one at a time, so even the first view of a session does not wait for one.
pub fn warm(state: crate::AppState) {
    tokio::spawn(async move {
        let Ok(profiles) = crate::db(&state, |s| s.list_endpoint_profiles()).await else {
            return;
        };
        for profile in profiles {
            let _ = discover_cached(&state, &profile).await;
        }
    });
}

pub async fn discover(
    state: &crate::AppState,
    profile: &EndpointProfile,
) -> Result<ModelCatalog, ApiError> {
    let state_dir = state.state_dir.as_path();
    providers::validate_profile(profile)?;
    // An official account is asked directly: its client is the authority on
    // which models that account can run, third-party mappings included.
    if let Some(reference) = &profile.native_config {
        return native_models(state, profile, reference).await;
    }
    let client = crate::adapters::agent(profile.provider)?;
    if client.lists_models_unconfigured()
        && profile.endpoint_url.is_none()
        && profile.secret_ref.is_none()
    {
        return unconfigured_models(state_dir, client, profile).await;
    }
    let secret = profile
        .secret_ref
        .as_deref()
        .map(|reference| crate::secrets::resolve_reference(state_dir, reference))
        .transpose()?;
    fetch(profile, secret.as_deref()).await
}

/// The models a client lists for its default account, asked in a scratch
/// home with no credential of the host's: what it offers anyone.
async fn unconfigured_models(
    state_dir: &std::path::Path,
    client: &'static dyn crate::adapters::ClientAdapter,
    profile: &EndpointProfile,
) -> Result<ModelCatalog, ApiError> {
    let directory = providers::private_dir(
        &std::env::temp_dir().join(format!("tendly-models-{}", uuid::Uuid::new_v4())),
    )?;
    let mut command =
        tokio::process::Command::new(crate::clients::program(state_dir, &profile.provider));
    command
        .args(client.catalog_args())
        .args(client.unconfigured_catalog_args())
        .current_dir(&directory);
    let prefixes: Vec<&str> = crate::adapters::all()
        .flat_map(|client| client.account_prefixes())
        .copied()
        .collect();
    for (key, _) in std::env::vars().filter(|(k, _)| {
        prefixes.iter().any(|prefix| k.starts_with(prefix)) || crate::bridge::is_tendly_secret(k)
    }) {
        command.env_remove(key);
    }
    command.env(client.config_key(), &directory);
    if let Some(proxy) = &profile.proxy_url {
        for key in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            command.env(key, proxy);
        }
        command.env("NO_PROXY", "").env("no_proxy", "");
    }
    let result = ask_client(client, command, Duration::from_secs(8)).await;
    // This directory was created solely for read-only discovery; never remove real session state.
    let _ = tokio::fs::remove_dir_all(&directory).await;
    result
}

/// Run the client's read-only model-list handshake: its opening line, then
/// each reply handed to its adapter until one completes the list. No prompt
/// is ever sent, so no turn runs against the account.
async fn ask_client(
    client: &'static dyn crate::adapters::ClientAdapter,
    mut command: tokio::process::Command,
    timeout: Duration,
) -> Result<ModelCatalog, ApiError> {
    use crate::adapters::CatalogStep;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    command
        .kill_on_drop(true)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let label = client.kind().label();
    let mut child = command.spawn().map_err(|_| {
        ApiError::bad(format!(
            "{label} is not installed on the server; enter a model ID manually"
        ))
    })?;
    let mut input = child
        .stdin
        .take()
        .ok_or_else(|| ApiError::bad("Unable to open the client input"))?;
    let output = child
        .stdout
        .take()
        .ok_or_else(|| ApiError::bad("Unable to open the client output"))?;
    let query = async {
        input
            .write_all(client.catalog_opening().as_bytes())
            .await
            .map_err(ApiError::internal)?;
        let mut lines = BufReader::new(output).lines();
        let mut bytes = 0usize;
        let mut stage = 0u8;
        while let Some(line) = lines.next_line().await.map_err(ApiError::internal)? {
            bytes += line.len();
            if bytes > MAX_BYTES {
                return Err(ApiError::bad("Native model list exceeds limit"));
            }
            let Ok(packet) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            match client.catalog_step(&packet, &mut stage) {
                CatalogStep::Wait => {}
                CatalogStep::Send(reply) => input
                    .write_all(reply.as_bytes())
                    .await
                    .map_err(ApiError::internal)?,
                CatalogStep::Done(result) => return result,
            }
        }
        Err(ApiError::bad("The client ended without listing its models"))
    };
    let result = match tokio::time::timeout(timeout, query).await {
        Ok(result) => result,
        Err(_) => Err(ApiError::bad("Native model discovery timed out")),
    };
    let _ = child.kill().await;
    let _ = child.wait().await;
    result
}
async fn fetch(profile: &EndpointProfile, secret: Option<&str>) -> Result<ModelCatalog, ApiError> {
    let url = list_url(profile)?;
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none());
    if let Some(proxy) = profile.proxy_url.as_deref() {
        builder = builder
            .no_proxy()
            .proxy(reqwest::Proxy::all(proxy).map_err(|_| ApiError::bad("Invalid HTTP(S) proxy"))?);
    }
    let client = builder
        .build()
        .map_err(|_| ApiError::bad("Unable to configure the model-list connection"))?;
    let request = crate::adapters::agent(profile.provider)?.authorize(
        profile,
        client.get(url.clone()).header("Accept", "application/json"),
        secret,
    );
    // Never return upstream error bodies: they may include echoed credentials.
    let mut response = request.send().await.map_err(|_| {
        ApiError::bad("Cannot connect to the endpoint. Check the URL, proxy and server network.")
    })?;
    let status = response.status();
    if status.is_redirection() {
        return Err(ApiError::bad(
            "Model endpoint redirects are blocked to protect credentials. Enter the final base URL.",
        ));
    }
    if !status.is_success() {
        return Err(ApiError::bad(format!(
            "Model list request failed (HTTP {}). Check API-key access; subscription login cannot be reused by this API request.",
            status.as_u16()
        )));
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_BYTES as u64)
    {
        return Err(ApiError::bad("Model list response exceeds 2 MiB"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ApiError::bad("Model list download failed"))?
    {
        if bytes.len() + chunk.len() > MAX_BYTES {
            return Err(ApiError::bad("Model list response exceeds 2 MiB"));
        }
        bytes.extend_from_slice(&chunk);
    }
    let value = serde_json::from_slice(&bytes).map_err(|_| {
        ApiError::bad("Model endpoint returned non-JSON content; enter a model ID manually")
    })?;
    parse_models(value, url.to_string())
}

/// Models an official account can actually run, asked of that client itself.
///
/// Both clients already own this answer, including any third-party mapping the
/// operator configured — Claude Code through its `ANTHROPIC_DEFAULT_*_MODEL`
/// variables, Codex through `model_providers`. Reading their list keeps those
/// mappings visible here instead of competing with a second table.
async fn native_models(
    state: &crate::AppState,
    profile: &EndpointProfile,
    reference: &tendly_domain::NativeConfigReference,
) -> Result<ModelCatalog, ApiError> {
    let directory = providers::private_dir(
        &std::env::temp_dir().join(format!("tendly-models-{}", uuid::Uuid::new_v4())),
    )?;
    let spec = crate::native_config::build(state, &profile.provider, reference, directory.clone())?;
    let client = crate::adapters::agent(profile.provider)?;
    let mut command = tokio::process::Command::new(&spec.program);
    command.args(client.catalog_args()).current_dir(&directory);
    for key in &spec.env_remove {
        command.env_remove(key);
    }
    for (key, value) in &spec.env {
        command.env(key, value);
    }
    let result = ask_client(client, command, Duration::from_secs(12)).await;
    // This directory existed only for a read-only handshake; real account state
    // lives elsewhere and is never touched here.
    let _ = tokio::fs::remove_dir_all(&directory).await;
    result
}

/// Claude publishes `{value, displayName, description, supportedEffortLevels}`,
/// including entries an operator remapped or added through its own environment
/// variables. A model that states no effort levels genuinely supports none.
pub(crate) fn parse_claude_models(value: Value) -> Result<ModelCatalog, ApiError> {
    let rows = value
        .as_array()
        .ok_or_else(|| ApiError::bad("The client did not return a supported model list"))?;
    let mut seen = BTreeSet::new();
    let mut models = Vec::new();
    for row in rows.iter().take(200) {
        let Some(id) = row
            .get("value")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty() && s.len() <= 200 && !s.chars().any(char::is_control))
        else {
            continue;
        };
        if !seen.insert(id.to_owned()) {
            continue;
        }
        let name = row
            .get("displayName")
            .and_then(Value::as_str)
            .filter(|s| s.len() <= 300 && !s.chars().any(char::is_control))
            .unwrap_or(id);
        let efforts = if row.get("supportsEffort").and_then(Value::as_bool) == Some(true) {
            row.get("supportedEffortLevels")
                .and_then(Value::as_array)
                .map(|levels| {
                    levels
                        .iter()
                        .filter_map(Value::as_str)
                        .filter(|level| providers::EFFORT_LEVELS.contains(level))
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        models.push(ModelEntry {
            id: id.into(),
            name: name.into(),
            efforts,
            // Codex marks the row its configuration resolves to; a catalog that
            // marks none leaves the field false rather than electing one.
            is_default: row
                .get("isDefault")
                .or_else(|| row.get("is_default"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
    }
    if models.is_empty() {
        return Err(ApiError::bad("The client returned no usable model IDs"));
    }
    Ok(ModelCatalog {
        models,
        source_url: "claude://initialize".into(),
        has_more: false,
    })
}

#[cfg(test)]
mod revalidating_tests {
    use super::Revalidating;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;

    fn counted(
        calls: &Arc<AtomicUsize>,
        value: &'static str,
    ) -> impl FnOnce() -> std::future::Ready<Result<String, ()>> + Send + 'static {
        let calls = calls.clone();
        move || {
            calls.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Ok(value.to_owned()))
        }
    }

    #[tokio::test(start_paused = true)]
    async fn only_the_first_read_waits_and_a_stale_one_is_answered_at_once() {
        let cache = Revalidating::new(Duration::from_secs(300), Duration::from_secs(3600));
        let calls = Arc::new(AtomicUsize::new(0));
        assert_eq!(
            cache.get("p".into(), counted(&calls, "v1")).await,
            Ok("v1".into())
        );
        assert_eq!(
            cache.get("p".into(), counted(&calls, "v2")).await,
            Ok("v1".into())
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1, "fresh: not asked again");
        tokio::time::advance(Duration::from_secs(301)).await;
        // Stale: the old answer at once, and one refresh behind it.
        assert_eq!(
            cache.get("p".into(), counted(&calls, "v2")).await,
            Ok("v1".into())
        );
        assert_eq!(
            cache.get("p".into(), counted(&calls, "v3")).await,
            Ok("v1".into())
        );
        tokio::task::yield_now().await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "one refresh, however many asked"
        );
        assert_eq!(
            cache.get("p".into(), counted(&calls, "v4")).await,
            Ok("v2".into())
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_refresh_keeps_what_was_there() {
        let cache = Revalidating::new(Duration::from_secs(300), Duration::from_secs(3600));
        let calls = Arc::new(AtomicUsize::new(0));
        assert_eq!(
            cache.get("p".into(), counted(&calls, "v1")).await,
            Ok("v1".into())
        );
        tokio::time::advance(Duration::from_secs(301)).await;
        let failing = || std::future::ready(Err::<String, ()>(()));
        assert_eq!(cache.get("p".into(), failing).await, Ok("v1".into()));
        tokio::task::yield_now().await;
        assert_eq!(
            cache.get("p".into(), counted(&calls, "v2")).await,
            Ok("v1".into())
        );
        // A first read that fails is the caller's to see.
        assert_eq!(cache.get("q".into(), failing).await, Err(()));
    }
}

#[cfg(test)]
mod tests {
    // A session that has not started has no model of its own. Losing this flag
    // left the composer with nothing to offer a Codex user before their first
    // message, while Claude showed a control from a hardcoded list of levels.
    #[test]
    fn the_catalog_reports_which_model_the_configuration_resolves_to() {
        let catalog = super::parse_models(
            serde_json::json!({"data":[
                {"id":"gpt-5.5","supportedReasoningEfforts":["low","high"]},
                {"id":"gpt-6-astra","isDefault":true,"supportedReasoningEfforts":["low","max"]}
            ]}),
            "codex://model/list".into(),
        )
        .expect("catalog");
        let default: Vec<_> = catalog.models.iter().filter(|m| m.is_default).collect();
        assert_eq!(default.len(), 1);
        assert_eq!(default[0].id, "gpt-6-astra");
        assert_eq!(default[0].efforts, ["low", "max"]);
        // A catalog that elects none must leave every row false rather than
        // promoting the first, which would point at another model's levels.
        let none = super::parse_models(
            serde_json::json!({"data":[{"id":"a"},{"id":"b"}]}),
            "x".into(),
        )
        .expect("catalog");
        assert!(none.models.iter().all(|m| !m.is_default));
    }

    use super::*;
    use tendly_domain::ProviderKind;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    fn profile(url: String) -> EndpointProfile {
        EndpointProfile {
            id: uuid::Uuid::new_v4(),
            name: "fixture".into(),
            provider: ProviderKind::Codex,
            endpoint_url: Some(url),
            model: None,
            permission_mode: "native".into(),
            secret_ref: None,
            proxy_url: None,
            effort: None,
            model_aliases: Default::default(),
            models: Vec::new(),
            api: None,
            native_config: None,
            environment: Default::default(),
            created_at: chrono::Utc::now(),
        }
    }
    #[tokio::test]
    async fn returns_real_ids_and_uses_auth_without_inference() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0; 8192];
            let n = socket.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).to_lowercase();
            assert!(request.starts_with("get /v1/models "));
            assert!(request.contains("authorization: bearer fixture-only"));
            let body = r#"{"data":[{"id":"actual-model-b"},{"id":"actual-model-a","display_name":"Actual A"},{"id":"actual-model-a"}]}"#;
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).as_bytes()).await.unwrap();
        });
        let result = fetch(
            &profile(format!("http://{address}/v1")),
            Some("fixture-only"),
        )
        .await
        .unwrap();
        assert_eq!(result.models.len(), 2);
        assert_eq!(result.models[0].id, "actual-model-a");
        server.await.unwrap();
    }
    #[test]
    fn url_and_alias_rules() {
        let mut p = profile("https://example.test".into());
        assert_eq!(list_url(&p).unwrap().path(), "/v1/models");
        p.endpoint_url = Some("https://example.test/prefix/v1".into());
        assert_eq!(list_url(&p).unwrap().path(), "/prefix/v1/models");
        p.provider = ProviderKind::ClaudeCode;
        p.endpoint_url = Some("https://example.test/gateway".into());
        assert_eq!(list_url(&p).unwrap().path(), "/gateway/v1/models");
        p.model = Some("fast".into());
        p.model_aliases
            .insert("fast".into(), "real-model-id".into());
        assert_eq!(
            providers::resolve_model(&p).as_deref(),
            Some("real-model-id")
        );
        p.proxy_url = Some("socks5://127.0.0.1:1080".into());
        assert!(providers::validate_profile(&p).is_err());
    }

    #[test]
    fn keeps_only_advertised_reasoning_levels() {
        let catalog = parse_models(
            serde_json::json!({"data":[
                {"id":"reasoning-model","supportedReasoningEfforts":[{"reasoningEffort":"low"},{"reasoningEffort":"high"},{"reasoningEffort":"unsupported"},"medium"]}
            ]}),
            "fixture://models".into(),
        ).unwrap();
        assert_eq!(catalog.models[0].efforts, ["low", "high", "medium"]);
    }
    #[tokio::test]
    async fn rejects_redirect_and_does_not_forward_secret() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut b = [0; 4096];
            let read = s.read(&mut b).await.unwrap();
            assert!(read > 0);
            s.write_all(b"HTTP/1.1 302 Found\r\nLocation: http://169.254.169.254/secret\r\nContent-Length: 0\r\n\r\n").await.unwrap();
        });
        let error = fetch(&profile(format!("http://{addr}/v1")), Some("fixture-only"))
            .await
            .unwrap_err();
        assert!(error.message.contains("redirect"));
        server.await.unwrap();
    }
    #[tokio::test]
    async fn selected_http_proxy_routes_the_model_request() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0; 8192];
            let n = socket.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).to_lowercase();
            assert!(request.starts_with("get http://models.invalid/v1/models "));
            let body = r#"{"data":[{"id":"proxy-model"}]}"#;
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let mut p = profile("http://models.invalid/v1".into());
        p.proxy_url = Some(format!("http://{address}"));
        assert_eq!(fetch(&p, None).await.unwrap().models[0].id, "proxy-model");
        server.await.unwrap();
    }
}
