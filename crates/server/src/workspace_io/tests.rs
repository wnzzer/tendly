use super::*;

#[tokio::test]
async fn a_terminal_paste_lands_with_a_full_path_and_old_pastes_are_cleared() {
    let root = std::env::temp_dir().join(format!("tendly-paste-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let old = root.join(ATTACHMENT_DIR).join(PASTE_DIR).join("2000-01-01");
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("stale.png"), b"x").unwrap();
    let keep = root.join(ATTACHMENT_DIR).join(PASTE_DIR).join("notes");
    fs::create_dir_all(&keep).unwrap();
    let saved = write_paste(&root, "Screen Shot (1).png", b"png bytes")
        .await
        .unwrap();
    assert!(
        saved.path.starts_with(".tendly-files/pastes/"),
        "{}",
        saved.path
    );
    assert!(std::path::Path::new(&saved.absolute).is_absolute());
    assert_eq!(fs::read(&saved.absolute).unwrap(), b"png bytes");
    assert!(saved.absolute.ends_with(".png"));
    assert!(!old.exists(), "a week-old paste folder is cleared");
    assert!(keep.exists(), "only dated folders are touched");
    let chat = write_attachment(&root, "a.txt", b"t").await.unwrap();
    assert!(
        !chat.path.contains("/pastes/"),
        "chat attachments are not pastes"
    );
    assert!(
        write_paste(
            &root,
            "big.bin",
            &vec![0; (MAX_ATTACHMENT_BYTES + 1) as usize]
        )
        .await
        .is_err()
    );
    fs::remove_dir_all(root).ok();
}
use std::process::Command as StdCommand;

#[tokio::test]
async fn a_new_file_is_only_ever_a_new_one_and_never_an_emptied_old_one() {
    let repo = TempRepo::new();
    let root = &repo.path;
    fs::write(root.join("kept.txt"), "precious").unwrap();

    let made = create_file(root, "notes.md").await.unwrap();
    assert_eq!(
        (made.path.as_str(), made.kind.as_str(), made.size),
        ("notes.md", "file", 0)
    );
    assert_eq!(fs::read_to_string(root.join("notes.md")).unwrap(), "");

    // Naming the path makes the folders it names.
    create_file(root, "docs/guide/start.md").await.unwrap();
    assert!(root.join("docs/guide/start.md").is_file());

    // A taken name is refused, so nothing here can empty a file that has
    // something in it.
    assert_eq!(create_file(root, "kept.txt").await.unwrap_err().status, 409);
    assert_eq!(
        fs::read_to_string(root.join("kept.txt")).unwrap(),
        "precious"
    );
    assert_eq!(create_file(root, "docs").await.unwrap_err().status, 409);

    // The workspace boundary and protected state hold here as everywhere.
    assert!(create_file(root, "../escape.txt").await.is_err());
    assert!(create_file(root, ".git/hooks/pre-commit").await.is_err());
    assert!(create_file(root, "").await.is_err());

    let folder = create_directory(root, "assets/icons").await.unwrap();
    assert_eq!(
        (
            folder.path.as_str(),
            folder.name.as_str(),
            folder.kind.as_str()
        ),
        ("assets/icons", "icons", "directory")
    );
    assert!(root.join("assets/icons").is_dir());
    // A taken name is refused whether a folder or a file holds it.
    assert_eq!(
        create_directory(root, "assets/icons")
            .await
            .unwrap_err()
            .status,
        409
    );
    assert_eq!(
        create_directory(root, "kept.txt").await.unwrap_err().status,
        409
    );
    assert!(create_directory(root, "../escape").await.is_err());
    assert!(create_directory(root, ".git/hooks").await.is_err());
    assert!(create_directory(root, "").await.is_err());
}

#[tokio::test]
async fn renaming_moves_a_name_without_replacing_what_already_holds_one() {
    let repo = TempRepo::new();
    let root = &repo.path;
    fs::write(root.join("draft.txt"), "body").unwrap();
    fs::write(root.join("taken.txt"), "someone else's").unwrap();
    fs::create_dir_all(root.join("docs")).unwrap();

    let renamed = rename_entry(root, "draft.txt", "final.txt").await.unwrap();
    assert_eq!(
        (
            renamed.path.as_str(),
            renamed.name.as_str(),
            renamed.kind.as_str()
        ),
        ("final.txt", "final.txt", "file")
    );
    assert_eq!(fs::read_to_string(root.join("final.txt")).unwrap(), "body");
    assert!(!root.join("draft.txt").exists());

    // A path, not just a name: an existing folder is a valid destination.
    rename_entry(root, "final.txt", "docs/final.txt")
        .await
        .unwrap();
    assert!(root.join("docs/final.txt").is_file());
    // A folder goes with everything under it, and reports itself as one.
    let moved = rename_entry(root, "docs", "guide").await.unwrap();
    assert_eq!(moved.kind, "directory");
    assert!(root.join("guide/final.txt").is_file());

    // What already holds a name keeps it.
    assert_eq!(
        rename_entry(root, "guide/final.txt", "taken.txt")
            .await
            .unwrap_err()
            .status,
        409
    );
    assert_eq!(
        fs::read_to_string(root.join("taken.txt")).unwrap(),
        "someone else's"
    );
    // And the destination's folder has to exist, rather than being invented.
    assert_eq!(
        rename_entry(root, "taken.txt", "nowhere/taken.txt")
            .await
            .unwrap_err()
            .status,
        404
    );
    assert_eq!(
        rename_entry(root, "missing.txt", "anything.txt")
            .await
            .unwrap_err()
            .status,
        404
    );
    assert!(rename_entry(root, "taken.txt", "taken.txt").await.is_err());

    // Neither end may leave the workspace or touch protected state.
    assert!(
        rename_entry(root, "taken.txt", "../escaped.txt")
            .await
            .is_err()
    );
    assert!(
        rename_entry(root, "../outside.txt", "inside.txt")
            .await
            .is_err()
    );
    assert!(rename_entry(root, ".git", "history").await.is_err());
    assert!(
        rename_entry(root, "taken.txt", ".ssh/id_rsa")
            .await
            .is_err()
    );
    assert!(root.join(".git").exists());

    #[cfg(unix)]
    {
        // Renaming a link renames the link. Its target is not this
        // workspace's to move, and following it would move it anyway.
        let outside = std::env::temp_dir().join(format!("tendly-rename-target-{}", Uuid::new_v4()));
        fs::write(&outside, "precious").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("outward")).unwrap();
        let link = rename_entry(root, "outward", "pointer").await.unwrap();
        assert_eq!(link.kind, "symlink");
        assert!(outside.exists(), "the link's target must not move");
        assert_eq!(fs::read_to_string(&outside).unwrap(), "precious");
        // But a link above either path is a way out of the workspace.
        std::os::unix::fs::symlink(std::env::temp_dir(), root.join("elsewhere")).unwrap();
        assert!(
            rename_entry(root, "taken.txt", "elsewhere/taken.txt")
                .await
                .is_err()
        );
        fs::remove_file(&outside).ok();
    }
}

