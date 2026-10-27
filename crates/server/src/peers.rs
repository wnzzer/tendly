//! Other machines running Tendly, paired with this one.
//!
//! There is no hub. Every machine is the only authority over its own sessions,
//! and a pairing is a pair of keys: each side issues the other a token, kept
//! by the side that presents it, and decides for itself what the other may do
//! there. Either side can end it alone. A machine that is off is simply shown
//! as unreachable; nothing anywhere waits for it.
//!
//! Pairing starts with an invite made on the machine to be reached: a link to
//! its own address with a one-time code in the fragment, good for minutes. The
//! other machine redeems it by calling that address, and sends back its own
//! key and, when it has one, the address it can be reached at.
//!
//! Two machines signed in to the same Tailscale account need no invite: the
//! one adding calls the other's tailnet address, and `tailscale serve` there
//! says which account the call comes from. Being that account is proof
//! enough, so both grants are decided in one place, on the machine adding.
//!
//! What a machine may do here, from least to most:
//!
//! - `view`: see the workspaces and sessions here (titles and status only);
//! - `spawn`: also start sessions here and follow the ones it started, which is
//!   what an agent on that machine reaches through `tendly_spawn` with
//!   `machine`;
//! - `control`: also open this machine's own window, signed in, from there.
//!
//! The peer API lives under `/api/peer/` and takes a peer's key and nothing
//! else: a key opens that prefix only, as a session's agent token opens
//! `/api/agent/` only. Calls that arrive from another machine never reach a
//! third: a forwarded call cannot be forwarded again, and a session another
//! machine started cannot start sessions anywhere.
use crate::{
    ApiError, AppState, Result,
    agent::{Caller, text},
    agent_sessions, db,
};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    routing::{get, post, put},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tendly_domain::{Session, SessionId};
use uuid::Uuid;

/// The version of the conversation between two machines. A machine refuses a
/// peer speaking another, rather than half-working with it.
const PEER_API: u64 = 1;
const STATE_FILE: &str = "peers.json";
const INVITE_LIFETIME: Duration = Duration::from_secs(10 * 60);
const TICKET_LIFETIME: Duration = Duration::from_secs(60);
/// Wrong invite codes tolerated before every open invite is withdrawn.
const INVITE_FAILURES: u32 = 10;
/// How long a look at another machine may take before it counts as offline.
const GLANCE: Duration = Duration::from_secs(4);
/// The longest a forwarded call may take: `tendly_session` wait is 120 s.
const LONGEST_CALL: Duration = Duration::from_secs(150);
/// The header `tailscale serve` puts the caller's Tailscale account in. It
/// replaces any the caller sent, and only `serve` reaches the tailnet listener.
const TAILSCALE_LOGIN: &str = "Tailscale-User-Login";
/// The fragment an invite link carries its code in.
pub const INVITE_FRAGMENT: &str = "tendly-pair";

/// What a paired machine may do on this one. Ordered: each includes the last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Grant {
    View,
    Spawn,
    Control,
}

/// A machine this one is paired with.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Peer {
    id: Uuid,
    name: String,
    /// Where it is reached; `None` when it has no address this machine can
    /// reach, and then only it can call here.
    url: Option<String>,
    /// The key this machine presents there.
    token: Option<String>,
    /// The SHA-256 of the key it presents here.
    inbound: String,
    /// What it may do here.
    grant: Grant,
    /// What it lets this machine do there, as it last said.
    granted: Option<Grant>,
    paired_at: String,
}

struct Invite {
    code: String,
    grant: Grant,
    until: Instant,
}

struct Ticket {
    code: String,
    until: Instant,
}

#[derive(Default)]
struct Inner {
    path: PathBuf,
    peers: Mutex<Vec<Peer>>,
    invites: Mutex<Vec<Invite>>,
    failures: Mutex<u32>,
    tickets: Mutex<Vec<Ticket>>,
    /// Sessions here that a paired machine started, until restart.
    children: Mutex<HashMap<SessionId, Uuid>>,
    /// Sessions on other machines that a session here started.
    remote_children: Mutex<HashSet<(SessionId, Uuid, String)>>,
    /// Sessions here the person let start sessions on other machines.
    remote_spawners: Mutex<HashSet<SessionId>>,
}

#[derive(Clone, Default)]
pub struct Peers(Arc<Inner>);

