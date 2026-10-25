//! `tendly service`: the gateway started by the operating system, so it is
//! there after a reboot without anyone typing `tendly`.
//!
//! Each system's own per-user mechanism, so nothing needs an administrator:
//! a systemd user unit on Linux, a launchd agent on macOS, and on Windows a
//! `Run` entry that does `tendly start` at sign-in. Linux and macOS run
//! `serve` under the manager, which restarts it after a crash or an update
//! (exit 75, see `update::RESTART_EXIT_CODE`); on Windows the result is the
//! same background gateway `tendly start` makes.
//!
//! The service is written with this binary's path, the state directory and
//! the PATH of the shell that installed it: a service manager starts with an
//! environment of its own, where `claude`, `codex` and `node` are not found.
use crate::{cli, daemon};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::fs;
use std::{
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

type Outcome = Result<(), Box<dyn std::error::Error>>;

#[cfg(target_os = "linux")]
const UNIT: &str = "tendly.service";
#[cfg(target_os = "macos")]
const LABEL: &str = "com.wnzzer.tendly";
#[cfg(windows)]
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

/// What the service runs, and with what around it.
struct Launch {
    program: PathBuf,
    flags: Vec<&'static str>,
    state_dir: PathBuf,
    path: Option<String>,
    home: Option<PathBuf>,
}

impl Launch {
    fn current(state_dir: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let program = env::current_exe()?;
        // The real binary, not a symlink npm may repoint: an update replaces
        // the file at this path, which is what the next start should run.
        let program = program.canonicalize().unwrap_or(program);
        let flags = ["--lan", "--tailscale"]
            .into_iter()
            .filter(|flag| env::args().any(|argument| argument == *flag))
            .collect();
        Ok(Self {
            program,
            flags,
            state_dir: state_dir.to_path_buf(),
            path: env::var("PATH").ok(),
            home: env::var_os("HOME")
                .or_else(|| env::var_os("USERPROFILE"))
                .map(PathBuf::from),
        })
    }
}

pub async fn command(state_dir: &Path, address: SocketAddr) -> Outcome {
    let sub = env::args()
        .skip(1)
        .filter(|argument| !argument.starts_with('-'))
        .nth(1)
        .unwrap_or_else(|| "status".into());
    match sub.as_str() {
        "install" => install(state_dir, address).await,
        "uninstall" | "remove" => uninstall(state_dir).await,
        "status" => {
            match installed() {
                Some(place) => println!(
                    "Installed: {place}\n{}",
                    if daemon::healthy(address).await {
                        format!("Answering at http://{address}/")
                    } else {
                        format!("Not answering at {address}; see `tendly logs`")
                    }
                ),
                None => println!(
                    "Not installed; `tendly service install` starts Tendly with this computer"
                ),
            }
            Ok(())
        }
        other => Err(format!(
            "Unknown service command {other:?}; use install, uninstall or status"
        )
        .into()),
    }
}

/// The first line of every file this writes, which is how one is told from a
/// unit someone wrote by hand.
const MARKER: &str = "Written by `tendly service install`";

async fn install(state_dir: &Path, address: SocketAddr) -> Outcome {
    let launch = Launch::current(state_dir)?;
    refuse_hand_written("replace")?;
    // The service is about to want the port a background gateway holds.
    if let Some(pid) = daemon::stop(state_dir, Duration::from_secs(20))? {
        println!("Stopped the background gateway (pid {pid}); the service takes over");
    }
    let place = register(&launch, address)?;
    if !daemon::wait_until_ready(address, Duration::from_secs(30)).await {
        return Err(format!(
            "Installed at {place}, but Tendly did not answer on {address}.\n\n{}\n\nFull log: {}",
            daemon::tail(state_dir, 20),
            daemon::log_file(state_dir).display()
        )
        .into());
    }
    println!(
        "Tendly {} now starts with this computer\n",
        env!("CARGO_PKG_VERSION")
    );
    cli::print_access(state_dir, address, true).await;
    println!("\n  service  {place}\n  logs     tendly logs\n  remove   tendly service uninstall");
    Ok(())
}

async fn uninstall(state_dir: &Path) -> Outcome {
    let Some(place) = installed() else {
        println!("Not installed");
        return Ok(());
    };
    refuse_hand_written("remove")?;
    unregister()?;
    // On Windows the entry only started an ordinary background gateway.
    let _ = daemon::stop(state_dir, Duration::from_secs(20));
    println!("Removed {place}; Tendly no longer starts with this computer, and is stopped");
    Ok(())
}

/// Where the service is registered, if it is.
pub fn installed() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let unit = unit_path()?;
        unit.is_file().then(|| unit.display().to_string())
    }
    #[cfg(target_os = "macos")]
    {
        let plist = plist_path()?;
        plist.is_file().then(|| plist.display().to_string())
    }
    #[cfg(windows)]
    {
        run(Command::new("reg").args(["query", RUN_KEY, "/v", "Tendly"]))
            .ok()
            .map(|_| format!(r"{RUN_KEY}\Tendly"))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        None
    }
}

