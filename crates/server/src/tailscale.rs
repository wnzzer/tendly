//! Publishing the gateway to a tailnet with `tailscale serve`.
//!
//! Reaching a workspace from a phone was the awkward part: SSH forwarding is
//! hard to do from one, `--lan` is plain HTTP, and a reverse proxy is something
//! to set up and keep. `tailscale serve` is all of that in one command — a real
//! certificate, a stable name, and only devices on the tailnet can connect —
//! while the gateway itself keeps listening on loopback.
//!
//! Tailnet traffic arrives on a loopback listener of its own rather than on the
//! gateway's port. Everything `tailscale serve` forwards comes from 127.0.0.1,
//! so the peer address cannot tell a phone from this machine; the port can.
//! Requests on that listener carry [`ViaTailnet`](crate::security::ViaTailnet)
//! and always need the access token, while the browser on this machine keeps
//! needing nothing.
//!
//! It can be switched on and off from the settings page without a restart, and
//! every failure on the way is reported as a code the page explains with the
//! one thing to do about it. A watchdog keeps it honest afterwards: a serve
//! configuration someone removed is put back, and a tailnet that went away is
//! reported rather than shown as live.
use crate::{ApiError, AppState, Result, security::ViaTailnet};
use axum::{
    Extension, Json, Router,
    extract::State,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{io::AsyncReadExt, process::Command};

/// The HTTPS port on the tailnet name. One is enough, and 443 is the one that
/// needs no port in the URL typed on a phone.
const HTTPS_PORT: u16 = 443;

/// Whether to publish, and what was published last, kept across restarts.
const STATE_FILE: &str = "tailnet.json";

/// How often a live publication is checked, and a failed one retried.
const WATCH_INTERVAL: Duration = Duration::from_secs(60);
/// How long a fresh publication is given to answer. The first one on a machine
/// waits for its certificate to be issued, which can take a minute or more.
const VERIFY_WINDOW: Duration = Duration::from_secs(150);

/// Whether this deployment asked to be published (`--tailscale`, or
/// `tailscale = true` in the settings file).
pub fn enabled() -> bool {
    std::env::var("TENDLY_TAILSCALE")
        .is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
}

/// The CLI to run. An explicit path wins; otherwise `tailscale` on PATH, and
/// where the installer puts it when it is not: inside the app on macOS, which
/// the App Store build does not put on PATH, and under Program Files on
/// Windows, which a gateway started before the install does not see on PATH.
fn program() -> String {
    if let Ok(explicit) = std::env::var("TENDLY_TAILSCALE_BIN")
        && !explicit.is_empty()
    {
        return explicit;
    }
    if which("tailscale").is_none()
        && let Some(installed) = installed_elsewhere().filter(|path| path.is_file())
    {
        return installed.to_string_lossy().into_owned();
    }
    "tailscale".to_owned()
}

fn installed_elsewhere() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        return Some("/Applications/Tailscale.app/Contents/MacOS/Tailscale".into());
    }
    if cfg!(windows) {
        let programs =
            std::env::var_os("ProgramFiles").unwrap_or_else(|| r"C:\Program Files".into());
        return Some(
            PathBuf::from(programs)
                .join("Tailscale")
                .join("tailscale.exe"),
        );
    }
    None
}

/// `name` on PATH, as the shell would find it (with `.exe` on Windows).
fn which(name: &str) -> Option<PathBuf> {
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(&file))
        .find(|path| path.is_file())
}

/// Something in the way, as a code the page explains in its own words, with
/// whatever fixes it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Problem {
    pub code: &'static str,
    /// Tailscale's own words, for when the explanation is not enough.
    pub detail: String,
    /// A command that fixes it, when there is one.
    pub command: Option<String>,
    /// A page that fixes it, when there is one.
    pub link: Option<String>,
}

impl Problem {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
            command: None,
            link: None,
        }
    }
    fn command(mut self, command: impl Into<String>) -> Self {
        self.command = Some(command.into());
        self
    }
    fn link(mut self, link: impl Into<String>) -> Self {
        self.link = Some(link.into());
        self
    }
}

const DNS_SETTINGS: &str = "https://login.tailscale.com/admin/dns";

fn not_installed(program: &str) -> Problem {
    let problem = Problem::new(
        "not_installed",
        format!(
            "{program:?} was not found. Set TENDLY_TAILSCALE_BIN if it is installed somewhere else."
        ),
    );
    if cfg!(target_os = "macos") {
        problem
            .command("brew install --cask tailscale")
            .link("https://tailscale.com/download/mac")
    } else if cfg!(windows) {
        problem
            .command("winget install --id Tailscale.Tailscale -e --source winget")
            .link("https://tailscale.com/download/windows")
    } else {
        problem
            .command("curl -fsSL https://tailscale.com/install.sh | sh")
            .link("https://tailscale.com/download/linux")
    }
}