#[tokio::test]
async fn delete_removes_workspace_entries_but_not_the_root_or_what_a_link_points_at() {
    let repo = TempRepo::new();
    let root = &repo.path;
    fs::write(root.join("notes.txt"), "keep").unwrap();
    fs::create_dir_all(root.join("build/inner")).unwrap();
    fs::write(root.join("build/inner/out.bin"), "x").unwrap();

    delete_entry(root, "notes.txt").await.unwrap();
    assert!(!root.join("notes.txt").exists());
    // A directory goes with everything under it.
    delete_entry(root, "build").await.unwrap();
    assert!(!root.join("build").exists());

    // The root, protected names and anything outside stay untouchable.
    assert!(delete_entry(root, ".").await.is_err());
    assert!(delete_entry(root, ".git").await.is_err());
    assert!(delete_entry(root, "../escape").await.is_err());
    assert!(root.join(".git").exists());

    // Deleting a link unlinks it; following it would destroy the target,
    // which is the one thing a workspace delete must never do.
    #[cfg(unix)]
    {
        let outside = std::env::temp_dir().join(format!("tendly-link-target-{}", Uuid::new_v4()));
        fs::write(&outside, "precious").unwrap();
        let inside = root.join("inside.txt");
        fs::write(&inside, "also precious").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("outward")).unwrap();
        std::os::unix::fs::symlink(&inside, root.join("inward")).unwrap();
        // A link out of the workspace resolves outside it and is refused.
        assert!(delete_entry(root, "outward").await.is_err());
        assert!(outside.exists());
        delete_entry(root, "inward").await.unwrap();
        assert!(!root.join("inward").exists());
        assert!(inside.exists(), "the link's target must survive");
        fs::remove_file(&outside).ok();
    }
}

