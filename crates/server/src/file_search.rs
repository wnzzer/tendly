//! Fuzzy file search over a workspace, served by the Codex app-server's index.
//!
//! The index is local and unauthenticated — verified against an empty
//! `CODEX_HOME`, which still returns results — so this runs against a config
//! directory holding no credentials and no history. It is therefore a property
//! of the workspace, available to any session, rather than a Codex feature.
//!
//! When Codex is absent the endpoint reports that plainly and the client keeps
//! filtering the entries it has already loaded, which is what it did before.
use crate::{ApiError, AppState, CheckoutParam, Result, checkout_root};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};
use tendly_domain::WorkspaceId;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Above the app-server's own deadline, so its degraded answer is preferred to
/// this one: it can say "truncated", a killed process can only say "failed".
const BRIDGE_TIMEOUT: Duration = Duration::from_secs(9);
const MAX_QUERY: usize = 256;

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct SearchResults {
    pub files: Vec<SearchHit>,
    pub truncated: bool,
    /// `false` when Codex is not installed, so the client can fall back without
    /// presenting an empty result as "no matches".
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timed_out: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchHit {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub score: i64,
    /// Character positions the index matched, for highlighting the same
    /// characters that produced the ranking.
    pub indices: Vec<u32>,
}

#[derive(Deserialize)]
struct BridgeOutput {
    #[serde(default)]
    files: Vec<SearchHit>,
    #[serde(default)]
    truncated: bool,
    #[serde(default)]
    timed_out: bool,
    #[serde(default)]
    error: Option<String>,
}

/// A credential-free home for the search process, kept out of the user's own.
fn search_home(state: &AppState) -> PathBuf {
    state.state_dir.join("file-search-home")
}

pub async fn search(
    state: &AppState,
    workspace: WorkspaceId,
    checkout: &CheckoutParam,
    query: &str,
    limit: u32,
) -> Result<SearchResults> {
    let query = query.trim();
    if query.is_empty() || query.len() > MAX_QUERY {
        return Ok(SearchResults {
            available: true,
            ..Default::default()
        });
    }
    let cwd = checkout_root(state, workspace, checkout).await?;
    let home = search_home(state);
    tokio::fs::create_dir_all(&home).await.map_err(|error| {
        ApiError::internal(std::io::Error::other(format!(
            "cannot prepare the search home: {error}"
        )))
    })?;

    let script = crate::bridge::script(&state.state_dir, "TENDLY_SEARCH_BRIDGE", "file-search.mjs");
    let job = serde_json::json!({
        "cwd": cwd,
        "query": query,
        "limit": limit.clamp(1, 50),
        "config_dir": home,
    });

    let mut command = crate::bridge::node_command(&[], &script);
    let Ok(mut child) = command.spawn() else {
        return Ok(SearchResults::default());
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(job.to_string().as_bytes()).await;
    }
    let Some(mut stdout) = child.stdout.take() else {
        return Ok(SearchResults::default());
    };

    let mut raw = String::new();
    let read = tokio::time::timeout(BRIDGE_TIMEOUT, stdout.read_to_string(&mut raw)).await;
    if read.is_err() || read.is_ok_and(|inner| inner.is_err()) {
        return Ok(SearchResults {
            available: true,
            truncated: true,
            timed_out: Some(true),
            ..Default::default()
        });
    }

    let Ok(output) = serde_json::from_str::<BridgeOutput>(raw.trim()) else {
        return Ok(SearchResults::default());
    };
    if output.error.is_some() {
        // An error here means Codex could not answer, not that the workspace has
        // no matching files. Reporting it as unavailable keeps the client's own
        // filter in play instead of showing a confident empty list.
        return Ok(SearchResults::default());
    }
    Ok(SearchResults {
        files: output.files,
        truncated: output.truncated,
        available: true,
        timed_out: output.timed_out.then_some(true),
    })
}