/// How to start a daemon that is installed but not running, where that is a
/// command: a service on Linux and Windows (from an administrator PowerShell
/// there), and on macOS the Homebrew formula, which is a CLI and daemon
/// without the app.
fn daemon_command() -> Option<&'static str> {
    if cfg!(target_os = "linux") {
        return Some("sudo systemctl enable --now tailscaled");
    }
    if cfg!(windows) {
        return Some("Start-Service Tailscale");
    }
    #[cfg(target_os = "macos")]
    if !std::path::Path::new("/Applications/Tailscale.app").exists()
        && which("tailscale")
            .is_some_and(|path| path.starts_with("/opt/homebrew") || path.starts_with("/usr/local"))
    {
        return Some("sudo brew services start tailscale");
    }
    None
}

/// Recognise the failures that have a known fix, from what the CLI said.
fn classify(output: &str) -> Problem {
    let lower = output.to_ascii_lowercase();
    // Serve, and with it HTTPS, has to be allowed for the tailnet once. The CLI
    // prints where to do that and then waits for it.
    if let Some(link) = consent_link(output) {
        return Problem::new("serve_consent", output.trim()).link(link);
    }
    if lower.contains("access denied") {
        // On Linux an ordinary user may not change serve until made operator.
        return Problem::new("access_denied", output.trim())
            .command("sudo tailscale set --operator=$USER");
    }
    if lower.contains("failed to connect to local tailscale")
        || lower.contains("is tailscaled running")
        || lower.contains("tailscaled not running")
    {
        let problem = Problem::new("daemon_down", output.trim());
        return match daemon_command() {
            Some(command) => problem.command(command),
            // The app runs its own daemon; opening it is the fix.
            None => problem,
        };
    }
    if lower.contains("flag provided but not defined") || lower.contains("unknown flag") {
        return Problem::new("outdated", output.trim()).link("https://tailscale.com/download");
    }
    if lower.contains("https") && (lower.contains("not enabled") || lower.contains("disabled"))
        || lower.contains("certificate")
    {
        return Problem::new("no_https", output.trim()).link(DNS_SETTINGS);
    }
    Problem::new("failed", output.trim())
}

/// The page the CLI asks the person to visit, if it printed one.
fn consent_link(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .find(|word| word.starts_with("https://login.tailscale.com/"))
        .map(|word| word.trim_end_matches(['.', ',', ')']).to_owned())
}

/// Run the CLI and collect what it says.
///
/// Read as it arrives rather than at exit, because `serve` asking for consent
/// prints a link and then waits indefinitely; the link is the useful part, so
/// the wait ends as soon as it shows up.
async fn tailscale(args: &[&str], limit: Duration) -> std::result::Result<String, Problem> {
    let program = program();
    let mut command = Command::new(&program);
    // The gateway runs without a console on Windows; without this every call
    // would flash one up.
    #[cfg(windows)]
    command.creation_flags(tendly_runtime::process::CREATE_NO_WINDOW);
    let mut child = command
        .args(args)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => not_installed(&program),
            _ => Problem::new("failed", format!("Could not run {program:?}: {error}")),
        })?;
    let output = Arc::new(Mutex::new(String::new()));
    let readers: Vec<_> = [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
    ]
    .into_iter()
    .flatten()
    .map(|mut stream| {
        let output = output.clone();
        tokio::spawn(async move {
            let mut buffer = [0u8; 4096];
            while let Ok(read) = stream.read(&mut buffer).await {
                if read == 0 {
                    break;
                }
                output
                    .lock()
                    .expect("tailscale output")
                    .push_str(&String::from_utf8_lossy(&buffer[..read]));
            }
        })
    })
    .collect();
    let deadline = Instant::now() + limit;
    let status = loop {
        if let Some(status) = child.try_wait().ok().flatten() {
            break Some(status);
        }
        if Instant::now() >= deadline
            || consent_link(&output.lock().expect("tailscale output")).is_some()
        {
            let _ = child.kill().await;
            break None;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    };
    for reader in readers {
        let _ = tokio::time::timeout(Duration::from_secs(1), reader).await;
    }
    let said = output.lock().expect("tailscale output").clone();
    match status {
        Some(status) if status.success() => Ok(said),
        Some(_) => Err(classify(&said)),
        None if consent_link(&said).is_some() => Err(classify(&said)),
        None => Err(Problem::new(
            "failed",
            format!(
                "`tailscale {}` did not finish in {} seconds. {}",
                args.join(" "),
                limit.as_secs(),
                said.trim()
            ),
        )),
    }
}

/// What the tailnet knows about this machine.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Machine {
    /// The MagicDNS name, without the trailing dot.
    pub name: String,
    /// Shown for diagnosis only: the certificate is for the name.
    pub ip: Option<String>,
    pub login: Option<String>,
    pub tailnet: Option<String>,
    /// Whether the tailnet issues certificates.
    pub https: bool,
}

