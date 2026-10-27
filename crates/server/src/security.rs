use crate::AppState;
use axum::{
    Json,
    extract::{Request, State},
    http::{Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use std::{collections::HashSet, env, net::SocketAddr};
use url::Url;

/// The token a client on this machine should present, if the deployment has
/// one: the environment's, else the one kept in the state directory. Never
/// creates one -- that is the server's business at startup.
pub fn existing_token() -> Option<String> {
    if let Some(explicit) = env::var("TENDLY_TOKEN").ok().filter(|v| !v.is_empty()) {
        return Some(explicit);
    }
    let state_dir = crate::installation::state_directory().ok()?;
    std::fs::read_to_string(token_file(&state_dir))
        .ok()
        .map(|token| token.trim().to_owned())
        .filter(|token| !token.is_empty())
}

fn token_file(state_dir: &std::path::Path) -> std::path::PathBuf {
    state_dir.join("token")
}

/// The access token for a binding that reaches other machines.
///
/// `TENDLY_TOKEN` still wins, so an existing deployment and a secret manager
/// keep working. Otherwise one is generated and kept, which is what makes a
/// background gateway usable at all: a token printed once by a detached process
/// is a token nobody can read afterwards, and `status` has to be able to say
/// what it is.
///
/// Keeping it on disk means whoever can read the file can reach the workspace.
/// That is a smaller step than it sounds: the same reader can already open the
/// database and the client configuration directories next to it, which is the
/// account itself. The file is owner-only, and deleting it issues a new token
/// on the next start.
pub fn resolve_token(
    state_dir: &std::path::Path,
    needed: bool,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    if let Some(explicit) = env::var("TENDLY_TOKEN").ok().filter(|v| !v.is_empty()) {
        return Ok(Some(explicit));
    }
    if !needed {
        return Ok(None);
    }
    let path = token_file(state_dir);
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let existing = existing.trim().to_owned();
        if !existing.is_empty() {
            return Ok(Some(existing));
        }
    }
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    write_owner_only(&path, &token)?;
    Ok(Some(token))
}

/// How long a browser stays signed in without entering the token again.
const SESSION_SECONDS: u32 = 30 * 24 * 60 * 60;

/// The secret behind browser sessions, kept in the state directory so a
/// restart -- every upgrade is one -- does not sign every browser out.
pub fn resolve_session_secret(state_dir: &std::path::Path) -> std::io::Result<String> {
    let path = state_dir.join("session-secret");
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let existing = existing.trim().to_owned();
        if existing.len() >= 32 {
            return Ok(existing);
        }
    }
    let secret = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    std::fs::create_dir_all(state_dir)?;
    write_owner_only(&path, &secret)?;
    Ok(secret)
}

/// Create or replace a file only its owner can read.
///
/// Deliberately not `providers::write_private`, which refuses to overwrite: a
/// token file left too short by an earlier attempt has to be replaced, and
/// there the no-op would keep the value that was already rejected.
pub(crate) fn write_owner_only(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(content.as_bytes())?;
    // An existing file keeps its old mode, so state it either way.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// The token a running gateway is using, for reporting it again later. Never
/// generates one: `status` describing a token the server does not have would be
/// worse than saying nothing.
pub fn stored_token(state_dir: &std::path::Path) -> Option<String> {
    env::var("TENDLY_TOKEN")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| {
            std::fs::read_to_string(token_file(state_dir))
                .ok()
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        })
}

/// Marks a request that arrived on the tailnet listener (see `tailscale.rs`).
///
/// Tailscale forwards from 127.0.0.1, so neither the peer address nor a header
/// a client could write tells a phone from this machine; which listener took
/// the connection does, and that is what this records.
#[derive(Clone, Copy)]
pub struct ViaTailnet;

