//! Registering and listing workspaces.

use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateWorkspace {
    name: String,
    root_path: String,
}

pub(super) async fn list_workspaces(State(state): State<AppState>) -> Result<Json<Vec<Workspace>>> {
    Ok(Json(db(&state, |s| s.list_workspaces()).await?))
}
pub(super) async fn get_workspace(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
) -> Result<Json<Workspace>> {
    Ok(Json(workspace(&state, id).await?))
}
pub(super) async fn create_workspace(
    State(state): State<AppState>,
    Json(input): Json<CreateWorkspace>,
) -> Result<(StatusCode, Json<Workspace>)> {
    let name = bounded_name(&input.name)?;
    let root_path = PathBuf::from(input.root_path);
    if !root_path.is_absolute() {
        return Err(ApiError::bad("Use an absolute host directory path"));
    }
    let canonical = crate::paths::canonicalize_async(root_path)
        .await
        .map_err(|_| ApiError::bad("Host directory does not exist"))?;
    if !canonical.is_dir() {
        return Err(ApiError::bad("Root must be a directory"));
    }
    // Creating a workspace is what decides which part of the host this server
    // can read and write, so it is bounded like browsing is. Workspaces that
    // already exist are untouched: this is a rule about new ones.
    if !directories::within_roots(&state.workspace_roots, &canonical) {
        return Err(ApiError::bad(
            "That directory is outside the roots this server may use. Set TENDLY_WORKSPACE_ROOTS to allow it.",
        ));
    }
    let path = canonical.to_string_lossy().into_owned();
    Ok((
        StatusCode::CREATED,
        Json(db(&state, move |s| s.create_workspace(&name, &path)).await?),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UpdateWorkspace {
    /// How new worktrees are prepared; `null` prepares nothing.
    worktree_setup: Option<tendly_domain::WorktreeSetup>,
}

/// Change a workspace's settings: for now, how a worktree Tendly makes for
/// one of its sessions is prepared.
pub(super) async fn update_workspace(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    Json(input): Json<UpdateWorkspace>,
) -> Result<Json<Workspace>> {
    let setup = input
        .worktree_setup
        .map(crate::worktree_setup::validate)
        .transpose()?
        .flatten();
    if !db(&state, move |s| s.set_worktree_setup(id, setup.as_ref())).await? {
        return Err(ApiError::missing("Workspace"));
    }
    Ok(Json(workspace(&state, id).await?))
}

/// The routes this module serves.
pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/workspaces",
            get(list_workspaces).post(create_workspace),
        )
        .route(
            "/api/workspaces/{id}",
            get(get_workspace).patch(update_workspace),
        )
}
