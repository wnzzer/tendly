//! What every route shares: the application state, the API error type, and
//! the small helpers that reach the store or resolve a workspace.

use super::*;

#[derive(Clone)]
pub(super) struct AppState {
    pub(super) store: Arc<Store>,
    pub(super) runtime: RuntimeManager,
    pub(super) state_dir: PathBuf,
    pub(super) browse_roots: Vec<PathBuf>,
    /// Files and folders the person opened to the read-only preview, until restart.
    pub(super) host_grants: host_grants::HostGrants,
    pub(super) workspace_roots: Vec<PathBuf>,
    pub(super) native_sources: Vec<native_history::NativeSource>,
    pub(super) native_bridge: PathBuf,
    pub(super) chat_bridge: PathBuf,
    /// Which layer conversations run on, for every client at once; `None`
    /// leaves it to each client's own setting (`chat_adapter::in_process`).
    pub(super) chat_in_process: Option<bool>,
    pub(super) chats: conversations::ChatManager,
    pub(super) watches: file_watch::FileWatches,
    pub(super) accounts: accounts::AccountManager,
    pub(super) security: security::Security,
    /// Publishing to a tailnet (`tailscale.rs`).
    pub(super) tailnet: tailscale::Tailnet,
    pub(super) claude_manual_mode: bool,
    pub(super) operations: Arc<tokio::sync::Mutex<()>>,
    pub(super) agents: agent::AgentRegistry,
    pub(super) activity: activity::Activity,
    /// Resource history; absent when its database could not be opened.
    pub(super) metrics: Option<Arc<tendly_persistence::metrics::MetricsStore>>,
}

#[derive(Debug)]
pub(super) struct ApiError {
    pub(super) status: StatusCode,
    pub(super) message: String,
}
impl ApiError {
    pub(super) fn bad(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }
    pub(super) fn missing(name: &str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: format!("{name} not found"),
        }
    }
    pub(super) fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }
    pub(super) fn internal(error: impl std::fmt::Display) -> Self {
        tracing::error!(%error,"operation failed");
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "Operation failed; see server logs".into(),
        }
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({"error":self.message}))).into_response()
    }
}
impl From<workspace_io::IoError> for ApiError {
    fn from(error: workspace_io::IoError) -> Self {
        Self {
            status: StatusCode::from_u16(error.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            message: error.message,
        }
    }
}
pub(super) type Result<T> = std::result::Result<T, ApiError>;

pub(super) async fn db<T: Send + 'static>(
    state: &AppState,
    operation: impl FnOnce(&Store) -> rusqlite_result::Result<T> + Send + 'static,
) -> Result<T> {
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || operation(&store))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)
}
// Keep rusqlite out of HTTP details while permitting typed persistence closures.
mod rusqlite_result {
    pub type Result<T> = std::result::Result<T, tendly_persistence::DbError>;
}
pub(super) async fn workspace(state: &AppState, id: WorkspaceId) -> Result<Workspace> {
    db(state, move |s| s.get_workspace(id))
        .await?
        .ok_or_else(|| ApiError::missing("Workspace"))
}
pub(super) async fn session_record(state: &AppState, id: SessionId) -> Result<Session> {
    db(state, move |s| s.get_session(id))
        .await?
        .ok_or_else(|| ApiError::missing("Session"))
}
pub(super) async fn root(state: &AppState, id: WorkspaceId) -> Result<PathBuf> {
    let w = workspace(state, id).await?;
    let path = crate::paths::canonicalize_async(w.root_path)
        .await
        .map_err(|_| ApiError::conflict("Workspace directory is unavailable"))?;
    if !path.is_dir() {
        return Err(ApiError::conflict("Workspace root is not a directory"));
    }
    Ok(path)
}
/// The checkout of a workspace a request reads or changes. `?checkout=`
/// names a worktree of its repository, as a session's `checkout_path` does,
/// and the views of a session working in one send it; without it, the
/// workspace directory.
#[derive(Debug, Default, Deserialize)]
pub(super) struct CheckoutParam {
    checkout: Option<String>,
}
impl<S: Send + Sync> axum::extract::FromRequestParts<S> for CheckoutParam {
    type Rejection = ApiError;
    async fn from_request_parts(parts: &mut axum::http::request::Parts, _: &S) -> Result<Self> {
        Query::<CheckoutParam>::try_from_uri(&parts.uri)
            .map(|Query(found)| found)
            .map_err(|_| ApiError::bad("Invalid query"))
    }
}
/// The directory a request works in: the workspace's own, or the worktree it
/// names, which must be one its repository lists and inside the roots this
/// server may use -- the same bounds a session's checkout has.
pub(super) async fn checkout_root(
    state: &AppState,
    id: WorkspaceId,
    checkout: &CheckoutParam,
) -> Result<PathBuf> {
    let main = root(state, id).await?;
    let Some(named) = checkout.checkout.as_deref().filter(|path| !path.is_empty()) else {
        return Ok(main);
    };
    let wanted = crate::paths::canonicalize_async(named)
        .await
        .map_err(|_| ApiError::conflict("Worktree directory is unavailable"))?;
    if wanted == main {
        return Ok(main);
    }
    let listing = workspace_io::git_text(&main, &["worktree", "list", "--porcelain"])
        .await?
        .unwrap_or_default();
    let listed = workspace_io::parse_worktrees(&listing)
        .into_iter()
        .any(|tree| !tree.main && dunce::canonicalize(&tree.path).is_ok_and(|path| path == wanted));
    if !listed || !directories::within_roots(&state.workspace_roots, &wanted) {
        return Err(ApiError::bad(
            "Not a worktree of this workspace's repository",
        ));
    }
    Ok(wanted)
}
pub(super) fn bounded_name(s: &str) -> Result<String> {
    let s = s.trim();
    if s.is_empty() || s.len() > 200 {
        Err(ApiError::bad("Name must be 1–200 characters"))
    } else {
        Ok(s.into())
    }
}
pub(super) fn optional(value: Option<String>) -> Option<String> {
    value.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_trimmed_and_bounded_and_blank_optionals_are_absent() {
        assert_eq!(bounded_name("  repo  ").unwrap(), "repo");
        assert!(bounded_name("   ").is_err());
        assert!(bounded_name(&"x".repeat(201)).is_err());
        assert_eq!(optional(Some("  x ".into())).as_deref(), Some("x"));
        assert_eq!(optional(Some("   ".into())), None);
        assert_eq!(optional(None), None);
    }
}
