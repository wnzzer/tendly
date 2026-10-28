//! Read-only host folder discovery for the workspace picker.
//!
//! Browsing is restricted to explicitly configured roots (or the current
//! directory and the server user's home directory). This is a UI discovery
//! boundary, not an OS sandbox for a trusted user's Agent processes.

use std::{
    env, fs,
    path::{Component, Path, PathBuf},
};

use serde::Serialize;

use crate::workspace_io::IoError;

const MAX_DIRECTORIES: usize = 2_000;
const MAX_SCANNED_ENTRIES: usize = 20_000;

#[derive(Debug, Clone, Serialize)]
pub struct DirectoryRoot {
    pub id: String,
    pub label: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DirectoryEntry {
    pub name: String,
    /// Path relative to the selected browsing root, not to the current folder.
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DirectoryListing {
    pub roots: Vec<DirectoryRoot>,
    pub root_id: String,
    pub path: String,
    pub relative_path: String,
    /// Relative path of the parent; `None` means this is the browsing root.
    pub parent: Option<String>,
    pub entries: Vec<DirectoryEntry>,
    pub truncated: bool,
}

/// Resolve this once at server startup so root IDs remain stable for its life.
/// An explicitly empty or invalid configuration fails closed instead of
/// unexpectedly making the server's home directory browsable.
pub fn roots() -> Result<Vec<PathBuf>, IoError> {
    let candidates = match env::var_os("TENDLY_BROWSE_ROOTS") {
        Some(value) => {
            let paths: Vec<_> = env::split_paths(&value).collect();
            if paths.is_empty() || paths.iter().any(|path| path.as_os_str().is_empty()) {
                return Err(IoError::new(
                    400,
                    "TENDLY_BROWSE_ROOTS must contain at least one existing directory",
                ));
            }
            paths
        }
        None => default_roots()?,
    };
    canonical_roots(candidates)
}

/// The working directory and the home folder: where code lives on macOS and
/// Linux, so a default that rarely turns anyone away.
#[cfg(not(windows))]
fn default_roots() -> Result<Vec<PathBuf>, IoError> {
    let mut paths = vec![env::current_dir().map_err(IoError::from)?];
    if let Some(home) = env::var_os("HOME").filter(|home| !home.is_empty()) {
        paths.push(PathBuf::from(home));
    }
    Ok(paths)
}

/// Every drive. On Windows code lives wherever there was room — `D:\code`,
/// `E:\work` — rather than under the home folder, so the macOS default turned
/// away the first workspace most people tried to add. A drive with nothing in
/// it (an empty card reader) is not a directory and is left out.
#[cfg(windows)]
fn default_roots() -> Result<Vec<PathBuf>, IoError> {
    Ok(('A'..='Z')
        .map(|letter| PathBuf::from(format!("{letter}:\\")))
        .filter(|drive| drive.is_dir())
        .collect())
}

/// Where a new workspace may be rooted.
///
/// Browsing roots bound the picker, but creating a workspace took any absolute
/// directory, so the API was not bounded at all — a caller could root a
/// workspace at `/` and then read and write through it. On a trusted single
/// machine that is the same authority the caller already had; reachable over a
/// network it is not, which is what this project is for.
///
/// Defaults to the browsing roots, so a host that never configures either keeps
/// one boundary rather than two that can disagree.
pub fn workspace_roots(browse: &[PathBuf]) -> Result<Vec<PathBuf>, IoError> {
    match env::var_os("TENDLY_WORKSPACE_ROOTS") {
        Some(value) => {
            let paths: Vec<_> = env::split_paths(&value).collect();
            if paths.is_empty() || paths.iter().any(|path| path.as_os_str().is_empty()) {
                return Err(IoError::new(
                    400,
                    "TENDLY_WORKSPACE_ROOTS must contain at least one existing directory",
                ));
            }
            canonical_roots(paths)
        }
        None => Ok(browse.to_vec()),
    }
}

/// Whether an already-canonical directory lies inside one of the allowed roots.
/// A root itself is allowed; a sibling whose name merely starts the same is not.
pub fn within_roots(roots: &[PathBuf], candidate: &Path) -> bool {
    roots.iter().any(|root| candidate.starts_with(root))
}

fn canonical_roots(candidates: Vec<PathBuf>) -> Result<Vec<PathBuf>, IoError> {
    let mut result = Vec::new();
    for path in candidates {
        let path = dunce::canonicalize(path).map_err(IoError::from)?;
        if !path.is_dir() {
            return Err(IoError::new(400, "Browsing root must be a directory"));
        }
        if protected(&path) {
            return Err(IoError::new(
                403,
                "Protected directory cannot be a browsing root",
            ));
        }
        if !result.contains(&path) {
            result.push(path);
        }
    }
    if result.is_empty() {
        return Err(IoError::new(503, "No host browsing roots are configured"));
    }
    Ok(result)
}

pub async fn list(
    allowed_roots: &[PathBuf],
    root_id: Option<String>,
    relative_path: Option<String>,
) -> Result<DirectoryListing, IoError> {
    let allowed_roots = allowed_roots.to_vec();
    tokio::task::spawn_blocking(move || {
        list_sync(
            &allowed_roots,
            root_id.as_deref(),
            relative_path.as_deref().unwrap_or(""),
        )
    })
    .await
    .map_err(|_| IoError::new(500, "Host directory lookup could not complete"))?
}

fn list_sync(
    allowed_roots: &[PathBuf],
    root_id: Option<&str>,
    relative_path: &str,
) -> Result<DirectoryListing, IoError> {
    let index = match root_id {
        None => 0,
        Some(value) if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) => {
            value
                .parse::<usize>()
                .map_err(|_| IoError::new(400, "Invalid browsing root ID"))?
        }
        Some(_) => return Err(IoError::new(400, "Invalid browsing root ID")),
    };
    let root = allowed_roots
        .get(index)
        .ok_or_else(|| IoError::new(404, "Host browsing root not found"))?;
    // Startup receives canonical roots. Validate again so a root replaced with
    // a symlink cannot silently grant access to a different tree.
    if dunce::canonicalize(root).map_err(IoError::from)? != *root || protected(root) {
        return Err(IoError::new(
            403,
            "Host browsing root has changed or is protected",
        ));
    }
    let relative = validate_relative(relative_path)?;
    let mut candidate = root.clone();
    for component in relative.components() {
        candidate.push(component.as_os_str());
        if fs::symlink_metadata(&candidate)
            .map_err(IoError::from)?
            .file_type()
            .is_symlink()
        {
            return Err(IoError::new(
                403,
                "Symbolic links are not available in the folder browser",
            ));
        }
    }
    let directory = dunce::canonicalize(candidate).map_err(IoError::from)?;
    if !directory.starts_with(root) || protected(&directory) {
        return Err(IoError::new(
            403,
            "Directory is outside the allowed browsing root",
        ));
    }
    if !directory.is_dir() {
        return Err(IoError::new(400, "Path is not a directory"));
    }
    let mut entries = Vec::new();
    for (count, item) in fs::read_dir(&directory).map_err(IoError::from)?.enumerate() {
        if count >= MAX_SCANNED_ENTRIES {
            return Err(IoError::new(
                413,
                "Folder contains too many entries; choose a more specific directory using the advanced path field",
            ));
        }
        let item = item.map_err(IoError::from)?;
        let path = item.path();
        // file_type never follows symlinks. Skip protected names before doing
        // any metadata lookup into their targets.
        if protected(&path) {
            continue;
        }
        let file_type = match item.file_type() {
            Ok(file_type) => file_type,
            Err(_) => continue, // An inaccessible/racing child does not hide its siblings.
        };
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        entries.push(DirectoryEntry {
            name: item.file_name().to_string_lossy().into_owned(),
            path: relative_string(root, &path),
        });
    }
    entries.sort_by_cached_key(|entry| (entry.name.to_lowercase(), entry.name.clone()));
    let truncated = entries.len() > MAX_DIRECTORIES;
    entries.truncate(MAX_DIRECTORIES);
    Ok(DirectoryListing {
        roots: allowed_roots
            .iter()
            .enumerate()
            .map(|(index, path)| DirectoryRoot {
                id: index.to_string(),
                label: path
                    .file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy()
                    .into_owned(),
                path: path.to_string_lossy().into_owned(),
            })
            .collect(),
        root_id: index.to_string(),
        path: directory.to_string_lossy().into_owned(),
        relative_path: relative_string(root, &directory),
        parent: if directory == *root {
            None
        } else {
            directory
                .parent()
                .map(|parent| relative_string(root, parent))
        },
        entries,
        truncated,
    })
}

fn validate_relative(value: &str) -> Result<PathBuf, IoError> {
    if value.as_bytes().contains(&0) {
        return Err(IoError::new(400, "Directory path contains NUL"));
    }
    let mut result = PathBuf::new();
    for component in Path::new(value).components() {
        match component {
            Component::CurDir => {}
            Component::Normal(name) => result.push(name),
            Component::ParentDir => {
                return Err(IoError::new(400, "Directory traversal is not allowed"));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(IoError::new(
                    400,
                    "Use a path relative to the selected browsing root",
                ));
            }
        }
    }
    if protected(&result) {
        return Err(IoError::new(403, "Protected directories are not browsable"));
    }
    Ok(result)
}

fn relative_string(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn protected(path: &Path) -> bool {
    const PROTECTED: &[&str] = &[
        ".git",
        ".ssh",
        ".gnupg",
        ".aws",
        ".azure",
        ".config",
        ".kube",
        ".tendly",
        ".docker",
        "credentials",
        "credential",
        "secrets",
    ];
    path.components().any(|component| {
        let Component::Normal(name) = component else {
            return false;
        };
        let name = name.to_string_lossy();
        PROTECTED
            .iter()
            .chain(crate::adapters::all().flat_map(|client| client.protected_names()))
            .any(|protected| name.eq_ignore_ascii_case(protected))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    struct TempDirectory(PathBuf);
    impl TempDirectory {
        fn new() -> Self {
            let path = env::temp_dir().join(format!("tendly-browse-test-{}", Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(dunce::canonicalize(path).unwrap())
        }
        fn roots(&self) -> Vec<PathBuf> {
            vec![self.0.clone()]
        }
    }
    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn lists_sorted_directories_and_navigates_with_relative_paths() {
        let temp = TempDirectory::new();
        for name in [
            "zebra",
            "Alpha",
            "Alpha/nested",
            ".ssh",
            ".codex",
            ".tendly",
            ".Claude",
        ] {
            fs::create_dir(temp.0.join(name)).unwrap();
        }
        fs::write(temp.0.join("note.txt"), "not a directory").unwrap();
        let listing = list(&temp.roots(), None, None).await.unwrap();
        assert_eq!(
            listing
                .entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["Alpha", "zebra"]
        );
        assert_eq!(listing.parent, None);
        assert!(!listing.truncated);
        let child = list(&temp.roots(), Some("0".into()), Some("Alpha/nested".into()))
            .await
            .unwrap();
        assert_eq!(child.relative_path, "Alpha/nested");
        assert_eq!(child.parent.as_deref(), Some("Alpha"));
        assert!(child.entries.is_empty());
    }

    #[tokio::test]
    async fn rejects_traversal_absolute_paths_and_protected_paths() {
        let temp = TempDirectory::new();
        for (path, expected_status) in [
            ("../", 400),
            ("safe/../../outside", 400),
            ("/", 400),
            (".ssh", 403),
            ("a/.config/private", 403),
            ("a\0b", 400),
        ] {
            assert_eq!(
                list(&temp.roots(), None, Some(path.into()))
                    .await
                    .unwrap_err()
                    .status,
                expected_status,
                "{path:?}"
            );
        }
        assert_eq!(
            list(&temp.roots(), Some("-1".into()), None)
                .await
                .unwrap_err()
                .status,
            400
        );
        assert_eq!(
            list(&temp.roots(), Some("50".into()), None)
                .await
                .unwrap_err()
                .status,
            404
        );
    }

    #[tokio::test]
    async fn rejects_files_and_missing_directories() {
        let temp = TempDirectory::new();
        fs::write(temp.0.join("file"), "hello").unwrap();
        assert_eq!(
            list(&temp.roots(), None, Some("file".into()))
                .await
                .unwrap_err()
                .status,
            400
        );
        assert_eq!(
            list(&temp.roots(), None, Some("missing".into()))
                .await
                .unwrap_err()
                .status,
            404
        );
        assert_eq!(
            canonical_roots(vec![temp.0.clone(), temp.0.clone()])
                .unwrap()
                .len(),
            1
        );
        assert_eq!(canonical_roots(Vec::new()).unwrap_err().status, 503);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn skips_and_rejects_symlinks_even_when_target_is_inside_root() {
        use std::os::unix::fs::symlink;
        let temp = TempDirectory::new();
        let outside = TempDirectory::new();
        fs::create_dir(temp.0.join("safe")).unwrap();
        fs::create_dir(temp.0.join(".ssh")).unwrap();
        symlink(&outside.0, temp.0.join("escape")).unwrap();
        symlink(temp.0.join("safe"), temp.0.join("alias")).unwrap();
        symlink(temp.0.join(".ssh"), temp.0.join("hidden-alias")).unwrap();
        let listing = list(&temp.roots(), None, None).await.unwrap();
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(listing.entries[0].name, "safe");
        for path in ["escape", "escape/child", "alias", "hidden-alias"] {
            assert_eq!(
                list(&temp.roots(), None, Some(path.into()))
                    .await
                    .unwrap_err()
                    .status,
                403
            );
        }
    }

    #[tokio::test]
    async fn caps_sorted_result_without_losing_the_first_entries() {
        let temp = TempDirectory::new();
        for index in (0..=MAX_DIRECTORIES).rev() {
            fs::create_dir(temp.0.join(format!("folder-{index:04}"))).unwrap();
        }
        let listing = list(&temp.roots(), None, None).await.unwrap();
        assert_eq!(listing.entries.len(), MAX_DIRECTORIES);
        assert_eq!(listing.entries[0].name, "folder-0000");
        assert!(listing.truncated);
    }

    #[test]
    fn workspace_roots_bound_a_candidate_to_a_root_or_below_it() {
        let base = std::env::temp_dir().join(format!("tendly-roots-{}", uuid::Uuid::new_v4()));
        let allowed = base.join("allowed");
        fs::create_dir_all(allowed.join("nested/deep")).unwrap();
        fs::create_dir_all(base.join("allowed-sibling")).unwrap();
        fs::create_dir_all(base.join("elsewhere")).unwrap();
        let roots = vec![dunce::canonicalize(&allowed).unwrap()];

        assert!(within_roots(
            &roots,
            &dunce::canonicalize(&allowed).unwrap()
        ));
        assert!(within_roots(
            &roots,
            &dunce::canonicalize(allowed.join("nested/deep")).unwrap()
        ));
        // A sibling whose name merely starts the same is a different directory,
        // which a plain string prefix would have accepted.
        assert!(!within_roots(
            &roots,
            &dunce::canonicalize(base.join("allowed-sibling")).unwrap()
        ));
        assert!(!within_roots(
            &roots,
            &dunce::canonicalize(base.join("elsewhere")).unwrap()
        ));
        assert!(!within_roots(&roots, Path::new("/")));

        // Without its own configuration the workspace boundary is the browsing
        // one, so a host that sets neither still has a single boundary.
        assert_eq!(workspace_roots(&roots).unwrap(), roots);
        fs::remove_dir_all(&base).ok();
    }
}
