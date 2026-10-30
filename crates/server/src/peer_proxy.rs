//! Another machine's session used from this machine's own window.
//!
//! A pane here asks `/api/machines/{id}/...`; this machine passes the call on
//! to `/api/peer/proxy/...` there with the key that machine issued it, so the
//! browser never holds a key or signs in anywhere else. Nothing is fetched
//! until a pane asks: the sidebar's list stays `/api/peers/sessions`.
//!
//! The machine called decides: a call needs the `control` grant, and reaches
//! only what a session's pane uses. What it reaches runs as the app's own
//! route, marked [`Proxied`], which only [`serve`] can set: a request cannot
//! carry it. `/api/peer/` and `/api/machines/` are not on that list, so a
//! forwarded call cannot be forwarded again.
use crate::{ApiError, AppState, Result, peers};
use axum::{
    Router,
    body::Body,
    extract::{
        ConnectInfo, Path, Request, State,
        ws::{Message, WebSocket, WebSocketUpgrade, rejection::WebSocketUpgradeRejection},
    },
    http::{Extensions, HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header},
    response::Response,
    routing::any,
};
use futures_util::{SinkExt, StreamExt};
use std::{net::SocketAddr, sync::LazyLock, time::Duration};
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest};
use tower::ServiceExt;
use uuid::Uuid;

/// A call [`serve`] let through; `security::guard` takes it as signed in.
#[derive(Clone, Copy)]
pub struct Proxied;

/// What is asked of the other machine, passed on as asked.
const ASKED: [HeaderName; 5] = [
    header::ACCEPT,
    header::CONTENT_TYPE,
    header::IF_NONE_MATCH,
    header::IF_MODIFIED_SINCE,
    header::RANGE,
];
/// Between the two machines only, or this machine's to set.
const NOT_ANSWERED: [HeaderName; 5] = [
    header::CONNECTION,
    header::TRANSFER_ENCODING,
    header::SET_COOKIE,
    header::UPGRADE,
    header::CONTENT_ENCODING,
];

static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    // One client, so a pane's calls share a connection. No overall limit: a
    // download takes as long as it takes.
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_default()
});

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/machines/{id}/{*path}", any(forward))
        .route("/api/peer/proxy/{*path}", any(serve))
}

/// Whether a session's pane uses this path. `rest` is as sent, still encoded,
/// so an encoded `/` or `.` is refused rather than read as anything.
fn permitted(method: &Method, rest: &str) -> bool {
    let parts: Vec<&str> = rest.split('/').collect();
    if parts.iter().any(|part| {
        part.is_empty()
            || *part == "."
            || *part == ".."
            || part.to_ascii_lowercase().contains("%2")
            || part.contains('\\')
    }) {
        return false;
    }
    let reading = *method == Method::GET || *method == Method::HEAD;
    match parts.as_slice() {
        ["sessions", _, ..] => true,
        ["workspaces", _, "attachments" | "asset" | "sessions"] => true,
        ["workspaces", _] | ["workspaces", _, "files", "search"] => reading,
        ["workspaces", _, "git", "branches" | "status"] => reading,
        ["host", "raw"] | ["models", "context"] => reading,
        ["endpoint-profiles"] | ["endpoint-profiles", _, "models"] => reading,
        _ => false,
    }
}

/// `/api/machines/{id}/...`, as the page sent it: the machine and the rest.
fn split(path: &str) -> Option<&str> {
    path.strip_prefix("/api/machines/")?
        .split_once('/')
        .map(|(_, rest)| rest)
}

