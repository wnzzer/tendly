//! Branches, worktrees and commits.

use super::*;

#[derive(Debug, Clone, Serialize)]
pub struct GitWorktree {
    pub path: String,
    pub branch: Option<String>,
    /// The repository's own checkout, as opposed to one added beside it.
    pub main: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct GitBranches {
    /// None when HEAD is detached.
    pub current: Option<String>,
    pub branches: Vec<String>,
    pub worktrees: Vec<GitWorktree>,
    /// The local branches again, with where each stands against its upstream.
    pub local: Vec<GitRef>,
    /// Remote-tracking branches, as `origin/main`.
    pub remote: Vec<GitRef>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GitRef {
    pub name: String,
    pub hash: String,
    pub subject: String,
    /// Committer time of the tip, in seconds.
    pub date: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ahead: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub behind: Option<u32>,
    /// The upstream it tracked has been deleted on the remote.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub gone: bool,
}

/// Branch tips, local and remote-tracking, read in one `for-each-ref`.
pub(super) async fn git_refs(root: &Path) -> Result<(Vec<GitRef>, Vec<GitRef>), IoError> {
    let listing = git_text(
        root,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname)%1f%(symref)%1f%(objectname)%1f%(committerdate:unix)%1f%(upstream:short)%1f%(upstream:track,nobracket)%1f%(subject)",
            "refs/heads",
            "refs/remotes",
        ],
    )
    .await?
    .unwrap_or_default();
    let (mut local, mut remote) = (Vec::new(), Vec::new());
    for line in listing.lines() {
        let fields: Vec<&str> = line.split('\x1f').collect();
        let [refname, symref, hash, date, upstream, track, subject] = fields[..] else {
            continue;
        };
        if !symref.is_empty() {
            continue;
        }
        let count = |label: &str| {
            track
                .split(", ")
                .find_map(|part| part.strip_prefix(label))
                .and_then(|n| n.trim().parse().ok())
        };
        let mut entry = GitRef {
            name: String::new(),
            hash: hash.to_owned(),
            subject: subject.to_owned(),
            date: date.parse().unwrap_or_default(),
            upstream: (!upstream.is_empty()).then(|| upstream.to_owned()),
            ahead: count("ahead "),
            behind: count("behind "),
            gone: track == "gone",
        };
        if let Some(name) = refname.strip_prefix("refs/heads/") {
            entry.name = name.to_owned();
            local.push(entry);
        } else if let Some(name) = refname.strip_prefix("refs/remotes/") {
            entry.name = name.to_owned();
            remote.push(entry);
        }
    }
    Ok((local, remote))
}

pub(crate) async fn git_text(root: &Path, args: &[&str]) -> Result<Option<String>, IoError> {
    let (stdout, _stderr, success) = run_git(
        root,
        args.iter().map(|arg| (*arg).to_owned()).collect(),
        GIT_TIMEOUT,
    )
    .await?;
    Ok(success.then(|| String::from_utf8_lossy(&stdout).trim().to_owned()))
}

/// Local branches and the worktrees of this repository.
pub async fn git_branches(root: &Path) -> Result<GitBranches, IoError> {
    let root = dunce::canonicalize(root).map_err(IoError::from)?;
    let branches = git_text(&root, &["branch", "--format=%(refname:short)"])
        .await?
        .ok_or_else(|| IoError::new(400, "Not a Git repository"))?;
    let current = git_text(&root, &["symbolic-ref", "--short", "-q", "HEAD"])
        .await?
        .filter(|name| !name.is_empty());
    let listing = git_text(&root, &["worktree", "list", "--porcelain"])
        .await?
        .unwrap_or_default();
    let (local, remote) = git_refs(&root).await?;
    Ok(GitBranches {
        local,
        remote,
        current,
        branches: branches
            .lines()
            .map(str::trim)
            .filter(|name| !name.is_empty() && !name.starts_with('('))
            .map(str::to_owned)
            .collect(),
        worktrees: parse_worktrees(&listing),
    })
}

pub(crate) fn parse_worktrees(listing: &str) -> Vec<GitWorktree> {
    listing
        .split("\n\n")
        .filter_map(|block| {
            let mut path = None;
            let mut branch = None;
            let mut bare = false;
            for line in block.lines() {
                if let Some(value) = line.strip_prefix("worktree ") {
                    // Git for Windows prints `C:/…`; rebuilding from components
                    // gives the native form every other path here is in.
                    let native: PathBuf = Path::new(value).components().collect();
                    path = Some(native.to_string_lossy().into_owned());
                } else if let Some(value) = line.strip_prefix("branch ") {
                    branch = Some(value.trim_start_matches("refs/heads/").to_owned());
                } else if line == "bare" {
                    bare = true;
                }
            }
            (!bare).then_some(())?;
            path.map(|path| GitWorktree {
                path,
                branch,
                main: false,
            })
        })
        .enumerate()
        .map(|(index, mut worktree)| {
            worktree.main = index == 0;
            worktree
        })
        .collect()
}