fn secret(prefix: &str) -> String {
    format!(
        "{prefix}{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    )
}

fn digest(token: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn equal(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |n, (x, y)| n | (x ^ y)) == 0
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
}

impl Peers {
    pub fn new(state_dir: &std::path::Path) -> Self {
        let path = state_dir.join(STATE_FILE);
        let peers = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self(Arc::new(Inner {
            path,
            peers: Mutex::new(peers),
            ..Inner::default()
        }))
    }

    fn list(&self) -> Vec<Peer> {
        self.0.peers.lock().expect("peers").clone()
    }

    fn get(&self, id: Uuid) -> Option<Peer> {
        self.list().into_iter().find(|peer| peer.id == id)
    }

    /// Change the paired machines and keep the result, owner-only: the file
    /// holds the keys this machine presents elsewhere.
    fn change<T>(&self, change: impl FnOnce(&mut Vec<Peer>) -> T) -> std::io::Result<T> {
        let mut peers = self.0.peers.lock().expect("peers");
        let outcome = change(&mut peers);
        if let Some(parent) = self.0.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(&*peers).map_err(std::io::Error::other)?;
        crate::security::write_owner_only(&self.0.path, &text)?;
        Ok(outcome)
    }

    /// As if every pairing had been removed here alone.
    #[cfg(test)]
    pub fn forget_all(&self) {
        self.change(Vec::clear).unwrap();
    }

    /// The machine presenting this request's key, if any does.
    fn presenting(&self, headers: &HeaderMap) -> Option<Peer> {
        let hash = digest(bearer(headers)?);
        self.list()
            .into_iter()
            .find(|peer| equal(&peer.inbound, &hash))
    }

    /// Whether a request carries a paired machine's key. The guard lets these
    /// through to `/api/peer/` only.
    pub fn is_peer_request(&self, headers: &HeaderMap) -> bool {
        self.presenting(headers).is_some()
    }

    pub(crate) fn invite(&self, grant: Grant) -> String {
        // Twelve characters a person could read aloud, from 60 random bits.
        let code: String = Uuid::new_v4().simple().to_string()[..12].to_ascii_uppercase();
        let mut invites = self.0.invites.lock().expect("invites");
        let now = Instant::now();
        invites.retain(|invite| invite.until > now);
        // A few at a time is plenty; the oldest goes first.
        while invites.len() >= 3 {
            invites.remove(0);
        }
        invites.push(Invite {
            code: code.clone(),
            grant,
            until: now + INVITE_LIFETIME,
        });
        code
    }

    /// Spend an invite: its grant once, then never again. Too many wrong codes
    /// withdraw every open invite, so guessing has a fixed, small budget.
    fn redeem_invite(&self, code: &str) -> Option<Grant> {
        let mut invites = self.0.invites.lock().expect("invites");
        let now = Instant::now();
        invites.retain(|invite| invite.until > now);
        let code = code.trim().to_ascii_uppercase();
        if let Some(index) = invites.iter().position(|invite| equal(&invite.code, &code)) {
            return Some(invites.remove(index).grant);
        }
        let mut failures = self.0.failures.lock().expect("invite failures");
        *failures += 1;
        if *failures >= INVITE_FAILURES {
            invites.clear();
            *failures = 0;
        }
        None
    }

    fn ticket(&self) -> String {
        let code = secret("tdt_");
        let mut tickets = self.0.tickets.lock().expect("tickets");
        let now = Instant::now();
        tickets.retain(|ticket| ticket.until > now);
        tickets.push(Ticket {
            code: code.clone(),
            until: now + TICKET_LIFETIME,
        });
        code
    }

    /// Spend a sign-in ticket a paired machine was given: once, within a minute.
    pub fn redeem_ticket(&self, code: &str) -> bool {
        let mut tickets = self.0.tickets.lock().expect("tickets");
        let now = Instant::now();
        tickets.retain(|ticket| ticket.until > now);
        match tickets.iter().position(|ticket| equal(&ticket.code, code)) {
            Some(index) => {
                tickets.remove(index);
                true
            }
            None => false,
        }
    }

    /// Whether another machine started this session.
    pub fn started_by_peer(&self, session: SessionId) -> bool {
        self.0
            .children
            .lock()
            .expect("peer children")
            .contains_key(&session)
    }

    fn adopt(&self, session: SessionId, peer: Uuid) {
        self.0
            .children
            .lock()
            .expect("peer children")
            .insert(session, peer);
    }

    fn children_of(&self, peer: Uuid) -> Vec<SessionId> {
        self.0
            .children
            .lock()
            .expect("peer children")
            .iter()
            .filter(|(_, owner)| **owner == peer)
            .map(|(id, _)| *id)
            .collect()
    }

    fn name_of(&self, id: Uuid) -> Option<String> {
        self.get(id).map(|peer| peer.name)
    }

    /// Who a call from a paired machine is, for the activity a person sees.
    pub fn who(&self, id: Uuid) -> Value {
        json!({ "kind": "machine", "peer_id": id, "title": self.name_of(id) })
    }
}

/// This machine's name, as the others show it.
fn own_name(state: &AppState) -> String {
    std::env::var("TENDLY_INSTANCE_LABEL")
        .ok()
        .filter(|label| !label.trim().is_empty())
        .or_else(|| {
            state
                .tailnet
                .name()
                .and_then(|name| name.split('.').next().map(str::to_owned))
        })
        .or_else(sysinfo::System::host_name)
        .unwrap_or_else(|| "Tendly".into())
}

/// Whether an address could be reached from another machine at all.
fn reachable_elsewhere(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(name)) => !name.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => !ip.is_loopback() && !ip.is_unspecified(),
        Some(url::Host::Ipv6(ip)) => !ip.is_loopback() && !ip.is_unspecified(),
        None => false,
    }
}

/// The address other machines reach this one at.
fn own_url(state: &AppState, page: Option<&str>) -> Option<String> {
    let configured = std::env::var("TENDLY_PUBLIC_URL").ok();
    choose_url(configured.as_deref(), state.tailnet.published_url(), page)
}

/// `TENDLY_PUBLIC_URL` when it is set, as it is behind a reverse proxy or a
/// LAN name: the person said so, and nothing here can check it. Else the
/// tailnet's, when it is published; else the page's own, when that is not this
/// machine's loopback.
fn choose_url(
    configured: Option<&str>,
    tailnet: Option<String>,
    page: Option<&str>,
) -> Option<String> {
    if let Some(url) = configured
        .filter(|url| !url.trim().is_empty())
        .and_then(|url| base_url(url).ok())
    {
        return Some(url.to_string());
    }
    if let Some(url) = tailnet {
        return Some(url);
    }
    let page = url::Url::parse(page?).ok()?;
    (matches!(page.scheme(), "http" | "https") && reachable_elsewhere(&page))
        .then(|| format!("{}/", page.origin().ascii_serialization()))
}

/// An address as a base URL ending in `/`, or an error saying what is wrong.
fn base_url(raw: &str) -> Result<url::Url> {
    let mut url =
        url::Url::parse(raw.trim()).map_err(|_| ApiError::bad("That is not a link to Tendly"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return Err(ApiError::bad("That is not a link to Tendly"));
    }
    url.set_fragment(None);
    url.set_query(None);
    url.set_path("/");
    Ok(url)
}

/// An invite link split into the address to call and the code to give it.
fn parse_invite(link: &str) -> Result<(url::Url, String)> {
    let parsed =
        url::Url::parse(link.trim()).map_err(|_| ApiError::bad("That is not an invite link"))?;
    let code = parsed
        .fragment()
        .and_then(|fragment| {
            url::form_urlencoded::parse(fragment.as_bytes())
                .find(|(key, _)| key == INVITE_FRAGMENT)
                .map(|(_, value)| value.trim().to_owned())
        })
        .filter(|code| !code.is_empty())
        .ok_or_else(|| ApiError::bad("That link has no invite code in it"))?;
    Ok((base_url(link)?, code))
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_default()
}

/// Why a call to another machine came to nothing.
#[derive(Debug)]
enum Failure {
    /// It could not be reached, or did not answer in time.
    Unreachable(String),
    /// It answered that it does not know this machine.
    Unknown,
    /// It answered with an error of its own.
    Refused(StatusCode, String),
}

impl Failure {
    fn into_api(self, name: &str) -> ApiError {
        match self {
            Failure::Unreachable(why) => {
                ApiError::conflict(format!("{name} cannot be reached: {why}"))
            }
            Failure::Unknown => ApiError::conflict(format!(
                "{name} no longer recognises this machine. Remove it here and pair again."
            )),
            Failure::Refused(status, message) => ApiError {
                status: if status.is_client_error() {
                    status
                } else {
                    StatusCode::BAD_GATEWAY
                },
                message: format!("{name}: {message}"),
            },
        }
    }
}

async fn send(
    base: &str,
    token: Option<&str>,
    method: reqwest::Method,
    path: &str,
    body: Option<&Value>,
    limit: Duration,
) -> std::result::Result<Value, Failure> {
    let url = format!("{}{}", base.trim_end_matches('/'), path);
    let mut request = client()
        .request(method, url)
        .timeout(limit)
        .header("X-Tendly-Client", "peer");
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request.send().await.map_err(|error| {
        Failure::Unreachable(if error.is_timeout() {
            "it did not answer in time".into()
        } else if error.is_connect() {
            "nothing answered at its address".into()
        } else {
            error.to_string()
        })
    })?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| Failure::Unreachable(error.to_string()))?;
    let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    if status == StatusCode::UNAUTHORIZED {
        return Err(Failure::Unknown);
    }
    if !status.is_success() {
        let message = value["error"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("it answered {status}; is that Tendly, and up to date?"));
        return Err(Failure::Refused(status, message));
    }
    Ok(value)
}

