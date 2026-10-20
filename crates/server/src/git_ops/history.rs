//! History: the log, one commit's details and the files it changed, and one
//! file's change in a commit or a stash.

use super::*;

const MAX_LOG: usize = 500;
const MAX_FILES: usize = 3000;

#[derive(Debug, Serialize)]
pub(super) struct Commit {
    hash: String,
    parents: Vec<String>,
    author: String,
    email: String,
    /// Author time, in seconds.
    date: i64,
    /// Branches, tags and HEAD pointing here, as Git decorates them.
    refs: Vec<String>,
    subject: String,
}

#[derive(Serialize)]
pub(super) struct Log {
    commits: Vec<Commit>,
    /// More commits follow the last one listed.
    more: bool,
}

#[derive(Deserialize)]
pub(super) struct LogQuery {
    /// Start here instead of HEAD.
    rev: Option<String>,
    /// Only commits that changed this path.
    path: Option<String>,
    #[serde(default)]
    all: bool,
    #[serde(default)]
    skip: usize,
    limit: Option<usize>,
}

pub(super) fn decorations(raw: &str) -> Vec<String> {
    raw.split(", ")
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(super) fn parse_log(raw: &str) -> Vec<Commit> {
    raw.split('\0')
        .filter_map(|record| {
            let record = record.trim_start_matches('\n');
            let mut fields = record.splitn(7, '\x1f');
            let hash = fields.next().filter(|hash| !hash.is_empty())?.to_owned();
            Some(Commit {
                hash,
                parents: fields
                    .next()?
                    .split(' ')
                    .filter(|p| !p.is_empty())
                    .map(str::to_owned)
                    .collect(),
                author: fields.next()?.to_owned(),
                email: fields.next()?.to_owned(),
                date: fields.next()?.parse().unwrap_or_default(),
                refs: decorations(fields.next()?),
                subject: fields.next()?.trim_end().to_owned(),
            })
        })
        .collect()
}

pub(super) async fn log(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Query(q): Query<LogQuery>,
) -> Result<Json<Log>> {
    let root = repo(&state, id, &checkout).await?;
    let limit = q.limit.unwrap_or(100).clamp(1, MAX_LOG);
    let mut list = args(&[
        "log",
        "-z",
        "--date-order",
        "--decorate=short",
        "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%D%x1f%s",
    ]);
    list.push(format!("--skip={}", q.skip));
    list.push(format!("--max-count={}", limit + 1));
    if q.all {
        list.extend(args(&["--exclude=refs/stash", "--all"]));
    } else if let Some(rev) = q.rev.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
        list.push(valid_rev(&root, rev).await?);
    } else if valid_rev(&root, "HEAD").await.is_err() {
        // A repository with no commits yet has no history to show.
        return Ok(Json(Log {
            commits: Vec::new(),
            more: false,
        }));
    }
    if let Some(path) = q.path.as_deref().filter(|p| !p.is_empty()) {
        list.extend(["--".into(), checked_path(path)?]);
    }
    let mut commits = parse_log(&git(&root, list, GIT_TIMEOUT).await?);
    let more = commits.len() > limit;
    commits.truncate(limit);
    Ok(Json(Log { commits, more }))
}

#[derive(Debug, Serialize)]
pub(super) struct ChangedFile {
    /// A, M, D, R, C or T, as `git diff --name-status` reports it.
    status: String,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    original_path: Option<String>,
    /// Lines added and removed; absent for a binary file.
    #[serde(skip_serializing_if = "Option::is_none")]
    added: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    removed: Option<u32>,
    /// From a stash: a file that was untracked when it was stashed.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    untracked: bool,
}

#[derive(Serialize)]
pub(super) struct CommitDetail {
    hash: String,
    parents: Vec<String>,
    author: String,
    email: String,
    date: i64,
    committer: String,
    committer_email: String,
    commit_date: i64,
    refs: Vec<String>,
    message: String,
    files: Vec<ChangedFile>,
    /// More files changed than are listed.
    truncated: bool,
}

#[derive(Deserialize)]
pub(super) struct CommitQuery {
    rev: String,
    /// Read the commit as a stash: its untracked files are listed too.
    #[serde(default)]
    stash: bool,
    path: Option<String>,
    original_path: Option<String>,
    /// From a stash: the file was untracked.
    #[serde(default)]
    untracked: bool,
}

/// The two sides of a commit's diff: its first parent, or nothing for a root
/// commit. A merge is shown against the branch it was merged into.
pub(super) fn diff_args(hash: &str, parent: Option<&str>, extra: &[&str]) -> Vec<String> {
    let mut list = match parent {
        Some(parent) => vec!["diff".to_owned(), parent.to_owned(), hash.to_owned()],
        None => args(&["diff-tree", "--root", "-r", "--no-commit-id", hash]),
    };
    list.extend(args(&["-M", "--no-ext-diff", "--no-color"]));
    list.extend(args(extra));
    list
}