/// Whether `start`, `stop` and `restart` belong to the service manager: it
/// runs the gateway there, and a gateway it did not start would be fought
/// over the port. Windows' entry starts an ordinary background gateway.
pub fn manages_gateway() -> bool {
    cfg!(any(target_os = "linux", target_os = "macos")) && installed().is_some()
}

/// `tendly start`, `stop` or `restart` for a gateway the service runs.
pub fn control(action: &str) -> Outcome {
    #[cfg(target_os = "linux")]
    {
        run(Command::new("systemctl").args(["--user", action, UNIT]))?;
    }
    #[cfg(target_os = "macos")]
    {
        let target = format!("gui/{}/{LABEL}", unsafe { libc::getuid() });
        match action {
            "stop" => {
                run(Command::new("launchctl").args(["bootout", &target]))?;
            }
            _ => {
                if run(Command::new("launchctl").args(["kickstart", "-k", &target])).is_err() {
                    // Stopped by `tendly stop`, which unloads it.
                    let plist = plist_path().ok_or("No home directory")?;
                    run(Command::new("launchctl").args([
                        "bootstrap",
                        &format!("gui/{}", unsafe { libc::getuid() }),
                        &plist.to_string_lossy(),
                    ]))?;
                }
            }
        }
    }
    let _ = action;
    Ok(())
}

/// The file the service lives in, where it is one.
fn service_file() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        unit_path()
    }
    #[cfg(target_os = "macos")]
    {
        plist_path()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// A service someone wrote by hand may load settings this one would not (an
/// EnvironmentFile, another port): replacing or removing it is theirs to ask
/// for, with --force.
fn refuse_hand_written(verb: &str) -> Outcome {
    let Some(file) = service_file().filter(|file| file.is_file()) else {
        return Ok(());
    };
    let ours = std::fs::read_to_string(&file).is_ok_and(|text| text.contains(MARKER));
    if ours || env::args().any(|argument| argument == "--force") {
        return Ok(());
    }
    Err(format!(
        "{} was not written by Tendly, and may already start it with settings of its \
         own. Leave it, or pass --force to {verb} it.",
        file.display()
    )
    .into())
}

#[cfg(target_os = "linux")]
fn unit_path() -> Option<PathBuf> {
    let config = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config.join("systemd/user").join(UNIT))
}

#[cfg(target_os = "macos")]
fn plist_path() -> Option<PathBuf> {
    let home = env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist")),
    )
}

#[cfg(target_os = "linux")]
fn register(launch: &Launch, _address: SocketAddr) -> Result<String, Box<dyn std::error::Error>> {
    if run(Command::new("systemctl").args(["--user", "show-environment"])).is_err() {
        return Err(
            "No systemd user manager here (a container, or WSL without systemd). \
                    Run `tendly serve` under whatever supervises this machine instead."
                .into(),
        );
    }
    let unit = unit_path().ok_or("No home directory for the systemd unit")?;
    fs::create_dir_all(unit.parent().expect("unit directory"))?;
    fs::write(&unit, systemd_unit(launch))?;
    run(Command::new("systemctl").args(["--user", "daemon-reload"]))?;
    run(Command::new("systemctl").args(["--user", "enable", UNIT]))?;
    run(Command::new("systemctl").args(["--user", "restart", UNIT]))?;
    // Without lingering, a user's services start at sign-in and stop at the
    // last sign-out: a server nobody logs in to would never start it.
    let user = env::var("USER").unwrap_or_default();
    let lingering = Command::new("loginctl")
        .args(["show-user", &user, "--property=Linger"])
        .output()
        .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).contains("Linger=yes"));
    if !lingering && run(Command::new("loginctl").args(["enable-linger", &user])).is_err() {
        println!(
            "Note: it starts when you sign in. To start it at boot without signing in, run\n  sudo loginctl enable-linger {user}\n"
        );
    }
    Ok(unit.display().to_string())
}

