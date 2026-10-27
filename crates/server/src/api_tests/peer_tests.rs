//! Two machines paired with each other, over real HTTP.

use super::*;

async fn serve(token: Option<&str>) -> (Fixture, SocketAddr) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let f = Fixture::new(address, token);
    let app = f.app();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (f, address)
}

/// `call`, addressed to a fixture bound somewhere other than 8787.
async fn call_on(
    f: &Fixture,
    address: SocketAddr,
    method: &str,
    path: &str,
    value: Value,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("host", address.to_string())
        .header("x-tendly-client", "web")
        .header("content-type", "application/json")
        .header(header::AUTHORIZATION, "Bearer there-token")
        .body(Body::from(value.to_string()))
        .unwrap();
    let response = f.app().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn tool_text(result: &Value) -> String {
    result["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test]
async fn a_machine_pairs_by_invite_and_reaches_only_what_it_was_granted() {
    let (there, address) = serve(Some("there-token")).await;
    let workspace = there
        .state
        .store
        .create_workspace("shared", there.path.join("repo").to_str().unwrap())
        .unwrap();
    there
        .state
        .store
        .create_session(workspace.id, ProviderKind::ClaudeCode, "remote work")
        .unwrap();
    let here = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);

    let code = there.state.peers.invite(peers::Grant::View);
    let link = format!("http://{address}/settings/machines#tendly-pair={code}");
    let (status, peer) = call(
        here.app(),
        "POST",
        "/api/peers",
        json!({ "invite": link, "grant": "spawn" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{peer}");
    assert_eq!(peer["granted"], "view");
    assert_eq!(peer["grant"], "spawn");
    let id = peer["id"].as_str().unwrap().to_owned();

    // The same invite does not pair a second machine.
    let other = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let (status, refused) = call(
        other.app(),
        "POST",
        "/api/peers",
        json!({ "invite": link, "grant": "view" }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");

    let (_, listed) = call(here.app(), "GET", "/api/peers", Value::Null).await;
    assert_eq!(listed["peers"][0]["online"], true, "{listed}");
    // The other side knows this machine, with what it said this one allows there.
    let (_, theirs) = call_on(&there, address, "GET", "/api/peers", Value::Null).await;
    assert_eq!(theirs["peers"][0]["grant"], "view", "{theirs}");
    assert_eq!(theirs["peers"][0]["granted"], "spawn");

    let (status, remote) = call(
        here.app(),
        "GET",
        &format!("/api/peers/{id}/sessions"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{remote}");
    assert_eq!(remote["workspaces"][0]["name"], "shared");
    assert_eq!(remote["sessions"][0]["title"], "remote work");
    // The sidebar asks every machine at once.
    let (status, all) = call(here.app(), "GET", "/api/peers/sessions", Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{all}");
    let machine = &all["machines"][0];
    assert_eq!(
        (&machine["online"], &machine["granted"]),
        (&json!(true), &json!("view"))
    );
    assert_eq!(machine["sessions"][0]["title"], "remote work");

    // An agent here lists the workspaces there by naming the machine.
    let (_, listed) = call(
        here.app(),
        "POST",
        "/api/agent/call",
        json!({ "name": "tendly_list", "arguments": { "what": "workspaces", "machine": id } }),
    )
    .await;
    assert_eq!(listed["isError"], false, "{listed}");
    assert!(tool_text(&listed).contains("shared"));
    // Starting one there takes a session of this machine, and that machine's leave.
    let (_, spawned) = call(
        here.app(),
        "POST",
        "/api/agent/call",
        json!({ "name": "tendly_spawn", "arguments": { "provider": "codex", "prompt": "hi", "machine": id } }),
    )
    .await;
    assert_eq!(spawned["isError"], true);

    // View does not open its window; control does, once.
    let (status, _) = call(
        here.app(),
        "POST",
        &format!("/api/peers/{id}/open"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let mine = theirs["peers"][0]["id"].as_str().unwrap();
    let (status, _) = call_on(
        &there,
        address,
        "PUT",
        &format!("/api/peers/{mine}"),
        json!({ "grant": "control" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, opened) = call(
        here.app(),
        "POST",
        &format!("/api/peers/{id}/open"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{opened}");
    let url = opened["url"].as_str().unwrap();
    let ticket = url.split("#tendly-ticket=").nth(1).unwrap();
    let sign_in = |ticket: String| async move {
        reqwest::Client::new()
            .post(format!("http://{address}/api/auth"))
            .header("x-tendly-client", "web")
            .json(&json!({ "ticket": ticket }))
            .send()
            .await
            .unwrap()
    };
    let signed = sign_in(ticket.to_owned()).await;
    assert_eq!(signed.status(), 200);
    assert!(signed.headers().get("set-cookie").is_some());
    assert_eq!(sign_in(ticket.to_owned()).await.status(), 401, "once only");

    // Its key opens the peer API there and nothing else.
    let saved: Value =
        serde_json::from_str(&fs::read_to_string(here.state.state_dir.join("peers.json")).unwrap())
            .unwrap();
    let key = saved[0]["token"].as_str().unwrap();
    let with_key = |path: &str| {
        reqwest::Client::new()
            .get(format!("http://{address}{path}"))
            .bearer_auth(key)
            .send()
    };
    assert_eq!(with_key("/api/peer/hello").await.unwrap().status(), 200);
    assert_eq!(with_key("/api/workspaces").await.unwrap().status(), 401);

    // Removing it here removes this machine there as well.
    let (status, _) = call(
        here.app(),
        "DELETE",
        &format!("/api/peers/{id}"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, theirs) = call_on(&there, address, "GET", "/api/peers", Value::Null).await;
    assert_eq!(theirs["peers"], json!([]));
    assert_eq!(with_key("/api/peer/hello").await.unwrap().status(), 401);
}

/// A machine paired with a served one: what this one calls it, and the key it
/// presents there.
async fn paired(
    here: &Fixture,
    there: &Fixture,
    address: SocketAddr,
    grant: peers::Grant,
) -> (String, String) {
    let code = there.state.peers.invite(grant);
    let (status, peer) = call(
        here.app(),
        "POST",
        "/api/peers",
        json!({ "invite": format!("http://{address}/#tendly-pair={code}"), "grant": "view" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{peer}");
    let saved: Value =
        serde_json::from_str(&fs::read_to_string(here.state.state_dir.join("peers.json")).unwrap())
            .unwrap();
    let id = peer["id"].as_str().unwrap().to_owned();
    let key = saved
        .as_array()
        .unwrap()
        .iter()
        .find(|saved| saved["id"] == id.as_str())
        .unwrap()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    (id, key)
}

/// What the machine at `address` answers a paired machine's call with.
async fn peer_call(address: SocketAddr, key: &str, name: &str, arguments: Value) -> Value {
    reqwest::Client::new()
        .post(format!("http://{address}/api/peer/call"))
        .header("x-tendly-client", "peer")
        .bearer_auth(key)
        .json(&json!({ "name": name, "arguments": arguments }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

#[tokio::test]
async fn a_key_reaches_only_as_far_as_its_grant_and_never_a_third_machine() {
    let (there, address) = serve(Some("there-token")).await;
    let here = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let (_, key) = paired(&here, &there, address, peers::Grant::View).await;

    let listed = peer_call(
        address,
        &key,
        "tendly_list",
        json!({ "what": "workspaces" }),
    )
    .await;
    assert!(listed["ok"].is_array(), "{listed}");
    for (name, arguments) in [
        (
            "tendly_spawn",
            json!({ "provider": "codex", "prompt": "hi", "workspace": "x" }),
        ),
        ("tendly_session", json!({ "action": "list" })),
    ] {
        let refused = peer_call(address, &key, name, arguments).await;
        assert!(
            refused["error"].as_str().unwrap().contains("does not let"),
            "{name}: {refused}"
        );
    }
    let refused = peer_call(
        address,
        &key,
        "tendly_endpoint",
        json!({ "action": "list" }),
    )
    .await;
    assert!(
        refused["error"].as_str().unwrap().contains("not available"),
        "{refused}"
    );
    let refused = peer_call(
        address,
        &key,
        "tendly_list",
        json!({ "what": "workspaces", "machine": "else" }),
    )
    .await;
    assert!(
        refused["error"].as_str().unwrap().contains("third"),
        "{refused}"
    );

    // The key is not the deployment's token, whatever header comes with it.
    let client = reqwest::Client::new();
    for (method, path, client_header) in [
        (reqwest::Method::GET, "/api/peers", "web"),
        (reqwest::Method::POST, "/api/workspaces", "peer"),
        (reqwest::Method::POST, "/api/agent/call", "agent"),
    ] {
        let status = client
            .request(method.clone(), format!("http://{address}{path}"))
            .header("x-tendly-client", client_header)
            .bearer_auth(&key)
            .json(&json!({}))
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(status, 401, "{method} {path}");
    }
    // And a paired machine's header opens nothing outside /api/peer/ for the token either.
    let status = client
        .post(format!("http://{address}/api/workspaces"))
        .header("x-tendly-client", "peer")
        .bearer_auth("there-token")
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, 403);
    let ticket = client
        .post(format!("http://{address}/api/peer/ticket"))
        .header("x-tendly-client", "peer")
        .bearer_auth(&key)
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(ticket, 403, "view does not open the window");
}

#[tokio::test]
async fn pairing_refuses_a_wrong_code_another_version_and_a_short_key() {
    let (there, address) = serve(Some("there-token")).await;
    let pair = |body: Value| async move {
        reqwest::Client::new()
            .post(format!("http://{address}/api/peer/pair"))
            .header("x-tendly-client", "peer")
            .json(&body)
            .send()
            .await
            .unwrap()
            .status()
    };
    let key = "k".repeat(40);
    let body = |code: &str, api: u64, token: &str| json!({ "code": code, "name": "here", "url": null, "token": token, "grant": "view", "peer_api": api });
    assert_eq!(pair(body("WRONGWRONG00", 1, &key)).await, 403);
    let code = there.state.peers.invite(peers::Grant::View);
    assert_eq!(pair(body(&code, 2, &key)).await, 409);
    let code = there.state.peers.invite(peers::Grant::View);
    assert_eq!(pair(body(&code, 1, "short")).await, 400);
    let (_, theirs) = call_on(&there, address, "GET", "/api/peers", Value::Null).await;
    assert_eq!(theirs["peers"], json!([]), "nothing was paired");
    // Pairing with oneself is refused before anything is sent.
    let here = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let (status, _) = call(
        here.app(),
        "POST",
        "/api/peers",
        json!({ "invite": "not a link", "grant": "view" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_changed_grant_is_told_to_the_other_side() {
    let (there, address) = serve(Some("there-token")).await;
    let here = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let (id, _) = paired(&here, &there, address, peers::Grant::View).await;
    let (status, changed) = call(
        here.app(),
        "PUT",
        &format!("/api/peers/{id}"),
        json!({ "grant": "control" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(changed["grant"], "control");
    let (_, theirs) = call_on(&there, address, "GET", "/api/peers", Value::Null).await;
    assert_eq!(theirs["peers"][0]["granted"], "control", "{theirs}");
    assert_eq!(
        theirs["peers"][0]["grant"], "view",
        "its own choice is unchanged"
    );
}

#[tokio::test]
async fn a_machine_that_stopped_answering_is_shown_offline_and_says_why() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let there = Fixture::new(address, Some("there-token"));
    let app = there.app();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let here = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let (id, _) = paired(&here, &there, address, peers::Grant::View).await;
    server.abort();
    let _ = server.await;

    let (status, listed) = call(here.app(), "GET", "/api/peers", Value::Null).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a dead machine does not break the list"
    );
    assert_eq!(listed["peers"][0]["online"], false);
    assert!(
        listed["peers"][0]["problem"]
            .as_str()
            .unwrap()
            .contains("cannot be reached"),
        "{listed}"
    );
    let (status, _) = call(
        here.app(),
        "GET",
        &format!("/api/peers/{id}/sessions"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, all) = call(here.app(), "GET", "/api/peers/sessions", Value::Null).await;
    assert_eq!(status, StatusCode::OK, "the sidebar still gets the others");
    assert_eq!(all["machines"][0]["online"], false);
    assert!(
        all["machines"][0]["problem"]
            .as_str()
            .unwrap()
            .contains("cannot be reached")
    );
    let (_, answer) = call(
        here.app(),
        "POST",
        "/api/agent/call",
        json!({ "name": "tendly_list", "arguments": { "what": "sessions", "machine": id } }),
    )
    .await;
    assert_eq!(answer["isError"], true);
    assert!(tool_text(&answer).contains("cannot be reached"), "{answer}");
    // Removing it still works here, though it cannot be told.
    let (status, _) = call(
        here.app(),
        "DELETE",
        &format!("/api/peers/{id}"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

/// Where the machine adding is reached, as its page would say.
const HERE: &str = "http://here.tail1.ts.net";

/// A fixture behind its tailnet listener, with `tailscale serve` in front
/// saying the call comes from `caller`'s Tailscale account.
async fn serve_on_tailnet(account: &str, caller: &'static str) -> (Fixture, SocketAddr) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut f = Fixture::new(address, Some("there-token"));
    f.state.tailnet = crate::tailscale::Tailnet::new(&f.path, address.port());
    f.state.tailnet.signed_in_as(account);
    let app = f
        .app()
        .layer(axum::middleware::map_request(
            move |mut request: Request<Body>| async move {
                request
                    .headers_mut()
                    .insert("tailscale-user-login", caller.parse().unwrap());
                request
            },
        ))
        .layer(axum::Extension(security::ViaTailnet));
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap()
    });
    (f, address)
}

#[tokio::test]
async fn a_machine_of_the_same_tailscale_account_pairs_without_an_invite() {
    let (there, address) = serve_on_tailnet("me@example.com", "Me@example.com").await;
    let here = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let (status, peer) = call(
        here.app(),
        "POST",
        "/api/peers",
        json!({ "url": format!("http://{address}/"), "grant": "view", "asked": "spawn", "origin": HERE }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{peer}");
    // Both grants were decided here, in one go.
    assert_eq!(
        (&peer["grant"], &peer["granted"]),
        (&json!("view"), &json!("spawn"))
    );
    let (_, theirs) = call_on(&there, address, "GET", "/api/peers", Value::Null).await;
    let back = &theirs["peers"][0];
    assert_eq!(
        (&back["grant"], &back["granted"]),
        (&json!("spawn"), &json!("view"))
    );

    // Pairing again replaces the pairing there rather than adding a second.
    here.state.peers.forget_all();
    let (status, _) = call(
        here.app(),
        "POST",
        "/api/peers",
        json!({ "url": format!("http://{address}/"), "grant": "control", "origin": HERE }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, theirs) = call_on(&there, address, "GET", "/api/peers", Value::Null).await;
    assert_eq!(theirs["peers"].as_array().unwrap().len(), 1);
    assert_eq!(theirs["peers"][0]["granted"], "control");
    assert_eq!(
        theirs["peers"][0]["grant"], "view",
        "nothing asked is the least"
    );
}

#[tokio::test]
async fn without_an_invite_another_account_or_another_listener_is_refused() {
    let here = Fixture::new("127.0.0.1:8787".parse().unwrap(), None);
    let pair = |address: SocketAddr| {
        call(
            here.app(),
            "POST",
            "/api/peers",
            json!({ "url": format!("http://{address}/"), "grant": "view", "asked": "control", "origin": HERE }),
        )
    };
    // Someone else's machine on a shared tailnet.
    let (there, address) = serve_on_tailnet("me@example.com", "else@example.com").await;
    let (status, refused) = pair(address).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains("same Tailscale account")
    );
    let (_, theirs) = call_on(&there, address, "GET", "/api/peers", Value::Null).await;
    assert_eq!(theirs["peers"], json!([]));

    // The header means nothing off the tailnet listener, where anyone may send it.
    let (there, address) = serve(Some("there-token")).await;
    there.state.tailnet.signed_in_as("me@example.com");
    let request = Request::builder()
        .method("POST")
        .uri(format!("http://{address}/api/peer/pair"))
        .header("host", address.to_string())
        .header("tailscale-user-login", "me@example.com")
        .header("x-tendly-client", "peer")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({ "name": "forger", "url": null, "token": "tdp_".repeat(10), "grant": "view", "asked": "control", "peer_api": 1 })
                .to_string(),
        ))
        .unwrap();
    let response = there.app().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let (_, theirs) = call_on(&there, address, "GET", "/api/peers", Value::Null).await;
    assert_eq!(theirs["peers"], json!([]));

    // An address and an invite at once, or neither, is a mistake.
    let (status, _) = call(here.app(), "POST", "/api/peers", json!({ "grant": "view" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // And a machine the other could not call back is told to fix that first.
    let (_, address) = serve_on_tailnet("me@example.com", "me@example.com").await;
    let (status, refused) = call(
        here.app(),
        "POST",
        "/api/peers",
        json!({ "url": format!("http://{address}/"), "grant": "view" }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(refused["error"].as_str().unwrap().contains("Remote access"));
}
