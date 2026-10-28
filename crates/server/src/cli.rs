//! The `tendly` command line: starting, stopping and checking the background
//! gateway, and the help text.

use super::*;

/// Start the background gateway and report an address that actually answers.
///
/// A start that returned as soon as the process existed would print a URL that
/// is not listening yet, and would call a configuration error a success: the
/// detached process writes its failure to the log and exits, where nobody sees
/// it. So this waits for the health endpoint, and on timeout shows the log tail
/// that explains why rather than a URL that will not load.
pub(super) async fn gateway_start(
    state_dir: &std::path::Path,
    address: SocketAddr,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    if daemon::healthy(address).await {
        println!("Tendly is already running at http://{address}/");
        return Ok(());
    }
    let pid = daemon::start(state_dir, address)?;
    if !daemon::wait_until_ready(address, Duration::from_secs(30)).await {
        let log = daemon::tail(state_dir, 20);
        let _ = daemon::stop(state_dir, Duration::from_secs(5));
        return Err(format!(
            "Tendly did not become ready on {address}.\n\n{log}\n\nFull log: {}",
            daemon::log_file(state_dir).display()
        )
        .into());
    }
    println!("Tendly {} (pid {pid})\n", env!("CARGO_PKG_VERSION"));
    print_access(state_dir, address, true).await;
    println!(
        "\n  logs    tendly logs\n  stop    tendly stop\n\nState: {}",
        state_dir.display()
    );
    Ok(())
}

/// Where to point a browser, and what it will ask for.
///
/// The token belongs here rather than only in the log: the gateway is detached,
/// so whatever it printed on its own stdout is somewhere nobody looks, and a URL
/// without the token it demands is only half an answer.
pub(super) async fn print_access(state_dir: &std::path::Path, address: SocketAddr, wait: bool) {
    for (label, url) in banner_urls(address) {
        println!("  {label}  {url}");
    }
    let tailnet = tailnet_line(address, wait).await;
    if let Some(line) = &tailnet {
        println!("  Tailnet  {line}");
    }
    if (!address.ip().is_loopback() || tailnet.is_some())
        && let Some(token) = security::stored_token(state_dir)
    {
        println!("  Token    {token}");
    }
}

/// What the running gateway says about its tailnet address, if it was asked
/// to publish one. A start waits for the attempt to finish, since the address
/// is what the person starting it came for.
pub(super) async fn tailnet_line(address: SocketAddr, wait: bool) -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .ok()?;
    let url = format!("http://127.0.0.1:{}/api/tailnet", address.port());
    let deadline = std::time::Instant::now() + Duration::from_secs(if wait { 60 } else { 0 });
    loop {
        let mut request = client.get(&url);
        if let Some(token) = security::existing_token() {
            request = request.bearer_auth(token);
        }
        let status: Value = request.send().await.ok()?.json().await.ok()?;
        match status["phase"].as_str()? {
            "off" => return None,
            "live" => {
                let url = status["url"].as_str()?.to_owned();
                return Some(if status["verified"] == json!(false) {
                    format!(
                        "{url}  (could not reach it from this machine; check Settings → Remote access)"
                    )
                } else {
                    url
                });
            }
            "starting" if std::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            "starting" => return Some("publishing… (see Settings → Remote access)".into()),
            _ => {
                let problem = &status["problem"];
                let fix = problem["command"]
                    .as_str()
                    .or_else(|| problem["link"].as_str())
                    .map(|fix| format!("\n           fix: {fix}"))
                    .unwrap_or_default();
                return Some(format!(
                    "not published: {}{fix}\n           details in Settings → Remote access",
                    problem["detail"].as_str().unwrap_or("unknown error")
                ));
            }
        }
    }
}

pub(super) async fn gateway_stop(
    state_dir: &std::path::Path,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    match daemon::stop(state_dir, Duration::from_secs(20))? {
        Some(pid) => println!("Stopped Tendly (pid {pid})"),
        None => println!("Tendly is not running"),
    }
    tailscale::withdraw_left_behind(state_dir).await;
    Ok(())
}

/// `start`, `stop` and `restart` handed to the service manager that runs the
/// gateway, reported as the background gateway's would be.
pub(super) async fn service_control(
    state_dir: &std::path::Path,
    address: SocketAddr,
    action: &str,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    if action == "start" && daemon::healthy(address).await {
        println!("Tendly is already running at http://{address}/");
        return Ok(());
    }
    service::control(action)?;
    if action == "stop" {
        println!(
            "Stopped Tendly; it starts again with this computer (`tendly service uninstall` to stop that)"
        );
        return Ok(());
    }
    if !daemon::wait_until_ready(address, Duration::from_secs(30)).await {
        return Err(format!(
            "Tendly did not become ready on {address}.\n\n{}\n\nFull log: {}",
            daemon::tail(state_dir, 20),
            daemon::log_file(state_dir).display()
        )
        .into());
    }
    println!("Tendly {} (service)\n", env!("CARGO_PKG_VERSION"));
    print_access(state_dir, address, true).await;
    Ok(())
}