fn machine_from_status(status: &str) -> std::result::Result<Machine, Problem> {
    let status: Value = serde_json::from_str(status).map_err(|error| {
        Problem::new(
            "failed",
            format!("`tailscale status --json` was not readable: {error}"),
        )
    })?;
    let state = status["BackendState"].as_str().unwrap_or("");
    if state != "Running" {
        // "NeedsLogin" and "Stopped" are the common ones.
        let problem =
            Problem::new("not_running", format!("Tailscale is {state:?}.")).command("tailscale up");
        return Err(
            match status["AuthURL"].as_str().filter(|url| !url.is_empty()) {
                Some(url) => problem.link(url),
                None => problem,
            },
        );
    }
    let own = &status["Self"];
    let name = own["DNSName"].as_str().unwrap_or("").trim_end_matches('.');
    if name.is_empty() {
        return Err(Problem::new(
            "no_magic_dns",
            "Tailscale reported no name for this machine.",
        )
        .link(DNS_SETTINGS));
    }
    let ips: Vec<&str> = own["TailscaleIPs"]
        .as_array()
        .map(|ips| ips.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let user = own["UserID"].as_i64().map(|id| id.to_string());
    Ok(Machine {
        name: name.to_owned(),
        ip: ips
            .iter()
            .find(|ip| ip.contains('.'))
            .or(ips.first())
            .map(|ip| (*ip).to_owned()),
        login: user
            .and_then(|id| status["User"][id]["LoginName"].as_str())
            .map(str::to_owned),
        tailnet: status["CurrentTailnet"]["Name"]
            .as_str()
            .or_else(|| status["MagicDNSSuffix"].as_str())
            .map(str::to_owned),
        https: status["CertDomains"]
            .as_array()
            .is_some_and(|domains| !domains.is_empty()),
    })
}

/// Another machine on the tailnet, as `tailscale status` lists it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TailnetPeer {
    /// The MagicDNS name, without the trailing dot.
    pub name: String,
    pub host: String,
    pub os: String,
    pub online: bool,
}

