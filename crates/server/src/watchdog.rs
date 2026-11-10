//! Keeping the background gateway up on Windows.
//!
//! Linux and macOS hand `serve` to systemd or launchd, which start it again
//! after a crash or an update's exit. Windows has no per-user service manager
//! that does that without an administrator, and the `Run` entry it does have
//! starts a program once at sign-in and forgets it. So `tendly service install`
//! there runs this instead: a small detached process of the same binary that
//! looks every few seconds and starts the background gateway again when it is
//! gone.
//!
//! It only starts what is not there. A gateway someone stopped on purpose
//! leaves a hold file (`tendly stop` writes it, `tendly start` removes it), so
//! stopping is not undone a few seconds later; and a port that already
//! answers -- a `tendly serve` in a terminal -- is left alone. A gateway that
//! dies as soon as it starts is retried less and less often, so a broken
//! configuration fills the log slowly rather than spinning.
#![cfg_attr(not(windows), allow(dead_code))]

use crate::daemon;
use std::{
    env, fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

type Outcome = Result<(), Box<dyn std::error::Error>>;

/// How often it looks.
const CHECK: Duration = Duration::from_secs(3);
/// The first wait between starts that did not stay up, and the longest.
const FIRST_BACKOFF: Duration = Duration::from_secs(5);
const LONGEST_BACKOFF: Duration = Duration::from_secs(300);
/// Consecutive looks at a gateway that is up before it counts as healthy
/// again, and the backoff starts over.
const STEADY_CHECKS: u32 = 5;

pub fn pid_file(state_dir: &Path) -> PathBuf {
    state_dir.join("tendly-watchdog.pid")
}

/// Present while the gateway is stopped on purpose.
pub fn hold_file(state_dir: &Path) -> PathBuf {
    state_dir.join("tendly.stopped")
}

pub fn running(state_dir: &Path) -> Option<i32> {
    daemon::alive_in(&pid_file(state_dir))
}

/// Keep the watchdog from starting the gateway again: `tendly stop`.
pub fn hold(state_dir: &Path) {
    let _ = fs::write(hold_file(state_dir), "");
}

/// Let it start the gateway again: `tendly start`.
pub fn release(state_dir: &Path) {
    let _ = fs::remove_file(hold_file(state_dir));
}

/// Start a detached watchdog for this state directory, unless one runs.
/// It starts the gateway itself straight away.
pub fn start(state_dir: &Path, address: SocketAddr) -> Result<i32, Box<dyn std::error::Error>> {
    if let Some(pid) = running(state_dir) {
        return Ok(pid);
    }
    release(state_dir);
    let pid = daemon::spawn_detached(state_dir, &env::current_exe()?, "watchdog", address)?;
    // It writes this itself as well; written here so a look straight after
    // finds it.
    fs::write(pid_file(state_dir), pid.to_string())?;
    Ok(pid)
}

/// Stop the watchdog. Only the watchdog: the gateway it started is a process
/// of its own, stopped by `daemon::stop`.
pub fn stop(state_dir: &Path) {
    if let Some(pid) = running(state_dir) {
        let _ = tendly_runtime::process::terminate_one(pid as u32);
    }
    let _ = fs::remove_file(pid_file(state_dir));
}

/// `tendly watchdog`: what the Windows sign-in entry runs.
pub async fn run(state_dir: &Path, address: SocketAddr) -> Outcome {
    let own = std::process::id() as i32;
    if let Some(pid) = running(state_dir).filter(|pid| *pid != own) {
        println!("watchdog: already running (pid {pid})");
        return Ok(());
    }
    fs::write(pid_file(state_dir), own.to_string())?;
    // Signing in is asking for Tendly, whatever the last session stopped.
    release(state_dir);
    // The path this started from, which is where an update puts the next
    // version; asked again later, Windows may name where the old one went.
    let program = env::current_exe()?;
    println!("watchdog: keeping Tendly up at {address} (pid {own})");
    let mut guard = Guard::new();
    loop {
        // The gateways it started are its children: one that died stays a
        // zombie, which still answers "alive", until reaped. Windows keeps
        // no such remains.
        #[cfg(unix)]
        // SAFETY: non-blocking waitpid on this process's own children.
        while unsafe { libc::waitpid(-1, std::ptr::null_mut(), libc::WNOHANG) } > 0 {}
        let look = if daemon::running(state_dir).is_some() {
            Look::Up
        } else if daemon::healthy(address).await {
            // Someone else's gateway holds the port.
            Look::Up
        } else if hold_file(state_dir).exists() {
            Look::Held
        } else {
            Look::Down
        };
        if guard.next(look, Instant::now()) == Act::Start {
            // Looked at again: a `tendly stop` may have landed meanwhile.
            if !hold_file(state_dir).exists() {
                match daemon::start_program(state_dir, &program, address) {
                    Ok(pid) => println!("watchdog: Tendly was not running; started it (pid {pid})"),
                    Err(error) => println!("watchdog: could not start Tendly: {error}"),
                }
            }
        }
        tokio::time::sleep(CHECK).await;
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Look {
    Up,
    Down,
    Held,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Act {
    Wait,
    Start,
}

/// When to start the gateway, from what each look saw.
struct Guard {
    first: bool,
    down: u32,
    up: u32,
    backoff: Duration,
    not_before: Option<Instant>,
}

impl Guard {
    fn new() -> Self {
        Guard {
            first: true,
            down: 0,
            up: 0,
            backoff: FIRST_BACKOFF,
            not_before: None,
        }
    }

    fn next(&mut self, look: Look, now: Instant) -> Act {
        let first = std::mem::replace(&mut self.first, false);
        match look {
            Look::Up => {
                self.down = 0;
                self.up += 1;
                if self.up >= STEADY_CHECKS {
                    self.backoff = FIRST_BACKOFF;
                }
                Act::Wait
            }
            Look::Held => {
                self.down = 0;
                self.up = 0;
                Act::Wait
            }
            Look::Down => {
                self.up = 0;
                self.down += 1;
                // Twice in a row, unless it is the first look: a stop and
                // start in progress (`tendly restart`) is a moment of nothing.
                if !first && self.down < 2 {
                    return Act::Wait;
                }
                if self.not_before.is_some_and(|at| now < at) {
                    return Act::Wait;
                }
                self.down = 0;
                self.not_before = Some(now + self.backoff);
                self.backoff = (self.backoff * 2).min(LONGEST_BACKOFF);
                Act::Start
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_starts_at_once_at_sign_in_and_after_two_looks_later() {
        let mut guard = Guard::new();
        let now = Instant::now();
        assert_eq!(guard.next(Look::Down, now), Act::Start, "sign-in");
        assert_eq!(guard.next(Look::Up, now + CHECK), Act::Wait);
        // A crash: one look of nothing could be a restart in progress.
        let later = now + Duration::from_secs(60);
        assert_eq!(guard.next(Look::Down, later), Act::Wait);
        assert_eq!(guard.next(Look::Down, later + CHECK), Act::Start);
    }

    #[test]
    fn a_held_gateway_stays_stopped() {
        let mut guard = Guard::new();
        let now = Instant::now();
        for step in 0..10 {
            assert_eq!(guard.next(Look::Held, now + CHECK * step), Act::Wait);
        }
    }

    #[test]
    fn a_gateway_that_keeps_dying_is_retried_less_often_until_it_stays_up() {
        let mut guard = Guard::new();
        let start = Instant::now();
        let mut starts = Vec::new();
        for step in 0..400u32 {
            let now = start + CHECK * step;
            if guard.next(Look::Down, now) == Act::Start {
                starts.push(now - start);
            }
        }
        let gaps: Vec<_> = starts.windows(2).map(|pair| pair[1] - pair[0]).collect();
        assert!(gaps.windows(2).all(|pair| pair[1] >= pair[0]), "{gaps:?}");
        assert!(
            gaps.iter().all(|gap| *gap <= LONGEST_BACKOFF + CHECK * 2),
            "{gaps:?}"
        );
        assert!(*gaps.last().unwrap() >= LONGEST_BACKOFF);

        // Up long enough, and a crash after that is retried promptly again.
        let now = start + CHECK * 500;
        for step in 0..STEADY_CHECKS {
            guard.next(Look::Up, now + CHECK * step);
        }
        let crash = now + Duration::from_secs(3600);
        assert_eq!(guard.next(Look::Down, crash), Act::Wait);
        assert_eq!(guard.next(Look::Down, crash + CHECK), Act::Start);
        assert_eq!(guard.next(Look::Down, crash + CHECK * 2), Act::Wait);
        assert_eq!(
            guard.next(Look::Down, crash + CHECK * 3),
            Act::Start,
            "after {FIRST_BACKOFF:?}"
        );
    }
}
