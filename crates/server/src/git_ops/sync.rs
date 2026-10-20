//! Sync with remotes: fetch, pull and push.

use super::*;

/// Fetch, pull and push wait on a remote that may be slow or far away.
const NETWORK_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FetchInput {
    /// One remote; every remote when absent.
    #[serde(default)]
    remote: Option<String>,
}

pub(super) async fn fetch(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Json(input): Json<FetchInput>,
) -> Result<StatusCode> {
    let root = repo(&state, id, &checkout).await?;
    let mut list = args(&["fetch", "--prune", "--tags"]);
    match input
        .remote
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty())
    {
        Some(remote) => list.push(known_remote(&root, remote).await?),
        None => list.push("--all".into()),
    }
    git(&root, list, NETWORK_TIMEOUT).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PullInput {
    /// `merge`, `rebase` or `ff-only`; the repository's own setting when
    /// absent, and a merge when it has none.
    #[serde(default)]
    mode: Option<String>,
}

pub(super) async fn pull(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Json(input): Json<PullInput>,
) -> Result<Json<Outcome>> {
    let _guard = state.operations.lock().await;
    let root = repo(&state, id, &checkout).await?;
    let mut list = args(&["pull", "--no-edit"]);
    match input.mode.as_deref() {
        Some("merge") => list.push("--no-rebase".into()),
        Some("rebase") => list.push("--rebase".into()),
        Some("ff-only") => list.push("--ff-only".into()),
        Some(_) => return Err(ApiError::bad("Unknown pull mode")),
        None => {
            let configured = git_text(&root, &["config", "--get", "pull.rebase"])
                .await?
                .is_some()
                || git_text(&root, &["config", "--get", "pull.ff"])
                    .await?
                    .is_some();
            if !configured {
                list.push("--no-rebase".into());
            }
        }
    }
    finish(&root, list, NETWORK_TIMEOUT).await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PushInput {
    #[serde(default)]
    remote: Option<String>,
    /// A branch other than the current one; with `delete`, the remote branch
    /// to delete.
    #[serde(default)]
    branch: Option<String>,
    /// Push the current branch and make it track what it was pushed to.
    #[serde(default)]
    set_upstream: bool,
    /// Overwrite the remote branch, but only if it is where it was last seen.
    #[serde(default)]
    force: bool,
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    delete: bool,
}

pub(super) async fn push(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Json(input): Json<PushInput>,
) -> Result<Json<Outcome>> {
    let root = repo(&state, id, &checkout).await?;
    let remote = match input
        .remote
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty())
    {
        Some(remote) => Some(known_remote(&root, remote).await?),
        None => None,
    };
    // Naming no remote leaves the choice to Git and the branch's upstream;
    // anything that names a ref needs one, and takes the only or `origin`.
    let default_remote = || async {
        let remotes = list_remotes(&root).await?;
        remotes
            .iter()
            .find(|r| r.name == "origin")
            .or(remotes.first())
            .map(|r| r.name.clone())
            .ok_or_else(|| ApiError::bad("This repository has no remote. Add one first."))
    };
    let mut list = args(&["push", "--porcelain"]);
    if let Some(tag) = input.tag.as_deref().map(str::trim) {
        valid_tag(&root, tag).await?;
        list.push(match remote {
            Some(remote) => remote,
            None => default_remote().await?,
        });
        list.push(format!(
            "{}refs/tags/{tag}",
            if input.delete { ":" } else { "" }
        ));
    } else if input.delete {
        let branch = input.branch.as_deref().map(str::trim).unwrap_or_default();
        valid_branch(&root, branch).await?;
        list.push(match remote {
            Some(remote) => remote,
            None => default_remote().await?,
        });
        list.push(format!(":refs/heads/{branch}"));
    } else {
        if input.force {
            list.push("--force-with-lease".into());
        }
        let branch = match input
            .branch
            .as_deref()
            .map(str::trim)
            .filter(|b| !b.is_empty())
        {
            Some(branch) => Some(branch.to_owned()),
            None if input.set_upstream => Some(
                git_text(&root, &["symbolic-ref", "--short", "-q", "HEAD"])
                    .await?
                    .filter(|b| !b.is_empty())
                    .ok_or_else(|| ApiError::bad("HEAD is detached: there is no branch to push"))?,
            ),
            None => None,
        };
        if input.set_upstream {
            list.push("--set-upstream".into());
        }
        match (remote, branch) {
            (remote, Some(branch)) => {
                valid_branch(&root, &branch).await?;
                list.push(match remote {
                    Some(remote) => remote,
                    None => default_remote().await?,
                });
                list.push(format!("refs/heads/{branch}:refs/heads/{branch}"));
            }
            (Some(remote), None) => list.push(remote),
            (None, None) => {}
        }
    }
    let (stdout, stderr, success) = run_git(&root, list, NETWORK_TIMEOUT).await?;
    if success {
        Ok(Json(Outcome {
            conflicts: false,
            message: said(&[], &stderr),
        }))
    } else {
        let message = said(&stdout, &stderr);
        Err(IoError::new(
            409,
            if message.is_empty() {
                git_error(&stderr)
            } else {
                message
            },
        )
        .into())
    }
}