#[derive(Clone)]
pub struct Security {
    /// What this binding itself demands; `None` on loopback.
    token: Option<String>,
    /// What a request from the tailnet must present. The same as `token`
    /// whenever this binding needs one; on loopback it is the one kept in the
    /// state directory, so the tailnet can be switched on without a restart.
    access_token: Option<String>,
    /// A configured origin is HTTPS, so cookies have to be `Secure` there.
    secure_cookies: bool,
    cookie: String,
    origins: HashSet<String>,
    hosts: HashSet<String>,
    /// The port that was bound, so a request naming this machine by its own
    /// address can be recognised without listing the interfaces it has.
    port: u16,
}
impl Security {
    /// Sessions that outlive this process: the cookie is derived from a stored
    /// secret and the token, so it survives a restart, and changing the token
    /// still signs every browser out.
    pub fn with_session_secret(mut self, secret: &str) -> Self {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(secret.as_bytes());
        hash.update([0]);
        hash.update(self.access_token.as_deref().unwrap_or("").as_bytes());
        self.cookie = hash
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        self
    }
    /// The token the tailnet asks for, where this binding asks for none. Set
    /// before `with_session_secret`, which signs sessions with it.
    pub fn with_access_token(mut self, token: Option<String>) -> Self {
        if self.token.is_none() {
            self.access_token = token.filter(|v| !v.is_empty());
        }
        self
    }
    /// The token that signs a device in from anywhere this server is reached.
    pub fn access_token(&self) -> Option<&str> {
        self.access_token.as_deref()
    }
    /// Whether a request arriving this way has to be signed in.
    pub fn required(&self, via_tailnet: bool) -> bool {
        via_tailnet || self.token.is_some()
    }
    /// This server as a process on the same machine reaches it.
    pub fn local_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
    pub fn new(
        address: SocketAddr,
        token: Option<String>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let token = token.filter(|v| !v.is_empty());
        if !address.ip().is_loopback() {
            // How long the token is, is the deployment's call; having none at
            // all would leave a shell on this machine open to the network.
            if token.is_none() {
                return Err(format!(
                    "Listening on {address} reaches other machines, so it needs an access token. Set TENDLY_TOKEN, or let Tendly generate one by not setting it."
                )
                .into());
            }
        }
        let mut origins: HashSet<String> = ["http://127.0.0.1:5173", "http://localhost:5173"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        origins.insert(format!("http://127.0.0.1:{}", address.port()));
        origins.insert(format!("http://localhost:{}", address.port()));
        for value in env::var("TENDLY_ALLOWED_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .filter(|s| !s.is_empty())
        {
            let parsed = Url::parse(value)?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return Err("Invalid allowed origin".into());
            }
            origins.insert(parsed.origin().ascii_serialization());
        }
        let hosts = origins
            .iter()
            .filter_map(|s| Url::parse(s).ok())
            .map(|u| u[url::Position::BeforeHost..url::Position::AfterPort].to_owned())
            .collect();
        let secure_cookies = origins.iter().any(|v| v.starts_with("https://"));
        Ok(Self {
            access_token: token.clone(),
            token,
            secure_cookies,
            cookie: format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
            origins,
            hosts,
            port: address.port(),
        })
    }

    /// Whether a `Host` header names this machine by an address rather than a
    /// name, on the port actually bound.
    ///
    /// The Host check exists to stop DNS rebinding, and rebinding needs a
    /// *name*: the attacker owns `evil.com` and repoints it at a private
    /// address, so the browser still sends `Host: evil.com`. A literal address
    /// in Host means the browser was pointed straight at an address, which no
    /// name the attacker controls can produce. Cross-site requests remain
    /// blocked by the Origin and `sec-fetch-site` checks, which are separate.
    ///
    /// This is what makes `TENDLY_ALLOWED_ORIGINS` unnecessary for reaching
    /// a LAN address: the server already knows its own port, and requiring the
    /// address to be repeated produced a 403 that named neither.
    fn addressed_by_ip(&self, host: &str) -> bool {
        let (name, port) = match host.rsplit_once(':') {
            // A bracketed IPv6 literal without a port cannot be split this way.
            Some((name, port)) if !name.ends_with('[') => (name, port.parse::<u16>().ok()),
            _ => (host, None),
        };
        if port != Some(self.port) {
            return false;
        }
        let name = name.trim_start_matches('[').trim_end_matches(']');
        name.parse::<std::net::IpAddr>().is_ok()
    }

    /// Whether an `Origin` is this server's own, named by address.
    ///
    /// The same reasoning as the Host check, and needed for the same reason:
    /// only a page this server actually served can send this origin, because a
    /// page on a name the attacker controls sends that name instead.
    ///
    /// It is separate from the Host check because a browser applies them to
    /// different requests. Typing the address sends no `Origin` at all, so the
    /// document loaded while every module script — fetched in CORS mode, which
    /// sends `Origin` even same-origin — was refused, and the workspace came up
    /// as a blank page with two 403s in the console.
    fn own_origin(&self, origin: &str) -> bool {
        let Some(authority) = origin
            .strip_prefix("http://")
            .or_else(|| origin.strip_prefix("https://"))
        else {
            return false;
        };
        // An origin has no path, so anything after the authority disqualifies it.
        !authority.contains('/') && self.addressed_by_ip(authority)
    }
    fn authenticated(&self, headers: &axum::http::HeaderMap, via_tailnet: bool) -> bool {
        let token = if via_tailnet {
            &self.access_token
        } else {
            &self.token
        };
        let Some(token) = token else {
            // No token at all is an open door on loopback and a shut one on
            // the tailnet.
            return !via_tailnet;
        };
        if headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .is_some_and(|v| equal(v, token))
        {
            return true;
        }
        headers
            .get(header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| {
                v.split(';').any(|part| {
                    part.trim()
                        .strip_prefix("tendly_session=")
                        .is_some_and(|s| equal(s, &self.cookie))
                })
            })
    }
}
fn equal(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |n, (x, y)| n | (x ^ y)) == 0
}
fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(serde_json::json!({"error":message}))).into_response()
}

