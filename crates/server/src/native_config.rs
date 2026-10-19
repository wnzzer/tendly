//! References to existing native client homes. Never parses or copies credentials.
use crate::{ApiError, AppState};
use serde_json::{Value, json};
use std::{collections::BTreeMap, env, fs, path::PathBuf};
use tendly_domain::{NativeConfigReference, ProviderKind};
use tendly_runtime::SpawnSpec;

pub fn source_views(state: &AppState) -> Vec<crate::native_history::SourceView> {
    state
        .native_sources
        .iter()
        .map(|source| {
            let reference = pin(state, &source.id).ok().map(|(_, reference)| reference);
            crate::native_history::SourceView {
                id: source.id.clone(),
                provider: source.provider,
                label: source.label.clone(),
                available: reference.is_some(),
                config_env: reference
                    .as_ref()
                    .and_then(|reference| reference.config_env.clone()),
                path: reference.map_or_else(
                    || source.config_dir.to_string_lossy().into_owned(),
                    |reference| reference.config_dir,
                ),
            }
        })
        .collect()
}

pub fn pin(
    state: &AppState,
    source_id: &str,
) -> Result<(ProviderKind, NativeConfigReference), ApiError> {
    if source_id.starts_with("account:") {
        return crate::accounts::pin(state, source_id);
    }
    let source = state
        .native_sources
        .iter()
        .find(|source| source.id == source_id)
        .ok_or_else(|| ApiError::missing("Host native configuration source"))?;
    if matches!(source.provider, ProviderKind::Terminal) {
        return Err(ApiError::bad(
            "Terminal has no native account configuration",
        ));
    }
    if source
        .config_env
        .as_ref()
        .is_some_and(|value| !std::path::Path::new(value).is_absolute())
    {
        return Err(ApiError::bad(
            "A relative native configuration environment path cannot be reused across workspaces; configure an absolute native path first",
        ));
    }
    let config_dir = dunce::canonicalize(&source.config_dir)
        .map_err(|_| ApiError::bad("Native configuration directory is unavailable"))?;
    if !config_dir.is_dir() {
        return Err(ApiError::bad(
            "Native configuration source must be a directory",
        ));
    }
    let config_dir = config_dir
        .to_str()
        .ok_or_else(|| ApiError::bad("Native configuration path must be valid UTF-8"))?
        .to_owned();
    Ok((
        source.provider,
        NativeConfigReference {
            source_id: source.id.clone(),
            config_dir,
            config_env: source
                .config_env
                .as_ref()
                .map(|value| {
                    value.to_str().map(str::to_owned).ok_or_else(|| {
                        ApiError::bad("Native configuration path must be valid UTF-8")
                    })
                })
                .transpose()?,
        },
    ))
}

pub fn validate_reference(
    state: &AppState,
    provider: &ProviderKind,
    reference: &NativeConfigReference,
) -> Result<PathBuf, ApiError> {
    if reference.source_id.starts_with("account:") {
        return crate::accounts::validate_reference(state, provider, reference);
    }
    let (current_provider, current) = pin(state, &reference.source_id)?;
    if &current_provider != provider {
        return Err(ApiError::bad(
            "Native configuration and session providers must match",
        ));
    }
    if current != *reference {
        return Err(ApiError::conflict(
            "The native configuration source changed; import the intended source again before opening a new session",
        ));
    }
    Ok(PathBuf::from(current.config_dir))
}