#[cfg(target_os = "linux")]
fn unregister() -> Outcome {
    let _ = run(Command::new("systemctl").args(["--user", "disable", "--now", UNIT]));
    if let Some(unit) = unit_path() {
        fs::remove_file(unit)?;
    }
    let _ = run(Command::new("systemctl").args(["--user", "daemon-reload"]));
    Ok(())
}

#[cfg(target_os = "macos")]
fn register(launch: &Launch, _address: SocketAddr) -> Result<String, Box<dyn std::error::Error>> {
    let plist = plist_path().ok_or("No home directory for the launch agent")?;
    fs::create_dir_all(plist.parent().expect("agent directory"))?;
    let domain = format!("gui/{}", unsafe { libc::getuid() });
    // A previous install is still loaded with what it was written with.
    let _ = run(Command::new("launchctl").args(["bootout", &format!("{domain}/{LABEL}")]));
    fs::write(&plist, launch_agent(launch))?;
    run(Command::new("launchctl").args(["bootstrap", &domain, &plist.to_string_lossy()]))?;
    Ok(plist.display().to_string())
}

#[cfg(target_os = "macos")]
fn unregister() -> Outcome {
    let target = format!("gui/{}/{LABEL}", unsafe { libc::getuid() });
    let _ = run(Command::new("launchctl").args(["bootout", &target]));
    if let Some(plist) = plist_path() {
        fs::remove_file(plist)?;
    }
    Ok(())
}

#[cfg(windows)]
fn register(launch: &Launch, _address: SocketAddr) -> Result<String, Box<dyn std::error::Error>> {
    run(Command::new("reg").args([
        "add",
        RUN_KEY,
        "/v",
        "Tendly",
        "/t",
        "REG_SZ",
        "/d",
        &run_entry(launch),
        "/f",
    ]))?;
    // The entry runs at the next sign-in; this one starts it now.
    daemon::start(&launch.state_dir, _address)?;
    Ok(format!(r"{RUN_KEY}\Tendly"))
}

#[cfg(windows)]
fn unregister() -> Outcome {
    run(Command::new("reg").args(["delete", RUN_KEY, "/v", "Tendly", "/f"]))?;
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn register(_launch: &Launch, _address: SocketAddr) -> Result<String, Box<dyn std::error::Error>> {
    Err("No service manager is supported on this system; run `tendly serve` under your own".into())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn unregister() -> Outcome {
    Ok(())
}

/// Run a manager's command, its own complaint as the error when it fails.
#[cfg_attr(
    not(any(target_os = "linux", target_os = "macos", windows)),
    allow(dead_code)
)]
fn run(command: &mut Command) -> Result<String, Box<dyn std::error::Error>> {
    let output = command.output()?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let said = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(format!(
        "{command:?} failed: {}",
        if said.is_empty() {
            output.status.to_string()
        } else {
            said
        }
    )
    .into())
}