pub(super) fn parse_name_status(raw: &str, untracked: bool) -> Vec<ChangedFile> {
    let mut tokens = raw.split('\0').filter(|token| !token.is_empty());
    let mut files = Vec::new();
    while let Some(code) = tokens.next() {
        let status = code.chars().next().unwrap_or('M').to_string();
        let first = tokens.next().unwrap_or_default().to_owned();
        let (path, original_path) = if matches!(status.as_str(), "R" | "C") {
            (tokens.next().unwrap_or_default().to_owned(), Some(first))
        } else {
            (first, None)
        };
        files.push(ChangedFile {
            status: if untracked { "A".into() } else { status },
            path,
            original_path,
            added: None,
            removed: None,
            untracked,
        });
    }
    files
}

/// `--numstat -z`: `added\tremoved\tpath\0`, or for a rename
/// `added\tremoved\t\0from\0to\0`. A binary file counts `-`.
pub(super) fn apply_numstat(raw: &str, files: &mut [ChangedFile]) {
    let mut tokens = raw.split('\0');
    while let Some(head) = tokens.next() {
        let mut parts = head.splitn(3, '\t');
        let (Some(added), Some(removed), Some(path)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let path = if path.is_empty() {
            tokens.next();
            tokens.next().unwrap_or_default()
        } else {
            path
        };
        if let Some(file) = files.iter_mut().find(|file| file.path == path) {
            file.added = added.parse().ok();
            file.removed = removed.parse().ok();
        }
    }
}

async fn changed_files(
    root: &std::path::Path,
    hash: &str,
    parent: Option<&str>,
    untracked: bool,
) -> Result<Vec<ChangedFile>> {
    let names = git(
        root,
        diff_args(hash, parent, &["--name-status", "-z"]),
        GIT_TIMEOUT,
    )
    .await?;
    let mut files = parse_name_status(&names, untracked);
    if files.len() <= MAX_FILES {
        let counts = git(
            root,
            diff_args(hash, parent, &["--numstat", "-z"]),
            GIT_TIMEOUT,
        )
        .await?;
        apply_numstat(&counts, &mut files);
    }
    Ok(files)
}

pub(super) async fn commit_detail(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Query(q): Query<CommitQuery>,
) -> Result<Json<CommitDetail>> {
    let root = repo(&state, id, &checkout).await?;
    let hash = valid_rev(&root, q.rev.trim()).await?;
    let raw = git(
        &root,
        vec![
            "show".into(),
            "-s".into(),
            "--decorate=short".into(),
            "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%cn%x1f%ce%x1f%ct%x1f%D%x1f%B".into(),
            hash.clone(),
        ],
        GIT_TIMEOUT,
    )
    .await?;
    let fields: Vec<&str> = raw.splitn(10, '\x1f').collect();
    let [
        _,
        parents,
        author,
        email,
        date,
        committer,
        committer_email,
        commit_date,
        refs,
        message,
    ] = fields[..]
    else {
        return Err(ApiError::internal("unreadable commit"));
    };
    let parents: Vec<String> = parents
        .split(' ')
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect();
    let mut files = changed_files(&root, &hash, parents.first().map(String::as_str), false).await?;
    // A stash keeps the untracked files it took in a third, parentless commit.
    if q.stash
        && let Some(third) = parents.get(2)
    {
        files.extend(changed_files(&root, third, None, true).await?);
    }
    let truncated = files.len() > MAX_FILES;
    files.truncate(MAX_FILES);
    Ok(Json(CommitDetail {
        hash,
        parents,
        author: author.to_owned(),
        email: email.to_owned(),
        date: date.parse().unwrap_or_default(),
        committer: committer.to_owned(),
        committer_email: committer_email.to_owned(),
        commit_date: commit_date.parse().unwrap_or_default(),
        refs: decorations(refs),
        message: message.trim_end().to_owned(),
        files,
        truncated,
    }))
}

/// One file's change in a commit (or a stash), for the diff view.
pub(super) async fn commit_diff(
    State(state): State<AppState>,
    Path(id): Path<WorkspaceId>,
    checkout: CheckoutParam,
    Query(q): Query<CommitQuery>,
) -> Result<Json<GitDiff>> {
    let root = repo(&state, id, &checkout).await?;
    let hash = valid_rev(&root, q.rev.trim()).await?;
    let path = checked_path(q.path.as_deref().unwrap_or_default())?;
    let parents: Vec<String> = git_text(&root, &["rev-list", "--parents", "-n", "1", &hash])
        .await?
        .unwrap_or_default()
        .split(' ')
        .skip(1)
        .map(str::to_owned)
        .collect();
    let (target, parent) = if q.untracked {
        let third = parents
            .get(2)
            .ok_or_else(|| ApiError::bad("This stash has no untracked files"))?;
        (third.clone(), None)
    } else {
        (hash, parents.first().cloned())
    };
    let mut list = diff_args(&target, parent.as_deref(), &["-p", "--no-textconv", "--"]);
    list.push(path.clone());
    if let Some(original) = q.original_path.as_deref().filter(|p| !p.is_empty()) {
        list.push(checked_path(original)?);
    }
    let diff = git(&root, list, GIT_TIMEOUT).await?;
    Ok(Json(GitDiff {
        binary: diff.contains("Binary files") || diff.contains("GIT binary patch"),
        diff,
        path: Some(path),
        staged: false,
        truncated: false,
    }))
}
