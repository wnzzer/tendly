//! Process-group ownership that means the same thing on every host.
//!
//! On Unix a session's process is the leader of a group of its own (portable-pty
//! calls `setsid`; plain children are spawned with `process_group(0)`), so a
//! signal to `-pid` reaches it and everything it started. Windows has no process
//! groups to signal. There the same ownership is the process tree rooted at the
//! pid, which `taskkill /T` walks, and there is no polite request a console
//! program is guaranteed to honour, so terminating is killing.
//!
//! Every function refuses pids 0 and 1: neither names a process this server
//! started, and on Unix `-0` and `-1` would address far more than one tree.

use std::{
    io,
    path::{Path, PathBuf},
};

fn owned(pid: u32) -> bool {
    pid > 1 && pid <= i32::MAX as u32
}

/// Ask the process group led by `pid` to stop. Returns whether it was delivered.
pub fn terminate_group(pid: u32) -> bool {
    #[cfg(unix)]
    {
        signal_group(pid, libc::SIGTERM)
    }
    #[cfg(windows)]
    {
        kill_tree(pid)
    }
}

/// Stop the process group led by `pid` without asking. Returns whether it was delivered.
pub fn kill_group(pid: u32) -> bool {
    #[cfg(unix)]
    {
        signal_group(pid, libc::SIGKILL)
    }
    #[cfg(windows)]
    {
        kill_tree(pid)
    }
}

/// Hang up the process group led by `pid`, as closing its terminal would: what
/// an interactive shell, which ignores SIGTERM, answers to.
#[cfg(unix)]
pub fn hangup_group(pid: u32) -> bool {
    signal_group(pid, libc::SIGHUP)
}

#[cfg(unix)]
fn signal_group(pid: u32, signal: libc::c_int) -> bool {
    // SAFETY: a positive pid > 1 addresses only the group this server created.
    owned(pid) && unsafe { libc::kill(-(pid as libc::pid_t), signal) == 0 }
}

#[cfg(windows)]
fn kill_tree(pid: u32) -> bool {
    use std::os::windows::process::CommandExt;
    owned(pid)
        && std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .is_ok_and(|status| status.success())
}

/// Ask a single process (not its group) to stop, as a service manager would.
pub fn terminate(pid: u32) -> io::Result<()> {
    if !owned(pid) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not an owned pid",
        ));
    }
    #[cfg(unix)]
    {
        // SAFETY: a signal to one positive pid.
        if unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        if kill_tree(pid) {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "taskkill could not stop pid {pid}"
            )))
        }
    }
}

/// Whether a process with this pid exists. Pids are reused, so this is evidence
/// rather than proof of identity.
pub fn alive(pid: u32) -> bool {
    if !owned(pid) {
        return false;
    }
    #[cfg(unix)]
    {
        // SAFETY: signal 0 performs the permission and existence checks only.
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, STILL_ACTIVE},
            System::Threading::{
                GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        };
        // SAFETY: the handle is checked before use and closed exactly once.
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return false;
            }
            let mut code = 0u32;
            let running = GetExitCodeProcess(handle, &mut code) != 0 && code == STILL_ACTIVE as u32;
            CloseHandle(handle);
            running
        }
    }
}

#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
pub const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

/// Give a child a process group of its own, so [`terminate_group`] and
/// [`kill_group`] reach it and its descendants and nothing else. On Windows the
/// tree is found by parentage instead; the child only needs not to open a
/// console window of its own on a server nobody is looking at.
pub fn own_group(command: &mut std::process::Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

/// A process group this server started with [`own_group`]. Dropping it stops
/// whatever is left in the group, so an aborted task or an early return cannot
/// leave a bridge or the native client beneath it running.
pub struct OwnedGroup(Option<u32>);

impl OwnedGroup {
    pub fn new(pid: Option<u32>) -> Self {
        Self(pid.filter(|pid| owned(*pid)))
    }
    pub fn pid(&self) -> Option<u32> {
        self.0
    }
    /// Ask the group to stop. Returns whether there was a group to ask.
    pub fn terminate(&self) -> bool {
        self.0.is_some_and(terminate_group)
    }
    /// Stop the group without asking. Returns whether there was a group.
    pub fn kill(&self) -> bool {
        self.0.is_some_and(kill_group)
    }
    /// Kill what remains of the group and stop owning it.
    pub fn finish(&mut self) {
        if let Some(pid) = self.0.take() {
            kill_group(pid);
        }
    }
    /// Stop owning the group without signalling it, once it is known to be gone.
    pub fn release(&mut self) {
        self.0 = None;
    }
}

impl Drop for OwnedGroup {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Stop a detached child from inheriting this process's own standard handles.
///
/// Windows hands every inheritable handle to a child, not only the three it is
/// given, so a gateway started from `tendly start | tee log` held that pipe
/// open for its whole life and the pipeline never ended. The child's stdio is
/// always set explicitly, so the parent's copies are never what it should use.
pub fn keep_standard_handles_private() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::{
            Foundation::{HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation},
            System::Console::{
                GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
            },
        };
        for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            // SAFETY: only the inherit flag of this process's own handles changes.
            unsafe {
                let handle = GetStdHandle(which);
                if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
                    SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0);
                }
            }
        }
    }
}

