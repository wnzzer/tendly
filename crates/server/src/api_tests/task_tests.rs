//! Isolated tasks: a worktree made for one session, prepared, then merged
//! back into its base or thrown away.

use super::*;

fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// A workspace whose repository has one commit on `main`, and a session in it.
async fn task_fixture(f: &Fixture, title: &str) -> (String, String) {
    let repo = f.path.join("repo");
    git(&repo, &["checkout", "-qb", "main"]);
    fs::write(repo.join("app.txt"), "one\n").unwrap();
    fs::write(repo.join(".gitignore"), ".env\nnode_modules/\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "start"]);
    let workspace = register(f).await;
    let (status, session) = call(
        f.app(),
        "POST",
        &format!("/api/workspaces/{workspace}/sessions"),
        json!({"provider":"codex","title":title}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{session}");
    (workspace, session["id"].as_str().unwrap().to_owned())
}

async fn isolate(f: &Fixture, session: &str, body: Value) -> (StatusCode, Value) {
    call(
        f.app(),
        "POST",
        &format!("/api/sessions/{session}/checkout"),
        body,
    )
    .await
}

#[tokio::test]
async fn an_isolated_task_gets_a_new_branch_and_worktree_of_its_own() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let (workspace, sid) = task_fixture(&f, "Fix the login page").await;
    let (status, moved) = isolate(&f, &sid, json!({"isolated":true})).await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert_eq!(moved["checkout_branch"], "tendly/fix-the-login-page");
    assert_eq!(moved["checkout_base"], "main");
    let tree = moved["checkout_path"].as_str().unwrap().to_owned();
    assert!(
        tree.ends_with("repo.worktrees/tendly-fix-the-login-page"),
        "{tree}"
    );
    assert_eq!(
        git(std::path::Path::new(&tree), &["branch", "--show-current"]),
        "tendly/fix-the-login-page"
    );

    // A second task of the same name gets the next name, never the same place.
    let (_, other) = call(
        f.app(),
        "POST",
        &format!("/api/workspaces/{workspace}/sessions"),
        json!({"provider":"codex","title":"Fix the login page"}),
    )
    .await;
    let other = other["id"].as_str().unwrap();
    let (status, second) = isolate(&f, other, json!({"isolated":true})).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["checkout_branch"], "tendly/fix-the-login-page-2");
    assert_ne!(second["checkout_path"], moved["checkout_path"]);

    // Isolation refuses a branch already checked out anywhere, the workspace
    // directory's included, rather than landing beside someone.
    for branch in ["main", "tendly/fix-the-login-page"] {
        let (_, third) = call(
            f.app(),
            "POST",
            &format!("/api/workspaces/{workspace}/sessions"),
            json!({"provider":"codex","title":"Third"}),
        )
        .await;
        let (status, refused) = isolate(
            &f,
            third["id"].as_str().unwrap(),
            json!({"isolated":true,"branch":branch}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{refused}");
        assert!(
            refused["error"]
                .as_str()
                .unwrap()
                .contains("already checked out"),
            "{refused}"
        );
    }
    // A base that is no local branch is refused before anything is made.
    let (status, _) = isolate(&f, other, json!({"isolated":true,"base":"nope"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // The Git pane's worktree route makes no isolated worktrees.
    let (status, _) = call(
        f.app(),
        "POST",
        &format!("/api/workspaces/{workspace}/git/worktrees"),
        json!({"isolated":true}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Moving it by hand afterwards is no longer an isolated task.
    let (status, home) = isolate(&f, &sid, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{home}");
    assert!(home["checkout_path"].is_null());
    assert!(home["checkout_base"].is_null());
}

#[cfg(unix)]
#[tokio::test]
async fn a_new_worktree_is_prepared_before_its_session_starts() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let (workspace, sid) = task_fixture(&f, "Prepared").await;
    let repo = f.path.join("repo");
    fs::write(repo.join(".env"), "SECRET=1\n").unwrap();
    fs::create_dir_all(repo.join("node_modules/pkg")).unwrap();
    fs::write(repo.join("node_modules/pkg/index.js"), "x").unwrap();
    // Invalid settings are refused; blank entries are dropped.
    let (status, _) = call(
        f.app(),
        "PATCH",
        &format!("/api/workspaces/{workspace}"),
        json!({"worktree_setup":{"copy":["../outside"]}}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, saved) = call(
        f.app(),
        "PATCH",
        &format!("/api/workspaces/{workspace}"),
        json!({"worktree_setup":{"copy":[".env"," node_modules/ ","", "missing"],"command":"sleep 1; cat \"$TENDLY_WORKSPACE_ROOT/app.txt\" > from-home.txt; echo prepared $TENDLY_BRANCH"}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert_eq!(
        saved["worktree_setup"]["copy"],
        json!([".env", "node_modules", "missing"])
    );

    let (status, moved) = isolate(&f, &sid, json!({"isolated":true})).await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    let tree = PathBuf::from(moved["checkout_path"].as_str().unwrap());
    let id: Uuid = sid.parse().unwrap();
    // A start waits for the setup; the launch refuses while it runs.
    let session = f.state.store.get_session(id).unwrap().unwrap();
    assert!(crate::worktree_setup::preparing(
        &f.state,
        tree.to_str().unwrap()
    ));
    assert!(crate::launch::prepare(&f.state, &session).await.is_err());
    tokio::time::timeout(
        Duration::from_secs(20),
        crate::worktree_setup::ready_for(&f.state, id),
    )
    .await
    .unwrap();
    assert!(crate::launch::prepare(&f.state, &session).await.is_ok());

    assert_eq!(fs::read_to_string(tree.join(".env")).unwrap(), "SECRET=1\n");
    assert_eq!(
        fs::read_to_string(tree.join("node_modules/pkg/index.js")).unwrap(),
        "x"
    );
    assert_eq!(
        fs::read_to_string(tree.join("from-home.txt")).unwrap(),
        "one\n"
    );
    // Copied dependencies are ignored files: only what the command made shows.
    assert_eq!(git(&tree, &["status", "--porcelain"]), "?? from-home.txt");
    let events = f.state.store.conversation(id).unwrap().events;
    let setups: Vec<&Value> = events.iter().filter(|e| e["type"] == "setup").collect();
    assert_eq!(setups.len(), 2, "{events:?}");
    assert_eq!(setups[0]["status"], "running");
    assert_eq!(setups[1]["status"], "completed");
    assert_eq!(setups[0]["id"], setups[1]["id"]);
    let log = setups[1]["log"].as_str().unwrap();
    assert!(log.contains("Copied .env."), "{log}");
    assert!(log.contains("Skipped missing"), "{log}");
    assert!(log.contains("prepared tendly/prepared"), "{log}");
}

#[cfg(unix)]
#[tokio::test]
async fn a_failing_setup_is_reported_and_the_session_still_starts() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let (workspace, sid) = task_fixture(&f, "Broken setup").await;
    call(
        f.app(),
        "PATCH",
        &format!("/api/workspaces/{workspace}"),
        json!({"worktree_setup":{"command":"echo nope >&2; exit 3"}}),
    )
    .await;
    isolate(&f, &sid, json!({"isolated":true})).await;
    let id: Uuid = sid.parse().unwrap();
    tokio::time::timeout(
        Duration::from_secs(20),
        crate::worktree_setup::ready_for(&f.state, id),
    )
    .await
    .unwrap();
    let events = f.state.store.conversation(id).unwrap().events;
    let done = events.iter().rfind(|e| e["type"] == "setup").unwrap();
    assert_eq!(done["status"], "failed");
    assert!(done["log"].as_str().unwrap().contains("nope"), "{done}");
    let session = f.state.store.get_session(id).unwrap().unwrap();
    assert!(crate::launch::prepare(&f.state, &session).await.is_ok());
}

#[tokio::test]
async fn a_finished_task_merges_into_its_base_and_is_put_away() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let (_, sid) = task_fixture(&f, "Add a feature").await;
    let repo = f.path.join("repo");
    let (_, moved) = isolate(&f, &sid, json!({"isolated":true})).await;
    let tree = PathBuf::from(moved["checkout_path"].as_str().unwrap());
    fs::write(tree.join("feature.txt"), "new\n").unwrap();
    fs::write(tree.join("app.txt"), "one\ntwo\n").unwrap();

    let (status, state) = call(
        f.app(),
        "GET",
        &format!("/api/sessions/{sid}/task"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{state}");
    assert_eq!(state["base"], "main");
    assert_eq!(state["ahead"], 0);
    assert_eq!(state["uncommitted"], 2);
    assert_eq!(state["base_is_workspace"], true);
    let files = state["files"].as_array().unwrap();
    assert!(
        files
            .iter()
            .any(|f| f["path"] == "app.txt" && f["added"] == 1)
    );
    assert!(
        files
            .iter()
            .any(|f| f["path"] == "feature.txt" && f["untracked"] == true)
    );
    let (_, diff) = call(
        f.app(),
        "GET",
        &format!("/api/sessions/{sid}/task/diff"),
        Value::Null,
    )
    .await;
    assert!(diff["diff"].as_str().unwrap().contains("+two"), "{diff}");

    // Uncommitted work refuses a merge, and closing without discarding.
    let (status, refused) = call(
        f.app(),
        "POST",
        &format!("/api/sessions/{sid}/task/merge"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    let (status, _) = call(
        f.app(),
        "POST",
        &format!("/api/sessions/{sid}/task/close"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    git(&tree, &["add", "."]);
    git(&tree, &["commit", "-qm", "feature"]);
    let (_, state) = call(
        f.app(),
        "GET",
        &format!("/api/sessions/{sid}/task"),
        Value::Null,
    )
    .await;
    assert_eq!(state["ahead"], 1);
    assert_eq!(state["uncommitted"], 0);

    let (status, done) = call(
        f.app(),
        "POST",
        &format!("/api/sessions/{sid}/task/merge"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert!(done["archived_at"].is_string());
    assert!(done["checkout_path"].is_null());
    assert!(done["checkout_base"].is_null());
    assert_eq!(
        fs::read_to_string(repo.join("feature.txt")).unwrap(),
        "new\n"
    );
    assert!(!tree.exists());
    assert_eq!(git(&repo, &["branch", "--list", "tendly/*"]), "");
}

#[tokio::test]
async fn a_conflicting_task_is_refused_before_the_base_changes() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let (_, sid) = task_fixture(&f, "Conflict").await;
    let repo = f.path.join("repo");
    let (_, moved) = isolate(&f, &sid, json!({"isolated":true})).await;
    let tree = PathBuf::from(moved["checkout_path"].as_str().unwrap());
    fs::write(tree.join("app.txt"), "task\n").unwrap();
    git(&tree, &["commit", "-qam", "task side"]);
    fs::write(repo.join("app.txt"), "home\n").unwrap();
    git(&repo, &["commit", "-qam", "home side"]);
    let head = git(&repo, &["rev-parse", "HEAD"]);

    let (status, refused) = call(
        f.app(),
        "POST",
        &format!("/api/sessions/{sid}/task/merge"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert!(
        refused["error"].as_str().unwrap().contains("app.txt"),
        "{refused}"
    );
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head);
    assert_eq!(git(&repo, &["status", "--porcelain"]), "");
    assert!(tree.exists());

    // Discarding throws the work away and puts the task away all the same.
    let (status, done) = call(
        f.app(),
        "POST",
        &format!("/api/sessions/{sid}/task/close"),
        json!({"discard":true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert!(!tree.exists());
    assert_eq!(git(&repo, &["branch", "--list", "tendly/*"]), "");
    assert_eq!(fs::read_to_string(repo.join("app.txt")).unwrap(), "home\n");
}

#[tokio::test]
async fn a_task_merges_into_a_base_checked_out_nowhere() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let (_, sid) = task_fixture(&f, "Elsewhere").await;
    let repo = f.path.join("repo");
    git(&repo, &["branch", "release"]);
    let (status, moved) = isolate(
        &f,
        &sid,
        json!({"isolated":true,"base":"release","branch":"fix/one"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert_eq!(moved["checkout_branch"], "fix/one");
    let tree = PathBuf::from(moved["checkout_path"].as_str().unwrap());
    fs::write(tree.join("fix.txt"), "fixed\n").unwrap();
    git(&tree, &["add", "."]);
    git(&tree, &["commit", "-qm", "fix"]);
    // release moves on meanwhile, so this is a true merge, made without a checkout.
    git(&repo, &["checkout", "-q", "release"]);
    fs::write(repo.join("other.txt"), "x\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "release side"]);
    git(&repo, &["checkout", "-q", "main"]);

    let (status, done) = call(
        f.app(),
        "POST",
        &format!("/api/sessions/{sid}/task/merge"),
        json!({"squash":true,"message":"One fix"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(
        git(&repo, &["log", "-1", "--format=%s", "release"]),
        "One fix"
    );
    assert_eq!(git(&repo, &["show", "release:fix.txt"]), "fixed");
    assert_eq!(git(&repo, &["show", "release:other.txt"]), "x");
    // The workspace directory, on main, was never touched.
    assert!(!repo.join("fix.txt").exists());
    assert_eq!(git(&repo, &["branch", "--list", "fix/*"]), "");
}

#[tokio::test]
async fn a_task_that_never_started_goes_with_its_temporary_session() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let (workspace, _) = task_fixture(&f, "Unused").await;
    let (_, temp) = call(
        f.app(),
        "POST",
        &format!("/api/workspaces/{workspace}/sessions"),
        json!({"provider":"codex","title":"Throwaway","ephemeral":true}),
    )
    .await;
    let temp = temp["id"].as_str().unwrap();
    let (_, moved) = isolate(&f, temp, json!({"isolated":true})).await;
    let tree = PathBuf::from(moved["checkout_path"].as_str().unwrap());
    assert!(tree.exists());
    let (status, _) = call(
        f.app(),
        "DELETE",
        &format!("/api/sessions/{temp}"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!tree.exists());
    assert_eq!(
        git(&f.path.join("repo"), &["branch", "--list", "tendly/*"]),
        ""
    );
}

#[tokio::test]
async fn a_merge_into_a_branch_other_than_head_still_puts_the_task_away() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let (workspace, sid) = task_fixture(&f, "First").await;
    let repo = f.path.join("repo");
    git(&repo, &["branch", "release"]);
    // Fast-forward: release has not moved.
    let (_, moved) = isolate(&f, &sid, json!({"isolated":true,"base":"release"})).await;
    let tree = PathBuf::from(moved["checkout_path"].as_str().unwrap());
    fs::write(tree.join("a.txt"), "a\n").unwrap();
    git(&tree, &["add", "."]);
    git(&tree, &["commit", "-qm", "a"]);
    let (status, done) = call(
        f.app(),
        "POST",
        &format!("/api/sessions/{sid}/task/merge"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(git(&repo, &["show", "release:a.txt"]), "a");
    // A true merge: release moved on after the task began.
    let (_, other) = call(
        f.app(),
        "POST",
        &format!("/api/workspaces/{workspace}/sessions"),
        json!({"provider":"codex","title":"Second"}),
    )
    .await;
    let other = other["id"].as_str().unwrap();
    let (_, moved) = isolate(&f, other, json!({"isolated":true,"base":"release"})).await;
    let tree = PathBuf::from(moved["checkout_path"].as_str().unwrap());
    fs::write(tree.join("b.txt"), "b\n").unwrap();
    git(&tree, &["add", "."]);
    git(&tree, &["commit", "-qm", "b"]);
    git(&repo, &["checkout", "-q", "release"]);
    fs::write(repo.join("c.txt"), "c\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "c"]);
    git(&repo, &["checkout", "-q", "main"]);
    let (status, done) = call(
        f.app(),
        "POST",
        &format!("/api/sessions/{other}/task/merge"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(git(&repo, &["show", "release:b.txt"]), "b");
    assert_eq!(git(&repo, &["show", "release:c.txt"]), "c");
    assert_eq!(
        git(&repo, &["rev-list", "--parents", "-n", "1", "release"])
            .split(' ')
            .count(),
        3
    );
    assert_eq!(git(&repo, &["branch", "--list", "tendly/*"]), "");
    assert!(!tree.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn a_merge_a_hook_refuses_leaves_the_base_as_it_was() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let (_, sid) = task_fixture(&f, "Hooked").await;
    let repo = f.path.join("repo");
    let (_, moved) = isolate(&f, &sid, json!({"isolated":true})).await;
    let tree = PathBuf::from(moved["checkout_path"].as_str().unwrap());
    fs::write(tree.join("app.txt"), "one\nhooked\n").unwrap();
    git(&tree, &["commit", "-qam", "hooked"]);
    let hook = repo.join(".git/hooks/pre-commit");
    fs::write(&hook, "#!/bin/sh\necho no commits today >&2\nexit 1\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    let head = git(&repo, &["rev-parse", "HEAD"]);
    let (status, refused) = call(
        f.app(),
        "POST",
        &format!("/api/sessions/{sid}/task/merge"),
        json!({"squash":true}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains("no commits today"),
        "{refused}"
    );
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head);
    assert_eq!(git(&repo, &["status", "--porcelain"]), "");
    assert!(tree.exists());
    assert_eq!(
        f.state
            .store
            .get_session(sid.parse().unwrap())
            .unwrap()
            .unwrap()
            .checkout_base
            .as_deref(),
        Some("main")
    );
}
