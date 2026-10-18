//! Running git with a timeout and an output cap, and the status, diff and
//! index operations built on it.

use super::*;

pub(super) fn git_command(root: &Path, args: &[String]) -> Command {
    let mut command = Command::new("git");
    for (key, _) in std::env::vars().filter(|(k, _)| k.starts_with("GIT_")) {
        command.env_remove(key);
    }
    // `stash` cleans what it took with a `:/` pathspec of its own, which
    // literal pathspecs would leave behind; it gets its paths as `:(literal)`.
    let pathspecs = if args.first().is_some_and(|arg| arg == "stash") {
        "--noglob-pathspecs"
    } else {
        "--literal-pathspecs"
    };
    command
        .args([
            pathspecs,
            "-c",
            "color.ui=false",
            "-c",
            "core.fsmonitor=false",
        ])
        .args(args)
        .current_dir(root)
        .kill_on_drop(true)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_PAGER", "cat")
        .env("GIT_OPTIONAL_LOCKS", "0")
        // Nothing here can answer an editor: continuing a merge or a rebase
        // keeps the message Git prepared instead of waiting on one.
        .env("GIT_EDITOR", "true")
        .env("GIT_SEQUENCE_EDITOR", "true")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    command
}
pub(crate) async fn run_git(
    root: &Path,
    args: Vec<String>,
    duration: Duration,
) -> Result<(Vec<u8>, Vec<u8>, bool), IoError> {
    let mut child = git_command(root, &args).spawn()?;
    let out = child
        .stdout
        .take()
        .ok_or_else(|| IoError::new(500, "missing Git stdout"))?;
    let err = child
        .stderr
        .take()
        .ok_or_else(|| IoError::new(500, "missing Git stderr"))?;
    async fn read_capped(stream: impl tokio::io::AsyncRead + Unpin) -> Result<Vec<u8>, IoError> {
        let mut data = Vec::new();
        stream
            .take((MAX_GIT_OUTPUT + 1) as u64)
            .read_to_end(&mut data)
            .await?;
        if data.len() > MAX_GIT_OUTPUT {
            return Err(IoError::new(413, "Git output exceeds 8 MiB limit"));
        }
        Ok(data)
    }
    let result = timeout(duration, async {
        let (stdout, stderr) = tokio::try_join!(read_capped(out), read_capped(err))?;
        let status = child.wait().await?;
        Ok::<_, IoError>((stdout, stderr, status.success()))
    })
    .await;
    match result {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(error)) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            Err(error)
        }
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            Err(IoError::new(504, "Git command timed out"))
        }
    }
}

pub async fn git_status(root: &Path) -> Result<GitStatus, IoError> {
    let root = dunce::canonicalize(root).map_err(IoError::from)?;
    let (stdout, stderr, success) = run_git(
        root.as_path(),
        vec![
            "status".into(),
            "--porcelain=v1".into(),
            "-z".into(),
            "--branch".into(),
            "--untracked-files=all".into(),
        ],
        GIT_TIMEOUT,
    )
    .await?;
    if !success {
        return Err(IoError::new(400, git_error(&stderr)));
    }
    let mut status = parse_git_status(&stdout);
    status.operation = git_operation(&root).await;
    Ok(status)
}

/// Which operation, if any, Git stopped in the middle of: its marker files sit
/// in this checkout's own Git directory, so a worktree reports only its own.
pub(crate) async fn git_operation(root: &Path) -> Option<String> {
    let dir = git_text(root, &["rev-parse", "--absolute-git-dir"])
        .await
        .ok()
        .flatten()?;
    let dir = Path::new(&dir);
    [
        ("rebase-merge", "rebase"),
        ("rebase-apply", "rebase"),
        ("MERGE_HEAD", "merge"),
        ("CHERRY_PICK_HEAD", "cherry-pick"),
        ("REVERT_HEAD", "revert"),
    ]
    .into_iter()
    .find(|(marker, _)| dir.join(marker).exists())
    .map(|(_, name)| name.to_owned())
}