/// The program and arguments that run `program args` without a shell.
///
/// On Unix this is the identity. On Windows npm installs `claude` and `codex` as
/// `.cmd` shims, which only `cmd.exe` can run, and `cmd.exe` reparses its whole
/// command line: a session title passed as `--name "a & b"` would run `b`.
/// Instead the shim is read for the script it would start and that script is
/// run with Node directly. A batch file that is not an npm shim is refused
/// rather than handed to `cmd.exe`.
pub fn launcher(program: &str, args: &[String]) -> io::Result<(String, Vec<String>)> {
    launch_with(program, args, None)
}

/// [`launcher`], running a script shim under `node` when it is a `.js` file or
/// an npm shim for one, instead of `TENDLY_NODE_BIN`: for a caller that
/// already chose the JavaScript runtime its own scripts use.
pub fn launcher_with_node(
    program: &str,
    args: &[String],
    node: &str,
) -> io::Result<(String, Vec<String>)> {
    launch_with(program, args, Some(node))
}

#[cfg_attr(not(windows), allow(unused_variables))]
fn launch_with(
    program: &str,
    args: &[String],
    node: Option<&str>,
) -> io::Result<(String, Vec<String>)> {
    #[cfg(not(windows))]
    {
        Ok((program.to_owned(), args.to_vec()))
    }
    #[cfg(windows)]
    {
        let path = find_program(program).unwrap_or_else(|| PathBuf::from(program));
        let extension = extension_of(&path);
        let prefixed = |runner: String, script: &Path| {
            let mut all = vec![script.to_string_lossy().into_owned()];
            all.extend(args.iter().cloned());
            (runner, all)
        };
        match extension.as_str() {
            "js" | "mjs" | "cjs" => Ok(prefixed(
                node.map_or_else(|| node_program(None), str::to_owned),
                &path,
            )),
            "cmd" | "bat" => {
                let target = shim_target(&path).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::Unsupported,
                        format!(
                            "{} is a batch file that is not an npm shim; point the client override at its .exe or .js instead",
                            path.display()
                        ),
                    )
                })?;
                if matches!(extension_of(&target).as_str(), "js" | "mjs" | "cjs") {
                    // Node beside the shim still comes first, as the shim prefers.
                    let runner = match node {
                        Some(node)
                            if !path.parent().is_some_and(|d| d.join("node.exe").is_file()) =>
                        {
                            node.to_owned()
                        }
                        _ => node_program(path.parent()),
                    };
                    Ok(prefixed(runner, &target))
                } else {
                    Ok((target.to_string_lossy().into_owned(), args.to_vec()))
                }
            }
            _ => Ok((program.to_owned(), args.to_vec())),
        }
    }
}

#[cfg(windows)]
fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// Node beside the shim when npm put one there, as the shim itself prefers.
#[cfg(windows)]
fn node_program(shim_directory: Option<&Path>) -> String {
    if let Some(local) = shim_directory.map(|directory| directory.join("node.exe"))
        && local.is_file()
    {
        return local.to_string_lossy().into_owned();
    }
    std::env::var("TENDLY_NODE_BIN")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "node".into())
}

/// The script an npm (or pnpm) `.cmd` shim starts. Both write it as a quoted
/// path relative to the shim's own directory, `"%dp0%\…"` or `"%~dp0\…"`; the
/// last such path that is not Node itself is the target.
#[cfg(windows)]
fn shim_target(shim: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(shim).ok()?;
    if text.len() > 16 * 1024 {
        return None;
    }
    let directory = shim.parent()?;
    let mut found = None;
    for quoted in text.split('"').skip(1).step_by(2) {
        let Some(rest) = quoted
            .strip_prefix("%dp0%")
            .or_else(|| quoted.strip_prefix("%~dp0"))
        else {
            continue;
        };
        let relative = rest.trim_start_matches(['\\', '/']);
        if relative.is_empty() || relative.eq_ignore_ascii_case("node.exe") {
            continue;
        }
        let candidate = directory.join(relative);
        if candidate.is_file() {
            found = Some(candidate);
        }
    }
    found
}

/// Resolve a bare command against `PATH` the way Windows would, trying each
/// `PATHEXT` extension. A path with a directory in it is taken as given.
pub fn find_program(program: &str) -> Option<PathBuf> {
    let given = Path::new(program);
    if given.components().count() > 1 || given.is_absolute() {
        return given.is_file().then(|| given.to_path_buf());
    }
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths).find_map(|directory| executable_in(&directory, program))
}

