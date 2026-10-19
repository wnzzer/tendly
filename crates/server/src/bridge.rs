//! Starting the Node bridges, and what no child of this server may inherit.
//!
//! Every bridge script (chat, accounts, history, file search) runs the same
//! way: under the configured JavaScript runtime, in a process group of its own,
//! with piped stdio, and without Tendly's own secrets in its environment.
//! Keeping that in one place is what keeps the four from drifting apart, and
//! [`is_tendly_secret`] is the one definition every launch path filters by.
use std::{
    env,
    ffi::{OsStr, OsString},
    path::Path,
    process::Stdio,
};
use tendly_runtime::process;

/// A variable that holds Tendly's own credentials: endpoint secrets
/// referenced as `env:TENDLY_SECRET_*`, and the gateway's access token.
/// These never reach a bridge, a native client or a terminal.
pub fn is_tendly_secret(key: &str) -> bool {
    key.starts_with("TENDLY_SECRET_") || key == "TENDLY_TOKEN"
}

/// The names of Tendly's secrets present in this process's environment.
pub fn tendly_secrets() -> impl Iterator<Item = String> {
    env::vars_os()
        .filter_map(|(key, _)| key.into_string().ok())
        .filter(|key| is_tendly_secret(key))
}

/// What a Claude Code process sets for the commands it runs, present when
/// Tendly itself was started from one. A client that inherits them takes
/// itself for part of that outer session: Claude Code then saves no
/// transcript, so neither its history nor its usage is found afterwards.
const HOST_CLIENT_SESSION: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_PID",
];

/// The names of [`HOST_CLIENT_SESSION`] present in this process's environment.
pub fn host_client_session() -> impl Iterator<Item = String> {
    HOST_CLIENT_SESSION
        .iter()
        .filter(|key| env::var_os(key).is_some())
        .map(|key| (*key).to_owned())
}

/// The JavaScript runtime the bridges run under: `TENDLY_JS_RUNTIME`, then
/// the legacy `TENDLY_NODE_BIN`, then `node`. An empty value is unset.
pub fn js_runtime() -> OsString {
    choose_runtime(
        env::var_os("TENDLY_JS_RUNTIME"),
        env::var_os("TENDLY_NODE_BIN"),
    )
}

fn choose_runtime(primary: Option<OsString>, legacy: Option<OsString>) -> OsString {
    primary
        .filter(|value| !value.is_empty())
        .or_else(|| legacy.filter(|value| !value.is_empty()))
        .unwrap_or_else(|| "node".into())
}

/// A command that runs `script` under [`js_runtime`], with `node_options`
/// before the script. Its stdin and stdout are piped and stderr discarded, it
/// is killed if dropped, it leads a process group of its own (so the caller
/// can stop it and everything it starts with an [`process::OwnedGroup`]), and
/// Tendly's secrets are removed from its environment.
pub fn node_command(node_options: &[&str], script: impl AsRef<OsStr>) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(js_runtime());
    command
        .args(node_options)
        .arg(script.as_ref())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    for key in tendly_secrets() {
        command.env_remove(key);
    }
    process::own_group(command.as_std_mut());
    command
}

/// The bridge script `name` beside the installed native bridge, unless the
/// environment variable `override_key` names another.
pub fn script(state_dir: &Path, override_key: &str, name: &str) -> std::path::PathBuf {
    env::var_os(override_key)
        .map(Into::into)
        .unwrap_or_else(|| crate::installation::native_bridge(state_dir).with_file_name(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_runtime_defaults_to_node_and_respects_both_overrides() {
        assert_eq!(choose_runtime(None, None), OsString::from("node"));
        assert_eq!(
            choose_runtime(Some("/opt/runtime/bun".into()), Some("node".into())),
            OsString::from("/opt/runtime/bun")
        );
        assert_eq!(
            choose_runtime(None, Some("/legacy/node".into())),
            OsString::from("/legacy/node")
        );
        assert_eq!(
            choose_runtime(Some(OsString::new()), Some("legacy-node".into())),
            OsString::from("legacy-node")
        );
        assert_eq!(
            choose_runtime(Some(OsString::new()), Some(OsString::new())),
            OsString::from("node")
        );
    }

    #[test]
    fn only_tendly_credentials_count_as_its_secrets() {
        for key in ["TENDLY_SECRET_WORK", "TENDLY_SECRET_", "TENDLY_TOKEN"] {
            assert!(is_tendly_secret(key), "{key}");
        }
        // Configuration is not a secret, and neither is a lookalike.
        for key in [
            "TENDLY_HOME",
            "TENDLY_TOKEN_FILE",
            "MY_TENDLY_TOKEN",
            "OPENAI_API_KEY",
        ] {
            assert!(!is_tendly_secret(key), "{key}");
        }
    }
}