async fn forward(
    State(state): State<AppState>,
    Path((id, _)): Path<(Uuid, String)>,
    socket: std::result::Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
    request: Request,
) -> Result<Response> {
    let peer = state
        .peers
        .get(id)
        .ok_or_else(|| ApiError::missing("Machine"))?;
    let (Some(url), Some(token)) = (peer.url.as_deref(), peer.token.as_deref()) else {
        return Err(ApiError::conflict(format!(
            "{} has no address this machine can reach.",
            peer.name
        )));
    };
    let rest = split(request.uri().path()).ok_or_else(|| ApiError::missing("API route"))?;
    let query = request
        .uri()
        .query()
        .map(|query| format!("?{query}"))
        .unwrap_or_default();
    let target = format!("{}/api/peer/proxy/{rest}{query}", url.trim_end_matches('/'));
    if let Ok(socket) = socket {
        return tunnel(socket, &target, token, &peer.name).await;
    }
    let mut outgoing = CLIENT
        .request(request.method().clone(), target)
        .bearer_auth(token)
        .header("X-Tendly-Client", "peer");
    for name in ASKED {
        if let Some(value) = request.headers().get(&name) {
            outgoing = outgoing.header(name, value);
        }
    }
    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        outgoing = outgoing.body(reqwest::Body::wrap_stream(
            request.into_body().into_data_stream(),
        ));
    }
    let answer = outgoing.send().await.map_err(|_| ApiError {
        status: StatusCode::BAD_GATEWAY,
        message: format!("{} cannot be reached", peer.name),
    })?;
    // Its 401 is about this machine's key; here it would read as this page
    // having to sign in.
    if answer.status() == StatusCode::UNAUTHORIZED {
        return Err(ApiError::conflict(format!(
            "{} no longer recognises this machine. Remove it here and pair again.",
            peer.name
        )));
    }
    let mut response = Response::builder().status(answer.status());
    if let Some(headers) = response.headers_mut() {
        for (name, value) in answer.headers() {
            if !NOT_ANSWERED.contains(name) {
                headers.insert(name, value.clone());
            }
        }
    }
    response
        .body(Body::from_stream(answer.bytes_stream()))
        .map_err(ApiError::internal)
}

/// The two ends of a pane's socket. It is answered only once the other machine
/// has: a refusal reaches the pane as a socket that failed to open.
async fn tunnel(
    socket: WebSocketUpgrade,
    target: &str,
    token: &str,
    name: &str,
) -> Result<Response> {
    let refused = || ApiError {
        status: StatusCode::BAD_GATEWAY,
        message: format!("{name} did not take the connection"),
    };
    let address = target
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1);
    let mut outgoing = address.into_client_request().map_err(|_| refused())?;
    let bearer = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| refused())?;
    outgoing.headers_mut().insert(header::AUTHORIZATION, bearer);
    outgoing
        .headers_mut()
        .insert("x-tendly-client", HeaderValue::from_static("peer"));
    let (there, _) = tokio_tungstenite::connect_async(outgoing)
        .await
        .map_err(|_| refused())?;
    Ok(socket.on_upgrade(move |here| relay(here, there)))
}

fn outbound(message: Message) -> tungstenite::Message {
    match message {
        Message::Text(text) => tungstenite::Message::text(text.as_str()),
        Message::Binary(bytes) => tungstenite::Message::Binary(bytes),
        Message::Ping(bytes) => tungstenite::Message::Ping(bytes),
        Message::Pong(bytes) => tungstenite::Message::Pong(bytes),
        Message::Close(_) => tungstenite::Message::Close(None),
    }
}

fn inbound(message: tungstenite::Message) -> Option<Message> {
    Some(match message {
        tungstenite::Message::Text(text) => Message::text(text.as_str()),
        tungstenite::Message::Binary(bytes) => Message::Binary(bytes),
        tungstenite::Message::Ping(bytes) => Message::Ping(bytes),
        tungstenite::Message::Pong(bytes) => Message::Pong(bytes),
        tungstenite::Message::Close(_) => Message::Close(None),
        tungstenite::Message::Frame(_) => return None,
    })
}