/// `command` in `directory`, as an executable file. On Windows a bare name also
/// matches with each `PATHEXT` extension, which is how `claude` finds
/// `claude.cmd` or `claude.exe`.
pub fn executable_in(directory: &Path, command: &str) -> Option<PathBuf> {
    let exact = directory.join(command);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(&exact)
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
            .then_some(exact)
    }
    #[cfg(windows)]
    {
        if Path::new(command).extension().is_some() && exact.is_file() {
            return Some(exact);
        }
        let extensions = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        extensions
            .split(';')
            .filter(|extension| !extension.is_empty())
            .map(|extension| directory.join(format!("{command}{}", extension.to_ascii_lowercase())))
            .find(|candidate| candidate.is_file())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pids_that_address_more_than_one_tree_are_never_signalled() {
        for pid in [0, 1, u32::MAX] {
            assert!(!alive(pid));
            assert!(!terminate_group(pid));
            assert!(!kill_group(pid));
        }
    }

    #[test]
    fn an_owned_group_never_holds_a_pid_that_addresses_more_than_one_tree() {
        for pid in [None, Some(0), Some(1), Some(u32::MAX)] {
            assert_eq!(OwnedGroup::new(pid).pid(), None, "{pid:?}");
        }
        let mut group = OwnedGroup::new(Some(4242));
        assert_eq!(group.pid(), Some(4242));
        // Released: dropping it afterwards signals nothing.
        group.release();
        assert_eq!(group.pid(), None);
    }

    #[test]
    fn this_process_is_alive() {
        assert!(alive(std::process::id()));
    }

    // `.cmd` shims, and the `\` separators they are written with, exist only on Windows.
    #[cfg(windows)]
    #[test]
    fn npm_and_pnpm_shims_name_their_script() {
        let directory = std::env::temp_dir().join(format!("tendly-shim-{}", std::process::id()));
        let package = directory.join("node_modules/@scope/tool");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(package.join("cli.js"), "").unwrap();
        let npm = directory.join("npm.cmd");
        std::fs::write(
            &npm,
            "@ECHO off\r\nIF EXIST \"%dp0%\\node.exe\" (\r\n  SET \"_prog=%dp0%\\node.exe\"\r\n)\r\nendLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & \"%_prog%\"  \"%dp0%\\node_modules\\@scope\\tool\\cli.js\" %*\r\n",
        )
        .unwrap();
        let pnpm = directory.join("pnpm.cmd");
        std::fs::write(
            &pnpm,
            "@IF EXIST \"%~dp0\\node.exe\" (\r\n  \"%~dp0\\node.exe\"  \"%~dp0\\node_modules\\@scope\\tool\\cli.js\" %*\r\n) ELSE (\r\n  node  \"%~dp0\\node_modules\\@scope\\tool\\cli.js\" %*\r\n)\r\n",
        )
        .unwrap();
        let other = directory.join("other.cmd");
        std::fs::write(&other, "@echo %*\r\n").unwrap();
        let expected = directory.join("node_modules\\@scope\\tool\\cli.js");
        assert_eq!(shim_target(&npm).unwrap(), expected);
        assert_eq!(shim_target(&pnpm).unwrap(), expected);
        assert_eq!(shim_target(&other), None);
        std::fs::remove_dir_all(&directory).ok();
    }

    #[cfg(windows)]
    #[test]
    fn a_shim_runs_its_script_with_node_and_keeps_every_argument_verbatim() {
        let directory = std::env::temp_dir().join(format!("tendly-launch-{}", std::process::id()));
        std::fs::create_dir_all(directory.join("lib")).unwrap();
        std::fs::write(directory.join("lib/cli.js"), "").unwrap();
        let shim = directory.join("tool.cmd");
        std::fs::write(&shim, "\"%_prog%\" \"%dp0%\\lib\\cli.js\" %*\r\n").unwrap();
        let args = vec!["--name".to_owned(), "a & calc".to_owned()];
        let (program, launched) = launcher(shim.to_str().unwrap(), &args).unwrap();
        assert!(program.ends_with("node") || program.ends_with("node.exe"));
        assert_eq!(launched[0], directory.join("lib\\cli.js").to_string_lossy());
        assert_eq!(&launched[1..], &args[..]);
        // A batch file that is not a shim is refused, never given to cmd.exe.
        let plain = directory.join("plain.bat");
        std::fs::write(&plain, "@echo %*\r\n").unwrap();
        assert!(launcher(plain.to_str().unwrap(), &args).is_err());
        std::fs::remove_dir_all(&directory).ok();
    }
}