struct TempRepo {
    path: PathBuf,
}
impl TempRepo {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("tendly-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        for args in [
            vec!["init"],
            vec!["config", "user.email", "tendly@example.com"],
            vec!["config", "user.name", "Tendly"],
            // Git for Windows commonly defaults this on, which rewrites the
            // fixtures' line endings on checkout.
            vec!["config", "core.autocrlf", "false"],
        ] {
            let mut cmd = StdCommand::new("git");
            cmd.args(args).current_dir(&path);
            assert!(cmd.output().unwrap().status.success());
        }
        Self { path }
    }
    fn git(&self, args: &[&str]) {
        assert!(
            StdCommand::new("git")
                .args(args)
                .current_dir(&self.path)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
}
impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[tokio::test]
async fn file_roundtrip_and_conflict() {
    let repo = TempRepo::new();
    let file = write_file(&repo.path, "src/a.txt", "hello", None)
        .await
        .unwrap();
    assert_eq!(
        read_file(&repo.path, "src/a.txt").await.unwrap().version,
        file.version
    );
    let err = write_file(&repo.path, "src/a.txt", "changed", Some("v1:deadbeef"))
        .await
        .unwrap_err();
    assert_eq!(err.status, 409);
    write_file(&repo.path, "src/a.txt", "changed", Some(&file.version))
        .await
        .unwrap();
}

#[tokio::test]
async fn git_status_rename_deleted_untracked() {
    let repo = TempRepo::new();
    fs::write(repo.path.join("a.txt"), "a").unwrap();
    repo.git(&["add", "a.txt"]);
    repo.git(&["commit", "-m", "init"]);
    fs::rename(repo.path.join("a.txt"), repo.path.join("b.txt")).unwrap();
    fs::write(repo.path.join("new.txt"), "new").unwrap();
    let status = git_status(&repo.path).await.unwrap();
    assert!(
        status
            .files
            .iter()
            .any(|f| f.path == "b.txt" || f.original_path.as_deref() == Some("a.txt"))
    );
    assert!(status.files.iter().any(|f| f.path == "new.txt"));
}

#[tokio::test]
async fn stage_diff_and_unstage() {
    let repo = TempRepo::new();
    fs::write(repo.path.join("new.txt"), "hello\n").unwrap();
    let diff = git_diff(&repo.path, None, false).await.unwrap();
    assert!(diff.diff.contains("new.txt"));
    git_stage(&repo.path, &["new.txt".into()]).await.unwrap();
    assert!(
        git_diff(&repo.path, None, true)
            .await
            .unwrap()
            .diff
            .contains("new.txt")
    );
    git_unstage(&repo.path, &["new.txt".into()]).await.unwrap();
}

#[tokio::test]
async fn branches_switch_and_worktrees_sit_beside_the_repository() {
    let repo = TempRepo::new();
    fs::write(repo.path.join("a.txt"), "a\n").unwrap();
    git_stage(&repo.path, &["a.txt".into()]).await.unwrap();
    git_commit(&repo.path, "first", false).await.unwrap();
    let start = git_branches(&repo.path).await.unwrap();
    let home = start.current.clone().expect("on a branch");
    assert_eq!(start.worktrees.len(), 1);
    assert!(start.worktrees[0].main);

    git_switch(&repo.path, "feature/x", true, None, false)
        .await
        .unwrap();
    assert_eq!(
        git_branches(&repo.path).await.unwrap().current.as_deref(),
        Some("feature/x")
    );
    git_switch(&repo.path, &home, false, None, false)
        .await
        .unwrap();
    assert_eq!(
        git_switch(&repo.path, "-rf", false, None, false)
            .await
            .unwrap_err()
            .status,
        400
    );
    assert_eq!(
        git_switch(&repo.path, "no-such-branch", false, None, false)
            .await
            .unwrap_err()
            .status,
        409
    );

    let tree = git_worktree_add(&repo.path, "feature/y", true)
        .await
        .unwrap();
    let canonical = dunce::canonicalize(&repo.path).unwrap();
    let expected = canonical.parent().unwrap().join(format!(
        "{}.worktrees",
        canonical.file_name().unwrap().to_string_lossy()
    ));
    assert_eq!(tree, expected.join("feature-y"));
    assert!(tree.join("a.txt").exists());
    let after = git_branches(&repo.path).await.unwrap();
    assert!(
        after
            .worktrees
            .iter()
            .any(|w| !w.main && w.branch.as_deref() == Some("feature/y"))
    );
    // The same branch cannot get a second directory.
    assert_eq!(
        git_worktree_add(&repo.path, "feature/y", false)
            .await
            .unwrap_err()
            .status,
        409
    );
    let _ = fs::remove_dir_all(&expected);
}

#[tokio::test]
async fn branches_rename_and_delete_and_worktrees_are_removed_but_never_the_main_one() {
    let repo = TempRepo::new();
    fs::write(repo.path.join("a.txt"), "a\n").unwrap();
    git_stage(&repo.path, &["a.txt".into()]).await.unwrap();
    git_commit(&repo.path, "first", false).await.unwrap();
    let tree = git_worktree_add(&repo.path, "side", true).await.unwrap();
    let tree_text = tree.to_string_lossy().into_owned();
    // A rename is followed by the worktree that holds the branch.
    git_branch_rename(&repo.path, "side", "renamed")
        .await
        .unwrap();
    let listed = git_branches(&repo.path).await.unwrap();
    assert!(
        listed
            .worktrees
            .iter()
            .any(|w| w.branch.as_deref() == Some("renamed"))
    );
    // A branch checked out somewhere cannot be deleted.
    assert_eq!(
        git_branch_delete(&repo.path, "renamed", true)
            .await
            .unwrap_err()
            .status,
        409
    );
    // Uncommitted work keeps a worktree unless forced.
    fs::write(tree.join("dirty.txt"), "x").unwrap();
    assert_eq!(
        git_worktree_remove(&repo.path, &tree_text, false)
            .await
            .unwrap_err()
            .status,
        409
    );
    git_worktree_remove(&repo.path, &tree_text, true)
        .await
        .unwrap();
    assert!(!tree.exists());
    git_branch_delete(&repo.path, "renamed", false)
        .await
        .unwrap();
    assert!(
        !git_branches(&repo.path)
            .await
            .unwrap()
            .branches
            .contains(&"renamed".to_owned())
    );
    let main = dunce::canonicalize(&repo.path)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        git_worktree_remove(&repo.path, &main, true)
            .await
            .unwrap_err()
            .status,
        400
    );
    let _ = fs::remove_dir_all(tree.parent().unwrap());
}