/// Call a paired machine with the key it issued this one.
async fn call(
    peer: &Peer,
    method: reqwest::Method,
    path: &str,
    body: Option<&Value>,
    limit: Duration,
) -> Result<Value> {
    let (Some(url), Some(token)) = (&peer.url, &peer.token) else {
        return Err(ApiError::conflict(format!(
            "{} has no address this machine can reach. It can reach this one; set up Remote access there to reach it from here.",
            peer.name
        )));
    };
    send(url, Some(token), method, path, body, limit)
        .await
        .map_err(|failure| failure.into_api(&peer.name))
}

// ---- What the person sees and does, from this machine's own window.

fn view(peer: &Peer) -> Value {
    json!({
        "id": peer.id,
        "name": peer.name,
        "url": peer.url,
        "grant": peer.grant,
        "granted": peer.granted,
        "paired_at": peer.paired_at,
    })
}

/// A paired machine, and how it answers just now.
async fn glance(state: &AppState, peer: Peer) -> Value {
    let mut out = view(&peer);
    if peer.url.is_none() || peer.token.is_none() {
        out["online"] = json!(null);
        return out;
    }
    match call(&peer, reqwest::Method::GET, "/api/peer/hello", None, GLANCE).await {
        Ok(hello) => {
            out["online"] = json!(true);
            out["version"] = hello["version"].clone();
            let granted = serde_json::from_value::<Grant>(hello["grant"].clone()).ok();
            let name = hello["name"].as_str().map(str::to_owned);
            if granted != peer.granted || name.as_ref().is_some_and(|name| *name != peer.name) {
                let id = peer.id;
                let _ = state.peers.change(|peers| {
                    if let Some(saved) = peers.iter_mut().find(|saved| saved.id == id) {
                        saved.granted = granted;
                        if let Some(name) = name {
                            saved.name = name;
                        }
                    }
                });
            }
            out["granted"] = json!(granted);
            if let Some(name) = hello["name"].as_str() {
                out["name"] = json!(name);
            }
            if hello["peer_api"].as_u64() != Some(PEER_API) {
                out["problem"] = json!("It runs a different version of Tendly. Update both.");
            }
        }
        Err(error) => {
            out["online"] = json!(false);
            out["problem"] = json!(error.message);
        }
    }
    out
}

