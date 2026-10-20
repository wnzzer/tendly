//! Merge, rebase, cherry-pick, revert and reset; continuing or abandoning an
//! operation that stopped part-way, and settling a conflicted file.

use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MergeInput {
    rev: String,
    /// Always record a merge commit, even when a fast-forward would do.
    #[serde(default)]
    no_ff: bool,
    /// Stage the combined changes for one commit instead of merging history.
    #[serde(default)]
    squash: bool,
}

pub(super) async fn merge(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Json(input): Json<MergeInput>,
) -> Result<Json<Outcome>> {
    let _guard = state.operations.lock().await;
    let root = repo(&state, id, &checkout).await?;
    let rev = input.rev.trim();
    valid_rev(&root, rev).await?;
    let mut list = args(&["merge", "--no-edit"]);
    if input.squash {
        list.push("--squash".into());
    } else if input.no_ff {
        list.push("--no-ff".into());
    }
    // The name rather than its hash, so the merge commit says what was merged.
    list.push(rev.to_owned());
    finish(&root, list, COMMIT_TIMEOUT).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RevInput {
    rev: String,
    /// Reset: `soft`, `mixed` or `hard`.
    #[serde(default)]
    mode: Option<String>,
}

pub(super) async fn rebase(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Json(input): Json<RevInput>,
) -> Result<Json<Outcome>> {
    let _guard = state.operations.lock().await;
    let root = repo(&state, id, &checkout).await?;
    let rev = input.rev.trim();
    valid_rev(&root, rev).await?;
    finish(&root, args(&["rebase", rev]), COMMIT_TIMEOUT).await
}

async fn parent_count(root: &std::path::Path, hash: &str) -> Result<usize> {
    Ok(git_text(root, &["rev-list", "--parents", "-n", "1", hash])
        .await?
        .unwrap_or_default()
        .split(' ')
        .count()
        .saturating_sub(1))
}

pub(super) async fn cherry_pick(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Json(input): Json<RevInput>,
) -> Result<Json<Outcome>> {
    let _guard = state.operations.lock().await;
    let root = repo(&state, id, &checkout).await?;
    let hash = valid_rev(&root, input.rev.trim()).await?;
    let mut list = args(&["cherry-pick"]);
    // A merge is picked as its change against the branch it was merged into.
    if parent_count(&root, &hash).await? > 1 {
        list.extend(args(&["-m", "1"]));
    }
    list.push(hash);
    finish(&root, list, COMMIT_TIMEOUT).await
}

pub(super) async fn revert(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Json(input): Json<RevInput>,
) -> Result<Json<Outcome>> {
    let _guard = state.operations.lock().await;
    let root = repo(&state, id, &checkout).await?;
    let hash = valid_rev(&root, input.rev.trim()).await?;
    let mut list = args(&["revert", "--no-edit"]);
    if parent_count(&root, &hash).await? > 1 {
        list.extend(args(&["-m", "1"]));
    }
    list.push(hash);
    finish(&root, list, COMMIT_TIMEOUT).await
}

/// Move the current branch to another commit. `soft` keeps everything
/// staged, `mixed` keeps it as unstaged changes, `hard` throws it away.
pub(super) async fn reset(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Json(input): Json<RevInput>,
) -> Result<StatusCode> {
    let _guard = state.operations.lock().await;
    let root = repo(&state, id, &checkout).await?;
    let hash = valid_rev(&root, input.rev.trim()).await?;
    let mode = match input.mode.as_deref().unwrap_or("mixed") {
        "soft" => "--soft",
        "mixed" => "--mixed",
        "hard" => "--hard",
        _ => return Err(ApiError::bad("Unknown reset mode")),
    };
    git(&root, args(&["reset", mode, &hash]), COMMIT_TIMEOUT).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OperationInput {
    /// `continue`, `abort` or `skip`.
    action: String,
}

/// Continue, abort or skip whichever operation stopped part-way.
pub(super) async fn operation(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Json(input): Json<OperationInput>,
) -> Result<Json<Outcome>> {
    let _guard = state.operations.lock().await;
    let root = repo(&state, id, &checkout).await?;
    let current = git_operation(&root)
        .await
        .ok_or_else(|| ApiError::conflict("Nothing is in progress"))?;
    let action = match input.action.as_str() {
        "continue" => "--continue",
        "abort" => "--abort",
        "skip" if current != "merge" => "--skip",
        _ => return Err(ApiError::bad("Unknown action")),
    };
    if action == "--continue" && unmerged(&root).await {
        return Err(ApiError::conflict(
            "Resolve every conflict and stage the files before continuing.",
        ));
    }
    if action == "--abort" {
        return Ok(Json(Outcome {
            conflicts: false,
            message: git(&root, args(&[&current, action]), COMMIT_TIMEOUT).await?,
        }));
    }
    finish(&root, args(&[&current, action]), COMMIT_TIMEOUT).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ResolveInput {
    path: String,
    /// `ours` (the branch being merged into) or `theirs` (what is coming in).
    side: String,
}

/// Settle a conflicted file by taking one side whole, and mark it resolved.
/// When that side deleted the file, resolving it deletes it.
pub(super) async fn resolve(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Json(input): Json<ResolveInput>,
) -> Result<StatusCode> {
    let _guard = state.operations.lock().await;
    let root = repo(&state, id, &checkout).await?;
    let path = checked_path(&input.path)?;
    let side = match input.side.as_str() {
        "ours" => "--ours",
        "theirs" => "--theirs",
        _ => return Err(ApiError::bad("Unknown side")),
    };
    let (_, stderr, success) = run_git(
        &root,
        vec!["checkout".into(), side.into(), "--".into(), path.clone()],
        GIT_TIMEOUT,
    )
    .await?;
    let gone = String::from_utf8_lossy(&stderr);
    if success {
        git(&root, vec!["add".into(), "--".into(), path], GIT_TIMEOUT).await?;
    } else if gone.contains("does not have our version")
        || gone.contains("does not have their version")
    {
        git(
            &root,
            vec!["rm".into(), "-q".into(), "--".into(), path],
            GIT_TIMEOUT,
        )
        .await?;
    } else {
        return Err(IoError::new(409, git_error(&stderr)).into());
    }
    Ok(StatusCode::NO_CONTENT)
}
