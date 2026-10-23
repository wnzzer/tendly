//! Which checkout of its repository a session works in.
//!
//! A branch belongs to a checkout, and every session in one directory shares
//! it. A session that needs a branch of its own runs in a git worktree of the
//! same repository instead, recorded on the session; `None` is the workspace
//! directory itself. Worktrees stay part of their workspace -- they are not
//! workspaces of their own -- so switching is a property of one session and
//! never moves the others.
//!
//! An isolated task is the one way to be sure of a directory of one's own:
//! Tendly names a new branch, makes a new worktree for it from a base branch,
//! prepares it (`worktree_setup.rs`) and records the base on the session, so
//! the work can be measured against it and merged back (`tasks.rs`). Moving a
//! session onto a branch by hand may land it beside others; this never does.
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
    /// With `isolated`, the new branch's name; absent names one for the session.
    pub branch: Option<String>,
    #[serde(default)]
    pub create: bool,
    /// An existing worktree of this repository.
    pub path: Option<String>,
    /// A new worktree on a branch checked out nowhere else, for this session
    /// alone: an isolated task.
    #[serde(default)]
    pub isolated: bool,
    /// With `isolated`: the local branch the task starts from and goes back
    /// to. Absent is the branch the session is on now.
    pub base: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Checkout {
    /// None for the workspace directory itself.
    pub path: Option<String>,
    pub branch: Option<String>,
    /// Whether this resolution made the worktree, so it is new and unprepared.
    #[serde(skip)]
    pub made: bool,
}

/// Where a new worktree for `branch` would go is bounded exactly as where a
/// new workspace may: this decides what the server can write to.
fn may_make(state: &AppState, listed: &workspace_io::GitBranches, branch: &str) -> Result<PathBuf> {
    let main = listed
        .worktrees
        .iter()
        .find(|w| w.main)
        .ok_or_else(|| ApiError::bad("Not a Git repository"))?;
    let planned = workspace_io::worktree_path(std::path::Path::new(&main.path), branch)?;
    let parent = planned.parent().and_then(|p| p.parent()).map(PathBuf::from);
    if !parent.is_some_and(|p| directories::within_roots(&state.workspace_roots, &p)) {
        return Err(ApiError::bad(
            "The worktree would sit outside the roots this server may use. Set TENDLY_WORKSPACE_ROOTS to allow it.",
        ));
    }
    Ok(planned)
}

/// A worktree directory as the session records it: canonical, inside the
/// roots, and `None` when it is the workspace directory after all.
async fn recorded(
    state: &AppState,
    repo: &std::path::Path,
    path: PathBuf,
) -> Result<Option<String>> {
    let canonical = crate::paths::canonicalize_async(&path)
        .await
        .map_err(|_| ApiError::conflict("Worktree directory is unavailable"))?;
    if canonical == repo
        || dunce::canonicalize(&canonical).is_ok_and(|p| p == repo)
        || canonical.to_string_lossy() == repo.to_string_lossy()
    {
        return Ok(None);
    }
    if !directories::within_roots(&state.workspace_roots, &canonical) {
        return Err(ApiError::bad(
            "That worktree is outside the roots this server may use. Set TENDLY_WORKSPACE_ROOTS to allow it.",
        ));
    }
    Ok(Some(canonical.to_string_lossy().into_owned()))
}

/// Resolve a choice to a checkout, making the worktree when there is none.
/// Neither branch nor path means the workspace directory.
pub async fn resolve(
    state: &AppState,
    workspace_id: WorkspaceId,
    choice: CheckoutChoice,
) -> Result<Checkout> {
    if choice.isolated || choice.base.is_some() {
        return Err(ApiError::bad(
            "An isolated worktree is made for a session; choose it there",
        ));
    }
    let repo = root(state, workspace_id).await?;
    let listed = workspace_io::git_branches(&repo).await?;
    let mut made = false;
    let (path, branch) = match (choice.path, choice.branch.map(|b| b.trim().to_owned())) {
        (None, None) => {
            return Ok(Checkout {
                path: None,
                branch: listed.current,
                made,
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
                    made,
                });
            }
            if let Some(found) = listed
                .worktrees
                .iter()
                .find(|w| w.branch.as_deref() == Some(branch.as_str()))
            {
                (PathBuf::from(&found.path), Some(branch))
            } else {
                may_make(state, &listed, &branch)?;
                let create = choice.create && !listed.branches.contains(&branch);
                made = true;
                (
                    workspace_io::git_worktree_add(&repo, &branch, create).await?,
                    Some(branch),
                )
            }
        }
        _ => return Err(ApiError::bad("Name a branch or a worktree, not both")),
    };
    Ok(Checkout {
        path: recorded(state, &repo, path).await?,
        branch,
        made,
    })
}

/// Where `branch` is checked out now, if anywhere: a worktree's directory, or
/// the workspace directory's.
fn checked_out_at(listed: &workspace_io::GitBranches, branch: &str) -> Option<String> {
    listed
        .worktrees
        .iter()
        .find(|w| w.branch.as_deref() == Some(branch))
        .map(|w| w.path.clone())
}