/// The one mutation `tendly resume` makes, which a command line names as
/// `X-Tendly-Client: cli`; it is still authenticated like any other.
fn cli_path(path: &str) -> bool {
    path.strip_prefix("/api/sessions/")
        .and_then(|rest| rest.strip_suffix("/launch-plan"))
        .is_some_and(|id| !id.is_empty() && !id.contains('/'))
}

pub async fn guard(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let via_tailnet = request.extensions().get::<ViaTailnet>().is_some();
    let headers = request.headers();
    if !headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            state.security.hosts.contains(v)
                || state.security.addressed_by_ip(v)
                || via_tailnet && state.tailnet.serves_host(v)
        })
    {
        return error(
            StatusCode::FORBIDDEN,
            "This address is not one Tendly was told to answer for. Add it to TENDLY_ALLOWED_ORIGINS.",
        );
    }
    if let Some(origin) = headers.get(header::ORIGIN)
        && !origin.to_str().ok().is_some_and(|v| {
            state.security.origins.contains(v)
                || state.security.own_origin(v)
                || via_tailnet && state.tailnet.serves_origin(v)
        })
    {
        return error(StatusCode::FORBIDDEN, "Untrusted Origin");
    }
    let path = request.uri().path();
    if headers
        .get("sec-fetch-site")
        .is_some_and(|v| v == "cross-site")
        && !page_opened_from_elsewhere(request.method(), path, headers)
    {
        return error(StatusCode::FORBIDDEN, "Cross-site requests are blocked");
    }
    if path.starts_with("/api/") {
        // Pairing is authenticated by the invite code it carries (peers.rs).
        let public = path == "/api/health" || path == "/api/auth" || path == "/api/peer/pair";
        // A session's agent token opens the agent tools and nothing else; the
        // rest of the API still takes the deployment's own credentials.
        let agent_path = path.starts_with("/api/agent/");
        let agent = agent_path && crate::agent::is_agent_request(&state, headers);
        // Likewise a paired machine's key opens /api/peer/ and nothing else.
        let peer_path = path.starts_with("/api/peer/");
        let peer = peer_path && state.peers.is_peer_request(headers);
        if !public && !agent && !peer && !state.security.authenticated(headers, via_tailnet) {
            return error(StatusCode::UNAUTHORIZED, "Authentication required");
        }
        if !matches!(
            *request.method(),
            Method::GET | Method::HEAD | Method::OPTIONS
        ) && headers.get("x-tendly-client").is_none_or(|v| {
            v != "web"
                && !(agent_path && v == "agent")
                && !(peer_path && v == "peer")
                && !(v == "cli" && cli_path(path))
        }) {
            return error(
                StatusCode::FORBIDDEN,
                "X-Tendly-Client: web is required for mutations",
            );
        }
        // Browser WebSocket requests always carry Origin; command-line clients may use bearer.
        if headers
            .get(header::UPGRADE)
            .is_some_and(|v| v == "websocket")
            && headers.get(header::ORIGIN).is_none()
            && state.security.required(via_tailnet)
            && headers.get(header::AUTHORIZATION).is_none()
        {
            return error(
                StatusCode::FORBIDDEN,
                "WebSocket Origin or bearer authentication required",
            );
        }
    }
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert("x-content-type-options", "nosniff".parse().unwrap());
    response
        .headers_mut()
        .insert("referrer-policy", "no-referrer".parse().unwrap());
    response.headers_mut().insert(
        "cross-origin-resource-policy",
        "same-origin".parse().unwrap(),
    );
    response
}

