//! Workspaces, files, git and layouts end to end; origin checks and asset ranges; file change notifications; attachments.

use super::*;

#[tokio::test]
async fn workspace_file_git_layout_workflow() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let id = register(&f).await;
    let base = format!("/api/workspaces/{id}");
    assert_eq!(
        call(
            f.app(),
            "POST",
            "/api/workspaces",
            json!({"name":"bad","root_path":"/not-an-tendly-directory"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (status, file) = call(
        f.app(),
        "PUT",
        &format!("{base}/file?path=hello.txt"),
        json!({"content":"one\n"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        call(
            f.app(),
            "PUT",
            &format!("{base}/file?path=hello.txt"),
            json!({"content":"two\n","expected_version":"stale"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            f.app(),
            "PUT",
            &format!("{base}/file?path=hello.txt"),
            json!({"content":"two\n","expected_version":file["version"]})
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, diff) = call(
        f.app(),
        "GET",
        &format!("{base}/git/diff?path=hello.txt"),
        Value::Null,
    )
    .await;
    assert!(diff["diff"].as_str().unwrap().contains("+two"));
    assert_eq!(
        call(
            f.app(),
            "POST",
            &format!("{base}/git/stage"),
            json!({"paths":["hello.txt"]})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            f.app(),
            "POST",
            &format!("{base}/git/unstage"),
            json!({"paths":["hello.txt"]})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert!(f.path.join("repo/hello.txt").exists());
    call(
        f.app(),
        "POST",
        &format!("{base}/git/stage"),
        json!({"paths":["hello.txt"]}),
    )
    .await;
    let (status, result) = call(
        f.app(),
        "POST",
        &format!("{base}/git/commit"),
        json!({"message":"fixture commit"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["commit"].as_str().unwrap().len(), 40);
    let (_, status) = call(f.app(), "GET", &format!("{base}/git/status"), Value::Null).await;
    assert!(status["files"].as_array().unwrap().is_empty());
    // A worktree is part of its workspace: made beside the repository, never
    // registered as a workspace of its own.
    let (status, tree) = call(
        f.app(),
        "POST",
        &format!("{base}/git/worktrees"),
        json!({"branch":"feature/tree","create":true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{tree}");
    let tree_path = tree["path"].as_str().unwrap().to_owned();
    assert!(std::path::Path::new(&tree_path).ends_with("repo.worktrees/feature-tree"));
    assert_eq!(tree["branch"], "feature/tree");
    let (_, listed) = call(f.app(), "GET", "/api/workspaces", Value::Null).await;
    assert_eq!(listed.as_array().unwrap().len(), 1, "no second workspace");
    // Asking again finds the same worktree rather than making another.
    let (_, again) = call(
        f.app(),
        "POST",
        &format!("{base}/git/worktrees"),
        json!({"branch":"feature/tree"}),
    )
    .await;
    assert_eq!(again["path"], tree["path"]);
    let (status, _) = call(
        f.app(),
        "POST",
        &format!("{base}/git/worktrees"),
        json!({"path":"/tmp"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Views of a session in that worktree read and change it, not the
    // workspace directory; a directory that is not one of its worktrees is
    // refused.
    let at = |path: &str| {
        let sep = if path.contains('?') { '&' } else { '?' };
        format!(
            "{base}/{path}{sep}checkout={}",
            url::form_urlencoded::byte_serialize(tree_path.as_bytes()).collect::<String>()
        )
    };
    let (status, _) = call(
        f.app(),
        "PUT",
        &at("file?path=tree-only.txt"),
        json!({"content":"tree\n"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        std::path::Path::new(&tree_path)
            .join("tree-only.txt")
            .exists()
    );
    assert!(!f.path.join("repo/tree-only.txt").exists());
    let (_, tree_status) = call(f.app(), "GET", &at("git/status"), Value::Null).await;
    assert_eq!(tree_status["branch"], "feature/tree");
    assert_eq!(tree_status["files"][0]["path"], "tree-only.txt");
    let (_, home_status) = call(f.app(), "GET", &format!("{base}/git/status"), Value::Null).await;
    assert!(home_status["files"].as_array().unwrap().is_empty());
    let (_, listed) = call(f.app(), "GET", &at("files"), Value::Null).await;
    assert!(
        listed
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["path"] == "tree-only.txt")
    );
    let outside = std::env::temp_dir().to_string_lossy().into_owned();
    let (status, _) = call(
        f.app(),
        "GET",
        &format!(
            "{base}/git/status?checkout={}",
            url::form_urlencoded::byte_serialize(outside.as_bytes()).collect::<String>()
        ),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    std::fs::remove_file(std::path::Path::new(&tree_path).join("tree-only.txt")).unwrap();

    // A session moves to that checkout on its own; the workspace is untouched.
    let (_, session) = call(
        f.app(),
        "POST",
        &format!("{base}/sessions"),
        json!({"provider":"claude_code","title":"On a branch"}),
    )
    .await;
    let sid = session["id"].as_str().unwrap().to_owned();
    let (status, moved) = call(
        f.app(),
        "POST",
        &format!("/api/sessions/{sid}/checkout"),
        json!({"branch":"feature/tree"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert_eq!(moved["checkout_path"].as_str(), Some(tree_path.as_str()));
    assert_eq!(moved["checkout_branch"], "feature/tree");
    assert_eq!(moved["configuration_revision"], 1, "a new native context");
    // A branch renamed inside its worktree is followed: the session keeps its
    // directory and shows the new name.
    assert!(
        std::process::Command::new("git")
            .args(["branch", "-m", "feature/tree", "feature/renamed"])
            .current_dir(f.path.join("repo"))
            .status()
            .unwrap()
            .success()
    );
    call(f.app(), "GET", &format!("{base}/git/branches"), Value::Null).await;
    let (_, renamed) = call(f.app(), "GET", &format!("/api/sessions/{sid}"), Value::Null).await;
    assert_eq!(renamed["checkout_branch"], "feature/renamed");
    assert_eq!(renamed["checkout_path"].as_str(), Some(tree_path.as_str()));
    assert_eq!(
        renamed["configuration_revision"], 1,
        "a label, not a new context"
    );
    // Renaming through the API is followed the same way.
    let (status, _) = call(
        f.app(),
        "POST",
        &format!("{base}/git/branches/rename"),
        json!({"from":"feature/renamed","to":"feature/final"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, followed) = call(f.app(), "GET", &format!("/api/sessions/{sid}"), Value::Null).await;
    assert_eq!(followed["checkout_branch"], "feature/final");
    // Choosing the workspace's own branch brings it back to the workspace.
    let (_, home) = call(f.app(), "GET", &format!("{base}/git/branches"), Value::Null).await;
    let current = home["current"].as_str().unwrap().to_owned();
    let (_, back) = call(
        f.app(),
        "POST",
        &format!("/api/sessions/{sid}/checkout"),
        json!({"branch":current}),
    )
    .await;
    assert!(back["checkout_path"].is_null());
    // Removing a worktree moves the stopped sessions bound to it home first.
    call(
        f.app(),
        "POST",
        &format!("/api/sessions/{sid}/checkout"),
        json!({"branch":"feature/final"}),
    )
    .await;
    let (status, _) = call(
        f.app(),
        "POST",
        &format!("{base}/git/worktrees/remove"),
        json!({"path":tree_path}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, homed) = call(f.app(), "GET", &format!("/api/sessions/{sid}"), Value::Null).await;
    assert!(homed["checkout_path"].is_null());
    let (status, _) = call(
        f.app(),
        "POST",
        &format!("{base}/git/branches/delete"),
        json!({"branch":"feature/final","force":true}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, conversation) = call(
        f.app(),
        "GET",
        &format!("/api/sessions/{sid}/conversation"),
        Value::Null,
    )
    .await;
    let boundaries = conversation["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["type"] == "configuration")
        .count();
    assert_eq!(boundaries, 4, "each move is marked in the conversation");
    // A session running in the workspace directory holds the switch until
    // the caller asks for it to be ended.
    f.state
        .store
        .set_session_status(sid.parse().unwrap(), SessionStatus::Running)
        .unwrap();
    let (status, _) = call(
        f.app(),
        "POST",
        &format!("{base}/git/switch"),
        json!({"branch":"side","create":true}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = call(
        f.app(),
        "POST",
        &format!("{base}/git/switch"),
        json!({"branch":"side","create":true,"stop_sessions":true}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, ended) = call(f.app(), "GET", &format!("/api/sessions/{sid}"), Value::Null).await;
    assert_eq!(ended["status"], "stopped");
    let layout = default_layout();
    assert_eq!(
        call(f.app(), "PUT", &format!("{base}/layout"), layout.clone())
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(f.app(), "GET", &format!("{base}/layout"), Value::Null)
            .await
            .1,
        layout
    );
    assert_eq!(
        call(
            f.app(),
            "PUT",
            &format!("{base}/layout"),
            json!({"version":1,"root":{"id":"a","type":"split"}})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            f.app(),
            "GET",
            &format!("{base}/file?path=../state"),
            Value::Null
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn origin_auth_and_asset_range() {
    let f = Fixture::new(
        "127.0.0.1:8787".parse().unwrap(),
        Some("fixture-token-long-enough"),
    );
    let app = f.app();
    assert_eq!(
        call(app.clone(), "GET", "/api/workspaces", Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let attack = Request::builder()
        .method("POST")
        .uri("/api/workspaces")
        .header("host", "127.0.0.1:8787")
        .header("origin", "https://attacker.invalid")
        .header("x-tendly-client", "web")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(attack).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    let login = Request::builder()
        .method("POST")
        .uri("/api/auth")
        .header("host", "127.0.0.1:8787")
        .header("x-tendly-client", "web")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"token":"fixture-token-long-enough"}).to_string(),
        ))
        .unwrap();
    let response = app.clone().oneshot(login).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("HttpOnly")
    );
    let w = f
        .state
        .store
        .create_workspace("range", f.path.join("repo").to_str().unwrap())
        .unwrap();
    fs::write(f.path.join("repo/image.png"), b"0123456789").unwrap();
    let req = Request::builder()
        .uri(format!("/api/workspaces/{}/asset?path=image.png", w.id))
        .header("host", "127.0.0.1:8787")
        .header("authorization", "Bearer fixture-token-long-enough")
        .header("range", "bytes=2-5")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        &to_bytes(response.into_body(), 100).await.unwrap()[..],
        b"2345"
    );
}

#[tokio::test]
async fn file_socket_reports_changes_on_disk() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let f = Fixture::new(addr, None);
    let repo = f.path.join("repo");
    fs::create_dir(repo.join("src")).unwrap();
    let w = f
        .state
        .store
        .create_workspace("watch", repo.to_str().unwrap())
        .unwrap();
    let app = f.app();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let mut req = format!("ws://{addr}/api/workspaces/{}/files/ws", w.id)
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert("origin", format!("http://{addr}").parse().unwrap());
    let (mut ws, _) = connect_async(req).await.unwrap();
    async fn message(
        ws: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) -> Value {
        loop {
            match tokio::time::timeout(Duration::from_secs(5), ws.next())
                .await
                .expect("file socket message")
            {
                Some(Ok(WsMessage::Text(text))) => return serde_json::from_str(&text).unwrap(),
                Some(Ok(_)) => continue,
                other => panic!("file socket ended: {other:?}"),
            }
        }
    }
    assert_eq!(message(&mut ws).await["type"], "ready");
    fs::write(repo.join("src/new.ts"), "export {};\n").unwrap();
    fs::create_dir(repo.join("node_modules")).unwrap();
    let mut seen = Vec::new();
    while !seen.iter().any(|path| path == "src/new.ts") {
        let change = message(&mut ws).await;
        assert_eq!(change["type"], "changed");
        seen.extend(
            change["paths"]
                .as_array()
                .unwrap()
                .iter()
                .map(|path| path.as_str().unwrap().to_owned()),
        );
    }
    assert!(!seen.iter().any(|path| path.starts_with("node_modules")));
    server.abort();
}

/// Attachments land inside the workspace under their own directory, keep a
/// recognisable name, and cannot be steered by a client-supplied path.
#[tokio::test]
async fn attachment_upload_stays_inside_its_own_workspace_directory() {
    let f = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let id = register(&f).await;
    let upload = |name: &str, body: &'static [u8]| {
        let app = f.app();
        let uri = format!(
            "/api/workspaces/{id}/attachments?name={}",
            urlencoding(name)
        );
        async move {
            let request = Request::builder()
                .method("POST")
                .uri(uri)
                .header("host", "127.0.0.1:8787")
                .header("x-tendly-client", "web")
                .header("content-type", "application/octet-stream")
                .body(Body::from(body))
                .unwrap();
            let response = app.oneshot(request).await.unwrap();
            let status = response.status();
            let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
            (
                status,
                serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
            )
        }
    };
    let (status, saved) = upload("my holiday photo.JPG", b"fixture-bytes").await;
    assert_eq!(status, StatusCode::OK);
    let path = saved["path"].as_str().unwrap();
    assert!(path.starts_with(".tendly-files/"), "{path}");
    assert!(path.ends_with(".jpg"), "{path}");
    // The stored path is handed to an agent as text, so it carries no spaces.
    assert!(!path.contains(' '), "{path}");
    assert!(path.contains("my-holiday-photo"), "{path}");
    // The label keeps the user's own spelling.
    assert_eq!(saved["name"], "my holiday photo.jpg");
    assert_eq!(saved["bytes"], 13);
    let stored = f.path.join("repo").join(path);
    assert_eq!(fs::read(&stored).unwrap(), b"fixture-bytes");
    // Uploads must not become pending Git changes in the user's repository,
    // and the ignore stays scoped to this directory.
    assert_eq!(
        fs::read_to_string(f.path.join("repo/.tendly-files/.gitignore")).unwrap(),
        "*\n"
    );
    assert!(!f.path.join("repo/.gitignore").exists());
    let (_, status) = call(
        f.app(),
        "GET",
        &format!("/api/workspaces/{id}/git/status"),
        Value::Null,
    )
    .await;
    assert!(
        !status["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["path"]
                .as_str()
                .is_some_and(|path| path.contains(".tendly-files"))),
        "{status:?}"
    );

    // Two uploads of the same camera name both survive.
    let (_, second) = upload("my holiday photo.JPG", b"second-bytes").await;
    assert_ne!(second["path"], saved["path"]);
    assert_eq!(fs::read(&stored).unwrap(), b"fixture-bytes");

    // A path in the name is reduced to its base name; nothing escapes.
    let (status, traversal) = upload("../../etc/passwd", b"nope").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        traversal["path"]
            .as_str()
            .unwrap()
            .starts_with(".tendly-files/"),
        "{traversal:?}"
    );
    assert!(!f.path.join("etc").exists());
    assert_eq!(traversal["name"], "passwd");

    // Empty uploads and blank names are refused rather than stored.
    assert_eq!(upload("empty.txt", b"").await.0, StatusCode::BAD_REQUEST);
    assert_eq!(upload("   ", b"data").await.0, StatusCode::BAD_REQUEST);
}
