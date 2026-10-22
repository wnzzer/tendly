//! Which checkout of its repository a session works in.
//!
//! A branch belongs to a checkout, and every session in one directory shares
//! it. A session that needs a branch of its own runs in a git worktree of the
//! same repository instead, recorded on the session; `None` is the workspace
//! directory itself. Worktrees stay part of their workspace -- they are not
//! workspaces of their own -- so switching is a property of one session and
//! never moves the others.
use crate::{ApiError, AppState, Result, db, directories, root, session_record, workspace_io};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::post,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tendly_domain::{Session, SessionId, WorkspaceId};

/// A checkout, chosen by branch or by an existing worktree's directory.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckoutChoice {
    /// A branch: its existing worktree, the workspace itself if it is checked
    /// out there, or a new worktree (creating the branch when `create`).
    pub branch: Option<String>,
    #[serde(default)]
    pub create: bool,
    /// An existing worktree of this repository.
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Checkout {
    /// None for the workspace directory itself.
    pub path: Option<String>,
    pub branch: Option<String>,
}

/// Resolve a choice to a checkout, making the worktree when there is none.
/// Neither branch nor path means the workspace directory.
pub async fn resolve(
    state: &AppState,
    workspace_id: WorkspaceId,
    choice: CheckoutChoice,
) -> Result<Checkout> {
    let repo = root(state, workspace_id).await?;
    let listed = workspace_io::git_branches(&repo).await?;
    let repo_text = repo.to_string_lossy().into_owned();
    let is_repo = |path: &str| {
        dunce::canonicalize(path)
            .map(|p| p == repo)
            .unwrap_or(false)
            || path == repo_text
    };
    let (path, branch) = match (choice.path, choice.branch.map(|b| b.trim().to_owned())) {
        (None, None) => {
            return Ok(Checkout {
                path: None,
                branch: listed.current,
            });
        }
        (Some(path), None) => {
            let found = listed
                .worktrees
                .iter()
                .find(|w| w.path == path)
                .ok_or_else(|| ApiError::bad("Not a worktree of this workspace's repository"))?;
            (PathBuf::from(&found.path), found.branch.clone())
        }
        (None, Some(branch)) => {
            if listed.current.as_deref() == Some(branch.as_str()) {
                return Ok(Checkout {
                    path: None,
                    branch: Some(branch),
                });
            }
            if let Some(found) = listed
                .worktrees
                .iter()
                .find(|w| w.branch.as_deref() == Some(branch.as_str()))
            {
                (PathBuf::from(&found.path), Some(branch))
            } else {
                let main = listed
                    .worktrees
                    .iter()
                    .find(|w| w.main)
                    .ok_or_else(|| ApiError::bad("Not a Git repository"))?;
                let planned =
                    workspace_io::worktree_path(std::path::Path::new(&main.path), &branch)?;
                // Where a new worktree may go is bounded exactly as where a new
                // workspace may: this decides what the server can write to.
                let parent = planned.parent().and_then(|p| p.parent()).map(PathBuf::from);
                if !parent.is_some_and(|p| directories::within_roots(&state.workspace_roots, &p)) {
                    return Err(ApiError::bad(
                        "The worktree would sit outside the roots this server may use. Set TENDLY_WORKSPACE_ROOTS to allow it.",
                    ));
                }
                let create = choice.create && !listed.branches.contains(&branch);
                (
                    workspace_io::git_worktree_add(&repo, &branch, create).await?,
                    Some(branch),
                )
            }
        }
        _ => return Err(ApiError::bad("Name a branch or a worktree, not both")),
    };
    let canonical = crate::paths::canonicalize_async(&path)
        .await
        .map_err(|_| ApiError::conflict("Worktree directory is unavailable"))?;
    if canonical == repo || is_repo(&canonical.to_string_lossy()) {
        return Ok(Checkout { path: None, branch });
    }
    if !directories::within_roots(&state.workspace_roots, &canonical) {
        return Err(ApiError::bad(
            "That worktree is outside the roots this server may use. Set TENDLY_WORKSPACE_ROOTS to allow it.",
        ));
    }
    Ok(Checkout {
        path: Some(canonical.to_string_lossy().into_owned()),
        branch,
    })
}