/// The branch an isolated task gets when it is not given one: `tendly/` and
/// the session's title, as far as that spells something in a branch name,
/// else a piece of its id; numbered on until neither the branch nor its
/// worktree directory exists yet.
fn task_branch(listed: &workspace_io::GitBranches, title: &str, id: SessionId) -> Result<String> {
    let mut slug = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
        if slug.len() >= 40 {
            break;
        }
    }
    let slug = slug.trim_matches('-');
    let stem = if slug.len() >= 3 {
        format!("tendly/{slug}")
    } else {
        format!("tendly/task-{}", &id.simple().to_string()[..8])
    };
    let main = listed
        .worktrees
        .iter()
        .find(|w| w.main)
        .map(|w| PathBuf::from(&w.path));
    let free = |name: &str| {
        !listed.branches.iter().any(|b| b == name)
            && main.as_ref().is_none_or(|main| {
                workspace_io::worktree_path(main, name).is_ok_and(|path| !path.exists())
            })
    };
    if free(&stem) {
        return Ok(stem);
    }
    (2..100)
        .map(|n| format!("{stem}-{n}"))
        .find(|name| free(name))
        .ok_or_else(|| ApiError::conflict("Name the branch for this task"))
}

/// An isolated checkout made for `session`: a new worktree on a branch
/// checked out nowhere else, started from `base`.
struct Isolated {
    checkout: Checkout,
    base: String,
    /// The branch did not exist before.
    created: bool,
}

async fn isolate(
    state: &AppState,
    session: &Session,
    branch: Option<String>,
    base: Option<String>,
) -> Result<Isolated> {
    let repo = root(state, session.workspace_id).await?;
    let listed = workspace_io::git_branches(&repo).await?;
    let base = match base.map(|b| b.trim().to_owned()).filter(|b| !b.is_empty()) {
        Some(base) => base,
        None => if session.checkout_path.is_some() {
            session.checkout_branch.clone()
        } else {
            listed.current.clone()
        }
        .ok_or_else(|| {
            ApiError::bad("This checkout is on no branch. Name the branch the task starts from.")
        })?,
    };
    if !listed.branches.contains(&base) {
        return Err(ApiError::bad(format!("{base} is not a local branch")));
    }
    let branch = match branch
        .map(|b| b.trim().to_owned())
        .filter(|b| !b.is_empty())
    {
        Some(branch) => {
            if let Some(holder) = checked_out_at(&listed, &branch) {
                return Err(ApiError::conflict(format!(
                    "{branch} is already checked out in {holder}. An isolated task needs a branch checked out nowhere else."
                )));
            }
            branch
        }
        None => task_branch(&listed, &session.title, session.id)?,
    };
    may_make(state, &listed, &branch)?;
    let created = !listed.branches.contains(&branch);
    let path = if created {
        workspace_io::git_worktree_add_from(&repo, &branch, &base).await?
    } else {
        workspace_io::git_worktree_add(&repo, &branch, false).await?
    };
    let path = recorded(state, &repo, path).await?;
    Ok(Isolated {
        checkout: Checkout {
            path,
            branch: Some(branch),
            made: true,
        },
        base,
        created,
    })
}

/// Undo an isolated worktree a session could not be moved into after all:
/// the worktree goes, and the branch too when it was made just now.
async fn unmake(repo: &std::path::Path, made: &Isolated) {
    let Some(path) = &made.checkout.path else {
        return;
    };
    let _ = workspace_io::git_worktree_remove(repo, path, true).await;
    if made.created
        && let Some(branch) = &made.checkout.branch
    {
        let _ = workspace_io::git_branch_delete(repo, branch, true).await;
    }
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
    if crate::worktree_setup::preparing(state, path) {
        return Err(ApiError::conflict(
            "This session's worktree is still being prepared. It starts when that is done.",
        ));
    }
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
    let (checkout, isolated) = if choice.isolated {
        // Stopped before anything is made, so a session that cannot move
        // leaves no worktree behind.
        crate::conversations::stop_if_idle(&state, id).await?;
        let made = isolate(&state, &session, choice.branch, choice.base).await?;
        (made.checkout.clone(), Some(made))
    } else {
        let checkout = resolve(&state, session.workspace_id, choice).await?;
        if checkout.path == session.checkout_path && session.checkout_base.is_none() {
            return Ok(Json(session));
        }
        crate::conversations::stop_if_idle(&state, id).await?;
        (checkout, None)
    };
    let base = isolated.as_ref().map(|made| made.base.clone());
    let label = checkout
        .branch
        .clone()
        .unwrap_or_else(|| "Detached HEAD".into());
    let (path, branch, recorded_base) = (checkout.path.clone(), checkout.branch.clone(), base);
    if !db(&state, move |s| {
        s.set_session_checkout(
            id,
            path.as_deref(),
            branch.as_deref(),
            recorded_base.as_deref(),
            &label,
        )
    })
    .await?
    {
        if let Some(made) = &isolated {
            unmake(&root(&state, session.workspace_id).await?, made).await;
        }
        return Err(ApiError::conflict(
            "The session must be idle or stopped before it moves",
        ));
    }
    crate::conversations::publish_snapshot(&state, id).await;
    if checkout.made
        && let (Some(path), Some(branch)) = (&checkout.path, &checkout.branch)
    {
        let workspace = crate::workspace(&state, session.workspace_id).await?;
        crate::worktree_setup::start(
            &state,
            id,
            workspace.worktree_setup,
            PathBuf::from(workspace.root_path),
            PathBuf::from(path),
            branch.clone(),
        );
    }
    Ok(Json(session_record(&state, id).await?))
}

/// For the agent tools: give a session that has not started an isolated
/// task's worktree, on `branch` or one named for it, started from the branch
/// its parent is on.
pub(crate) async fn isolate_child(
    state: &AppState,
    id: SessionId,
    branch: Option<String>,
    base: Option<String>,
) -> Result<Session> {
    let choice = CheckoutChoice {
        branch,
        isolated: true,
        base,
        ..CheckoutChoice::default()
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
            s.set_session_checkout(sid, None, branch.as_deref(), None, &label)
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