pub(super) fn parse_git_status(bytes: &[u8]) -> GitStatus {
    let mut branch = None;
    let mut ahead = None;
    let mut behind = None;
    let mut upstream = None;
    let mut files = Vec::new();
    let mut tokens = bytes.split(|b| *b == 0).peekable();
    while let Some(token) = tokens.next() {
        if token.is_empty() {
            continue;
        }
        if token.starts_with(b"## ") {
            let value = String::from_utf8_lossy(&token[3..]).to_string();
            let (name, tracking) = value
                .split_once("...")
                .map_or((value.as_str(), ""), |(n, t)| (n, t));
            upstream = tracking
                .split(" [")
                .next()
                .filter(|name| !name.is_empty())
                .map(str::to_owned);
            branch = Some(
                if let Some(branch_name) = value.strip_prefix("No commits yet on ") {
                    branch_name.to_owned()
                } else {
                    name.split(' ').next().unwrap_or(name).to_string()
                },
            );
            if let Some(start) = tracking.find("[ahead ")
                && let Some(n) = tracking[start + 7..]
                    .split([',', ']'])
                    .next()
                    .and_then(|n| n.parse().ok())
            {
                ahead = Some(n);
            }
            if let Some(start) = tracking.find("behind ")
                && let Some(n) = tracking[start + 7..]
                    .split([',', ']'])
                    .next()
                    .and_then(|n| n.parse().ok())
            {
                behind = Some(n);
            }
            continue;
        }
        if token.len() < 3 {
            continue;
        }
        let index = String::from_utf8_lossy(&token[0..1]).to_string();
        let worktree = String::from_utf8_lossy(&token[1..2]).to_string();
        let path = String::from_utf8_lossy(&token[3..]).to_string();
        let original_path = if index == "R" || index == "C" || worktree == "R" || worktree == "C" {
            tokens
                .next()
                .map(|raw| String::from_utf8_lossy(raw).to_string())
        } else {
            None
        };
        files.push(GitFile {
            index,
            worktree,
            path,
            original_path,
        });
    }
    GitStatus {
        branch,
        files,
        ahead,
        behind,
        upstream,
        operation: None,
    }
}

pub(crate) fn git_error(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr).trim().to_string();
    if text.is_empty() {
        "git command failed".into()
    } else {
        text
    }
}

pub async fn git_diff(root: &Path, path: Option<&str>, staged: bool) -> Result<GitDiff, IoError> {
    let root = dunce::canonicalize(root).map_err(IoError::from)?;
    let safe = if let Some(path) = path {
        let rel = safe_relative(path)?;
        if is_protected_path(&rel) {
            return Err(IoError::new(403, "protected path"));
        }
        let _ = safe_path(&root, path)?;
        Some(path.to_owned())
    } else {
        None
    };
    let mut args = vec!["diff".to_string()];
    if staged {
        args.push("--cached".into());
    }
    args.extend([
        "--no-ext-diff".into(),
        "--no-textconv".into(),
        "--no-color".into(),
        "--binary".into(),
    ]);
    if let Some(ref path) = safe {
        args.extend(["--".into(), path.clone()]);
    }
    let (stdout, stderr, success) = run_git(&root, args, GIT_TIMEOUT).await?;
    if !success {
        return Err(IoError::new(400, git_error(&stderr)));
    }
    let mut diff = String::from_utf8_lossy(&stdout).to_string();
    // Ask Git for individual paths: obey ignore rules, never walk symlinked folders.
    if !staged {
        let (raw, stderr, ok) = run_git(
            &root,
            vec![
                "ls-files".into(),
                "--others".into(),
                "--exclude-standard".into(),
                "-z".into(),
            ],
            GIT_TIMEOUT,
        )
        .await?;
        if !ok {
            return Err(IoError::new(400, git_error(&stderr)));
        }
        let candidates: Vec<_> = raw.split(|b| *b == 0).filter(|s| !s.is_empty()).collect();
        if candidates.len() > 500 {
            return Err(IoError::new(
                413,
                "Select a file to review this large untracked set",
            ));
        }
        for raw in candidates {
            let untracked = String::from_utf8_lossy(raw).into_owned();
            if safe.as_ref().is_some_and(|p| p != &untracked) {
                continue;
            }
            if is_protected_path(Path::new(&untracked)) {
                continue;
            }
            let target = match asset_path(&root, &untracked) {
                Ok(p) => p,
                Err(_) => continue,
            };
            if !target.is_file() {
                continue;
            }
            let (out, _, _) = run_git(
                &root,
                vec![
                    "diff".into(),
                    "--no-index".into(),
                    "--no-ext-diff".into(),
                    "--no-textconv".into(),
                    "--no-color".into(),
                    "--".into(),
                    "/dev/null".into(),
                    untracked,
                ],
                GIT_TIMEOUT,
            )
            .await?;
            if diff.len() + out.len() > MAX_GIT_OUTPUT {
                return Err(IoError::new(413, "Diff exceeds limit; select one file"));
            }
            diff.push_str(&String::from_utf8_lossy(&out));
        }
    }

    let binary = diff.contains("Binary files") || diff.contains("GIT binary patch");
    Ok(GitDiff {
        diff,
        path: safe,
        staged,
        binary,
        truncated: false,
    })
}

/// Shared guard for media and text reads. Both lexical and resolved paths matter.
pub fn asset_path(root: &Path, relative: &str) -> Result<PathBuf, IoError> {
    let canonical = dunce::canonicalize(root)?;
    let rel = safe_relative(relative)?;
    if is_protected_path(&rel) {
        return Err(IoError::new(403, "protected path"));
    }
    let target = safe_path(&canonical, relative)?;
    if is_protected_path(
        target
            .strip_prefix(&canonical)
            .map_err(|_| IoError::new(403, "path escapes workspace"))?,
    ) {
        return Err(IoError::new(403, "protected path"));
    }
    if !target.is_file() {
        return Err(IoError::new(404, "file not found"));
    }
    Ok(target)
}