#[tokio::test]
async fn discarding_restores_what_is_staged_and_removes_what_git_never_knew() {
    let repo = TempRepo::new();
    fs::write(repo.path.join("kept.txt"), "one\n").unwrap();
    git_stage(&repo.path, &["kept.txt".into()]).await.unwrap();
    git_commit(&repo.path, "first", false).await.unwrap();
    // Staged work survives: discard goes back to the index, not to HEAD.
    fs::write(repo.path.join("kept.txt"), "two\n").unwrap();
    git_stage(&repo.path, &["kept.txt".into()]).await.unwrap();
    fs::write(repo.path.join("kept.txt"), "three\n").unwrap();
    fs::write(repo.path.join("scratch.txt"), "junk\n").unwrap();
    git_discard(&repo.path, &["kept.txt".into(), "scratch.txt".into()])
        .await
        .unwrap();
    assert_eq!(
        fs::read_to_string(repo.path.join("kept.txt")).unwrap(),
        "two\n"
    );
    assert!(!repo.path.join("scratch.txt").exists());
    // A deleted tracked file comes back.
    fs::remove_file(repo.path.join("kept.txt")).unwrap();
    git_discard(&repo.path, &["kept.txt".into()]).await.unwrap();
    assert!(repo.path.join("kept.txt").exists());
    // Nothing is discarded without being named.
    assert_eq!(
        git_discard(&repo.path, &[".".into()])
            .await
            .unwrap_err()
            .status,
        400
    );
}