/// A name Git itself accepts for a branch, checked by Git rather than guessed.
pub(crate) async fn valid_branch(root: &Path, branch: &str) -> Result<(), IoError> {
    if branch.is_empty() || branch.len() > 200 || branch.starts_with('-') {
        return Err(IoError::new(400, "Invalid branch name"));
    }
    match git_text(root, &["check-ref-format", "--branch", branch]).await? {
        Some(_) => Ok(()),
        None => Err(IoError::new(400, "Invalid branch name")),
    }
}

/// A revision -- a commit hash, branch, remote branch or tag -- that names a
/// commit here, resolved to that commit's full hash. Git checks it; nothing
/// shaped like an option ever reaches a command line as one.
pub(crate) async fn valid_rev(root: &Path, rev: &str) -> Result<String, IoError> {
    if rev.is_empty()
        || rev.len() > 256
        || rev.starts_with('-')
        || rev.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(IoError::new(400, "Invalid revision"));
    }
    git_text(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &format!("{rev}^{{commit}}"),
        ],
    )
    .await?
    .filter(|hash| !hash.is_empty())
    .ok_or_else(|| IoError::new(404, format!("{rev} is not a commit in this repository")))
}

/// Switch this checkout to another local branch, or create one here --
/// from HEAD, or from `start` (a commit or a remote branch, which the new
/// branch then tracks). `detach` checks out the commit `branch` names without
/// any branch. Git refuses a switch that would overwrite uncommitted work, and
/// that refusal is passed on rather than forced through.
pub async fn git_switch(
    root: &Path,
    branch: &str,
    create: bool,
    start: Option<&str>,
    detach: bool,
) -> Result<(), IoError> {
    let root = dunce::canonicalize(root).map_err(IoError::from)?;
    let mut args: Vec<String> = vec!["switch".into()];
    if detach {
        args.extend(["--detach".into(), valid_rev(&root, branch).await?]);
    } else {
        valid_branch(&root, branch).await?;
        if create {
            args.push("-c".into());
        }
        args.push(branch.to_owned());
        if let Some(start) = start.filter(|_| create) {
            valid_rev(&root, start).await?;
            args.push(start.to_owned());
        }
    }
    let (_stdout, stderr, success) = run_git(&root, args, GIT_TIMEOUT).await?;
    if success {
        Ok(())
    } else {
        Err(IoError::new(409, git_error(&stderr)))
    }
}

/// Rename a local branch. A worktree that has it checked out follows the new
/// name -- Git updates it -- so no checkout has to move.
pub async fn git_branch_rename(root: &Path, from: &str, to: &str) -> Result<(), IoError> {
    let root = dunce::canonicalize(root).map_err(IoError::from)?;
    valid_branch(&root, from).await?;
    valid_branch(&root, to).await?;
    let args = vec!["branch".into(), "-m".into(), from.to_owned(), to.to_owned()];
    let (_stdout, stderr, success) = run_git(&root, args, GIT_TIMEOUT).await?;
    if success {
        Ok(())
    } else {
        Err(IoError::new(409, git_error(&stderr)))
    }
}

/// Delete a local branch. Git refuses one that is checked out anywhere, and
/// -- unless `force` -- one whose commits are not merged; both are passed on.
pub async fn git_branch_delete(root: &Path, branch: &str, force: bool) -> Result<(), IoError> {
    let root = dunce::canonicalize(root).map_err(IoError::from)?;
    valid_branch(&root, branch).await?;
    let args = vec![
        "branch".into(),
        if force { "-D" } else { "-d" }.into(),
        branch.to_owned(),
    ];
    let (_stdout, stderr, success) = run_git(&root, args, GIT_TIMEOUT).await?;
    if success {
        Ok(())
    } else {
        Err(IoError::new(409, git_error(&stderr)))
    }
}

/// Remove a worktree of this repository -- its directory, not its branch.
/// Git refuses one with uncommitted changes unless `force`; the repository's
/// own checkout can never be removed this way.
pub async fn git_worktree_remove(root: &Path, path: &str, force: bool) -> Result<(), IoError> {
    let root = dunce::canonicalize(root).map_err(IoError::from)?;
    let listing = git_text(&root, &["worktree", "list", "--porcelain"])
        .await?
        .ok_or_else(|| IoError::new(400, "Not a Git repository"))?;
    let tree = parse_worktrees(&listing)
        .into_iter()
        .find(|tree| tree.path == path)
        .ok_or_else(|| IoError::new(400, "Not a worktree of this repository"))?;
    if tree.main {
        return Err(IoError::new(
            400,
            "The repository's own checkout cannot be removed",
        ));
    }
    let mut args: Vec<String> = vec!["worktree".into(), "remove".into()];
    if force {
        args.push("--force".into());
    }
    args.push(tree.path);
    let (_stdout, stderr, success) = run_git(&root, args, GIT_TIMEOUT).await?;
    if success {
        Ok(())
    } else {
        Err(IoError::new(409, git_error(&stderr)))
    }
}