/// Bring the branch each session in this workspace shows back in line with
/// what its worktree has checked out. The path is what binds a session; the
/// branch is a label recorded when it was chosen, and a branch renamed or
/// switched inside the worktree afterwards would otherwise leave it stale.
pub async fn refresh_labels(
    state: &AppState,
    workspace_id: WorkspaceId,
    listed: &workspace_io::GitBranches,
) {
    let actual: Vec<(String, Option<String>)> = listed
        .worktrees
        .iter()
        .map(|w| (w.path.clone(), w.branch.clone()))
        .collect();
    let _ = db(state, move |s| {
        for session in s.list_sessions(Some(workspace_id))? {
            let Some(path) = session.checkout_path.as_deref() else {
                continue;
            };
            if let Some((_, branch)) = actual.iter().find(|(p, _)| p == path)
                && branch.as_deref() != session.checkout_branch.as_deref()
            {
                s.set_session_checkout_branch(session.id, branch.as_deref())?;
            }
        }
        Ok(())
    })
    .await;
}

/// The directory a session's client starts in.
///
/// A recorded checkout is used only while it is still a worktree of the
/// workspace's repository: a worktree removed or re-pointed outside Tendly
/// fails the start with a reason instead of silently running somewhere else.
pub async fn session_cwd(state: &AppState, session: &Session) -> Result<PathBuf> {
    let repo = root(state, session.workspace_id).await?;
    let Some(path) = session.checkout_path.as_deref() else {
        return Ok(repo);
    };
    let listed = workspace_io::git_branches(&repo).await.ok();
    let valid = listed.is_some_and(|l| l.worktrees.iter().any(|w| w.path == path));
    let canonical = crate::paths::canonicalize_async(path)
        .await
        .ok()
        .filter(|p| p.is_dir());
    match canonical {
        Some(dir) if valid && directories::within_roots(&state.workspace_roots, &dir) => Ok(dir),
        _ => Err(ApiError::conflict(format!(
            "This session's worktree {path} is no longer available. Choose another branch for it."
        ))),
    }
}

/// Move a session to another checkout. Stops it first if it is idle, as an
/// endpoint change does; a turn in progress or an open approval refuses.
async fn set_checkout(
    State(state): State<AppState>,
    Path(id): Path<SessionId>,
    Json(choice): Json<CheckoutChoice>,
) -> Result<Json<Session>> {
    let _guard = state.operations.lock().await;
    let session = session_record(&state, id).await?;
    // A terminal cannot change its directory while it runs, and ending it is
    // the user's decision, not a side effect of choosing a branch.
    if state
        .runtime
        .get(&id.to_string())
        .is_some_and(|r| r.running())
    {
        return Err(ApiError::conflict(
            "End the native terminal before moving it to another branch",
        ));
    }
    let checkout = resolve(&state, session.workspace_id, choice).await?;
    if checkout.path == session.checkout_path {
        return Ok(Json(session));
    }
    crate::conversations::stop_if_idle(&state, id).await?;
    let label = checkout
        .branch
        .clone()
        .unwrap_or_else(|| "Detached HEAD".into());
    let (path, branch) = (checkout.path.clone(), checkout.branch.clone());
    if !db(&state, move |s| {
        s.set_session_checkout(id, path.as_deref(), branch.as_deref(), &label)
    })
    .await?
    {
        return Err(ApiError::conflict(
            "The session must be idle or stopped before it moves",
        ));
    }
    crate::conversations::publish_snapshot(&state, id).await;
    Ok(Json(session_record(&state, id).await?))
}

/// For the agent tools: move a session that has not started onto a branch's
/// worktree, making the branch when it does not exist yet.
pub(crate) async fn move_to_branch(
    state: &AppState,
    id: SessionId,
    branch: String,
) -> Result<Session> {
    let choice = CheckoutChoice {
        branch: Some(branch),
        create: true,
        path: None,
    };
    set_checkout(State(state.clone()), Path(id), Json(choice))
        .await
        .map(|Json(session)| session)
}