fn peers_from_status(status: &str) -> std::result::Result<Vec<TailnetPeer>, Problem> {
    machine_from_status(status)?;
    let status: Value = serde_json::from_str(status).unwrap_or_default();
    let mut peers: Vec<TailnetPeer> = status["Peer"]
        .as_object()
        .map(|peers| {
            peers
                .values()
                .filter_map(|peer| {
                    let name = peer["DNSName"].as_str()?.trim_end_matches('.');
                    (!name.is_empty()).then(|| TailnetPeer {
                        name: name.to_owned(),
                        host: peer["HostName"].as_str().unwrap_or(name).to_owned(),
                        os: peer["OS"].as_str().unwrap_or("").to_owned(),
                        online: peer["Online"].as_bool().unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    peers.sort_by(|a, b| (!a.online, &a.name).cmp(&(!b.online, &b.name)));
    Ok(peers)
}

/// The other machines on this tailnet, for finding Tendly on them.
pub async fn peers() -> std::result::Result<Vec<TailnetPeer>, Problem> {
    peers_from_status(&tailscale(&["status", "--json"], Duration::from_secs(10)).await?)
}

/// The URL the tailnet reaches this gateway at.
pub fn url(name: &str) -> String {
    format!("https://{name}/")
}

/// What the tailnet name serves at 443 already, if it is something other than
/// this gateway. Publishing over it would take someone else's service away.
/// `ours` lists what this gateway forwards to now and what it forwarded to on
/// an earlier run, whose leftover is safe to replace.
fn occupied_by(serve_status: &str, name: &str, ours: &[&str]) -> Option<String> {
    let status: Value = serde_json::from_str(serve_status).ok()?;
    let web = &status["Web"][format!("{name}:{HTTPS_PORT}")]["Handlers"];
    if let Some(handlers) = web.as_object() {
        for (path, handler) in handlers {
            let what = handler["Proxy"]
                .as_str()
                .or_else(|| handler["Path"].as_str())
                .or_else(|| handler["Text"].as_str().map(|_| "a text response"))
                .unwrap_or("another handler");
            if path != "/" || !ours.contains(&what.trim_end_matches('/')) {
                return Some(format!("{path} → {what}"));
            }
        }
        return None;
    }
    // A raw TCP forward on the port leaves no room for an HTTPS handler.
    status["TCP"][HTTPS_PORT.to_string()]["TCPForward"]
        .as_str()
        .map(|forward| format!("TCP → {forward}"))
}

/// Whether the name serves exactly this gateway at 443 right now.
fn serves_target(serve_status: &str, name: &str, target: &str) -> bool {
    serde_json::from_str::<Value>(serve_status).is_ok_and(|status| {
        status["Web"][format!("{name}:{HTTPS_PORT}")]["Handlers"]["/"]["Proxy"]
            .as_str()
            .is_some_and(|proxy| proxy.trim_end_matches('/') == target)
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Off,
    Starting,
    Live,
    Error,
}

/// GET /api/tailnet.
#[derive(Clone, Debug, Serialize)]
pub struct Status {
    /// Whether the person wants it published.
    enabled: bool,
    /// Turned on by `--tailscale` or the settings file, which the next start
    /// applies again whatever the page says.
    forced: bool,
    phase: Phase,
    url: Option<String>,
    machine: Option<Machine>,
    /// Whether this machine reached the published URL; `None` until checked.
    verified: Option<bool>,
    /// Unix milliseconds of the last check.
    checked_at: Option<i64>,
    problem: Option<Problem>,
    /// Where `tailscale serve` forwards to.
    target: String,
    /// Whether the request asking came in over the tailnet.
    via_tailnet: bool,
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    enabled: bool,
    /// What `tailscale serve` was last told to forward to, so a start after a
    /// crash recognises the leftover as its own.
    #[serde(default)]
    published: Option<String>,
}

struct Current {
    enabled: bool,
    phase: Phase,
    machine: Option<Machine>,
    verified: Option<bool>,
    checked_at: Option<i64>,
    problem: Option<Problem>,
}

/// The tailnet publication of this gateway, shared by the API, the watchdog
/// and shutdown.
#[derive(Clone)]
pub struct Tailnet {
    inner: Arc<Inner>,
}

struct Inner {
    state_dir: PathBuf,
    /// `http://127.0.0.1:<port>` of the tailnet listener.
    target: String,
    forced: bool,
    current: Mutex<Current>,
    /// One publish or withdrawal at a time.
    operation: tokio::sync::Mutex<()>,
    /// A check of the published address is under way.
    verifying: std::sync::atomic::AtomicBool,
}

impl Tailnet {
    pub fn new(state_dir: &Path, port: u16) -> Self {
        let forced = enabled();
        let saved = read_saved(state_dir);
        Self {
            inner: Arc::new(Inner {
                state_dir: state_dir.to_owned(),
                target: format!("http://127.0.0.1:{port}"),
                forced,
                current: Mutex::new(Current {
                    enabled: forced || saved.enabled,
                    phase: Phase::Off,
                    machine: None,
                    verified: None,
                    checked_at: None,
                    problem: None,
                }),
                operation: tokio::sync::Mutex::new(()),
                verifying: std::sync::atomic::AtomicBool::new(false),
            }),
        }
    }

    fn current(&self) -> std::sync::MutexGuard<'_, Current> {
        self.inner.current.lock().expect("tailnet state")
    }

    pub fn status(&self, via_tailnet: bool) -> Status {
        let current = self.current();
        Status {
            enabled: current.enabled,
            forced: self.inner.forced,
            phase: current.phase,
            url: current
                .machine
                .as_ref()
                .filter(|_| current.phase == Phase::Live)
                .map(|machine| url(&machine.name)),
            machine: current.machine.clone(),
            verified: current.verified,
            checked_at: current.checked_at,
            problem: current.problem.clone(),
            target: self.inner.target.clone(),
            via_tailnet,
        }
    }

    /// The address the tailnet reaches this gateway at, while it is live.
    pub fn published_url(&self) -> Option<String> {
        self.status(false).url
    }

    /// The tailnet name, once known.
    pub fn name(&self) -> Option<String> {
        self.current()
            .machine
            .as_ref()
            .map(|machine| machine.name.clone())
    }

    /// Whether a `Host` header is one the tailnet listener answers for: the
    /// tailnet name, or the listener itself should Tailscale rewrite it.
    pub fn serves_host(&self, host: &str) -> bool {
        let host = host.strip_suffix(":443").unwrap_or(host);
        self.name()
            .is_some_and(|name| host.eq_ignore_ascii_case(&name))
            || self.inner.target.strip_prefix("http://") == Some(host)
    }

    /// Whether an `Origin` is a page the tailnet listener serves: the tailnet
    /// name, or the listener's own address, as with `serves_host`.
    pub fn serves_origin(&self, origin: &str) -> bool {
        origin == self.inner.target
            || origin.strip_prefix("https://").is_some_and(|host| {
                self.name()
                    .is_some_and(|name| host.eq_ignore_ascii_case(&name))
            })
    }

    /// Publish if asked to, and keep watching. Called once the listeners are
    /// bound, so Tailscale never forwards to nothing.
    pub fn start(&self) {
        if self.current().enabled {
            self.begin();
        }
        let this = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(WATCH_INTERVAL);
            tick.tick().await;
            loop {
                tick.tick().await;
                let (enabled, phase) = {
                    let current = this.current();
                    (current.enabled, current.phase)
                };
                match (enabled, phase) {
                    (true, Phase::Live) => this.recheck().await,
                    (true, Phase::Error) => this.bring_up().await,
                    _ => {}
                }
            }
        });
    }

    /// Show the attempt as under way at once, so the page that asked sees it,
    /// and run it in the background.
    fn begin(&self) {
        {
            let mut current = self.current();
            current.phase = Phase::Starting;
            current.problem = None;
            current.verified = None;
        }
        let this = self.clone();
        tokio::spawn(async move { this.bring_up().await });
    }

    pub fn enable(&self) {
        self.current().enabled = true;
        self.save(|saved| saved.enabled = true);
        self.begin();
    }

    /// Try again now rather than at the next watchdog tick.
    pub fn retry(&self) {
        if self.current().enabled {
            self.begin();
        }
    }

    pub async fn disable(&self) {
        self.current().enabled = false;
        self.save(|saved| saved.enabled = false);
        let _operation = self.inner.operation.lock().await;
        self.withdraw().await;
        let mut current = self.current();
        current.phase = Phase::Off;
        current.problem = None;
        current.verified = None;
    }

    /// Take the name down on the way out, so it does not keep forwarding to a
    /// port nothing listens on.
    pub async fn shutdown(&self) {
        let _operation = self.inner.operation.lock().await;
        self.withdraw().await;
    }

    async fn bring_up(&self) {
        let _operation = self.inner.operation.lock().await;
        if !self.current().enabled {
            return;
        }
        self.current().phase = Phase::Starting;
        let outcome = self.publish().await;
        let mut current = self.current();
        current.checked_at = Some(chrono::Utc::now().timestamp_millis());
        match outcome {
            Ok(name) => {
                // Live as soon as Tailscale accepted it; whether it answers is
                // found out alongside, without holding up a switch-off.
                current.phase = Phase::Live;
                current.verified = None;
                current.problem = None;
                drop(current);
                self.verify(name);
            }
            Err(problem) => {
                tracing::warn!(code = problem.code, detail = %problem.detail, "tailnet publish failed");
                current.phase = Phase::Error;
                current.verified = None;
                current.problem = Some(problem);
            }
        }
    }

    /// Open the published address from this machine until it answers or
    /// [`VERIFY_WINDOW`] runs out, in the background. Gives up quietly when the
    /// publication changed in the meantime.
    fn verify(&self, name: String) {
        use std::sync::atomic::Ordering;
        if self.inner.verifying.swap(true, Ordering::AcqRel) {
            return;
        }
        let ip = self
            .current()
            .machine
            .as_ref()
            .and_then(|machine| machine.ip.as_deref()?.parse().ok());
        let this = self.clone();
        tokio::spawn(async move {
            let verified = reachable(&name, ip, || this.serves(&name)).await;
            this.inner.verifying.store(false, Ordering::Release);
            let Some(verified) = verified else { return };
            let mut current = this.current();
            if !this.serves_locked(&current, &name) {
                return;
            }
            current.verified = Some(verified);
            current.checked_at = Some(chrono::Utc::now().timestamp_millis());
            current.problem = (!verified).then(|| {
                Problem::new(
                    "unreachable",
                    "This machine could not open the published address.",
                )
            });
        });
    }

    /// Still live, under the same name.
    fn serves(&self, name: &str) -> bool {
        self.serves_locked(&self.current(), name)
    }

    fn serves_locked(&self, current: &Current, name: &str) -> bool {
        current.enabled
            && current.phase == Phase::Live
            && current
                .machine
                .as_ref()
                .is_some_and(|machine| machine.name == name)
    }

    /// Every step from finding the CLI to `tailscale serve`. `Ok` carries the
    /// name it is published under.
    async fn publish(&self) -> std::result::Result<String, Problem> {
        let status = tailscale(&["status", "--json"], Duration::from_secs(15)).await?;
        let machine = machine_from_status(&status)?;
        let name = machine.name.clone();
        self.current().machine = Some(machine);
        let target = self.inner.target.clone();
        let previous = read_saved(&self.inner.state_dir).published;
        let ours: Vec<&str> = [Some(target.as_str()), previous.as_deref()]
            .into_iter()
            .flatten()
            .collect();
        // A failed read is not a reason to stop: the publish below reports
        // whatever is actually wrong, in Tailscale's own words.
        if let Ok(current) =
            tailscale(&["serve", "status", "--json"], Duration::from_secs(15)).await
            && let Some(other) = occupied_by(&current, &name, &ours)
        {
            return Err(Problem::new(
                "port_taken",
                format!("https://{name}/ already serves {other}."),
            )
            .command(format!("tailscale serve --https={HTTPS_PORT} off")));
        }
        // Recorded first, so a crash between here and the end still leaves the
        // next start able to recognise what it finds.
        self.save(|saved| saved.published = Some(target.clone()));
        let https = format!("--https={HTTPS_PORT}");
        tailscale(&["serve", "--bg", &https, &target], Duration::from_secs(30)).await?;
        tracing::info!(url = %url(&name), "published to the tailnet");
        Ok(name)
    }

    /// The watchdog's look at a live publication: still there, still ours,
    /// and reachable if it was not before.
    async fn recheck(&self) {
        let Some(name) = self.name() else { return };
        let still = tailscale(&["serve", "status", "--json"], Duration::from_secs(15))
            .await
            .is_ok_and(|status| serves_target(&status, &name, &self.inner.target));
        if !still {
            // Removed by hand, or Tailscale restarted without it: put it back,
            // which reports anything now in the way.
            tracing::info!("tailnet publication went missing; publishing again");
            return self.bring_up().await;
        }
        if self.current().verified != Some(true) {
            self.verify(name);
        }
    }

    /// Remove what this gateway published, and only that.
    async fn withdraw(&self) {
        let Some(published) = read_saved(&self.inner.state_dir).published else {
            return;
        };
        if let Some(name) = self.name()
            && let Ok(status) =
                tailscale(&["serve", "status", "--json"], Duration::from_secs(15)).await
            && !serves_target(&status, &name, &published)
            && !serves_target(&status, &name, &self.inner.target)
        {
            // Something else is there now; it is not ours to remove.
            self.save(|saved| saved.published = None);
            return;
        }
        let https = format!("--https={HTTPS_PORT}");
        match tailscale(&["serve", &https, "off"], Duration::from_secs(15)).await {
            Ok(_) => self.save(|saved| saved.published = None),
            Err(problem) => {
                tracing::warn!(detail = %problem.detail, "could not remove the Tailscale serve configuration")
            }
        }
    }

    fn save(&self, change: impl FnOnce(&mut Saved)) {
        save(&self.inner.state_dir, change);
    }

    #[cfg(test)]
    pub fn disabled() -> Self {
        Self::new(&std::env::temp_dir().join("tendly-no-tailnet"), 0)
    }
}

fn save(state_dir: &Path, change: impl FnOnce(&mut Saved)) {
    let path = state_dir.join(STATE_FILE);
    let mut saved = read_saved(state_dir);
    change(&mut saved);
    if let Err(error) = serde_json::to_string_pretty(&saved)
        .map_err(std::io::Error::other)
        .and_then(|text| std::fs::write(&path, text))
    {
        tracing::warn!(%error, path = %path.display(), "could not save the tailnet setting");
    }
}

/// Remove what a gateway stopped from outside left published.
///
/// Windows has no signal a detached gateway can receive, so `tendly stop`
/// ends it without its shutdown, and the name would go on forwarding to a
/// closed port: a phone would get an error page instead of nothing at all.
/// Elsewhere the gateway withdrew on its way out and this finds nothing.
pub async fn withdraw_left_behind(state_dir: &Path) {
    let Some(published) = read_saved(state_dir).published else {
        return;
    };
    let Ok(status) = tailscale(&["serve", "status", "--json"], Duration::from_secs(15)).await
    else {
        return;
    };
    if forwards_to(&status, &published) {
        let https = format!("--https={HTTPS_PORT}");
        if tailscale(&["serve", &https, "off"], Duration::from_secs(15))
            .await
            .is_err()
        {
            return;
        }
    }
    save(state_dir, |saved| saved.published = None);
}

/// Whether any name serves `target` at 443.
fn forwards_to(serve_status: &str, target: &str) -> bool {
    let Ok(status) = serde_json::from_str::<Value>(serve_status) else {
        return false;
    };
    status["Web"].as_object().is_some_and(|names| {
        names.iter().any(|(host, web)| {
            host.ends_with(&format!(":{HTTPS_PORT}"))
                && web["Handlers"]["/"]["Proxy"]
                    .as_str()
                    .is_some_and(|proxy| proxy.trim_end_matches('/') == target)
        })
    })
}

fn read_saved(state_dir: &Path) -> Saved {
    std::fs::read_to_string(state_dir.join(STATE_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Whether this machine reaches its own published address within
/// [`VERIFY_WINDOW`]. Until the certificate is issued the handshake fails, so
/// it keeps trying; `None` when `still` says the answer no longer matters.
///
/// With the machine's tailnet address it connects there directly: what is in
/// question is the publication and its certificate, which is still checked
/// against the name, not whether this machine's own DNS knows MagicDNS.
async fn reachable(
    name: &str,
    ip: Option<std::net::IpAddr>,
    still: impl Fn() -> bool,
) -> Option<bool> {
    let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(15));
    if let Some(ip) = ip {
        builder = builder.resolve(name, std::net::SocketAddr::new(ip, HTTPS_PORT));
    }
    let Ok(client) = builder.build() else {
        return Some(false);
    };
    let health = format!("{}api/health", url(name));
    let deadline = Instant::now() + VERIFY_WINDOW;
    loop {
        if !still() {
            return None;
        }
        if client
            .get(&health)
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            return Some(true);
        }
        if Instant::now() >= deadline {
            return Some(false);
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

type Via = Option<Extension<ViaTailnet>>;

async fn read(State(state): State<AppState>, via: Via) -> Json<Status> {
    Json(state.tailnet.status(via.is_some()))
}

#[derive(Deserialize)]
struct Switch {
    enabled: bool,
}

async fn switch(
    State(state): State<AppState>,
    via: Via,
    Json(input): Json<Switch>,
) -> Json<Status> {
    if input.enabled {
        state.tailnet.enable();
    } else {
        state.tailnet.disable().await;
    }
    Json(state.tailnet.status(via.is_some()))
}

async fn check(State(state): State<AppState>, via: Via) -> Json<Status> {
    state.tailnet.retry();
    Json(state.tailnet.status(via.is_some()))
}

/// A link that signs a phone in as it opens, and the same as a QR code. The
/// token travels in the fragment, which the browser never sends anywhere; the
/// page reads it, signs in and removes it from the address bar.
async fn login_code(State(state): State<AppState>) -> Result<Json<Value>> {
    let status = state.tailnet.status(false);
    let Some(url) = status.url else {
        return Err(ApiError::conflict("The tailnet address is not live yet"));
    };
    let Some(token) = state.security.access_token() else {
        return Err(ApiError::conflict("This gateway has no access token"));
    };
    let link = format!("{url}#tendly-token={token}");
    let code = qrcode::QrCode::new(link.as_bytes()).map_err(ApiError::internal)?;
    let svg = code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(220, 220)
        .dark_color(qrcode::render::svg::Color("#000000"))
        .light_color(qrcode::render::svg::Color("#ffffff"))
        .build();
    Ok(Json(serde_json::json!({ "link": link, "svg": svg })))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/tailnet", get(read).put(switch))
        .route("/api/tailnet/check", post(check))
        .route("/api/tailnet/login-code", get(login_code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_comes_from_a_connected_tailnet_without_its_trailing_dot() {
        let status = r#"{"BackendState":"Running","Self":{"DNSName":"mac.tail1234.ts.net.","TailscaleIPs":["fd7a:115c::1","100.101.102.103"],"UserID":42},"User":{"42":{"LoginName":"me@example.com"}},"CurrentTailnet":{"Name":"me@example.com"},"CertDomains":["mac.tail1234.ts.net"]}"#;
        assert_eq!(
            machine_from_status(status).unwrap(),
            Machine {
                name: "mac.tail1234.ts.net".into(),
                ip: Some("100.101.102.103".into()),
                login: Some("me@example.com".into()),
                tailnet: Some("me@example.com".into()),
                https: true,
            }
        );
        let bare = machine_from_status(
            r#"{"BackendState":"Running","Self":{"DNSName":"mac.tail1234.ts.net."}}"#,
        )
        .unwrap();
        assert!(!bare.https);
        assert_eq!(bare.ip, None);
    }

    #[test]
    fn the_other_machines_are_listed_online_first_without_their_trailing_dot() {
        let status = serde_json::json!({
            "BackendState": "Running",
            "Self": { "DNSName": "mac.tail1.ts.net.", "TailscaleIPs": ["100.64.0.1"] },
            "Peer": {
                "a": { "DNSName": "zeta.tail1.ts.net.", "HostName": "zeta", "OS": "linux", "Online": true },
                "b": { "DNSName": "alpha.tail1.ts.net.", "HostName": "alpha", "OS": "macOS", "Online": false },
                "c": { "DNSName": "", "HostName": "nameless" }
            }
        })
        .to_string();
        let peers = peers_from_status(&status).unwrap();
        let names: Vec<_> = peers.iter().map(|peer| peer.name.as_str()).collect();
        assert_eq!(names, ["zeta.tail1.ts.net", "alpha.tail1.ts.net"]);
        assert!(peers[0].online && !peers[1].online);
    }

    #[test]
    fn a_disconnected_tailnet_says_how_to_connect_it() {
        let problem = machine_from_status(r#"{"BackendState":"NeedsLogin","AuthURL":"https://login.tailscale.com/a/xyz","Self":{}}"#).unwrap_err();
        assert_eq!(problem.code, "not_running");
        assert_eq!(problem.command.as_deref(), Some("tailscale up"));
        assert_eq!(
            problem.link.as_deref(),
            Some("https://login.tailscale.com/a/xyz")
        );
        let problem =
            machine_from_status(r#"{"BackendState":"Running","Self":{"DNSName":""}}"#).unwrap_err();
        assert_eq!(problem.code, "no_magic_dns");
        assert_eq!(problem.link.as_deref(), Some(DNS_SETTINGS));
    }

    #[test]
    fn failures_with_a_known_fix_are_recognised() {
        let consent = "Serve is not enabled on your tailnet.\nTo enable, visit:\n\n         https://login.tailscale.com/f/serve?node=abc123\n";
        let problem = classify(consent);
        assert_eq!(problem.code, "serve_consent");
        assert_eq!(
            problem.link.as_deref(),
            Some("https://login.tailscale.com/f/serve?node=abc123")
        );
        assert_eq!(
            classify("Access denied: serve config denied").code,
            "access_denied"
        );
        assert_eq!(
            classify("failed to connect to local tailscaled; it doesn't appear to be running").code,
            "daemon_down"
        );
        // What 1.102 says.
        assert_eq!(
            classify("failed to connect to local Tailscale service; is Tailscale running?").code,
            "daemon_down"
        );
        assert_eq!(
            classify("flag provided but not defined: -bg").code,
            "outdated"
        );
        assert_eq!(
            classify("HTTPS is not enabled for this tailnet").code,
            "no_https"
        );
        let other = classify("something unexpected\n");
        assert_eq!(
            (other.code, other.detail.as_str()),
            ("failed", "something unexpected")
        );
    }

    #[test]
    fn publishing_again_over_itself_is_fine_but_not_over_something_else() {
        let name = "mac.tail1234.ts.net";
        let ours = ["http://127.0.0.1:28789"];
        assert_eq!(occupied_by("{}", name, &ours), None);
        assert_eq!(occupied_by("not json", name, &ours), None);
        let own = r#"{"TCP":{"443":{"HTTPS":true}},"Web":{"mac.tail1234.ts.net:443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:28789"}}}}}"#;
        assert_eq!(occupied_by(own, name, &ours), None);
        let theirs = r#"{"TCP":{"443":{"HTTPS":true}},"Web":{"mac.tail1234.ts.net:443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:3000"}}}}}"#;
        assert_eq!(
            occupied_by(theirs, name, &ours).as_deref(),
            Some("/ → http://127.0.0.1:3000")
        );
        let beside = r#"{"Web":{"mac.tail1234.ts.net:443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:28789"},"/grafana":{"Proxy":"http://127.0.0.1:3000"}}}}}"#;
        assert!(occupied_by(beside, name, &ours).is_some());
        let tcp = r#"{"TCP":{"443":{"TCPForward":"127.0.0.1:8443"}}}"#;
        assert_eq!(
            occupied_by(tcp, name, &ours).as_deref(),
            Some("TCP → 127.0.0.1:8443")
        );
        // Another port on the same name is not in the way.
        let elsewhere = r#"{"Web":{"mac.tail1234.ts.net:8443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:3000"}}}}}"#;
        assert_eq!(occupied_by(elsewhere, name, &ours), None);
    }

    /// The tailnet listener's port changes with every start, so what an earlier
    /// run left behind has to be recognised as replaceable.
    #[test]
    fn an_earlier_runs_leftover_is_replaced_rather_than_refused() {
        let name = "mac.tail1234.ts.net";
        let leftover = r#"{"Web":{"mac.tail1234.ts.net:443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:51000"}}}}}"#;
        assert!(occupied_by(leftover, name, &["http://127.0.0.1:52000"]).is_some());
        assert_eq!(
            occupied_by(
                leftover,
                name,
                &["http://127.0.0.1:52000", "http://127.0.0.1:51000"]
            ),
            None
        );
        assert!(serves_target(leftover, name, "http://127.0.0.1:51000"));
        // Found without knowing the name, which a stopped gateway no longer has.
        assert!(forwards_to(leftover, "http://127.0.0.1:51000"));
        assert!(!forwards_to(leftover, "http://127.0.0.1:52000"));
        assert!(!forwards_to("{}", "http://127.0.0.1:51000"));
        assert!(!serves_target(leftover, name, "http://127.0.0.1:52000"));
    }

    #[test]
    fn the_tailnet_listener_answers_for_its_name_and_itself_only() {
        let tailnet = Tailnet::new(&std::env::temp_dir().join("tendly-tailnet-hosts"), 51000);
        assert!(!tailnet.serves_host("mac.tail1234.ts.net"), "no name yet");
        tailnet.current().machine = Some(Machine {
            name: "mac.tail1234.ts.net".into(),
            ..Machine::default()
        });
        for host in [
            "mac.tail1234.ts.net",
            "mac.tail1234.ts.net:443",
            "127.0.0.1:51000",
        ] {
            assert!(tailnet.serves_host(host), "{host}");
        }
        for host in [
            "evil.ts.net",
            "mac.tail1234.ts.net.evil.com",
            "127.0.0.1:28789",
        ] {
            assert!(!tailnet.serves_host(host), "{host}");
        }
        assert!(tailnet.serves_origin("https://mac.tail1234.ts.net"));
        assert!(!tailnet.serves_origin("http://mac.tail1234.ts.net"));
        assert!(tailnet.serves_origin("http://127.0.0.1:51000"));
        assert!(!tailnet.serves_origin("http://127.0.0.1:28789"));
        assert!(!tailnet.serves_origin("https://mac.tail1234.ts.net.evil.com"));
    }
}