async fn list(State(state): State<AppState>) -> Json<Value> {
    let looks = state
        .peers
        .list()
        .into_iter()
        .map(|peer| glance(&state, peer));
    let peers = futures_util::future::join_all(looks).await;
    Json(json!({ "name": own_name(&state), "peers": peers }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InviteInput {
    grant: Grant,
    #[serde(default)]
    origin: Option<String>,
}

async fn invite(
    State(state): State<AppState>,
    Json(input): Json<InviteInput>,
) -> Result<Json<Value>> {
    let Some(url) = own_url(&state, input.origin.as_deref()) else {
        return Err(ApiError::conflict(
            "Other machines cannot reach this one yet. Turn on Remote access first, or open Tendly here at an address they can reach.",
        ));
    };
    let code = state.peers.invite(input.grant);
    Ok(Json(json!({
        "link": format!("{url}#{INVITE_FRAGMENT}={code}"),
        "code": code,
        "url": url,
        "expires_in": INVITE_LIFETIME.as_secs(),
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddInput {
    /// The invite link the other machine made.
    #[serde(default)]
    invite: Option<String>,
    /// Or its tailnet address, when it is signed in to the same Tailscale
    /// account as this one.
    #[serde(default)]
    url: Option<String>,
    /// What that machine may do here.
    grant: Grant,
    /// What this machine may do there; only a machine of the same account
    /// takes it, as an invite carries its own.
    #[serde(default)]
    asked: Option<Grant>,
    #[serde(default)]
    origin: Option<String>,
}

async fn add(State(state): State<AppState>, Json(input): Json<AddInput>) -> Result<Json<Value>> {
    let (url, code) = match (input.invite.as_deref(), input.url.as_deref()) {
        (Some(invite), None) => {
            let (url, code) = parse_invite(invite)?;
            (url, Some(code))
        }
        (None, Some(url)) => (base_url(url)?, None),
        _ => return Err(ApiError::bad("Give an invite link or a tailnet address")),
    };
    let base = url.to_string();
    // Pairing without an invite is the two-way kind: the other machine has to
    // be able to call back.
    if code.is_none() && own_url(&state, input.origin.as_deref()).is_none() {
        return Err(ApiError::conflict(
            "The other machine could not reach this one back. Turn on Remote access here first.",
        ));
    }
    if own_url(&state, input.origin.as_deref()).as_deref() == Some(base.as_str()) {
        return Err(ApiError::bad(if code.is_some() {
            "That invite is from this machine. Use it on the other one."
        } else {
            "That is this machine."
        }));
    }
    if state
        .peers
        .list()
        .iter()
        .any(|peer| peer.url.as_deref() == Some(base.as_str()))
    {
        return Err(ApiError::conflict(
            "That machine is paired already. Remove it first to pair again.",
        ));
    }
    let key = secret("tdp_");
    let mut request = json!({
        "name": own_name(&state),
        "url": own_url(&state, input.origin.as_deref()),
        "token": key,
        "grant": input.grant,
        "peer_api": PEER_API,
    });
    match code {
        Some(code) => request["code"] = json!(code),
        None => request["asked"] = json!(input.asked.unwrap_or(Grant::View)),
    }
    let answer = send(
        &base,
        None,
        reqwest::Method::POST,
        "/api/peer/pair",
        Some(&request),
        Duration::from_secs(15),
    )
    .await
    .map_err(|failure| failure.into_api(url.host_str().unwrap_or("That machine")))?;
    if answer["peer_api"].as_u64() != Some(PEER_API) {
        return Err(ApiError::conflict(
            "That machine runs a different version of Tendly. Update both, then pair again.",
        ));
    }
    let (Some(name), Some(token)) = (answer["name"].as_str(), answer["token"].as_str()) else {
        return Err(ApiError::conflict("That machine's answer was not readable"));
    };
    let peer = Peer {
        id: Uuid::new_v4(),
        name: name.to_owned(),
        url: Some(base),
        token: Some(token.to_owned()),
        inbound: digest(&key),
        grant: input.grant,
        granted: serde_json::from_value(answer["grant"].clone()).ok(),
        paired_at: chrono::Utc::now().to_rfc3339(),
    };
    let saved = peer.clone();
    state
        .peers
        .change(move |peers| peers.push(saved))
        .map_err(ApiError::internal)?;
    tracing::info!(peer = %peer.name, "paired with another machine");
    Ok(Json(view(&peer)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GrantInput {
    grant: Grant,
}

async fn regrant(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(input): Json<GrantInput>,
) -> Result<Json<Value>> {
    let changed = state
        .peers
        .change(|peers| {
            peers.iter_mut().find(|peer| peer.id == id).map(|peer| {
                peer.grant = input.grant;
                peer.clone()
            })
        })
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::missing("Machine"))?;
    // Only so the other side's window says it; what is allowed here is
    // decided here whether or not it hears of it.
    let _ = call(
        &changed,
        reqwest::Method::POST,
        "/api/peer/granted",
        Some(&json!({ "grant": input.grant })),
        GLANCE,
    )
    .await;
    Ok(Json(view(&changed)))
}

async fn remove(State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<StatusCode> {
    let removed = state
        .peers
        .change(|peers| {
            let found = peers.iter().position(|peer| peer.id == id);
            found.map(|index| peers.remove(index))
        })
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::missing("Machine"))?;
    // Its key here is gone already; telling it is a courtesy that lets its
    // window stop showing this machine.
    let _ = call(
        &removed,
        reqwest::Method::POST,
        "/api/peer/unpair",
        None,
        GLANCE,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

fn paired(state: &AppState, id: Uuid) -> Result<Peer> {
    state
        .peers
        .get(id)
        .ok_or_else(|| ApiError::missing("Machine"))
}

async fn remote_sessions(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>> {
    let peer = paired(&state, id)?;
    call(
        &peer,
        reqwest::Method::GET,
        "/api/peer/sessions",
        None,
        Duration::from_secs(10),
    )
    .await
    .map(Json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenInput {
    #[serde(default)]
    session_id: Option<SessionId>,
}

/// A link that opens the other machine's own window, signed in. The ticket in
/// its fragment is good once, for a minute, and never reaches a server log.
async fn open(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(input): Json<OpenInput>,
) -> Result<Json<Value>> {
    let peer = paired(&state, id)?;
    let answer = call(
        &peer,
        reqwest::Method::POST,
        "/api/peer/ticket",
        Some(&json!({})),
        GLANCE,
    )
    .await?;
    let ticket = answer["ticket"]
        .as_str()
        .ok_or_else(|| ApiError::conflict("That machine gave no sign-in ticket"))?;
    let base = peer.url.as_deref().unwrap_or_default();
    let page = input
        .session_id
        .map(|session| format!("session/{session}"))
        .unwrap_or_default();
    Ok(Json(
        json!({ "url": format!("{base}{page}#tendly-ticket={ticket}") }),
    ))
}

/// Machines on the tailnet, and which of them run Tendly.
async fn discover(State(state): State<AppState>) -> Result<Json<Value>> {
    let machines = crate::tailscale::peers().await.map_err(|problem| {
        ApiError::conflict(format!("The tailnet could not be read: {}", problem.detail))
    })?;
    let paired: Vec<String> = state
        .peers
        .list()
        .into_iter()
        .filter_map(|peer| peer.url)
        .collect();
    let looks = machines.into_iter().filter(|machine| machine.online).map(|machine| {
        let paired = &paired;
        async move {
            let url = crate::tailscale::url(&machine.name);
            let health = send(&url, None, reqwest::Method::GET, "/api/health", None, GLANCE).await;
            let tendly = health
                .as_ref()
                .ok()
                .filter(|health| health["service"] == "tendly-server");
            json!({
                "name": machine.host,
                "url": url,
                "os": machine.os,
                "tendly": tendly.is_some(),
                "version": tendly.map(|health| health["version"].clone()),
                "pairs": tendly.is_some_and(|health| health["capabilities"].as_array().is_some_and(|caps| caps.iter().any(|cap| cap == "peers"))),
                "paired": paired.contains(&url),
                "mine": machine.mine,
            })
        }
    });
    Ok(Json(json!(futures_util::future::join_all(looks).await)))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/peers", get(list).post(add))
        .route("/api/peers/invite", post(invite))
        .route("/api/peers/discover", get(discover))
        .route("/api/peers/{id}", put(regrant).delete(remove))
        .route("/api/peers/{id}/sessions", get(remote_sessions))
        .route("/api/peers/{id}/open", post(open))
        // What other machines call, with the key this machine issued them.
        .route("/api/peer/pair", post(pair))
        .route("/api/peer/hello", get(hello))
        .route("/api/peer/granted", post(granted))
        .route("/api/peer/unpair", post(unpair))
        .route("/api/peer/sessions", get(sessions))
        .route("/api/peer/ticket", post(ticket))
        .route("/api/peer/call", post(peer_call))
}

// ---- What other machines call.

fn calling(state: &AppState, headers: &HeaderMap) -> Result<Peer> {
    state.peers.presenting(headers).ok_or_else(|| ApiError {
        status: StatusCode::UNAUTHORIZED,
        message: "This machine does not know yours".into(),
    })
}

fn allowed(peer: &Peer, needed: Grant) -> Result<()> {
    if peer.grant >= needed {
        return Ok(());
    }
    let what = match needed {
        Grant::View => "see its sessions",
        Grant::Spawn => "start sessions there",
        Grant::Control => "open its window",
    };
    Err(ApiError {
        status: StatusCode::FORBIDDEN,
        message: format!(
            "It does not let this machine {what}. That is changed in its Settings → Machines."
        ),
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PairInput {
    /// The invite's code; absent when the caller is of the same Tailscale
    /// account.
    #[serde(default)]
    code: Option<String>,
    /// What the caller may do here, when there is no invite to say.
    #[serde(default)]
    asked: Option<Grant>,
    name: String,
    url: Option<String>,
    token: String,
    /// What the inviting side may do there.
    grant: Grant,
    peer_api: u64,
}

/// Whether a request came over the tailnet from the Tailscale account this
/// machine is signed in to.
fn same_account(state: &AppState, via_tailnet: bool, headers: &HeaderMap) -> bool {
    let theirs = headers
        .get(TAILSCALE_LOGIN)
        .and_then(|value| value.to_str().ok())
        .filter(|login| !login.is_empty());
    via_tailnet
        && theirs
            .zip(state.tailnet.login())
            .is_some_and(|(theirs, mine)| theirs.eq_ignore_ascii_case(&mine))
}

async fn pair(
    State(state): State<AppState>,
    via: Option<Extension<crate::security::ViaTailnet>>,
    headers: HeaderMap,
    Json(input): Json<PairInput>,
) -> Result<Json<Value>> {
    let grant = match input.code.as_deref() {
        Some(code) => state.peers.redeem_invite(code).ok_or_else(|| ApiError {
            status: StatusCode::FORBIDDEN,
            message: "That invite is not valid. It may have expired or been used; make a new one."
                .into(),
        })?,
        None if same_account(&state, via.is_some(), &headers) => input.asked.unwrap_or(Grant::View),
        None => {
            return Err(ApiError {
                status: StatusCode::FORBIDDEN,
                message: "It is not signed in to the same Tailscale account as this machine, so pairing takes an invite: make one in this machine's Settings → Machines."
                    .into(),
            });
        }
    };
    if input.peer_api != PEER_API {
        return Err(ApiError::conflict(
            "The two machines run different versions of Tendly. Update both, then pair again.",
        ));
    }
    if input.token.len() < 32 {
        return Err(ApiError::bad("That key is too short"));
    }
    let name = crate::bounded_name(&input.name)?;
    let url = input
        .url
        .as_deref()
        .map(base_url)
        .transpose()?
        .map(|url| url.to_string());
    let key = secret("tdp_");
    let again = url.clone();
    let peer = Peer {
        id: Uuid::new_v4(),
        name: name.clone(),
        url,
        token: Some(input.token),
        inbound: digest(&key),
        grant,
        granted: Some(input.grant),
        paired_at: chrono::Utc::now().to_rfc3339(),
    };
    // A machine pairing again replaces the pairing it had: the other side
    // has forgotten it.
    state
        .peers
        .change(move |peers| {
            peers.retain(|known| again.is_none() || known.url != again);
            peers.push(peer)
        })
        .map_err(ApiError::internal)?;
    state.activity.notify(
        json!({ "kind": "machine", "title": name }),
        "Paired with this machine",
        json!({}),
        None,
    );
    Ok(Json(json!({
        "name": own_name(&state),
        "token": key,
        "grant": grant,
        "peer_api": PEER_API,
        "version": env!("CARGO_PKG_VERSION"),
    })))
}

async fn hello(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let peer = calling(&state, &headers)?;
    Ok(Json(json!({
        "name": own_name(&state),
        "version": env!("CARGO_PKG_VERSION"),
        "peer_api": PEER_API,
        "grant": peer.grant,
    })))
}

async fn granted(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<GrantInput>,
) -> Result<StatusCode> {
    let peer = calling(&state, &headers)?;
    state
        .peers
        .change(|peers| {
            if let Some(saved) = peers.iter_mut().find(|saved| saved.id == peer.id) {
                saved.granted = Some(input.grant);
            }
        })
        .map_err(ApiError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn unpair(State(state): State<AppState>, headers: HeaderMap) -> Result<StatusCode> {
    let peer = calling(&state, &headers)?;
    state
        .peers
        .change(|peers| peers.retain(|saved| saved.id != peer.id))
        .map_err(ApiError::internal)?;
    state.activity.notify(
        json!({ "kind": "machine", "title": peer.name }),
        "Unpaired from this machine",
        json!({}),
        None,
    );
    Ok(StatusCode::NO_CONTENT)
}

async fn sessions(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let peer = calling(&state, &headers)?;
    allowed(&peer, Grant::View)?;
    let workspaces = db(&state, |store| store.list_workspaces()).await?;
    let sessions = db(&state, |store| store.list_sessions(None)).await?;
    Ok(Json(json!({
        "name": own_name(&state),
        "grant": peer.grant,
        "workspaces": workspaces.iter().map(|w| json!({ "id": w.id, "name": w.name })).collect::<Vec<_>>(),
        "sessions": sessions
            .iter()
            .filter(|session| session.archived_at.is_none())
            .take(200)
            .map(|session| {
                let mut view = crate::agent::session_view(session);
                view["activity"] = json!(state.chats.get(session.id).map_or("stopped", |chat| chat.activity()));
                view
            })
            .collect::<Vec<_>>(),
    })))
}

async fn ticket(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>> {
    let peer = calling(&state, &headers)?;
    allowed(&peer, Grant::Control)?;
    if state.security.access_token().is_none() {
        return Err(ApiError::conflict(
            "It has no access token, so its window cannot be signed in to from elsewhere",
        ));
    }
    Ok(Json(json!({ "ticket": state.peers.ticket() })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CallInput {
    name: String,
    #[serde(default)]
    arguments: Value,
}

async fn peer_call(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<CallInput>,
) -> Result<Json<Value>> {
    let peer = calling(&state, &headers)?;
    let arguments = if input.arguments.is_null() {
        json!({})
    } else {
        input.arguments
    };
    if arguments.get("machine").is_some() {
        return Err(ApiError::bad(
            "A call from another machine cannot reach a third",
        ));
    }
    let caller = Caller::Peer(peer.id);
    let outcome = match input.name.as_str() {
        "tendly_status" => Ok(json!({
            "tendly": { "version": env!("CARGO_PKG_VERSION") },
            "machine": own_name(&state),
            "you": { "kind": "machine", "name": peer.name, "may": peer.grant },
        })),
        "tendly_list" => match allowed(&peer, Grant::View) {
            Ok(()) => crate::agent::list(&state, caller, &arguments).await,
            Err(error) => Err(error),
        },
        "tendly_spawn" => serve_spawn(&state, &peer, &arguments).await,
        "tendly_session" => serve_session(&state, &peer, &arguments).await,
        other => Err(ApiError::bad(format!(
            "{other} is not available to another machine"
        ))),
    };
    // A tool that fails is still an answer: the agent that asked reads it.
    Ok(Json(match outcome {
        Ok(value) => json!({ "ok": value }),
        Err(error) => json!({ "error": error.message }),
    }))
}

/// The workspace a call names, by id or by name.
async fn workspace_named(state: &AppState, named: &str) -> Result<tendly_domain::Workspace> {
    let workspaces = db(state, |store| store.list_workspaces()).await?;
    workspaces
        .into_iter()
        .find(|w| w.id.to_string() == named || w.name.eq_ignore_ascii_case(named))
        .ok_or_else(|| {
            ApiError::bad(format!(
                "There is no workspace {named:?} here; tendly_list with what=workspaces shows them"
            ))
        })
}

async fn serve_spawn(state: &AppState, peer: &Peer, arguments: &Value) -> Result<Value> {
    allowed(peer, Grant::Spawn)?;
    let named = text(arguments, "workspace")
        .or_else(|| text(arguments, "workspace_id"))
        .ok_or_else(|| ApiError::bad("workspace is required: which workspace there to start in"))?;
    let workspace = workspace_named(state, named).await?;
    let running = state
        .peers
        .children_of(peer.id)
        .into_iter()
        .filter(|id| crate::agent::live(state, *id))
        .count();
    if running >= 4 {
        return Err(ApiError::conflict(
            "4 sessions your machine started there are still running; wait for one to finish",
        ));
    }
    // Kept in its list by default: nobody there opened it, so a temporary
    // session would be one nobody could find.
    let mut start = agent_sessions::start_of(arguments, true)?;
    start.title = format!("⇠ {} · {}", peer.name, start.title.trim_start_matches("↳ "));
    let (owner, title) = (peer.id, crate::bounded_name(&start.title)?);
    agent_sessions::start_in(
        state,
        workspace.id,
        start,
        None,
        state.peers.who(peer.id),
        |id| async move {
            state.peers.adopt(id, owner);
            // Set, not derived: the first message would otherwise replace it,
            // and with it the one sign of where the session came from.
            db(state, move |s| s.update_session_title(id, &title)).await?;
            Ok(())
        },
    )
    .await
}

async fn peer_child(state: &AppState, peer: &Peer, id: SessionId) -> Result<Session> {
    if !state.peers.children_of(peer.id).contains(&id) {
        return Err(ApiError::bad("That is not a session your machine started"));
    }
    db(state, move |s| s.get_session(id))
        .await?
        .ok_or_else(|| ApiError::missing("Session"))
}

async fn serve_session(state: &AppState, peer: &Peer, arguments: &Value) -> Result<Value> {
    allowed(peer, Grant::Spawn)?;
    match text(arguments, "action") {
        Some("list") => {
            let mut list = Vec::new();
            for id in state.peers.children_of(peer.id) {
                if let Some(session) = db(state, move |s| s.get_session(id)).await? {
                    list.push(agent_sessions::child_view(state, &session));
                }
            }
            Ok(json!(list))
        }
        Some(action @ ("result" | "wait" | "message")) => {
            let target = peer_child(state, peer, agent_sessions::session_id(arguments)?).await?;
            agent_sessions::follow(state, target, action, arguments).await
        }
        _ => Err(ApiError::bad(
            "From another machine, action must be list, result, wait or message",
        )),
    }
}

// ---- The agent tools on this machine that reach another one.

pub fn tool() -> Value {
    crate::agent::tool(
        "tendly_machines",
        "Other machines running Tendly that this one is paired with, whether each is reachable now, and what each lets you do there: view (list its workspaces and sessions), spawn (also start sessions there and follow them) or control. Pass a machine's name as `machine` to tendly_list, tendly_spawn or tendly_session to run that call there.",
        json!({ "type": "object", "properties": {}, "additionalProperties": false }),
    )
}

/// The property that sends a call to another machine, for the tools that take it.
pub fn machine_property() -> Value {
    json!({ "type": "string", "description": "Run this call on another machine Tendly is paired with, by its name from tendly_machines. With tendly_spawn, also give workspace: the workspace there, by name or id; the default is one named like yours." })
}

pub async fn machines(state: &AppState) -> Result<Value> {
    let looks = state
        .peers
        .list()
        .into_iter()
        .map(|peer| glance(state, peer));
    let peers = futures_util::future::join_all(looks).await;
    Ok(json!({
        "this_machine": own_name(state),
        "machines": peers.iter().map(|peer| json!({
            "name": peer["name"],
            "reachable": peer["online"] == true,
            "you_may": peer["granted"],
            "problem": peer["problem"],
        })).collect::<Vec<_>>(),
    }))
}

fn peer_named(state: &AppState, named: &str) -> Result<Peer> {
    let named = named.trim();
    named_in(&state.peers.list(), named)
        .cloned()
        .ok_or_else(|| {
            ApiError::bad(format!(
                "No paired machine is called {named:?}; tendly_machines lists them"
            ))
        })
}

/// The paired machine a name means: its id, its name, or its host name or
/// the first label of it (`box` for `box.tail1.ts.net`), ignoring case.
fn named_in<'a>(peers: &'a [Peer], named: &str) -> Option<&'a Peer> {
    let host_of = |peer: &Peer| {
        peer.url
            .as_deref()
            .and_then(|url| url::Url::parse(url).ok())
            .and_then(|url| url.host_str().map(str::to_owned))
    };
    peers
        .iter()
        .find(|peer| peer.id.to_string() == named || peer.name.eq_ignore_ascii_case(named))
        .or_else(|| {
            peers.iter().find(|peer| {
                host_of(peer).is_some_and(|host| {
                    host.eq_ignore_ascii_case(named)
                        || host
                            .split('.')
                            .next()
                            .is_some_and(|first| first.eq_ignore_ascii_case(named))
                })
            })
        })
}

/// Run a tool call on another machine and return what it answered.
pub async fn forward(
    state: &AppState,
    caller: Caller,
    name: &str,
    arguments: &Value,
) -> Result<Value> {
    let session = match caller {
        Caller::Peer(_) => {
            return Err(ApiError::bad(
                "A call from another machine cannot reach a third",
            ));
        }
        Caller::Session(id) => {
            if agent_sessions::started_by_another(state, id).await? && name != "tendly_list" {
                return Err(ApiError::bad(
                    "A session started by another session cannot start or follow sessions on other machines",
                ));
            }
            Some(id)
        }
        Caller::External => None,
    };
    let peer = peer_named(state, text(arguments, "machine").unwrap_or(""))?;
    let mut arguments = arguments.clone();
    if let Some(object) = arguments.as_object_mut() {
        object.remove("machine");
    }
    let owner = match (name, session) {
        ("tendly_list", _) => None,
        (_, Some(session)) => Some(session),
        (_, None) => {
            return Err(ApiError::bad(format!(
                "{name} on another machine works only from a session Tendly launched"
            )));
        }
    };
    if let Some(session) = owner {
        match name {
            "tendly_spawn" => prepare_spawn(state, caller, session, &peer, &mut arguments).await?,
            "tendly_session" => {
                if let Some(id) = text(&arguments, "id") {
                    let key = (session, peer.id, id.to_owned());
                    if !state
                        .peers
                        .0
                        .remote_children
                        .lock()
                        .expect("remote children")
                        .contains(&key)
                    {
                        return Err(ApiError::bad("That is not a session you started there"));
                    }
                }
            }
            _ => {}
        }
    }
    let answer = call(
        &peer,
        reqwest::Method::POST,
        "/api/peer/call",
        Some(&json!({ "name": name, "arguments": arguments })),
        LONGEST_CALL,
    )
    .await?;
    if let Some(error) = answer["error"].as_str() {
        return Err(ApiError::bad(format!("{}: {error}", peer.name)));
    }
    let mut value = answer["ok"].clone();
    if let Some(session) = owner {
        let mine = |id: &str| {
            state
                .peers
                .0
                .remote_children
                .lock()
                .expect("remote children")
                .contains(&(session, peer.id, id.to_owned()))
        };
        match name {
            "tendly_spawn" => {
                if let Some(id) = value["started"]["id"].as_str() {
                    state
                        .peers
                        .0
                        .remote_children
                        .lock()
                        .expect("remote children")
                        .insert((session, peer.id, id.to_owned()));
                }
                value["machine"] = json!(peer.name);
                value["next"] = json!(format!(
                    "Call tendly_session with machine {:?}, action wait or result and this id to follow it.",
                    peer.name
                ));
            }
            // Of the sessions this machine started there, the ones this session did.
            "tendly_session" if text(&arguments, "action") == Some("list") => {
                if let Some(list) = value.as_array_mut() {
                    list.retain(|entry| entry["id"].as_str().is_some_and(mine));
                }
            }
            _ => {}
        }
    }
    Ok(json!({ "machine": peer.name, "result": value }))
}

/// Before a session's first start on another machine, the person here says
/// yes; and a start names a workspace there, defaulting to one named like the
/// session's own.
async fn prepare_spawn(
    state: &AppState,
    caller: Caller,
    session: SessionId,
    peer: &Peer,
    arguments: &mut Value,
) -> Result<()> {
    let me = db(state, move |s| s.get_session(session))
        .await?
        .ok_or_else(|| ApiError::missing("Session"))?;
    if text(arguments, "workspace").is_none() && text(arguments, "workspace_id").is_none() {
        let workspace_id = me.workspace_id;
        if let Some(workspace) = db(state, move |s| s.get_workspace(workspace_id)).await? {
            arguments["workspace"] = json!(workspace.name);
        }
    }
    // Branches and endpoint profiles are that machine's; ids from here mean nothing there.
    if let Some(object) = arguments.as_object_mut() {
        object.remove("workspace_id");
    }
    let start = agent_sessions::start_of(arguments, true)?;
    let first = !state
        .peers
        .0
        .remote_spawners
        .lock()
        .expect("remote spawners")
        .contains(&session);
    if first {
        let excerpt: String = start.prompt.chars().take(300).collect();
        let answer = state
            .activity
            .ask(
                crate::agent_config::who(state, caller).await,
                "Start a {client} session on {machine}",
                json!({ "client": start.provider.label(), "machine": peer.name }),
                vec![
                    ("Task", excerpt),
                    ("Workspace", text(arguments, "workspace").unwrap_or("").to_owned()),
                ],
                vec!["It runs on that machine, in its own copy of the workspace. This session may then start more there without asking, read their replies and message them."],
                None,
            )
            .await?;
        if !matches!(answer, crate::activity::Answer::Approved { .. }) {
            return Err(ApiError::conflict(
                "The person declined. No session was started.",
            ));
        }
        state
            .peers
            .0
            .remote_spawners
            .lock()
            .expect("remote spawners")
            .insert(session);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grants_include_everything_below_them() {
        assert!(Grant::Control > Grant::Spawn && Grant::Spawn > Grant::View);
        assert_eq!(serde_json::to_value(Grant::Spawn).unwrap(), json!("spawn"));
    }

    #[test]
    fn an_invite_link_is_its_address_and_its_code() {
        let (url, code) =
            parse_invite("https://box.tail1.ts.net/settings/machines#tendly-pair=ab12CD34ef56")
                .unwrap();
        assert_eq!(url.as_str(), "https://box.tail1.ts.net/");
        assert_eq!(code, "ab12CD34ef56");
        assert!(parse_invite("https://box.tail1.ts.net/").is_err());
        assert!(parse_invite("not a link").is_err());
        assert!(parse_invite("ftp://box/#tendly-pair=x").is_err());
    }

    #[test]
    fn an_invite_is_good_once_and_guessing_has_a_budget() {
        let peers = Peers::default();
        let code = peers.invite(Grant::Spawn);
        assert_eq!(
            peers.redeem_invite(&code.to_ascii_lowercase()),
            Some(Grant::Spawn)
        );
        assert_eq!(peers.redeem_invite(&code), None, "used already");
        let open = peers.invite(Grant::View);
        for _ in 0..INVITE_FAILURES {
            assert_eq!(peers.redeem_invite("WRONGWRONG00"), None);
        }
        assert_eq!(
            peers.redeem_invite(&open),
            None,
            "withdrawn after too many guesses"
        );
    }

    fn peer(name: &str, url: Option<&str>) -> Peer {
        Peer {
            id: Uuid::new_v4(),
            name: name.into(),
            url: url.map(str::to_owned),
            token: None,
            inbound: String::new(),
            grant: Grant::View,
            granted: None,
            paired_at: String::new(),
        }
    }

    #[test]
    fn a_machine_is_named_by_id_name_or_host_ignoring_case() {
        let peers = [
            peer("Linux Box", Some("https://box.tail1.ts.net/")),
            peer("studio", Some("http://192.168.0.9:28789/")),
            peer("phone-only", None),
        ];
        let named = |name: &str| named_in(&peers, name).map(|peer| peer.name.as_str());
        assert_eq!(named(&peers[1].id.to_string()), Some("studio"));
        assert_eq!(named("linux box"), Some("Linux Box"));
        assert_eq!(
            named("BOX"),
            Some("Linux Box"),
            "the tailnet name's first label"
        );
        assert_eq!(named("box.tail1.ts.net"), Some("Linux Box"));
        assert_eq!(named("192.168.0.9"), Some("studio"));
        assert_eq!(named("phone-only"), Some("phone-only"));
        assert_eq!(named("tail1"), None);
        assert_eq!(named(""), None);
    }

    #[test]
    fn the_configured_address_wins_then_the_tailnet_then_a_reachable_page() {
        let tailnet = || Some("https://mac.tail1.ts.net/".to_owned());
        assert_eq!(
            choose_url(Some("https://tendly.example.com/app?x#y"), tailnet(), None).as_deref(),
            Some("https://tendly.example.com/"),
            "a base address, whatever was pasted"
        );
        assert_eq!(
            choose_url(Some("  "), tailnet(), None).as_deref(),
            Some("https://mac.tail1.ts.net/")
        );
        assert_eq!(
            choose_url(
                Some("not a url"),
                None,
                Some("http://192.168.0.5:28789/settings")
            )
            .as_deref(),
            Some("http://192.168.0.5:28789/")
        );
        assert_eq!(choose_url(None, None, Some("http://127.0.0.1:28789")), None);
        assert_eq!(choose_url(None, None, None), None);
    }

    #[test]
    fn three_tools_take_a_machine_and_the_list_names_them() {
        let tools = crate::agent::tools();
        let schema = |name: &str| {
            tools
                .iter()
                .find(|tool| tool["name"] == name)
                .unwrap_or_else(|| panic!("{name}"))["inputSchema"]
                .clone()
        };
        for name in ["tendly_list", "tendly_spawn", "tendly_session"] {
            assert!(schema(name)["properties"]["machine"].is_object(), "{name}");
        }
        assert!(schema("tendly_spawn")["properties"]["workspace"].is_object());
        for name in [
            "tendly_status",
            "tendly_show",
            "tendly_endpoint",
            "tendly_canvas",
        ] {
            assert!(
                schema(name)["properties"].get("machine").is_none(),
                "{name}"
            );
        }
        assert!(
            schema("tendly_machines")["properties"]
                .as_object()
                .unwrap()
                .is_empty()
        );
        assert!(crate::agent::INSTRUCTIONS.contains("tendly_machines"));
    }

    #[test]
    fn an_invite_left_too_long_is_not_good() {
        let peers = Peers::default();
        let code = peers.invite(Grant::View);
        peers.0.invites.lock().unwrap()[0].until = Instant::now() - Duration::from_secs(1);
        assert_eq!(peers.redeem_invite(&code), None);
    }

    #[test]
    fn only_three_invites_are_open_at_once_the_oldest_going_first() {
        let peers = Peers::default();
        let codes: Vec<_> = (0..4).map(|_| peers.invite(Grant::View)).collect();
        assert_eq!(peers.redeem_invite(&codes[0]), None);
        for code in &codes[1..] {
            assert_eq!(peers.redeem_invite(code), Some(Grant::View));
        }
    }

    #[test]
    fn a_ticket_signs_in_once() {
        let peers = Peers::default();
        let ticket = peers.ticket();
        assert!(peers.redeem_ticket(&ticket));
        assert!(!peers.redeem_ticket(&ticket));
        assert!(!peers.redeem_ticket("tdt_guess"));
    }

    #[test]
    fn only_an_address_another_machine_could_reach_is_offered() {
        for (page, offered) in [
            ("http://127.0.0.1:28789", false),
            ("http://localhost:28789", false),
            ("http://[::1]:28789", false),
            ("http://0.0.0.0:28789", false),
            ("http://192.168.0.5:28789", true),
            ("https://box.example.com", true),
        ] {
            let url = url::Url::parse(page).unwrap();
            assert_eq!(reachable_elsewhere(&url), offered, "{page}");
        }
    }

    #[test]
    fn keys_are_kept_owner_only_and_recognised_by_their_hash_alone() {
        let dir = std::env::temp_dir().join(format!("tendly-peers-{}", Uuid::new_v4()));
        let peers = Peers::new(&dir);
        let key = secret("tdp_");
        let peer = Peer {
            id: Uuid::new_v4(),
            name: "box".into(),
            url: None,
            token: Some("theirs".into()),
            inbound: digest(&key),
            grant: Grant::View,
            granted: None,
            paired_at: String::new(),
        };
        peers.change(|all| all.push(peer)).unwrap();
        let saved = std::fs::read_to_string(dir.join(STATE_FILE)).unwrap();
        assert!(
            !saved.contains(&key),
            "the key it presents here is never written"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(STATE_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let reloaded = Peers::new(&dir);
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {key}").parse().unwrap(),
        );
        assert!(reloaded.is_peer_request(&headers));
        headers.insert(header::AUTHORIZATION, "Bearer tdp_other".parse().unwrap());
        assert!(!reloaded.is_peer_request(&headers));
        std::fs::remove_dir_all(dir).ok();
    }
}