/// A page of this app opened from a link on another site: the address of
/// another paired machine's window, say. Only the document itself, fetched as
/// a top-level navigation; it carries no cookie (they are `SameSite=Strict`)
/// and changes nothing, and everything it then asks for is same-site.
fn page_opened_from_elsewhere(
    method: &Method,
    path: &str,
    headers: &axum::http::HeaderMap,
) -> bool {
    let is = |name: &str, value: &str| headers.get(name).is_some_and(|v| v == value);
    *method == Method::GET
        && !path.starts_with("/api/")
        && is("sec-fetch-mode", "navigate")
        && is("sec-fetch-dest", "document")
}

pub async fn status(
    State(state): State<AppState>,
    via: Option<axum::Extension<ViaTailnet>>,
    headers: axum::http::HeaderMap,
) -> Json<serde_json::Value> {
    let via_tailnet = via.is_some();
    Json(serde_json::json!({
        "required": state.security.required(via_tailnet),
        "authenticated": state.security.authenticated(&headers, via_tailnet),
        "via_tailnet": via_tailnet,
    }))
}
#[derive(Deserialize)]
pub struct Login {
    #[serde(default)]
    token: String,
    /// A one-time ticket a paired machine was given (peers.rs), instead of the token.
    #[serde(default)]
    ticket: Option<String>,
}
pub async fn login(
    State(state): State<AppState>,
    via: Option<axum::Extension<ViaTailnet>>,
    Json(input): Json<Login>,
) -> Response {
    let via_tailnet = via.is_some();
    let expected = if via_tailnet {
        &state.security.access_token
    } else {
        &state.security.token
    };
    let ticket = input
        .ticket
        .as_deref()
        .is_some_and(|ticket| state.peers.redeem_ticket(ticket));
    if input.ticket.is_some() && !ticket {
        return error(
            StatusCode::UNAUTHORIZED,
            "This sign-in link has expired or was used already. Open it again from the other machine.",
        );
    }
    if !ticket
        && (via_tailnet && expected.is_none()
            || expected.as_ref().is_some_and(|t| !equal(t, &input.token)))
    {
        return error(StatusCode::UNAUTHORIZED, "Invalid access token");
    }
    // Decided per request: the tailnet is HTTPS while a LAN address beside it
    // is not, and a Secure cookie on plain HTTP is one the browser drops.
    let secure = if via_tailnet || state.security.secure_cookies {
        "; Secure"
    } else {
        ""
    };
    let cookie = format!(
        "tendly_session={}; HttpOnly; SameSite=Strict; Path=/; Max-Age={SESSION_SECONDS}{secure}",
        state.security.cookie
    );
    (
        [(header::SET_COOKIE, cookie)],
        Json(serde_json::json!({"authenticated":true})),
    )
        .into_response()
}