/// Where a new worktree for `branch` goes: beside the repository's own
/// checkout, in `<repo>.worktrees/<branch>`, so every worktree of one
/// repository sits together and none of them inside another.
pub fn worktree_path(main_checkout: &Path, branch: &str) -> Result<PathBuf, IoError> {
    let name = main_checkout
        .file_name()
        .ok_or_else(|| IoError::new(400, "The repository has no directory name"))?;
    let parent = main_checkout
        .parent()
        .ok_or_else(|| IoError::new(400, "The repository has no parent directory"))?;
    let slug: String = branch
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches(|c| c == '-' || c == '.');
    if slug.is_empty() {
        return Err(IoError::new(400, "Invalid branch name"));
    }
    Ok(parent
        .join(format!("{}.worktrees", name.to_string_lossy()))
        .join(slug))
}

/// Add a worktree for `branch` (created from HEAD when `create`), returning
/// its directory. An existing directory is never reused or overwritten.
pub async fn git_worktree_add(root: &Path, branch: &str, create: bool) -> Result<PathBuf, IoError> {
    let root = dunce::canonicalize(root).map_err(IoError::from)?;
    valid_branch(&root, branch).await?;
    let listing = git_text(&root, &["worktree", "list", "--porcelain"])
        .await?
        .ok_or_else(|| IoError::new(400, "Not a Git repository"))?;
    let main = parse_worktrees(&listing)
        .into_iter()
        .find(|worktree| worktree.main)
        .ok_or_else(|| IoError::new(400, "Not a Git repository"))?;
    let target = worktree_path(Path::new(&main.path), branch)?;
    if target.exists() {
        return Err(IoError::new(
            409,
            format!("{} already exists", target.display()),
        ));
    }
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut args: Vec<String> = vec!["worktree".into(), "add".into()];
    if create {
        args.extend([
            "-b".into(),
            branch.to_owned(),
            target.to_string_lossy().into_owned(),
        ]);
    } else {
        args.extend([target.to_string_lossy().into_owned(), branch.to_owned()]);
    }
    let (_stdout, stderr, success) = run_git(&root, args, COMMIT_TIMEOUT).await?;
    if !success {
        return Err(IoError::new(409, git_error(&stderr)));
    }
    Ok(target)
}

pub async fn git_commit(root: &Path, message: &str, amend: bool) -> Result<String, IoError> {
    if message.trim().is_empty() {
        return Err(IoError::new(400, "commit message is required"));
    }
    let root = dunce::canonicalize(root).map_err(IoError::from)?;
    let mut args: Vec<String> = vec!["commit".into()];
    if amend {
        args.push("--amend".into());
    }
    args.extend(["-m".into(), message.to_owned()]);
    let (_stdout, stderr, success) = run_git(&root, args, COMMIT_TIMEOUT).await?;
    if !success {
        return Err(IoError::new(400, git_error(&stderr)));
    }
    let (stdout, stderr, success) =
        run_git(&root, vec!["rev-parse".into(), "HEAD".into()], GIT_TIMEOUT).await?;
    if !success {
        return Err(IoError::new(400, git_error(&stderr)));
    }
    Ok(String::from_utf8_lossy(&stdout).trim().to_owned())
}

/// A local branch that can go, and why: its work already reached the base
/// branch -- merged as it was, squashed into one commit, or rebased commit
/// by commit -- or its upstream was deleted with nothing showing it merged.
#[derive(Debug, Clone, Serialize)]
pub struct CleanupCandidate {
    pub name: String,
    /// "merged", "squashed", "rebased" or "gone".
    pub reason: &'static str,
    /// The worktree that has it checked out, if one does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GitCleanup {
    /// What "merged" is measured against; None when there is nothing to measure by.
    pub base: Option<String>,
    pub branches: Vec<CleanupCandidate>,
}

/// More than this many branches are not checked; the rest wait for a cleanup.
const MAX_CLEANUP_BRANCHES: usize = 200;