/// A worktree for a branch, made or found, without moving any session.
async fn worktree(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    Json(choice): Json<CheckoutChoice>,
) -> Result<Json<Checkout>> {
    let _guard = state.operations.lock().await;
    Ok(Json(resolve(&state, id, choice).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameInput {
    from: String,
    to: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteBranchInput {
    branch: String,
    #[serde(default)]
    force: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoveWorktreeInput {
    path: String,
    #[serde(default)]
    force: bool,
}

/// Rename a branch. Sessions in its worktree stay where they are and show
/// the new name, since what binds them is the directory.
async fn rename_branch(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    Json(input): Json<RenameInput>,
) -> Result<Json<workspace_io::GitBranches>> {
    let _guard = state.operations.lock().await;
    let repo = root(&state, id).await?;
    workspace_io::git_branch_rename(&repo, input.from.trim(), input.to.trim()).await?;
    let listed = workspace_io::git_branches(&repo).await?;
    refresh_labels(&state, id, &listed).await;
    Ok(Json(listed))
}

async fn delete_branch(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    Json(input): Json<DeleteBranchInput>,
) -> Result<Json<workspace_io::GitBranches>> {
    let _guard = state.operations.lock().await;
    let repo = root(&state, id).await?;
    workspace_io::git_branch_delete(&repo, input.branch.trim(), input.force).await?;
    Ok(Json(workspace_io::git_branches(&repo).await?))
}

/// The sessions bound to a worktree, archived ones included.
async fn bound_to(state: &AppState, id: WorkspaceId, path: &str) -> Result<Vec<Session>> {
    let path = path.to_owned();
    Ok(db(state, move |s| s.list_sessions(Some(id)))
        .await?
        .into_iter()
        .filter(|session| session.checkout_path.as_deref() == Some(path.as_str()))
        .collect())
}

fn running(state: &AppState, sessions: &[Session]) -> usize {
    sessions
        .iter()
        .filter(|session| {
            state.chats.get(session.id).is_some_and(|r| r.running())
                || state
                    .runtime
                    .get(&session.id.to_string())
                    .is_some_and(|r| r.running())
        })
        .count()
}

/// Stopped sessions of a removed worktree go back to the workspace directory,
/// so none is left pointing at a directory that is gone.
async fn send_home(
    state: &AppState,
    sessions: Vec<Session>,
    listed: &workspace_io::GitBranches,
) -> Result<()> {
    for session in sessions {
        let sid = session.id;
        let branch = listed.current.clone();
        let label = branch.clone().unwrap_or_else(|| "Detached HEAD".into());
        db(state, move |s| {
            s.set_session_checkout(sid, None, branch.as_deref(), &label)
        })
        .await?;
        crate::conversations::publish_snapshot(state, sid).await;
    }
    Ok(())
}

/// Remove a worktree. A session running in it refuses the removal; stopped
/// ones bound to it move back to the workspace directory first.
async fn remove_worktree(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    Json(input): Json<RemoveWorktreeInput>,
) -> Result<Json<workspace_io::GitBranches>> {
    let _guard = state.operations.lock().await;
    let repo = root(&state, id).await?;
    let bound = bound_to(&state, id, &input.path).await?;
    let running = running(&state, &bound);
    if running > 0 {
        return Err(ApiError::conflict(format!(
            "{running} session(s) are running in this worktree. End them or move them to another branch first."
        )));
    }
    workspace_io::git_worktree_remove(&repo, &input.path, input.force).await?;
    let listed = workspace_io::git_branches(&repo).await?;
    send_home(&state, bound, &listed).await?;
    Ok(Json(listed))
}

/// The branches that could be cleaned up, each with why. Read when the
/// branch menu opens, apart from the branch list, since it runs a few Git
/// commands per branch.
async fn cleanup_candidates(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
) -> Result<Json<workspace_io::GitCleanup>> {
    let repo = root(&state, id).await?;
    let mut found = workspace_io::git_cleanup(&repo).await?;
    let mut kept = Vec::with_capacity(found.branches.len());
    for candidate in found.branches {
        if !just_moved_to(&state, id, &repo, &candidate).await? {
            kept.push(candidate);
        }
    }
    found.branches = kept;
    Ok(Json(found))
}

/// A branch a session was just moved to: its worktree is that session's
/// checkout and nothing was committed there yet. It reads as merged, but it
/// is where the session's work is about to go, so it is not cleaned up.
async fn just_moved_to(
    state: &AppState,
    id: WorkspaceId,
    repo: &std::path::Path,
    candidate: &workspace_io::CleanupCandidate,
) -> Result<bool> {
    let Some(path) = &candidate.worktree else {
        return Ok(false);
    };
    Ok(!bound_to(state, id, path).await?.is_empty()
        && workspace_io::git_never_committed(repo, &candidate.name).await)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CleanupInput {
    branches: Vec<String>,
}

#[derive(Serialize)]
struct CleanupResult {
    name: String,
    removed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

#[derive(Serialize)]
struct CleanupOutcome {
    results: Vec<CleanupResult>,
    branches: workspace_io::GitBranches,
}

/// Delete branches whose work is already on the base branch, checked again
/// here rather than taken on the page's word. A worktree holding one goes
/// with it only when nothing runs there and nothing is uncommitted; a branch
/// whose upstream is merely gone is never deleted this way, since its work
/// was not found upstream.
async fn cleanup_branches(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    Json(input): Json<CleanupInput>,
) -> Result<Json<CleanupOutcome>> {
    let _guard = state.operations.lock().await;
    let repo = root(&state, id).await?;
    let found = workspace_io::git_cleanup(&repo).await?;
    let mut names = input.branches;
    names.sort();
    names.dedup();
    let mut results = Vec::new();
    let mut homeless = Vec::new();
    for name in names.into_iter().take(200) {
        let skip = |reason: &str| CleanupResult {
            name: name.clone(),
            removed: false,
            reason: Some(reason.to_owned()),
        };
        let Some(candidate) = found
            .branches
            .iter()
            .find(|candidate| candidate.name == name)
        else {
            results.push(skip("It is not merged into the base branch."));
            continue;
        };
        if candidate.reason == "gone" {
            results.push(skip("Its work was not found on the base branch."));
            continue;
        }
        if just_moved_to(&state, id, &repo, candidate).await? {
            results.push(skip(
                "A session was just moved to it; nothing is committed there yet.",
            ));
            continue;
        }
        if let Some(path) = &candidate.worktree {
            let bound = bound_to(&state, id, path).await?;
            if running(&state, &bound) > 0 {
                results.push(skip("A session is running in its worktree."));
                continue;
            }
            let dirty =
                workspace_io::git_text(std::path::Path::new(path), &["status", "--porcelain"])
                    .await
                    .ok()
                    .flatten();
            if dirty.as_deref().is_none_or(|changes| !changes.is_empty()) {
                results.push(skip("Its worktree has uncommitted changes."));
                continue;
            }
            if let Err(error) = workspace_io::git_worktree_remove(&repo, path, false).await {
                results.push(skip(&error.message));
                continue;
            }
            homeless.extend(bound);
        }
        // -D, since Git does not count a squash as merged; the check above did.
        match workspace_io::git_branch_delete(&repo, &name, true).await {
            Ok(()) => results.push(CleanupResult {
                name,
                removed: true,
                reason: None,
            }),
            Err(error) => results.push(skip(&error.message)),
        }
    }
    let listed = workspace_io::git_branches(&repo).await?;
    send_home(&state, homeless, &listed).await?;
    Ok(Json(CleanupOutcome {
        results,
        branches: listed,
    }))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/sessions/{id}/checkout", post(set_checkout))
        .route("/api/workspaces/{id}/git/worktrees", post(worktree))
        .route(
            "/api/workspaces/{id}/git/worktrees/remove",
            post(remove_worktree),
        )
        .route(
            "/api/workspaces/{id}/git/branches/rename",
            post(rename_branch),
        )
        .route(
            "/api/workspaces/{id}/git/branches/delete",
            post(delete_branch),
        )
        .route(
            "/api/workspaces/{id}/git/branches/cleanup",
            axum::routing::get(cleanup_candidates).post(cleanup_branches),
        )
}