#[tokio::test]
async fn staging_and_discarding_take_more_paths_than_one_command_line_holds() {
    // An unignored build directory: thousands of untracked files whose names
    // together run far past the 32,767 characters of a Windows command line.
    let repo = TempRepo::new();
    fs::write(repo.path.join("kept.txt"), "one\n").unwrap();
    git_stage(&repo.path, &["kept.txt".into()]).await.unwrap();
    git_commit(&repo.path, "first", false).await.unwrap();
    let dir = repo.path.join("target").join("debug").join("deps");
    fs::create_dir_all(&dir).unwrap();
    let mut paths: Vec<String> = (0..1500)
        .map(|n| {
            let name = format!("artifact-with-a-fairly-long-name-{n:04}.rmeta");
            fs::write(dir.join(&name), "x").unwrap();
            format!("target/debug/deps/{name}")
        })
        .collect();
    git_stage(&repo.path, &paths).await.unwrap();
    git_unstage(&repo.path, &paths).await.unwrap();
    fs::write(repo.path.join("kept.txt"), "two\n").unwrap();
    paths.push("kept.txt".into());
    git_discard(&repo.path, &paths).await.unwrap();
    assert_eq!(
        fs::read_to_string(repo.path.join("kept.txt")).unwrap(),
        "one\n"
    );
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_escape_is_rejected() {
    let repo = TempRepo::new();
    let outside = std::env::temp_dir().join(format!("tendly-outside-{}", Uuid::new_v4()));
    fs::write(&outside, "secret").unwrap();
    std::os::unix::fs::symlink(&outside, repo.path.join("link.txt")).unwrap();
    assert!(safe_path(&repo.path, "link.txt").is_err());
    let _ = fs::remove_file(outside);
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_ancestor_cannot_be_written() {
    let repo = TempRepo::new();
    let real = repo.path.join("real");
    fs::create_dir_all(&real).unwrap();
    std::os::unix::fs::symlink(&real, repo.path.join("alias")).unwrap();
    let err = write_file(&repo.path, "alias/file.txt", "nope", None)
        .await
        .unwrap_err();
    assert_eq!(err.status, 403);
}
#[tokio::test]
async fn host_files_are_read_only_inside_the_browsing_roots() {
    let base = std::env::temp_dir().join(format!("tendly-host-{}", uuid::Uuid::new_v4()));
    let root = base.join("root");
    fs::create_dir_all(root.join(".ssh")).unwrap();
    fs::write(root.join("notes.md"), "# notes\n").unwrap();
    fs::write(root.join(".ssh/id"), "secret").unwrap();
    fs::write(base.join("outside.txt"), "outside").unwrap();
    let roots = vec![dunce::canonicalize(&root).unwrap()];
    let file = read_host_file(&roots, root.join("notes.md").to_str().unwrap())
        .await
        .unwrap();
    assert_eq!(file.content, "# notes\n");
    for (path, status) in [
        (root.join(".ssh/id"), 403),
        (base.join("outside.txt"), 403),
        (root.join("missing.md"), 404),
        (root.clone(), 400),
    ] {
        let error = read_host_file(&roots, path.to_str().unwrap())
            .await
            .unwrap_err();
        assert_eq!(error.status, status, "{}", path.display());
    }
    assert_eq!(
        read_host_file(&roots, "notes.md").await.unwrap_err().status,
        400
    );
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn relative_paths_stay_inside_the_workspace() {
    assert_eq!(
        safe_relative("./src/../src/main.rs")
            .err()
            .map(|e| e.status),
        Some(400),
        "a parent step is refused even when it would land back inside"
    );
    assert_eq!(
        safe_relative("./src/./main.rs").unwrap(),
        PathBuf::from("src/main.rs")
    );
    assert_eq!(safe_relative("").unwrap(), PathBuf::new());
    assert_eq!(
        safe_relative("/etc/passwd").err().map(|e| e.status),
        Some(400)
    );
    assert_eq!(safe_relative("a\0b").err().map(|e| e.status), Some(400));
}

#[test]
fn repository_metadata_and_credentials_are_protected_wherever_they_sit() {
    for path in [
        ".git",
        "nested/.git/config",
        ".ssh/id_rsa",
        "deploy/secrets",
    ] {
        assert!(is_protected_path(Path::new(path)), "{path}");
    }
    for path in [
        "src/main.rs",
        ".github/workflows/ci.yml",
        "secrets.md",
        "my.git",
    ] {
        assert!(!is_protected_path(Path::new(path)), "{path}");
    }
}

#[test]
fn git_status_reads_branch_tracking_and_renames() {
    let raw = b"## main...origin/main [ahead 2, behind 3]\0 M src/a.rs\0R  new.rs\0old.rs\0?? notes.txt\0";
    let status = parse_git_status(raw);
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert_eq!(status.upstream.as_deref(), Some("origin/main"));
    assert_eq!((status.ahead, status.behind), (Some(2), Some(3)));
    let files: Vec<_> = status
        .files
        .iter()
        .map(|f| {
            (
                f.index.as_str(),
                f.worktree.as_str(),
                f.path.as_str(),
                f.original_path.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        files,
        [
            (" ", "M", "src/a.rs", None),
            ("R", " ", "new.rs", Some("old.rs")),
            ("?", "?", "notes.txt", None),
        ]
    );
}

#[test]
fn git_status_without_upstream_or_commits() {
    let fresh = parse_git_status(b"## No commits yet on trunk\0");
    assert_eq!(fresh.branch.as_deref(), Some("trunk"));
    assert_eq!(fresh.upstream, None);
    let local = parse_git_status(b"## feature\0");
    assert_eq!(local.branch.as_deref(), Some("feature"));
    assert_eq!(
        (local.ahead, local.behind, local.upstream),
        (None, None, None)
    );
    assert!(parse_git_status(b"").files.is_empty());
}

#[test]
fn a_silent_git_failure_still_says_something() {
    assert_eq!(git_error(b"  \n"), "git command failed");
    assert_eq!(
        git_error(b"fatal: not a git repository\n"),
        "fatal: not a git repository"
    );
}

#[test]
fn worktree_listing_marks_the_main_checkout_and_skips_bare_repositories() {
    let listing = "worktree /repo\nHEAD abc\nbranch refs/heads/main\n\nworktree /repo.worktrees/feat\nHEAD def\nbranch refs/heads/feat/x\n\nworktree /detached\nHEAD 123\ndetached\n\nworktree /bare.git\nbare\n";
    let worktrees = parse_worktrees(listing);
    let summary: Vec<_> = worktrees
        .iter()
        .map(|w| {
            (
                Path::new(&w.path).to_path_buf(),
                w.branch.as_deref(),
                w.main,
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (PathBuf::from("/repo"), Some("main"), true),
            (PathBuf::from("/repo.worktrees/feat"), Some("feat/x"), false),
            (PathBuf::from("/detached"), None, false),
        ]
    );
}

#[test]
fn a_branch_gets_a_worktree_directory_beside_the_repository() {
    let main = Path::new("/work/app");
    assert_eq!(
        worktree_path(main, "feat/login page").unwrap(),
        PathBuf::from("/work/app.worktrees/feat-login-page")
    );
    assert_eq!(
        worktree_path(main, "-.release.-").unwrap(),
        PathBuf::from("/work/app.worktrees/release")
    );
    assert_eq!(
        worktree_path(main, "///").err().map(|e| e.status),
        Some(400)
    );
}

#[test]
fn attachment_names_keep_the_display_name_and_give_agents_a_plain_path() {
    let (display, extension, slug) = attachment_name("C:\\Users\\me\\Screen Shot 1.PNG");
    assert_eq!(
        (display.as_str(), extension.as_str(), slug.as_str()),
        ("Screen Shot 1.png", ".png", "Screen-Shot-1")
    );
    // No usable name at all still yields one.
    assert_eq!(attachment_name("...").0, "attachment");
    // An "extension" that is too long or not alphanumeric is part of the name.
    let (display, extension, _) = attachment_name("notes.final version");
    assert_eq!(
        (display.as_str(), extension.as_str()),
        ("notes.final version", "")
    );
    // Control characters cannot reach the path.
    assert!(!attachment_name("a\u{7}b.txt").2.contains('\u{7}'));
}

#[test]
fn pastes_are_kept_for_a_week() {
    let root = std::env::temp_dir().join(format!("tendly-prune-{}", Uuid::new_v4()));
    let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 15).unwrap();
    for day in ["2026-10-08", "2026-10-07"] {
        fs::create_dir_all(root.join(day)).unwrap();
    }
    prune_pastes(&root, today);
    assert!(root.join("2026-10-08").exists(), "seven days old is kept");
    assert!(
        !root.join("2026-10-07").exists(),
        "eight days old is cleared"
    );
    let _ = fs::remove_dir_all(&root);
}
