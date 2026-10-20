//! Reading, writing, creating, renaming and searching files in a workspace.

use super::*;

#[derive(Deserialize, Default)]
pub(super) struct PathQuery {
    pub(super) path: Option<String>,
    /// `png`: a preview of an image the browser cannot decode itself.
    #[serde(rename = "as")]
    pub(super) format: Option<String>,
    #[serde(default)]
    pub(super) staged: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileWrite {
    content: String,
    expected_version: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileCreate {
    path: String,
    /// Make an empty folder at `path` instead of an empty file.
    #[serde(default)]
    directory: bool,
}
/// Both ends, because a rename is a move: the tree offers it as renaming in
/// place, but the path is what is sent and a different folder is a valid one.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileRename {
    from: String,
    to: String,
}

pub(super) async fn search_files(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Query(q): Query<file_search::SearchQuery>,
) -> Result<Json<file_search::SearchResults>> {
    Ok(Json(
        file_search::search(
            &state,
            id,
            &checkout,
            q.q.as_deref().unwrap_or(""),
            q.limit.unwrap_or(20),
        )
        .await?,
    ))
}
pub(super) async fn list_files(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Query(q): Query<PathQuery>,
) -> Result<Json<Vec<workspace_io::FileEntry>>> {
    Ok(Json(
        workspace_io::list_files(
            &checkout_root(&state, id, &checkout).await?,
            q.path.as_deref().unwrap_or(""),
        )
        .await?,
    ))
}
pub(super) async fn read_file(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Query(q): Query<PathQuery>,
) -> Result<Json<workspace_io::TextFile>> {
    Ok(Json(
        workspace_io::read_file(
            &checkout_root(&state, id, &checkout).await?,
            q.path
                .as_deref()
                .ok_or_else(|| ApiError::bad("Path required"))?,
        )
        .await?,
    ))
}
pub(super) async fn write_file(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Query(q): Query<PathQuery>,
    Json(input): Json<FileWrite>,
) -> Result<Json<workspace_io::TextFile>> {
    let _guard = state.operations.lock().await;
    Ok(Json(
        workspace_io::write_file(
            &checkout_root(&state, id, &checkout).await?,
            q.path
                .as_deref()
                .ok_or_else(|| ApiError::bad("Path required"))?,
            &input.content,
            input.expected_version.as_deref(),
        )
        .await?,
    ))
}
/// Add an empty file to a workspace. Refuses a name already taken rather than
/// writing over what holds it.
pub(super) async fn create_file(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Json(input): Json<FileCreate>,
) -> Result<(StatusCode, Json<workspace_io::FileEntry>)> {
    let _guard = state.operations.lock().await;
    let root = checkout_root(&state, id, &checkout).await?;
    let entry = if input.directory {
        workspace_io::create_directory(&root, &input.path).await?
    } else {
        workspace_io::create_file(&root, &input.path).await?
    };
    Ok((StatusCode::CREATED, Json(entry)))
}
/// Rename a file or directory within one workspace. Nothing is overwritten and
/// nothing leaves the workspace, so this is reversible by renaming back.
pub(super) async fn rename_file(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Json(input): Json<FileRename>,
) -> Result<Json<workspace_io::FileEntry>> {
    let _guard = state.operations.lock().await;
    Ok(Json(
        workspace_io::rename_entry(
            &checkout_root(&state, id, &checkout).await?,
            &input.from,
            &input.to,
        )
        .await?,
    ))
}
/// Delete a file or directory a workspace owns. Destructive and not undoable,
/// so the caller states the path explicitly and the UI confirms it first.
pub(super) async fn delete_file(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Query(q): Query<PathQuery>,
) -> Result<StatusCode> {
    let _guard = state.operations.lock().await;
    workspace_io::delete_entry(
        &checkout_root(&state, id, &checkout).await?,
        q.path
            .as_deref()
            .ok_or_else(|| ApiError::bad("Path required"))?,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
pub(super) async fn read_asset(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Query(q): Query<PathQuery>,
    request: Request,
) -> Result<Response> {
    let root = checkout_root(&state, id, &checkout).await?;
    let target = workspace_io::asset_path(
        &root,
        q.path
            .as_deref()
            .ok_or_else(|| ApiError::bad("Path required"))?,
    )?;
    media::serve(target, q.format.as_deref() == Some("png"), request).await
}

/// The routes this module serves.
pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/workspaces/{id}/files", get(list_files))
        .route("/api/workspaces/{id}/files/search", get(search_files))
        .route(
            "/api/workspaces/{id}/file",
            get(read_file).put(write_file).delete(delete_file),
        )
        .route("/api/workspaces/{id}/file/create", post(create_file))
        .route("/api/workspaces/{id}/file/rename", post(rename_file))
        .route("/api/workspaces/{id}/asset", get(read_asset))
}
