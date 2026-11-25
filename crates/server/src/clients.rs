//! Native client discovery and optional npm installation.
//!
//! Tendly drives the user's installed Claude Code and Codex clients, so the
//! first question for a new host is simply whether they are there. When one is
//! missing Tendly can install it from npm into its own state directory. That
//! copy is a fallback, never an override: a client already on `PATH` keeps
//! winning, so installing here cannot silently change which binary a session
//! has been using.
use crate::{ApiError, AppState, Result};
use axum::{
    Json, Router,
    extract::{Path as RoutePath, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tendly_domain::ProviderKind;
use tendly_runtime::process::executable_in;

/// Where a resolved program came from. Surfaced so the page can say which copy
/// a session will actually run instead of only whether one exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramSource {
    /// An explicit `TENDLY_*_BIN` override on the server.
    Override,
    /// Found on the server's `PATH`.
    Path,
    /// Installed by Tendly into its state directory.
    Managed,
    /// Nothing found; the command name is kept so errors stay recognisable.
    Missing,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClientView {
    pub provider: ProviderKind,
    pub command: String,
    pub npm_package: String,
    pub program: String,
    pub source: ProgramSource,
    pub installed: bool,
    pub version: Option<String>,
    pub managed_path: Option<String>,
    /// npm has to exist on the server before an install can be offered at all.
    pub install_available: bool,
    /// A managed copy exists but `PATH` wins, so installing changed nothing for
    /// sessions. Saying so avoids an install that looks like a no-op.
    pub shadowed: bool,
}

pub fn npm_package(provider: &ProviderKind) -> Option<&'static str> {
    crate::adapters::adapter(*provider).map(|client| client.npm_package())
}
pub fn command(provider: &ProviderKind) -> Option<&'static str> {
    crate::adapters::adapter(*provider).map(|client| client.command())
}
fn override_key(provider: &ProviderKind) -> Option<&'static str> {
    crate::adapters::adapter(*provider).map(|client| client.bin_override())
}

pub fn managed_root(state_dir: &Path) -> PathBuf {
    state_dir.join("clients")
}
fn managed_program(state_dir: &Path, command: &str) -> Option<PathBuf> {
    executable_in(&managed_root(state_dir).join("node_modules/.bin"), command)
}

/// Resolve a command against the server's own `PATH`, the same way spawning it
/// by bare name would.
pub fn on_path(command: &str) -> Option<PathBuf> {
    on_search_path(command, env::var_os("PATH")?)
}
fn on_search_path(command: &str, paths: std::ffi::OsString) -> Option<PathBuf> {
    env::split_paths(&paths).find_map(|directory| executable_in(&directory, command))
}

/// The program a session will actually launch, and where it came from.
pub fn resolve(state_dir: &Path, provider: &ProviderKind) -> (String, ProgramSource) {
    resolve_in(state_dir, provider, |key| env::var_os(key))
}