pub async fn git_stage(root: &Path, paths: &[String]) -> Result<(), IoError> {
    git_index_op(root, "add", paths).await
}

pub async fn git_unstage(root: &Path, paths: &[String]) -> Result<(), IoError> {
    git_index_op(root, "reset", paths).await
}

pub(super) async fn git_index_op(
    root: &Path,
    operation: &str,
    paths: &[String],
) -> Result<(), IoError> {
    let root = dunce::canonicalize(root).map_err(IoError::from)?;
    if paths.is_empty() {
        return Ok(());
    }
    let mut args = if operation == "reset"
        && !run_git(
            &root,
            vec!["rev-parse".into(), "--verify".into(), "HEAD".into()],
            GIT_TIMEOUT,
        )
        .await?
        .2
    {
        vec![
            "rm".into(),
            "--cached".into(),
            "--ignore-unmatch".into(),
            "--".into(),
        ]
    } else {
        vec![operation.to_owned(), "--".into()]
    };
    if paths.len() > MAX_PATHS {
        return Err(IoError::new(413, "Too many paths"));
    }
    for path in paths {
        if path.is_empty() || path == "." {
            return Err(IoError::new(400, "Explicit file paths required"));
        }
        let rel = safe_relative(path)?;
        if is_protected_path(&rel) {
            return Err(IoError::new(403, "protected path"));
        }
        let _ = safe_path(&root, path)?;
    }
    let base = args.len();
    for run in path_runs(paths) {
        args.truncate(base);
        args.extend_from_slice(run);
        let (_stdout, stderr, success) = run_git(&root, args.clone(), GIT_TIMEOUT).await?;
        if !success {
            return Err(IoError::new(400, git_error(&stderr)));
        }
    }
    Ok(())
}

/// The most paths one stage, unstage or discard names: "Stage all" over an
/// unignored build directory sends every file in it.
const MAX_PATHS: usize = 50_000;

/// Paths go to Git in runs that fit one command line. Windows allows 32,767
/// UTF-16 units for the whole of it; counting bytes stays under that, and the
/// rest of the budget is left for Git's own arguments.
fn path_runs(paths: &[String]) -> impl Iterator<Item = &[String]> {
    const BUDGET: usize = 24 * 1024;
    let mut rest = paths;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let mut used = 0;
        let end = rest
            .iter()
            .position(|path| {
                used += path.len() + 3;
                used > BUDGET
            })
            .unwrap_or(rest.len())
            .max(1);
        let (run, tail) = rest.split_at(end);
        rest = tail;
        Some(run)
    })
}

/// Throw away working-tree changes to the named files.
///
/// A tracked file goes back to what is staged for it -- the index, not HEAD --
/// so discarding never touches work that was already staged. An untracked file
/// has nothing to go back to and is removed, through the same guarded delete
/// the file explorer uses, which never follows a link out of the workspace.
/// Paths are named one by one: there is no "discard everything" spelling.
pub async fn git_discard(root: &Path, paths: &[String]) -> Result<(), IoError> {
    let root = dunce::canonicalize(root).map_err(IoError::from)?;
    if paths.is_empty() {
        return Ok(());
    }
    if paths.len() > MAX_PATHS {
        return Err(IoError::new(413, "Too many paths"));
    }
    for path in paths {
        if path.is_empty() || path == "." {
            return Err(IoError::new(400, "Explicit file paths required"));
        }
        let rel = safe_relative(path)?;
        if is_protected_path(&rel) {
            return Err(IoError::new(403, "protected path"));
        }
    }
    // Which of them the index knows, asked once per run of paths rather than
    // once per path: a process each is minutes on Windows for a build folder.
    let mut known = std::collections::HashSet::new();
    for run in path_runs(paths) {
        let mut args: Vec<String> = vec!["ls-files".into(), "-z".into(), "--".into()];
        args.extend_from_slice(run);
        let (stdout, stderr, success) = run_git(&root, args, GIT_TIMEOUT).await?;
        if !success {
            return Err(IoError::new(400, git_error(&stderr)));
        }
        known.extend(
            stdout
                .split(|byte| *byte == 0)
                .filter(|name| !name.is_empty())
                .map(|name| String::from_utf8_lossy(name).into_owned()),
        );
    }
    let (tracked, untracked): (Vec<String>, Vec<String>) =
        paths.iter().cloned().partition(|path| known.contains(path));
    for run in path_runs(&tracked) {
        let mut args: Vec<String> = vec!["checkout".into(), "--".into()];
        args.extend_from_slice(run);
        let (_stdout, stderr, success) = run_git(&root, args, GIT_TIMEOUT).await?;
        if !success {
            return Err(IoError::new(400, git_error(&stderr)));
        }
    }
    for path in untracked {
        delete_entry(&root, &path).await?;
    }
    Ok(())
}
