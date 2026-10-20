//! Running Git for these routes: the repository a request names, commands
//! that must succeed, the paths a request may name, and how an operation
//! that can stop on conflicts ended.

use super::*;

pub(super) async fn repo(
    state: &AppState,
    id: WorkspaceId,
    checkout: &CheckoutParam,
) -> Result<PathBuf> {
    Ok(dunce::canonicalize(checkout_root(state, id, checkout).await?).map_err(IoError::from)?)
}

pub(super) fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|arg| (*arg).to_owned()).collect()
}

/// What Git said, stdout and stderr together: a merge reports its conflicts on
/// stdout and its refusal on stderr, and both are worth showing.
pub(super) fn said(stdout: &[u8], stderr: &[u8]) -> String {
    [stdout, stderr]
        .iter()
        .map(|bytes| String::from_utf8_lossy(bytes).trim().to_owned())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Run a command that must succeed, returning its stdout.
pub(super) async fn git(
    root: &std::path::Path,
    list: Vec<String>,
    wait: Duration,
) -> Result<String> {
    let (stdout, stderr, success) = run_git(root, list, wait).await?;
    if success {
        Ok(String::from_utf8_lossy(&stdout).into_owned())
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

/// A workspace-relative path a request names, refused if it leaves the
/// workspace or touches what Tendly never edits.
pub(super) fn checked_path(path: &str) -> Result<String> {
    let relative = safe_relative(path)?;
    if path.is_empty() || path == "." || is_protected_path(&relative) {
        return Err(ApiError::bad("Invalid path"));
    }
    Ok(path.to_owned())
}

/// How an operation that can stop part-way ended: done, or stopped on
/// conflicts that are now in the working tree waiting to be resolved.
#[derive(Serialize)]
pub(super) struct Outcome {
    pub(super) conflicts: bool,
    pub(super) message: String,
}

pub(super) async fn unmerged(root: &std::path::Path) -> bool {
    run_git(root, args(&["ls-files", "--unmerged", "-z"]), GIT_TIMEOUT)
        .await
        .is_ok_and(|(stdout, _, success)| success && !stdout.is_empty())
}

/// Conflicts are an outcome, not a failure: the merge happened as far as Git
/// could take it, and the person finishes it from the changes list.
pub(super) async fn finish(
    root: &std::path::Path,
    list: Vec<String>,
    wait: Duration,
) -> Result<Json<Outcome>> {
    let (stdout, stderr, success) = run_git(root, list, wait).await?;
    let message = said(&stdout, &stderr);
    if success {
        return Ok(Json(Outcome {
            conflicts: false,
            message,
        }));
    }
    if unmerged(root).await || git_operation(root).await.is_some() {
        return Ok(Json(Outcome {
            conflicts: true,
            message,
        }));
    }
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