/// `resolve`, reading the environment through `var` so tests can describe one
/// without changing the process's own -- which every other test running in
/// parallel shares, and which spawning `git` reads.
fn resolve_in(
    state_dir: &Path,
    provider: &ProviderKind,
    var: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> (String, ProgramSource) {
    let Some(command) = command(provider) else {
        return (String::new(), ProgramSource::Missing);
    };
    if let Some(value) = override_key(provider)
        .and_then(&var)
        .filter(|value| !value.is_empty())
    {
        return (
            value.to_string_lossy().into_owned(),
            ProgramSource::Override,
        );
    }
    // PATH before the managed copy: installing from this page must never
    // repoint sessions away from the client the host already had.
    if let Some(path) = var("PATH").and_then(|paths| on_search_path(command, paths)) {
        return (path.to_string_lossy().into_owned(), ProgramSource::Path);
    }
    if let Some(path) = managed_program(state_dir, command) {
        return (path.to_string_lossy().into_owned(), ProgramSource::Managed);
    }
    (command.into(), ProgramSource::Missing)
}

/// Program name for spawning. Callers that only need something to exec keep
/// using this; the bare command name stays the fallback so a missing client
/// still fails with its own recognisable error.
pub fn program(state_dir: &Path, provider: &ProviderKind) -> String {
    resolve(state_dir, provider).0
}

async fn version(program: &str) -> Option<String> {
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::process::Command::new(program)
            .arg("--version")
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().next()?.trim();
    (!line.is_empty()).then(|| line.chars().take(120).collect())
}

pub async fn view(state: &AppState, provider: ProviderKind) -> Option<ClientView> {
    let command = command(&provider)?;
    let package = npm_package(&provider)?;
    let (program, source) = resolve(&state.state_dir, &provider);
    let managed = managed_program(&state.state_dir, command);
    let version = if source == ProgramSource::Missing {
        None
    } else {
        version(&program).await
    };
    Some(ClientView {
        provider,
        command: command.into(),
        npm_package: package.into(),
        source,
        installed: source != ProgramSource::Missing && version.is_some(),
        version,
        shadowed: managed.is_some() && source != ProgramSource::Managed,
        managed_path: managed.map(|path| path.to_string_lossy().into_owned()),
        // A client not published on npm (Gemini's agy) installs its own way.
        install_available: !package.is_empty() && on_path("npm").is_some(),
        program,
    })
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/clients", get(list))
        .route("/api/clients/{provider}/install", post(install))
}

async fn list(State(state): State<AppState>) -> Result<Json<Vec<ClientView>>> {
    let mut views = Vec::new();
    for client in crate::adapters::all() {
        if let Some(view) = view(&state, client.kind()).await {
            views.push(view);
        }
    }
    Ok(Json(views))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Install {
    confirmed: bool,
}

async fn install(
    State(state): State<AppState>,
    RoutePath(provider): RoutePath<String>,
    Json(input): Json<Install>,
) -> Result<Json<ClientView>> {
    if !input.confirmed {
        return Err(ApiError::bad(
            "Installing a native client downloads and runs package code; explicit confirmation is required",
        ));
    }
    let provider = ProviderKind::parse_agent(&provider)
        .ok_or_else(|| ApiError::bad("Only an agent client can be installed"))?;
    let package = npm_package(&provider)
        .filter(|package| !package.is_empty())
        .ok_or_else(|| ApiError::bad("This client is not installed from npm"))?;
    let npm = on_path("npm").ok_or_else(|| {
        ApiError::bad("Installing a native client needs npm on the server's PATH")
    })?;
    // One install at a time: concurrent npm runs share a cache and a prefix.
    let _guard = state.operations.lock().await;
    let root = managed_root(&state.state_dir);
    fs::create_dir_all(&root).map_err(ApiError::internal)?;
    let output = tokio::time::timeout(
        Duration::from_secs(600),
        tokio::process::Command::new(npm)
            .args([
                "install",
                "--no-fund",
                "--no-audit",
                "--prefix",
                &root.to_string_lossy(),
                package,
            ])
            .current_dir(&root)
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .output(),
    )
    .await
    .map_err(|_| ApiError::bad("The client install did not finish in time"))?
    .map_err(|_| ApiError::bad("The client install could not be started"))?;
    if !output.status.success() {
        // npm's own diagnostic is the useful part; keep the tail, not a guess.
        let text = String::from_utf8_lossy(&output.stderr);
        let detail: String = text
            .trim()
            .chars()
            .rev()
            .take(600)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        return Err(ApiError::bad(if detail.is_empty() {
            "The client install failed. Check npm on the server.".to_owned()
        } else {
            format!("The client install failed: {detail}")
        }));
    }
    view(&state, provider)
        .await
        .map(Json)
        .ok_or_else(|| ApiError::internal("Installed client could not be inspected"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_copies_never_override_a_client_the_host_already_has() {
        let root = env::temp_dir().join(format!("tendly-clients-{}", uuid::Uuid::new_v4()));
        let bin = managed_root(&root).join("node_modules/.bin");
        fs::create_dir_all(&bin).unwrap();
        // npm installs a `.cmd` shim on Windows, where a bare name is not runnable.
        let executable = if cfg!(windows) { "codex.cmd" } else { "codex" };
        let managed = bin.join(executable);
        fs::write(&managed, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&managed, fs::Permissions::from_mode(0o755)).unwrap();
        }
        // Each case describes its own environment instead of changing the
        // process's: tests run in parallel, and clearing PATH here once made
        // other tests fail to find git.
        let env = |pairs: &'static [(&'static str, &'static str)], path: Option<PathBuf>| {
            move |key: &str| {
                if key == "PATH" {
                    return path.clone().map(|p| p.into_os_string());
                }
                pairs
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, v)| (*v).into())
            }
        };
        // Nothing on PATH: the managed copy is the fallback that gets used.
        assert_eq!(
            resolve_in(&root, &ProviderKind::Codex, env(&[], None)),
            (
                managed.to_string_lossy().into_owned(),
                ProgramSource::Managed
            )
        );
        // A host copy on PATH wins, so an install here cannot repoint sessions.
        let host = root.join("host-bin");
        fs::create_dir_all(&host).unwrap();
        let host_codex = host.join(executable);
        fs::write(&host_codex, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&host_codex, fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(
            resolve_in(&root, &ProviderKind::Codex, env(&[], Some(host.clone()))),
            (
                host_codex.to_string_lossy().into_owned(),
                ProgramSource::Path
            )
        );
        // An explicit server override still beats both.
        assert_eq!(
            resolve_in(
                &root,
                &ProviderKind::Codex,
                env(
                    &[("TENDLY_CODEX_BIN", "/explicit/codex")],
                    Some(host.clone())
                )
            ),
            ("/explicit/codex".to_owned(), ProgramSource::Override)
        );
        // A missing client keeps its bare command name so its own error shows.
        assert_eq!(
            resolve_in(&root, &ProviderKind::ClaudeCode, env(&[], None)),
            ("claude".to_owned(), ProgramSource::Missing)
        );
        assert!(npm_package(&ProviderKind::Terminal).is_none());
        let _ = fs::remove_dir_all(&root);
    }
}