pub(super) async fn gateway_status(
    state_dir: &std::path::Path,
    address: SocketAddr,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let pid = daemon::running(state_dir);
    let answering = daemon::healthy(address).await;
    match (pid, answering) {
        (Some(pid), true) => {
            println!("Running (pid {pid})\n");
            print_access(state_dir, address, false).await;
        }
        // A process that is up but silent is the shape a hung or still-starting
        // server takes, and saying "running" would hide it.
        (Some(pid), false) => println!("Process {pid} is up but {address} is not answering"),
        // Something else owns the port: reporting "not running" alone would be
        // true and useless when a start is about to fail on the bind.
        (None, true) => match service::installed() {
            Some(place) => {
                println!("Running as a service ({place})\n");
                print_access(state_dir, address, false).await;
            }
            None => println!("Not started from this state directory, but {address} answers"),
        },
        (None, false) => println!("Not running"),
    }
    Ok(())
}

/// The addresses worth printing for a given bind. A wildcard bind is reachable
/// at every interface, and the loopback name alone would not be the one someone
/// on the network needs.
pub(super) fn banner_urls(address: SocketAddr) -> Vec<(&'static str, String)> {
    let mut urls = vec![("Local  ", format!("http://127.0.0.1:{}/", address.port()))];
    if address.ip().is_unspecified() {
        urls.push((
            "Network",
            format!("http://<this machine>:{}/", address.port()),
        ));
    } else if !address.ip().is_loopback() {
        urls[0] = ("Address", format!("http://{address}/"));
    }
    urls
}

/// Where the gateway listens unless told otherwise.
///
/// Deliberately not a common development port: 8787 collides with several
/// things a developer machine already runs, and a workspace that quietly failed
/// to bind is worse than one on a number nobody else claims.
pub(super) const DEFAULT_ADDRESS: &str = "127.0.0.1:28789";

pub(super) const HELP: &str = r#"Tendly — a host workspace for Claude Code and Codex

  tendly                    Start the gateway in the background
  tendly --lan              Same, reachable from other machines on the network
  tendly --tailscale        Same, and published to your tailnet over HTTPS
                            (or switch it on in Settings → Remote access)
  tendly stop               Stop it
  tendly restart            Stop it, then start it again
  tendly status             Whether it is running, and where
  tendly logs               What the background gateway has said
  tendly service install    Start Tendly with this computer (systemd user unit,
                            launchd agent, or a Windows sign-in entry); takes
                            --lan / --tailscale like `tendly`
  tendly service uninstall  Stop doing that
  tendly service status     Whether it is installed and answering
  tendly update             Install the latest version (npm installs); the
                            gateway keeps running the old one until restarted
  tendly update --check     Only say whether a newer version exists
  tendly serve              Run in the foreground instead (no background process)
  tendly init               Create user state without starting anything
  tendly mcp                Tendly's agent tools as a stdio MCP server, e.g.
                            claude mcp add tendly -- tendly mcp
  tendly resume <id>        Continue a Tendly session in this terminal, with
                            its client, endpoint, model and checkout; <id> is
                            the Tendly session ID (Copy resume command)

  --version                 Print the version
  --help                    This text

Settings live in <state>/config.toml, and an environment variable overrides the
file:

  lan = true              # reachable from other machines (same as --lan)
  port = 28789            # or addr = "192.168.0.9:28789"
  token = "..."           # TENDLY_TOKEN
  tailscale = true        # publish to the tailnet (same as --tailscale)

Any other key names the TENDLY_ variable it spells: shell, claude-bin,
instance-label, allowed-origins = ["http://..."], and so on. Folder lists
take single quotes on Windows: workspace-roots = ['E:\code', 'D:\work']

Listens on 127.0.0.1:28789 otherwise. A binding that reaches other machines
needs an access token; one is generated and kept in the state directory unless a
token is given, and `status` prints it again. State lives in ~/.tendly;
TENDLY_HOME or TENDLY_STATE_DIR override it (that one cannot come from the
file, which lives inside it), and existing project-local .tendly databases
are preserved."#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_banner_names_where_the_gateway_can_be_reached() {
        let local = banner_urls("127.0.0.1:28789".parse().unwrap());
        assert_eq!(local, [("Local  ", "http://127.0.0.1:28789/".to_owned())]);
        let every = banner_urls("0.0.0.0:28789".parse().unwrap());
        assert_eq!(every.len(), 2, "local, and the network");
        assert_eq!(every[1].1, "http://<this machine>:28789/");
        let one = banner_urls("192.168.1.5:9000".parse().unwrap());
        assert_eq!(one, [("Address", "http://192.168.1.5:9000/".to_owned())]);
    }
}