async fn relay<S>(here: WebSocket, there: tokio_tungstenite::WebSocketStream<S>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut to_page, mut from_page) = here.split();
    let (mut to_machine, mut from_machine) = there.split();
    // Either side ending ends both: the pane reconnects as it does anyway.
    let up = async {
        while let Some(Ok(message)) = from_page.next().await {
            if to_machine.send(outbound(message)).await.is_err() {
                break;
            }
        }
    };
    let down = async {
        while let Some(Ok(message)) = from_machine.next().await {
            let Some(message) = inbound(message) else {
                continue;
            };
            if to_page.send(message).await.is_err() {
                break;
            }
        }
    };
    tokio::select! { _ = up => {}, _ = down => {} }
}

async fn serve(State(state): State<AppState>, mut request: Request) -> Result<Response> {
    let peer = peers::calling(&state, request.headers())?;
    peers::allowed(&peer, peers::Grant::Control)?;
    let path = request.uri().path();
    let rest = path.strip_prefix("/api/peer/proxy/").unwrap_or_default();
    if !permitted(request.method(), rest) {
        return Err(ApiError {
            status: StatusCode::FORBIDDEN,
            message: "That is not part of a session".into(),
        });
    }
    let query = request
        .uri()
        .query()
        .map(|query| format!("?{query}"))
        .unwrap_or_default();
    *request.uri_mut() = format!("/api/{rest}{query}")
        .parse()
        .map_err(|_| ApiError::bad("That address is not readable"))?;
    let headers: &mut HeaderMap = request.headers_mut();
    headers.remove(header::AUTHORIZATION);
    headers.remove(header::COOKIE);
    headers.insert("x-tendly-client", HeaderValue::from_static("web"));
    // A fresh set: this route's own path parameters would be read as the
    // next route's. Only what says how the request arrived goes along.
    let (mut parts, body) = request.into_parts();
    let mut extensions = Extensions::new();
    extensions.insert(Proxied);
    if let Some(upgrade) = parts.extensions.remove::<hyper::upgrade::OnUpgrade>() {
        extensions.insert(upgrade);
    }
    if let Some(via) = parts.extensions.remove::<crate::security::ViaTailnet>() {
        extensions.insert(via);
    }
    if let Some(from) = parts.extensions.remove::<ConnectInfo<SocketAddr>>() {
        extensions.insert(from);
    }
    parts.extensions = extensions;
    state
        .peers
        .app(&state)
        .oneshot(Request::from_parts(parts, body))
        .await
        .map_err(|never| match never {})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_a_sessions_pane_uses_is_within_reach() {
        for rest in [
            "sessions/abc/conversation",
            "sessions/abc/chat/ws",
            "sessions/abc/pty/ws",
            "workspaces/w/attachments",
        ] {
            assert!(permitted(&Method::POST, rest), "{rest}");
        }
        assert!(permitted(&Method::GET, "endpoint-profiles"));
        assert!(permitted(&Method::GET, "host/raw"));
        for (method, rest) in [
            (Method::POST, "endpoint-profiles"),
            (Method::PUT, "host/raw"),
            (Method::GET, "peers"),
            (Method::POST, "peer/ticket"),
            (Method::GET, "peer/proxy/sessions/abc"),
            (Method::GET, "machines/m/sessions/abc"),
            (Method::GET, "auth"),
            (Method::GET, "preferences"),
            (Method::GET, "secrets"),
            (Method::POST, "update/apply"),
            (Method::GET, "workspaces/w/file"),
            (Method::GET, "sessions/../peers"),
            (Method::GET, "sessions/a/%2e%2e/%2e%2e/peers"),
            (Method::GET, "sessions/a%2F..%2Fpeers"),
            (Method::GET, "sessions//"),
        ] {
            assert!(!permitted(&method, rest), "{method} {rest}");
        }
    }

    #[test]
    fn the_machine_is_read_from_the_path_and_the_rest_stays_encoded() {
        assert_eq!(
            split("/api/machines/m/sessions/a%20b/x"),
            Some("sessions/a%20b/x")
        );
        assert_eq!(split("/api/peers"), None);
    }
}
