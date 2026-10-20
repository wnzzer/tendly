//! Files and directories on the host outside any workspace, history imported
//! from a client's own records, and files uploaded into a workspace or session.

use super::*;

#[derive(Deserialize, Default)]
pub(super) struct DirectoryQuery {
    root: Option<String>,
    path: Option<String>,
}
/// A file an agent mentioned outside every workspace, read-only.
pub(super) async fn read_host_file(
    State(state): State<AppState>,
    Query(q): Query<PathQuery>,
) -> Result<Json<workspace_io::TextFile>> {
    let path = q
        .path
        .as_deref()
        .ok_or_else(|| ApiError::bad("Path required"))?;
    Ok(Json(
        workspace_io::read_host_file(
            &[state.browse_roots.clone(), state.host_grants.roots()].concat(),
            path,
        )
        .await?,
    ))
}
pub(super) async fn browse_directories(
    State(state): State<AppState>,
    Query(q): Query<DirectoryQuery>,
) -> Result<Json<directories::DirectoryListing>> {
    Ok(Json(
        directories::list(&state.browse_roots, q.root, q.path).await?,
    ))
}

pub(super) async fn native_sources(
    State(state): State<AppState>,
) -> Json<Vec<native_history::SourceView>> {
    Json(native_history::source_views(&state.native_sources))
}
#[derive(Deserialize)]
pub(super) struct HistoryQuery {
    source_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HistoryImport {
    #[serde(default)]
    environment: Option<tendly_domain::EnvironmentOverrides>,
    source_id: String,
    native_id: String,
    confirmed_original_config: bool,
}
pub(super) async fn list_native_history(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<native_history::HistoryList>> {
    native_history::list(&state, id, &q.source_id)
        .await
        .map(Json)
}
pub(super) async fn import_native_history(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    Json(input): Json<HistoryImport>,
) -> Result<Json<Session>> {
    if !input.confirmed_original_config {
        return Err(ApiError::bad(
            "Confirm using the original client configuration before loading history",
        ));
    }
    let session = match input.environment {
        Some(environment) => {
            native_history::import_with_environment(
                &state,
                id,
                input.source_id,
                input.native_id,
                Some(environment),
            )
            .await?
        }
        None => native_history::import(&state, id, input.source_id, input.native_id).await?,
    };
    Ok(Json(session))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AttachmentQuery {
    name: String,
    /// Read by `CheckoutParam`; named here so it is not an unknown field.
    #[serde(default, rename = "checkout")]
    _checkout: Option<String>,
}

/// Store an upload inside the directory the session works in -- its worktree,
/// if it has one -- so it can read it with its own tools. This keeps
/// attachments working for every client and file type, rather than depending
/// on one provider's inline-content protocol.
pub(super) async fn upload_attachment(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Query(query): Query<AttachmentQuery>,
    body: axum::body::Bytes,
) -> Result<Json<workspace_io::Attachment>> {
    if query.name.trim().is_empty() || query.name.len() > 400 {
        return Err(ApiError::bad("Attachment needs a file name"));
    }
    let root = checkout_root(&state, id, &checkout).await?;
    Ok(Json(
        workspace_io::write_attachment(&root, &query.name, &body).await?,
    ))
}
/// A file pasted or dropped into a session's terminal: stored in the directory
/// that session works in -- its worktree, if it has one -- so the path handed
/// to the program is one it can read.
pub(super) async fn upload_paste(
    State(state): State<AppState>,
    Path(id): Path<SessionId>,
    Query(query): Query<AttachmentQuery>,
    body: axum::body::Bytes,
) -> Result<Json<workspace_io::Attachment>> {
    if query.name.trim().is_empty() || query.name.len() > 400 {
        return Err(ApiError::bad("Attachment needs a file name"));
    }
    let session = session_record(&state, id).await?;
    let cwd = checkouts::session_cwd(&state, &session).await?;
    Ok(Json(
        workspace_io::write_paste(&cwd, &query.name, &body).await?,
    ))
}

/// The bytes of a file outside every workspace, for previewing an image,
/// video, audio or PDF an agent mentioned. Same reach as `/api/host/file`.
pub(super) async fn read_host_raw(
    State(state): State<AppState>,
    Query(q): Query<PathQuery>,
    request: Request,
) -> Result<Response> {
    let path = q
        .path
        .clone()
        .ok_or_else(|| ApiError::bad("Path required"))?;
    let roots = [state.browse_roots.clone(), state.host_grants.roots()].concat();
    let (target, _) =
        tokio::task::spawn_blocking(move || workspace_io::resolve_host_file(&roots, &path))
            .await
            .map_err(ApiError::internal)??;
    media::serve(target, q.format.as_deref() == Some("png"), request).await
}

/// The routes this module serves.
pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/host/directories", get(browse_directories))
        .route("/api/host/file", get(read_host_file))
        .route("/api/host/raw", get(read_host_raw))
        .route("/api/native-history/sources", get(native_sources))
        .route(
            "/api/workspaces/{id}/native-history",
            get(list_native_history),
        )
        .route(
            "/api/workspaces/{id}/native-history/import",
            post(import_native_history),
        )
        .route(
            "/api/workspaces/{id}/attachments",
            // Raw bytes, so a phone upload costs no base64 inflation. The limit
            // is per-route: the shared 3 MiB JSON cap is for API payloads.
            post(upload_attachment).layer(DefaultBodyLimit::max(
                workspace_io::MAX_ATTACHMENT_BYTES as usize + 4096,
            )),
        )
        .route(
            "/api/sessions/{id}/uploads",
            post(upload_paste).layer(DefaultBodyLimit::max(
                workspace_io::MAX_ATTACHMENT_BYTES as usize + 4096,
            )),
        )
}
