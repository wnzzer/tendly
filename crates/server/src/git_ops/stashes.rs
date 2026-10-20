//! Stashes: listing, stashing, applying or popping, and dropping them.

use super::*;

#[derive(Serialize)]
pub(super) struct Stash {
    index: u32,
    hash: String,
    message: String,
    date: i64,
    /// It holds untracked files too.
    untracked: bool,
}

async fn list_stashes(root: &std::path::Path) -> Result<Vec<Stash>> {
    let (stdout, _, success) = run_git(
        root,
        args(&[
            "stash",
            "list",
            "-z",
            "--format=%gd%x1f%H%x1f%ct%x1f%P%x1f%gs",
        ]),
        GIT_TIMEOUT,
    )
    .await?;
    if !success {
        return Ok(Vec::new());
    }
    Ok(String::from_utf8_lossy(&stdout)
        .split('\0')
        .filter_map(|record| {
            let mut fields = record.trim_start_matches('\n').splitn(5, '\x1f');
            let index = fields
                .next()?
                .strip_prefix("stash@{")?
                .strip_suffix('}')?
                .parse()
                .ok()?;
            Some(Stash {
                index,
                hash: fields.next()?.to_owned(),
                date: fields.next()?.parse().unwrap_or_default(),
                untracked: fields.next()?.split(' ').count() > 2,
                message: fields.next()?.to_owned(),
            })
        })
        .collect())
}

pub(super) async fn stashes(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
) -> Result<Json<Vec<Stash>>> {
    Ok(Json(
        list_stashes(&repo(&state, id, &checkout).await?).await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StashInput {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    include_untracked: bool,
    /// Leave what is staged in place as well as stashing it.
    #[serde(default)]
    keep_index: bool,
    /// Only these files; every change when empty.
    #[serde(default)]
    paths: Vec<String>,
}

pub(super) async fn stash_push(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Json(input): Json<StashInput>,
) -> Result<Json<Vec<Stash>>> {
    let _guard = state.operations.lock().await;
    let root = repo(&state, id, &checkout).await?;
    let mut list = args(&["stash", "push"]);
    if input.include_untracked {
        list.push("--include-untracked".into());
    }
    if input.keep_index {
        list.push("--keep-index".into());
    }
    if let Some(message) = input
        .message
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
    {
        list.extend(["-m".into(), message.to_owned()]);
    }
    if input.paths.len() > 1000 {
        return Err(ApiError::bad("Too many paths"));
    }
    if !input.paths.is_empty() {
        list.push("--".into());
        for path in &input.paths {
            list.push(format!(":(literal){}", checked_path(path)?));
        }
    }
    let before = list_stashes(&root).await?.first().map(|s| s.hash.clone());
    git(&root, list, COMMIT_TIMEOUT).await?;
    let after = list_stashes(&root).await?;
    if after.first().map(|s| &s.hash) == before.as_ref() {
        return Err(ApiError::conflict("There were no local changes to stash."));
    }
    Ok(Json(after))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StashRef {
    index: u32,
    /// The stash the person saw: refused if the list moved underneath them.
    hash: String,
    /// Apply and drop it, rather than only apply.
    #[serde(default)]
    pop: bool,
}

async fn stash_entry(root: &std::path::Path, input: &StashRef) -> Result<String> {
    let name = format!("stash@{{{}}}", input.index);
    let hash = git_text(root, &["rev-parse", "--verify", "--quiet", &name]).await?;
    if hash.as_deref() != Some(input.hash.as_str()) {
        return Err(ApiError::conflict(
            "The stash list changed. Refresh and try again.",
        ));
    }
    Ok(name)
}

pub(super) async fn stash_apply(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Json(input): Json<StashRef>,
) -> Result<Json<Outcome>> {
    let _guard = state.operations.lock().await;
    let root = repo(&state, id, &checkout).await?;
    let name = stash_entry(&root, &input).await?;
    let verb = if input.pop { "pop" } else { "apply" };
    match finish(
        &root,
        args(&["stash", verb, "--index", &name]),
        COMMIT_TIMEOUT,
    )
    .await
    {
        // `--index` refuses, changing nothing, when the staged part no longer
        // applies; the changes themselves may still, unstaged.
        Err(_) => finish(&root, args(&["stash", verb, &name]), COMMIT_TIMEOUT).await,
        done => done,
    }
}

pub(super) async fn stash_drop(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Json(input): Json<StashRef>,
) -> Result<Json<Vec<Stash>>> {
    let _guard = state.operations.lock().await;
    let root = repo(&state, id, &checkout).await?;
    let name = stash_entry(&root, &input).await?;
    git(&root, args(&["stash", "drop", &name]), GIT_TIMEOUT).await?;
    Ok(Json(list_stashes(&root).await?))
}