#[cfg(test)]
impl Security {
    pub fn for_test(address: SocketAddr, token: Option<&str>) -> Self {
        Self {
            token: token.map(str::to_owned),
            access_token: token.map(str::to_owned),
            secure_cookies: false,
            cookie: "test-cookie".into(),
            origins: [format!("http://{address}"), "http://127.0.0.1:5173".into()].into(),
            hosts: [address.to_string(), "127.0.0.1:5173".into()].into(),
            port: address.port(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bound(port: u16) -> Security {
        Security::for_test(format!("127.0.0.1:{port}").parse().unwrap(), None)
    }

    /// Reaching the workspace at the machine's own address must not require
    /// that address to have been configured in advance. The 403 that produced
    /// named neither the address it wanted nor the variable that supplies it.
    #[test]
    fn an_address_on_the_bound_port_is_answered_for_without_being_configured() {
        let security = bound(28789);
        for host in [
            "192.168.0.252:28789",
            "10.0.0.5:28789",
            "127.0.0.1:28789",
            "[::1]:28789",
            "[fe80::1]:28789",
        ] {
            assert!(security.addressed_by_ip(host), "{host}");
        }
    }

    /// A module script is fetched in CORS mode and sends `Origin` even when it
    /// is same-origin, so relaxing only the Host check left every asset
    /// refused: the document loaded, nothing else did, and the workspace was a
    /// blank page. curl does not send that header, which is why this needed a
    /// browser to find.
    #[test]
    fn this_servers_own_address_is_accepted_as_an_origin_not_only_as_a_host() {
        let security = bound(28789);
        for origin in [
            "http://192.168.0.252:28789",
            "http://10.100.5.41:28789",
            "https://192.168.0.252:28789",
            "http://[fe80::1]:28789",
        ] {
            assert!(security.own_origin(origin), "{origin}");
        }
    }

    #[test]
    fn an_origin_that_is_not_this_server_is_still_refused() {
        let security = bound(28789);
        for origin in [
            // A name, which is what an attacker's page actually sends.
            "http://evil.com:28789",
            // The right address on somebody else's port.
            "http://192.168.0.252:28790",
            // Not an origin at all: an origin carries no path.
            "http://192.168.0.252:28789/evil",
            "http://192.168.0.252:28789@evil.com",
            // Schemes a page cannot be served from here under.
            "ftp://192.168.0.252:28789",
            "null",
            "",
        ] {
            assert!(!security.own_origin(origin), "{origin}");
        }
    }

    /// A name is exactly what DNS rebinding needs, so a name is what the
    /// configured list stays responsible for.
    #[test]
    fn a_hostname_is_never_answered_for_merely_because_the_port_matches() {
        let security = bound(28789);
        for host in [
            "evil.com:28789",
            "tendly.local:28789",
            "192.168.0.252.evil.com:28789",
            "localhost:28789",
        ] {
            assert!(!security.addressed_by_ip(host), "{host}");
        }
    }

    /// Another service on the same machine is a different server. Answering for
    /// its port would let a page served there drive this one.
    #[test]
    fn an_address_on_a_different_port_is_not_this_server() {
        let security = bound(28789);
        for host in [
            "192.168.0.252:28790",
            "192.168.0.252",
            "192.168.0.252:",
            "192.168.0.252:notaport",
        ] {
            assert!(!security.addressed_by_ip(host), "{host}");
        }
    }

    /// Requiring a token is the whole protection for a network binding, so it
    /// is stated here rather than left to whoever reads the constructor.
    /// `Security` holds the token, so it deliberately has no `Debug`; reading
    /// the rejection means matching rather than `unwrap_err`.
    fn rejection(address: SocketAddr, token: Option<String>) -> String {
        match Security::new(address, token) {
            Ok(_) => String::new(),
            Err(error) => error.to_string(),
        }
    }

    /// Another machine's window links here; only the page itself may arrive
    /// that way, never the API and never a frame.
    #[test]
    fn only_a_top_level_page_may_be_opened_from_another_site() {
        let headers = |mode: &str, dest: &str| {
            let mut headers = axum::http::HeaderMap::new();
            headers.insert("sec-fetch-mode", mode.parse().unwrap());
            headers.insert("sec-fetch-dest", dest.parse().unwrap());
            headers
        };
        let page = headers("navigate", "document");
        assert!(page_opened_from_elsewhere(
            &Method::GET,
            "/session/x",
            &page
        ));
        assert!(!page_opened_from_elsewhere(
            &Method::POST,
            "/session/x",
            &page
        ));
        assert!(!page_opened_from_elsewhere(
            &Method::GET,
            "/api/sessions",
            &page
        ));
        assert!(!page_opened_from_elsewhere(
            &Method::GET,
            "/",
            &headers("navigate", "iframe")
        ));
        assert!(!page_opened_from_elsewhere(
            &Method::GET,
            "/",
            &headers("cors", "empty")
        ));
    }

    #[test]
    fn a_browser_session_survives_a_restart_but_not_a_new_token() {
        let address: SocketAddr = "0.0.0.0:28789".parse().unwrap();
        let dir = std::env::temp_dir().join(format!("tendly-session-{}", uuid::Uuid::new_v4()));
        let secret = resolve_session_secret(&dir).unwrap();
        assert_eq!(
            resolve_session_secret(&dir).unwrap(),
            secret,
            "kept across restarts"
        );
        let session = |token: &str| {
            Security::new(address, Some(token.into()))
                .ok()
                .unwrap()
                .with_session_secret(&secret)
                .cookie
        };
        assert_eq!(session("qwe41235"), session("qwe41235"));
        assert_ne!(session("qwe41235"), session("another"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_binding_that_reaches_other_machines_needs_a_token_of_any_length() {
        let network: SocketAddr = "0.0.0.0:28789".parse().unwrap();
        let absent = rejection(network, None);
        assert!(absent.contains("needs an access token"), "{absent}");
        // An empty one is the same as none.
        assert!(!rejection(network, Some(String::new())).is_empty());
        assert!(rejection(network, Some("qwe41235".into())).is_empty());
        assert!(rejection(network, Some("x".into())).is_empty());
    }

    /// Loopback keeps working with no token at all: the single-user host case
    /// is the common one and must not acquire a password.
    #[test]
    fn a_loopback_binding_still_needs_nothing() {
        assert!(rejection("127.0.0.1:28789".parse().unwrap(), None).is_empty());
    }

    fn bearer(token: &str) -> axum::http::HeaderMap {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        headers
    }

    /// The tailnet listener is reached from 127.0.0.1 like everything else on
    /// this machine, so a loopback binding's open door must not extend to it.
    #[test]
    fn the_tailnet_always_needs_the_token_while_this_machine_still_does_not() {
        let loopback: SocketAddr = "127.0.0.1:28789".parse().unwrap();
        let security = Security::new(loopback, None)
            .ok()
            .unwrap()
            .with_access_token(Some("qwe41235".into()))
            .with_session_secret(&"s".repeat(32));
        let none = axum::http::HeaderMap::new();
        assert!(security.authenticated(&none, false));
        assert!(!security.authenticated(&none, true));
        assert!(security.authenticated(&bearer("qwe41235"), true));
        assert!(!security.authenticated(&bearer("guess"), true));
        assert!((security.required(true), security.required(false)) == (true, false));
        // With no token at all, the tailnet stays shut rather than open.
        let tokenless = Security::new(loopback, None).ok().unwrap();
        assert!(!tokenless.authenticated(&bearer(""), true));
    }

    /// Browsers signed in to a network binding stay signed in: the access token
    /// is that binding's own token, and the session is derived from it as before.
    #[test]
    fn a_network_bindings_sessions_are_unchanged_by_the_access_token() {
        let network: SocketAddr = "0.0.0.0:28789".parse().unwrap();
        let secret = "s".repeat(32);
        let session = |access: Option<&str>| {
            Security::new(network, Some("qwe41235".into()))
                .ok()
                .unwrap()
                .with_access_token(access.map(str::to_owned))
                .with_session_secret(&secret)
                .cookie
        };
        assert_eq!(session(None), session(Some("another")));
        assert!(
            Security::new(network, Some("qwe41235".into()))
                .ok()
                .unwrap()
                .authenticated(&bearer("qwe41235"), true)
        );
    }
}