pub fn build(
    state: &AppState,
    provider: &ProviderKind,
    reference: &NativeConfigReference,
    cwd: PathBuf,
) -> Result<SpawnSpec, ApiError> {
    if reference.source_id.starts_with("account:") {
        return crate::accounts::build(state, provider, reference, cwd);
    }
    validate_reference(state, provider, reference)?;
    let client = crate::adapters::adapter(*provider)
        .ok_or_else(|| ApiError::bad("Terminal has no native account configuration"))?;
    let program = crate::clients::program(&state.state_dir, provider);
    let (mut environment, remove) =
        launch_environment(client.config_key(), reference.config_env.as_deref());
    // New native session, not a history resume or login attempt.
    let (args, extras) = client.launch_extras(state, reference.config_env.as_deref());
    environment.extend(extras);
    Ok(SpawnSpec {
        program,
        args,
        cwd,
        env: environment,
        env_remove: remove,
    })
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
/// Whether the operator allowed Tendly to add its usage capture to the
/// status line of Claude sessions it launches. Off unless explicitly enabled.
pub fn usage_capture_enabled() -> bool {
    env::var("TENDLY_CLAUDE_USAGE_CAPTURE")
        .map(|value| matches!(value.trim(), "1" | "true" | "on"))
        .unwrap_or(false)
}
/// Effective Claude configuration directory for a launch, mirroring the client:
/// `CLAUDE_CONFIG_DIR` when set, otherwise the host default.
pub fn claude_config_home(config_env: Option<&str>) -> Option<PathBuf> {
    match config_env {
        Some(value) if !value.trim().is_empty() => Some(PathBuf::from(value)),
        _ => env::var_os("HOME")
            .or_else(|| env::var_os("USERPROFILE"))
            .map(|home| PathBuf::from(home).join(".claude")),
    }
}
/// Arguments and environment that let a Claude session record its own usage.
///
/// Claude Code hands its status line command a documented JSON payload that
/// carries subscription usage for the signed-in account. Capturing that payload
/// locally is what allows the account page to answer an explicit usage check
/// without reading credentials or calling a private endpoint.
///
/// The overlay is passed per launch through `--settings`, so no settings file
/// belonging to the user is modified. A status line the account already
/// configured is forwarded the same payload and its output is passed through,
/// so starting a session from Tendly does not change what the status line
/// looks like.
///
/// It is still an override of the account's own configuration, so it is opt-in:
/// without `TENDLY_CLAUDE_USAGE_CAPTURE=1` a session launches exactly as the
/// native client would launch it, and an explicit usage check simply reports
/// that nothing has been captured.
pub fn claude_statusline(
    state: &AppState,
    config_env: Option<&str>,
) -> (Vec<String>, BTreeMap<String, String>) {
    if !usage_capture_enabled() {
        return (Vec::new(), BTreeMap::new());
    }
    let script = crate::installation::native_bridge(&state.state_dir)
        .with_file_name("statusline-capture.mjs");
    if !script.is_file() {
        return (Vec::new(), BTreeMap::new());
    }
    let runtime = crate::bridge::js_runtime().to_string_lossy().into_owned();
    let command = format!("{} {}", quote(&runtime), quote(&script.to_string_lossy()));
    let overlay = json!({"statusLine": {"type": "command", "command": command}});
    let mut environment = BTreeMap::new();
    if let Some(existing) = claude_config_home(config_env)
        .map(|home| home.join("settings.json"))
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|settings| {
            settings
                .get("statusLine")
                .and_then(|line| line.get("command"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .filter(|command| !command.trim().is_empty() && !command.contains("statusline-capture.mjs"))
    {
        environment.insert("TENDLY_STATUSLINE_DELEGATE".into(), existing);
    }
    (vec!["--settings".into(), overlay.to_string()], environment)
}

pub fn launch_environment(
    key: &str,
    config_env: Option<&str>,
) -> (BTreeMap<String, String>, Vec<String>) {
    let mut environment = BTreeMap::new();
    if let Some(value) = config_env {
        environment.insert(key.into(), value.into());
    }
    // Preserve the host's native environment and config precedence. Tendly's
    // own secret references/access token never become native client overrides.
    // Shell aliases/functions and interactive shell startup files are not loaded.
    let mut remove: Vec<String> = crate::bridge::tendly_secrets()
        .chain(crate::bridge::host_client_session())
        .collect();
    if config_env.is_none() {
        // In particular, CLAUDE_CONFIG_DIR=<default path> can select a different
        // macOS Keychain entry than leaving the variable unset. Preserve absence.
        remove.push(key.into());
    }
    (environment, remove)
}