/// A value systemd reads as one word: quoted, with `\`, `"`, and the `%`
/// specifier and `$` expansion characters escaped.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn systemd_word(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%")
        .replace('$', "$$");
    format!("\"{escaped}\"")
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn systemd_unit(launch: &Launch) -> String {
    let mut exec = vec![
        systemd_word(&launch.program.to_string_lossy()),
        "serve".to_owned(),
    ];
    exec.extend(launch.flags.iter().map(|flag| (*flag).to_owned()));
    let mut environment = vec![format!(
        "TENDLY_STATE_DIR={}",
        launch.state_dir.to_string_lossy()
    )];
    if let Some(path) = &launch.path {
        environment.push(format!("PATH={path}"));
    }
    let environment: String = environment
        .iter()
        .map(|pair| format!("Environment={}\n", systemd_word(pair)))
        .collect();
    let log = daemon::log_file(&launch.state_dir)
        .to_string_lossy()
        .replace('%', "%%");
    format!(
        "# {MARKER}; `tendly service uninstall` removes it.\n\
         [Unit]\n\
         Description=Tendly\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         ExecStart={}\n\
         {environment}\
         WorkingDirectory=%h\n\
         StandardOutput=append:{log}\n\
         StandardError=append:{log}\n\
         # Exit 75 after an update, and a crash, both start it again.\n\
         Restart=on-failure\n\
         RestartSec=3\n\
         # Stop the gateway, which stops its sessions; what they deliberately\n\
         # left running stays, as it does for `tendly stop`.\n\
         KillMode=process\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        exec.join(" ")
    )
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn launch_agent(launch: &Launch) -> String {
    let string = |value: &str| format!("<string>{}</string>", xml(value));
    let mut arguments = vec![string(&launch.program.to_string_lossy()), string("serve")];
    arguments.extend(launch.flags.iter().map(|flag| string(flag)));
    let mut environment = vec![(
        "TENDLY_STATE_DIR",
        launch.state_dir.to_string_lossy().into_owned(),
    )];
    if let Some(path) = &launch.path {
        environment.push(("PATH", path.clone()));
    }
    let environment: String = environment
        .iter()
        .map(|(key, value)| format!("    <key>{key}</key>{}\n", string(value)))
        .collect();
    let log = string(&daemon::log_file(&launch.state_dir).to_string_lossy());
    let home = launch
        .home
        .as_deref()
        .map(|home| {
            format!(
                "  <key>WorkingDirectory</key>{}\n",
                string(&home.to_string_lossy())
            )
        })
        .unwrap_or_default();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<!-- {MARKER}; `tendly service uninstall` removes it. -->
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL_TEXT}</string>
  <key>ProgramArguments</key>
  <array>{}</array>
  <key>EnvironmentVariables</key>
  <dict>
{environment}  </dict>
{home}  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key>
  <dict><key>SuccessfulExit</key><false/></dict>
  <key>ThrottleInterval</key><integer>5</integer>
  <key>StandardOutPath</key>{log}
  <key>StandardErrorPath</key>{log}
</dict>
</plist>
"#,
        arguments.join(""),
        LABEL_TEXT = "com.wnzzer.tendly",
    )
}

/// The `Run` value: `tendly start` with the state directory it was installed
/// for, which a fresh sign-in would not otherwise know.
#[cfg_attr(not(windows), allow(dead_code))]
fn run_entry(launch: &Launch) -> String {
    let mut entry = format!(
        "\"{}\" start \"--state-dir={}\"",
        launch.program.to_string_lossy(),
        launch.state_dir.to_string_lossy()
    );
    for flag in &launch.flags {
        entry.push(' ');
        entry.push_str(flag);
    }
    entry
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launch() -> Launch {
        Launch {
            program: PathBuf::from("/opt/node modules/tendly-server"),
            flags: vec!["--lan"],
            state_dir: PathBuf::from("/home/me/.tendly"),
            path: Some("/usr/bin:/home/me/100%/bin:$odd".into()),
            home: Some(PathBuf::from("/home/me")),
        }
    }

    #[test]
    fn the_unit_runs_serve_with_this_binary_its_state_and_its_path() {
        let unit = systemd_unit(&launch());
        assert!(
            unit.starts_with(&format!("# {MARKER}")),
            "told from a hand-written unit"
        );
        assert!(unit.contains("ExecStart=\"/opt/node modules/tendly-server\" serve --lan\n"));
        assert!(unit.contains("Environment=\"TENDLY_STATE_DIR=/home/me/.tendly\"\n"));
        // `%` and `$` would otherwise be specifiers and expansions.
        assert!(unit.contains("Environment=\"PATH=/usr/bin:/home/me/100%%/bin:$$odd\"\n"));
        assert!(unit.contains("StandardOutput=append:/home/me/.tendly/tendly.log\n"));
        // What brings it back after an update's exit 75.
        assert!(unit.contains("Restart=on-failure\n"));
        assert!(unit.contains("WantedBy=default.target\n"));
    }

    #[test]
    fn the_launch_agent_keeps_serve_alive_and_escapes_what_it_quotes() {
        let mut launch = launch();
        launch.path = Some("/usr/bin:/a&b".into());
        let plist = launch_agent(&launch);
        assert!(plist.contains(
            "<array><string>/opt/node modules/tendly-server</string><string>serve</string><string>--lan</string></array>"
        ));
        assert!(plist.contains("<key>PATH</key><string>/usr/bin:/a&amp;b</string>"));
        assert!(plist.contains("<dict><key>SuccessfulExit</key><false/></dict>"));
        assert!(plist.contains("<key>Label</key><string>com.wnzzer.tendly</string>"));
    }

    #[test]
    fn the_run_entry_starts_the_background_gateway_for_this_state() {
        assert_eq!(
            run_entry(&launch()),
            "\"/opt/node modules/tendly-server\" start \"--state-dir=/home/me/.tendly\" --lan"
        );
    }
}
