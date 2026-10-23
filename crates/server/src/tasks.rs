//! An isolated task's way back (see `checkouts.rs`): what it changed against
//! the branch it started from, merging it into that branch, and putting its
//! worktree and branch away once it is done -- or throwing them away.
//!
//! Merging never leaves the base half-merged. Where the base is checked out,
//! Git's `merge-tree` checks first that the two combine cleanly, so a merge
//! that would stop on conflicts is refused before any file changes there.
//! Where it is checked out nowhere, the merge is made without a working tree
//! at all and the branch moved to it.
use crate::{
    ApiError, AppState, Result, db, root, session_record,
    workspace_io::{self, COMMIT_TIMEOUT, GIT_TIMEOUT, GitDiff, git_error, git_text, run_git},
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::path::{Path as FsPath, PathBuf};
use tendly_domain::{Session, SessionId};

/// The most changed files a task lists.
const MAX_FILES: usize = 500;
const MAX_DIFF: usize = 4 * 1024 * 1024;

/// What an isolated session's task stands at.
struct Task {
    repo: PathBuf,
    worktree: String,
    branch: String,
    base: String,
}

async fn task_of(state: &AppState, session: &Session) -> Result<Task> {
    let (Some(worktree), Some(branch), Some(base)) = (
        session.checkout_path.clone(),
        session.checkout_branch.clone(),
        session.checkout_base.clone(),
    ) else {
        return Err(ApiError::bad("This session is not an isolated task"));
    };
    let repo = root(state, session.workspace_id).await?;
    workspace_io::valid_branch(&repo, &branch).await?;
    workspace_io::valid_branch(&repo, &base).await?;
    Ok(Task {
        repo,
        worktree,
        branch,
        base,
    })
}

async fn git_lines(root: &FsPath, args: &[&str]) -> Result<Vec<String>> {
    Ok(git_text(root, args)
        .await?
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

/// Commits the task has that its base lacks, and the reverse.
async fn ahead_behind(task: &Task) -> Result<(u32, u32)> {
    let counts = git_text(
        &task.repo,
        &[
            "rev-list",
            "--left-right",
            "--count",
            &format!("{}...{}", task.base, task.branch),
        ],
    )
    .await?
    .unwrap_or_default();
    let mut parts = counts.split_whitespace().map(|n| n.parse().unwrap_or(0));
    let behind = parts.next().unwrap_or(0);
    let ahead = parts.next().unwrap_or(0);
    Ok((ahead, behind))
}

async fn merge_base(task: &Task) -> Result<String> {
    git_text(&task.repo, &["merge-base", &task.base, &task.branch])
        .await?
        .filter(|hash| !hash.is_empty())
        .ok_or_else(|| {
            ApiError::conflict(format!(
                "{} and {} share no history",
                task.branch, task.base
            ))
        })
}

/// Uncommitted changes in the worktree, untracked files included: what
/// removing it would lose. Ignored files -- the dependencies its setup copied
/// in -- are not counted.
async fn uncommitted(task: &Task) -> Result<usize> {
    let worktree = FsPath::new(&task.worktree);
    if !worktree.is_dir() {
        return Ok(0);
    }
    Ok(git_lines(worktree, &["status", "--porcelain"]).await?.len())
}

/// Where a branch is checked out, if anywhere.
async fn checked_out(repo: &FsPath, branch: &str) -> Result<Option<String>> {
    Ok(workspace_io::git_branches(repo)
        .await?
        .worktrees
        .into_iter()
        .find(|w| w.branch.as_deref() == Some(branch))
        .map(|w| w.path))
}

#[derive(Serialize)]
struct ChangedFile {
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    added: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    removed: Option<u32>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    untracked: bool,
}

#[derive(Serialize)]
struct TaskState {
    branch: String,
    base: String,
    worktree: String,
    ahead: u32,
    behind: u32,
    /// Files changed in the worktree and not committed yet.
    uncommitted: usize,
    /// What the task changed since it left its base, committed or not.
    files: Vec<ChangedFile>,
    files_truncated: bool,
    /// The directory the base is checked out in, if any.
    base_checkout: Option<String>,
    /// The base is the workspace directory's branch.
    base_is_workspace: bool,
    /// The remote branch the task's branch was pushed to.
    upstream: Option<String>,
    /// Where the forge opens a pull request for it, when it is one Tendly knows.
    compare_url: Option<String>,
    /// Its worktree is still being prepared.
    preparing: bool,
}

async fn changed_files(task: &Task, since: &str) -> Result<(Vec<ChangedFile>, bool)> {
    let worktree = FsPath::new(&task.worktree);
    let mut files = Vec::new();
    for line in git_lines(worktree, &["diff", "--numstat", "--no-renames", since]).await? {
        let mut parts = line.splitn(3, '\t');
        let (Some(added), Some(removed), Some(path)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        files.push(ChangedFile {
            path: path.to_owned(),
            added: added.parse().ok(),
            removed: removed.parse().ok(),
            untracked: false,
        });
    }
    for path in git_lines(worktree, &["ls-files", "--others", "--exclude-standard"]).await? {
        files.push(ChangedFile {
            path,
            added: None,
            removed: None,
            untracked: true,
        });
    }
    let truncated = files.len() > MAX_FILES;
    files.truncate(MAX_FILES);
    Ok((files, truncated))
}

/// A web page for opening a pull request, for a remote on GitHub.
fn compare_url(remote: &str, base: &str, branch: &str) -> Option<String> {
    let remote = remote.trim().trim_end_matches('/').trim_end_matches(".git");
    let path = remote
        .strip_prefix("git@github.com:")
        .or_else(|| remote.strip_prefix("ssh://git@github.com/"))
        .or_else(|| remote.strip_prefix("https://github.com/"))?;
    let mut parts = path.split('/');
    let (Some(owner), Some(repo), None) = (parts.next(), parts.next(), parts.next()) else {
        return None;
    };
    let encode = |name: &str| {
        name.replace('%', "%25")
            .replace('#', "%23")
            .replace('?', "%3F")
    };
    Some(format!(
        "https://github.com/{owner}/{repo}/compare/{}...{}?expand=1",
        encode(base),
        encode(branch)
    ))
}

async fn upstream(task: &Task) -> Option<(String, Option<String>)> {
    let upstream = git_text(
        &task.repo,
        &[
            "rev-parse",
            "--abbrev-ref",
            "--end-of-options",
            &format!("{}@{{upstream}}", task.branch),
        ],
    )
    .await
    .ok()
    .flatten()
    .filter(|name| !name.is_empty())?;
    let remote = upstream.split('/').next().unwrap_or_default().to_owned();
    if remote.is_empty() || remote.starts_with('-') {
        return Some((upstream, None));
    }
    let url = git_text(&task.repo, &["remote", "get-url", &remote])
        .await
        .ok()
        .flatten();
    let remote_branch = upstream
        .strip_prefix(&format!("{remote}/"))
        .unwrap_or(&task.branch)
        .to_owned();
    let link = url.and_then(|url| compare_url(&url, &task.base, &remote_branch));
    Some((upstream, link))
}

async fn task_state(
    State(state): State<AppState>,
    Path(id): Path<SessionId>,
) -> Result<Json<TaskState>> {
    let session = session_record(&state, id).await?;
    let task = task_of(&state, &session).await?;
    let (ahead, behind) = ahead_behind(&task).await?;
    let since = merge_base(&task).await?;
    let (files, files_truncated) = changed_files(&task, &since).await?;
    let base_checkout = checked_out(&task.repo, &task.base).await?;
    let base_is_workspace = base_checkout
        .as_deref()
        .is_some_and(|path| dunce::canonicalize(path).is_ok_and(|path| path == task.repo));
    let (upstream, compare_url) = match upstream(&task).await {
        Some((name, link)) => (Some(name), link),
        None => (None, None),
    };
    Ok(Json(TaskState {
        uncommitted: uncommitted(&task).await?,
        preparing: crate::worktree_setup::preparing(&state, &task.worktree),
        branch: task.branch,
        base: task.base,
        worktree: task.worktree,
        ahead,
        behind,
        files,
        files_truncated,
        base_checkout,
        base_is_workspace,
        upstream,
        compare_url,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DiffQuery {
    path: Option<String>,
}

/// Everything the task changed since it left its base, committed or not, as
/// one diff from the worktree; untracked files are listed in its state.
async fn task_diff(
    State(state): State<AppState>,
    Path(id): Path<SessionId>,
    Query(query): Query<DiffQuery>,
) -> Result<Json<GitDiff>> {
    let session = session_record(&state, id).await?;
    let task = task_of(&state, &session).await?;
    let since = merge_base(&task).await?;
    let mut args: Vec<String> = [
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--no-renames",
        &since,
    ]
    .map(str::to_owned)
    .into();
    if let Some(path) = &query.path {
        let relative = workspace_io::safe_relative(path)?;
        if workspace_io::is_protected_path(&relative) {
            return Err(ApiError::bad("Invalid path"));
        }
        args.extend(["--".to_owned(), path.clone()]);
    }
    let (stdout, stderr, success) = run_git(FsPath::new(&task.worktree), args, GIT_TIMEOUT).await?;
    if !success {
        return Err(ApiError::conflict(git_error(&stderr)));
    }
    let truncated = stdout.len() > MAX_DIFF;
    let mut diff = String::from_utf8_lossy(&stdout[..stdout.len().min(MAX_DIFF)]).into_owned();
    if truncated && let Some(cut) = diff.rfind("\ndiff --git ") {
        diff.truncate(cut + 1);
    }
    Ok(Json(GitDiff {
        binary: diff.contains("Binary files"),
        diff,
        path: query.path,
        staged: false,
        truncated,
    }))
}

/// A session is about to lose its worktree: idle or stopped, and not a
/// terminal still running in it.
async fn settle(state: &AppState, session: &Session) -> Result<()> {
    if state
        .runtime
        .get(&session.id.to_string())
        .is_some_and(|runtime| runtime.running())
    {
        return Err(ApiError::conflict(
            "End the native terminal before finishing its task",
        ));
    }
    crate::conversations::stop_if_idle(state, session.id).await
}

/// Files `merge-tree` says would conflict, or `None` when the two merge
/// cleanly. Git too old for `merge-tree --write-tree` answers `Err`.
async fn conflicts(repo: &FsPath, base: &str, branch: &str) -> Result<Option<Vec<String>>> {
    let (stdout, stderr, success) = run_git(
        repo,
        [
            "merge-tree",
            "--write-tree",
            "--name-only",
            "--no-messages",
            base,
            branch,
        ]
        .map(str::to_owned)
        .into(),
        COMMIT_TIMEOUT,
    )
    .await?;
    if success {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&stdout);
    let mut lines = text.lines();
    // A tree id first, then the conflicted files; anything else is an error.
    if lines
        .next()
        .is_some_and(|tree| tree.len() >= 40 && tree.chars().all(|c| c.is_ascii_hexdigit()))
    {
        return Ok(Some(
            lines.filter(|l| !l.is_empty()).map(str::to_owned).collect(),
        ));
    }
    Err(ApiError::conflict(git_error(&stderr)))
}

/// The merged tree `merge-tree` writes, for a merge made without a checkout.
async fn merged_tree(repo: &FsPath, base: &str, branch: &str) -> Result<String> {
    let tree = git_text(repo, &["merge-tree", "--write-tree", base, branch])
        .await?
        .and_then(|out| out.lines().next().map(str::to_owned))
        .filter(|tree| !tree.is_empty())
        .ok_or_else(|| ApiError::conflict("Git could not merge without a checkout"))?;
    Ok(tree)
}

async fn git(root: &FsPath, args: &[&str]) -> Result<String> {
    let (stdout, stderr, success) = run_git(
        root,
        args.iter().map(|arg| (*arg).to_owned()).collect(),
        COMMIT_TIMEOUT,
    )
    .await?;
    if success {
        Ok(String::from_utf8_lossy(&stdout).trim().to_owned())
    } else {
        let said = [&stdout[..], &stderr[..]]
            .iter()
            .map(|bytes| String::from_utf8_lossy(bytes).trim().to_owned())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        Err(ApiError::conflict(if said.is_empty() {
            git_error(&stderr)
        } else {
            said
        }))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MergeInput {
    /// One commit on the base instead of the task's history.
    #[serde(default)]
    squash: bool,
    /// The squashed commit's message; default is the session's title.
    #[serde(default)]
    message: Option<String>,
}

/// Merge the task into its base, then put the task away: its worktree and
/// branch removed, the session archived.
async fn merge_task(
    State(state): State<AppState>,
    Path(id): Path<SessionId>,
    Json(input): Json<MergeInput>,
) -> Result<Json<Session>> {
    let _guard = state.operations.lock().await;
    let session = session_record(&state, id).await?;
    let task = task_of(&state, &session).await?;
    if crate::worktree_setup::preparing(&state, &task.worktree) {
        return Err(ApiError::conflict(
            "The task's worktree is still being prepared",
        ));
    }
    settle(&state, &session).await?;
    let dirty = uncommitted(&task).await?;
    if dirty > 0 {
        return Err(ApiError::conflict(format!(
            "{dirty} file(s) in the task's worktree are not committed. Commit them, or ask its agent to, before merging."
        )));
    }
    let (ahead, _) = ahead_behind(&task).await?;
    if ahead > 0 {
        if let Some(files) = conflicts(&task.repo, &task.base, &task.branch).await? {
            return Err(ApiError::conflict(format!(
                "{} and {} change the same lines in {}. Bring the task up to date first: ask its agent to rebase it onto {}, or merge {} into it, and resolve the conflicts.",
                task.branch,
                task.base,
                files.join(", "),
                task.base,
                task.base
            )));
        }
        let message = input
            .message
            .map(|m| m.trim().to_owned())
            .filter(|m| !m.is_empty())
            .unwrap_or_else(|| session.title.trim().to_owned());
        let message = if message.is_empty() {
            format!("Merge {}", task.branch)
        } else {
            message
        };
        match checked_out(&task.repo, &task.base).await? {
            Some(place) => {
                let place = PathBuf::from(place);
                if workspace_io::git_operation(&place).await.is_some() {
                    return Err(ApiError::conflict(format!(
                        "A merge or rebase is in progress where {} is checked out. Finish or abort it first.",
                        task.base
                    )));
                }
                if input.squash {
                    // What is staged there already would go into the squashed commit.
                    if git_text(&place, &["diff", "--cached", "--quiet"])
                        .await?
                        .is_none()
                    {
                        return Err(ApiError::conflict(format!(
                            "Changes are staged where {} is checked out. Commit or unstage them before squashing a task into it.",
                            task.base
                        )));
                    }
                    git(&place, &["merge", "--squash", &task.branch]).await?;
                    if let Err(error) = git(&place, &["commit", "-m", &message]).await {
                        // A hook refused the commit: what the squash staged
                        // goes, so the base is left as it was found.
                        let _ = git(&place, &["reset", "--merge"]).await;
                        return Err(error);
                    }
                } else if let Err(error) = git(&place, &["merge", "--no-edit", &task.branch]).await
                {
                    if workspace_io::git_operation(&place).await.is_some() {
                        let _ = git(&place, &["merge", "--abort"]).await;
                    }
                    return Err(error);
                }
            }
            None => {
                // No working tree to merge in: the merged commit is made from
                // trees alone and the base moved to it, if it has not moved.
                let old = git(
                    &task.repo,
                    &[
                        "rev-parse",
                        "--verify",
                        &format!("refs/heads/{}", task.base),
                    ],
                )
                .await?;
                let tip = git(
                    &task.repo,
                    &[
                        "rev-parse",
                        "--verify",
                        &format!("refs/heads/{}", task.branch),
                    ],
                )
                .await?;
                let fast = git_text(&task.repo, &["merge-base", "--is-ancestor", &old, &tip])
                    .await?
                    .is_some();
                let commit = if fast && !input.squash {
                    tip
                } else {
                    let tree = merged_tree(&task.repo, &task.base, &task.branch).await?;
                    let mut args = vec!["commit-tree", &tree, "-p", &old];
                    let merge_message =
                        format!("Merge branch '{}' into {}", task.branch, task.base);
                    if !input.squash {
                        args.extend(["-p", &tip, "-m", &merge_message]);
                    } else {
                        args.extend(["-m", &message]);
                    }
                    git(&task.repo, &args).await?
                };
                git(
                    &task.repo,
                    &[
                        "update-ref",
                        &format!("refs/heads/{}", task.base),
                        &commit,
                        &old,
                    ],
                )
                .await?;
            }
        }
    }
    put_away(&state, &session, &task, false).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CloseInput {
    /// Throw away the task's work: uncommitted changes and commits not merged.
    #[serde(default)]
    discard: bool,
}

/// Put a task away without merging it: one that has nothing left to merge --
/// merged elsewhere, by a pull request say -- or, with `discard`, any.
async fn close_task(
    State(state): State<AppState>,
    Path(id): Path<SessionId>,
    Json(input): Json<CloseInput>,
) -> Result<Json<Session>> {
    let _guard = state.operations.lock().await;
    let session = session_record(&state, id).await?;
    let task = task_of(&state, &session).await?;
    if crate::worktree_setup::preparing(&state, &task.worktree) {
        return Err(ApiError::conflict(
            "The task's worktree is still being prepared",
        ));
    }
    settle(&state, &session).await?;
    if !input.discard {
        let dirty = uncommitted(&task).await?;
        let (ahead, _) = ahead_behind(&task).await?;
        if dirty > 0 || ahead > 0 {
            return Err(ApiError::conflict(format!(
                "The task still has work not on {}: {ahead} commit(s) and {dirty} uncommitted file(s). Merge it, or discard it.",
                task.base
            )));
        }
    }
    put_away(&state, &session, &task, input.discard).await
}

/// The worktree removed, the branch deleted, the session back in the
/// workspace directory and archived. Any other session left bound to that
/// worktree goes home too; one running there refuses all of it.
///
/// The caller has made sure the branch's work is on the base, or is being
/// thrown away (`discard`), so the branch goes by force: Git counts a branch
/// merged only into the branch checked out here, and never a squash.
async fn put_away(
    state: &AppState,
    session: &Session,
    task: &Task,
    discard: bool,
) -> Result<Json<Session>> {
    let workspace_id = session.workspace_id;
    let (own, path) = (session.id, task.worktree.clone());
    let others: Vec<Session> = db(state, move |s| s.list_sessions(Some(workspace_id)))
        .await?
        .into_iter()
        .filter(|other| other.id != own && other.checkout_path.as_deref() == Some(path.as_str()))
        .collect();
    if others.iter().any(|other| {
        state.chats.get(other.id).is_some_and(|r| r.running())
            || state
                .runtime
                .get(&other.id.to_string())
                .is_some_and(|r| r.running())
    }) {
        return Err(ApiError::conflict(
            "Another session is running in this task's worktree. End it or move it first.",
        ));
    }
    if FsPath::new(&task.worktree).exists() {
        workspace_io::git_worktree_remove(&task.repo, &task.worktree, discard).await?;
    } else {
        let _ = git_text(&task.repo, &["worktree", "prune"]).await;
    }
    workspace_io::git_branch_delete(&task.repo, &task.branch, true).await?;
    let current = workspace_io::git_branches(&task.repo).await?.current;
    let label = current.clone().unwrap_or_else(|| "Detached HEAD".into());
    for moved in others.iter().map(|other| other.id).chain([own]) {
        let (branch, label) = (current.clone(), label.clone());
        db(state, move |s| {
            s.set_session_checkout(moved, None, branch.as_deref(), None, &label)
        })
        .await?;
        crate::conversations::publish_snapshot(state, moved).await;
    }
    let archived = db(state, move |s| s.set_session_archived(own, true))
        .await?
        .ok_or_else(|| ApiError::missing("Session"))?;
    Ok(Json(archived))
}

/// A temporary isolated session is being discarded: its worktree and branch
/// go with it when they hold nothing -- no commits past the base, nothing
/// uncommitted. Work is never thrown away this way; it stays for the person.
pub async fn forget(state: &AppState, session: &Session) {
    let Ok(task) = task_of(state, session).await else {
        return;
    };
    let empty = matches!(ahead_behind(&task).await, Ok((0, _)))
        && matches!(uncommitted(&task).await, Ok(0));
    let workspace_id = session.workspace_id;
    let (own, path) = (session.id, task.worktree.clone());
    let shared = db(state, move |s| s.list_sessions(Some(workspace_id)))
        .await
        .map(|all| {
            all.iter().any(|other| {
                other.id != own && other.checkout_path.as_deref() == Some(path.as_str())
            })
        })
        .unwrap_or(true);
    if !empty || shared || crate::worktree_setup::preparing(state, &task.worktree) {
        return;
    }
    if workspace_io::git_worktree_remove(&task.repo, &task.worktree, false)
        .await
        .is_ok()
    {
        // Nothing past the base, checked above; Git would ask for it merged into HEAD.
        let _ = workspace_io::git_branch_delete(&task.repo, &task.branch, true).await;
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/sessions/{id}/task", get(task_state))
        .route("/api/sessions/{id}/task/diff", get(task_diff))
        .route("/api/sessions/{id}/task/merge", post(merge_task))
        .route("/api/sessions/{id}/task/close", post(close_task))
}

#[cfg(test)]
mod tests {
    use super::compare_url;

    #[test]
    fn compare_links_only_for_github() {
        assert_eq!(
            compare_url("git@github.com:wnzzer/tendly.git", "main", "tendly/fix").as_deref(),
            Some("https://github.com/wnzzer/tendly/compare/main...tendly/fix?expand=1")
        );
        assert_eq!(
            compare_url("https://github.com/wnzzer/tendly", "main", "a#b").as_deref(),
            Some("https://github.com/wnzzer/tendly/compare/main...a%23b?expand=1")
        );
        assert!(compare_url("https://gitlab.com/a/b.git", "main", "x").is_none());
        assert!(compare_url("https://github.com/a/b/c", "main", "x").is_none());
    }
}
