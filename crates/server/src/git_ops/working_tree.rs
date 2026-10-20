//! Changes made straight to the working tree or index: one hunk applied or
//! taken back, a new repository, and a path added to `.gitignore`.

use super::*;

const MAX_PATCH: usize = 4 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ApplyInput {
    /// The one file the patch changes; anything else in it is ignored.
    path: String,
    patch: String,
    /// Apply to the index (stage) rather than the working tree.
    #[serde(default)]
    cached: bool,
    /// Take the change back out: unstage, or discard.
    #[serde(default)]
    reverse: bool,
}

/// `git apply --include` reads a glob; the path is meant literally.
pub(super) fn literal_glob(path: &str) -> String {
    path.chars()
        .flat_map(|c| match c {
            '*' | '?' | '[' | ']' | '\\' => vec!['\\', c],
            _ => vec![c],
        })
        .collect()
}

/// Stage, unstage or discard part of a file: the browser sends one hunk of
/// the diff it showed, and Git applies it to the index or the working tree.
pub(super) async fn apply(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Json(input): Json<ApplyInput>,
) -> Result<StatusCode> {
    let _guard = state.operations.lock().await;
    let root = repo(&state, id, &checkout).await?;
    let path = checked_path(&input.path)?;
    if input.patch.len() > MAX_PATCH || input.patch.matches("diff --git ").count() != 1 {
        return Err(ApiError::bad("A patch for exactly one file is required"));
    }
    let file = std::env::temp_dir().join(format!("tendly-hunk-{}.patch", uuid::Uuid::new_v4()));
    tokio::fs::write(&file, input.patch.as_bytes())
        .await
        .map_err(IoError::from)?;
    let mut list = args(&["apply", "--recount", "--whitespace=nowarn"]);
    if input.cached {
        list.push("--cached".into());
    }
    if input.reverse {
        list.push("--reverse".into());
    }
    list.push(format!("--include={}", literal_glob(&path)));
    list.push(file.to_string_lossy().into_owned());
    let result = git(&root, list, GIT_TIMEOUT).await;
    let _ = tokio::fs::remove_file(&file).await;
    result.map(|_| StatusCode::NO_CONTENT)
}

/// Make the workspace directory a repository of its own.
pub(super) async fn init(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
) -> Result<StatusCode> {
    let _guard = state.operations.lock().await;
    let root = repo(&state, id, &checkout).await?;
    if git_text(&root, &["rev-parse", "--is-inside-work-tree"])
        .await?
        .is_some()
    {
        return Err(ApiError::conflict(
            "This directory is already in a Git repository.",
        ));
    }
    git(&root, args(&["init"]), GIT_TIMEOUT).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct IgnoreInput {
    path: String,
}

/// Add a path to the workspace's `.gitignore`, anchored to the root so it
/// names that one file or directory and nothing else of the same name.
pub(super) async fn ignore(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Json(input): Json<IgnoreInput>,
) -> Result<StatusCode> {
    let _guard = state.operations.lock().await;
    let root = repo(&state, id, &checkout).await?;
    let path = checked_path(input.path.trim_end_matches('/'))?;
    let escaped: String = path
        .chars()
        .flat_map(|c| match c {
            '*' | '?' | '[' | '\\' | '!' | '#' => vec!['\\', c],
            _ => vec![c],
        })
        .collect();
    let line = format!("/{}", escaped.trim_end());
    let target = workspace_io::safe_path(&root, ".gitignore")?;
    let existing = tokio::fs::read_to_string(&target).await.unwrap_or_default();
    if existing.lines().any(|entry| entry.trim() == line) {
        return Ok(StatusCode::NO_CONTENT);
    }
    let mut next = existing;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(&line);
    next.push('\n');
    tokio::fs::write(&target, next)
        .await
        .map_err(IoError::from)?;
    Ok(StatusCode::NO_CONTENT)
}