/// Where merged work lands: the remote's default branch, as a pull request
/// merges into it, else its main or master, else the local ones, else HEAD.
async fn cleanup_base(root: &Path) -> Result<Option<String>, IoError> {
    if let Some(remote) = git_text(
        root,
        &["symbolic-ref", "--short", "-q", "refs/remotes/origin/HEAD"],
    )
    .await?
    .filter(|name| !name.is_empty())
    {
        return Ok(Some(remote));
    }
    for name in ["origin/main", "origin/master", "main", "master"] {
        let full = if name.contains('/') {
            format!("refs/remotes/{name}")
        } else {
            format!("refs/heads/{name}")
        };
        if git_text(root, &["rev-parse", "--verify", "--quiet", &full])
            .await?
            .is_some_and(|hash| !hash.is_empty())
        {
            return Ok(Some(name.to_owned()));
        }
    }
    Ok(git_text(root, &["symbolic-ref", "--short", "-q", "HEAD"])
        .await?
        .filter(|name| !name.is_empty()))
}

/// How `branch`'s work reached `base`, if it did. A squash is found as
/// git-delete-squashed finds it: the branch is folded into one commit on its
/// merge base -- a dangling object, no ref moves -- and `git cherry` looks
/// upstream for the same patch, which still matches after later commits.
pub(crate) async fn merged_into(
    root: &Path,
    base: &str,
    branch: &str,
) -> Result<Option<&'static str>, IoError> {
    let head = format!("refs/heads/{branch}");
    let (_out, _err, ancestor) = run_git(
        root,
        vec![
            "merge-base".into(),
            "--is-ancestor".into(),
            head.clone(),
            base.to_owned(),
        ],
        GIT_TIMEOUT,
    )
    .await?;
    if ancestor {
        return Ok(Some("merged"));
    }
    let Some(fork) = git_text(root, &["merge-base", base, &head])
        .await?
        .filter(|hash| !hash.is_empty())
    else {
        return Ok(None);
    };
    let rebased = git_text(root, &["cherry", base, &head])
        .await?
        .unwrap_or_default();
    if !rebased.is_empty() && rebased.lines().all(|line| line.starts_with('-')) {
        return Ok(Some("rebased"));
    }
    let tree = format!("{head}^{{tree}}");
    let folded = git_text(
        root,
        &[
            "-c",
            "user.name=Tendly",
            "-c",
            "user.email=tendly@localhost",
            "commit-tree",
            &tree,
            "-p",
            &fork,
            "-m",
            "tendly: squash check",
        ],
    )
    .await?
    .filter(|hash| !hash.is_empty());
    let Some(folded) = folded else {
        return Ok(None);
    };
    let squashed = git_text(root, &["cherry", base, &folded])
        .await?
        .unwrap_or_default();
    Ok(squashed.starts_with('-').then_some("squashed"))
}

/// The local branches that can be cleaned up, other than the one checked out
/// here and the base itself, each with why and the worktree holding it.
pub async fn git_cleanup(root: &Path) -> Result<GitCleanup, IoError> {
    use futures_util::stream::{self, StreamExt};
    let root = dunce::canonicalize(root).map_err(IoError::from)?;
    git_text(&root, &["rev-parse", "--git-dir"])
        .await?
        .ok_or_else(|| IoError::new(400, "Not a Git repository"))?;
    let base = cleanup_base(&root).await?;
    let current = git_text(&root, &["symbolic-ref", "--short", "-q", "HEAD"])
        .await?
        .filter(|name| !name.is_empty());
    let listing = git_text(&root, &["worktree", "list", "--porcelain"])
        .await?
        .unwrap_or_default();
    let held: std::collections::HashMap<String, String> = parse_worktrees(&listing)
        .into_iter()
        .filter(|tree| !tree.main)
        .filter_map(|tree| tree.branch.map(|branch| (branch, tree.path)))
        .collect();
    let base_branch = base
        .as_deref()
        .map(|name| name.strip_prefix("origin/").unwrap_or(name).to_owned());
    let (local, _remote) = git_refs(&root).await?;
    let candidates: Vec<GitRef> = local
        .into_iter()
        .filter(|entry| {
            Some(&entry.name) != current.as_ref() && Some(&entry.name) != base_branch.as_ref()
        })
        .take(MAX_CLEANUP_BRANCHES)
        .collect();
    let checked: Vec<(GitRef, Option<&'static str>)> = stream::iter(candidates)
        .map(|entry| {
            let (root, base) = (root.clone(), base.clone());
            async move {
                let kind = match base.as_deref() {
                    Some(base) => merged_into(&root, base, &entry.name).await.ok().flatten(),
                    None => None,
                };
                (entry, kind)
            }
        })
        .buffered(6)
        .collect()
        .await;
    let branches = checked
        .into_iter()
        .filter_map(|(entry, kind)| {
            let reason = kind.or(entry.gone.then_some("gone"))?;
            Some(CleanupCandidate {
                worktree: held.get(&entry.name).cloned(),
                name: entry.name,
                reason,
            })
        })
        .collect();
    Ok(GitCleanup { base, branches })
}
