//! One file for how this gateway is run.
//!
//! Every setting was an environment variable, which is fine for one flag and
//! poor for a deployment: `--lan`, the port, the token and its floor had to be
//! remembered together and set together, in a shell profile or a unit file,
//! every time. They belong in one place that can be read and edited.
//!
//! The file seeds the environment rather than replacing it, so nothing
//! downstream has to know it exists and the precedence falls out on its own:
//! a variable already set wins, because whoever set it did so later and more
//! deliberately than whoever wrote the file.
//!
//! The state directory itself cannot come from here — this file lives inside
//! it — so `TENDLY_HOME` stays an environment variable.
use std::{collections::BTreeMap, path::Path};

pub const FILE: &str = "config.toml";
const DEFAULT_PORT: u16 = 28789;

/// Read the file beside the state, and hand back what it asks the environment
/// to say. Nothing is read from the environment here and nothing is written to
/// it: the caller decides, while it is still the only thread.
pub fn pairs(state_dir: &Path, cli_lan: bool) -> Result<Vec<(String, String)>, String> {
    let path = state_dir.join(FILE);
    let Ok(document) = std::fs::read_to_string(&path) else {
        return Ok(Vec::new());
    };
    parse(&document, cli_lan).map_err(|error| format!("{}: {error}", path.display()))
}

fn parse(document: &str, cli_lan: bool) -> Result<Vec<(String, String)>, String> {
    let table: BTreeMap<String, toml::Value> =
        toml::from_str(document).map_err(|error| error.message().to_owned())?;
    let mut pairs = Vec::new();
    let mut lan = cli_lan;
    let mut port: Option<u16> = None;
    let mut addr: Option<String> = None;
    for (key, value) in &table {
        match key.as_str() {
            // These three describe one thing between them, and are resolved
            // together below rather than each mapping to a variable.
            "lan" => {
                lan |= value
                    .as_bool()
                    .ok_or_else(|| "lan must be true or false".to_owned())?;
            }
            "port" => {
                port = Some(
                    value
                        .as_integer()
                        .filter(|n| (1..=65535).contains(n))
                        .ok_or_else(|| "port must be a number from 1 to 65535".to_owned())?
                        as u16,
                );
            }
            "addr" | "address" => addr = Some(text(key, value)?),
            _ => {
                let name = variable(key);
                let value = if name.ends_with("_ROOTS") {
                    paths(key, value)?
                } else {
                    text(key, value)?
                };
                pairs.push((name, value));
            }
        }
    }
    if let Some(resolved) = address(lan, port, addr.as_deref()) {
        pairs.push(("TENDLY_ADDR".to_owned(), resolved));
    }
    Ok(pairs)
}

/// What the three network keys mean together.
///
/// `--lan` on the command line outranks the file, but only about the host: a
/// port written in the file is still the port that was meant, so the flag
/// raises the binding to every interface and leaves the rest alone.
fn address(lan: bool, port: Option<u16>, addr: Option<&str>) -> Option<String> {
    let written = addr.and_then(|addr| addr.rsplit(':').next()?.parse::<u16>().ok());
    match (lan, addr) {
        (false, Some(addr)) => Some(addr.to_owned()),
        (true, _) => Some(format!(
            "0.0.0.0:{}",
            written.or(port).unwrap_or(DEFAULT_PORT)
        )),
        (false, None) => port.map(|port| format!("127.0.0.1:{port}")),
    }
}

/// A key is the name of the variable without the prefix everything here shares.
/// Writing the full name works too, for anyone copying from a unit file.
fn variable(key: &str) -> String {
    let upper = key.to_uppercase().replace('-', "_");
    if upper.starts_with("TENDLY_") {
        upper
    } else {
        format!("TENDLY_{upper}")
    }
}

/// TOML has types and the environment has strings. A list becomes the
/// comma-separated form the variables already accept.
fn text(key: &str, value: &toml::Value) -> Result<String, String> {
    Ok(match value {
        toml::Value::String(value) => value.clone(),
        toml::Value::Integer(value) => value.to_string(),
        toml::Value::Float(value) => value.to_string(),
        toml::Value::Boolean(value) => if *value { "1" } else { "0" }.to_owned(),
        toml::Value::Array(items) => items
            .iter()
            .map(|item| text(key, item))
            .collect::<Result<Vec<_>, _>>()?
            .join(","),
        _ => return Err(format!("{key} must be a value, not a table")),
    })
}

/// A list of folders is read the way the system reads `PATH` — `:` between
/// them here, `;` on Windows — not with commas, which would hand over one
/// folder that does not exist and stop the gateway from starting.
fn paths(key: &str, value: &toml::Value) -> Result<String, String> {
    let toml::Value::Array(items) = value else {
        return text(key, value);
    };
    let folders = items
        .iter()
        .map(|item| text(key, item))
        .collect::<Result<Vec<_>, _>>()?;
    std::env::join_paths(folders)
        .map_err(|_| format!("{key}: a folder's name contains the list separator"))?
        .into_string()
        .map_err(|_| format!("{key} must be text"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(document: &str, cli_lan: bool) -> BTreeMap<String, String> {
        parse(document, cli_lan).unwrap().into_iter().collect()
    }

    #[test]
    fn a_deployment_is_described_once_instead_of_remembered_as_four_variables() {
        let settings = read(
            r#"
            lan = true
            port = 9000
            token = "abc123"
            max_sessions = 6
            instance_label = "novel box"
            allowed_origins = ["http://one.local", "http://two.local"]
            "#,
            false,
        );
        assert_eq!(settings["TENDLY_ADDR"], "0.0.0.0:9000");
        assert_eq!(settings["TENDLY_TOKEN"], "abc123");
        // Numbers and lists are written as themselves here and handed over in
        // the form the variables have always taken.
        assert_eq!(settings["TENDLY_MAX_SESSIONS"], "6");
        assert_eq!(settings["TENDLY_INSTANCE_LABEL"], "novel box");
        assert_eq!(
            settings["TENDLY_ALLOWED_ORIGINS"],
            "http://one.local,http://two.local"
        );
    }

    #[test]
    fn the_three_network_keys_describe_one_binding_between_them() {
        // A port alone stays on this machine. Nothing else reaches it.
        assert_eq!(read("port = 9000", false)["TENDLY_ADDR"], "127.0.0.1:9000");
        assert_eq!(read("lan = true", false)["TENDLY_ADDR"], "0.0.0.0:28789");
        // An address is taken as written, since writing one is saying exactly
        // what was meant.
        assert_eq!(
            read("addr = \"192.168.0.9:8080\"", false)["TENDLY_ADDR"],
            "192.168.0.9:8080"
        );
        // `--lan` outranks the file about the host and leaves the port alone:
        // the port in the file is still the port that was meant.
        assert_eq!(read("port = 9000", true)["TENDLY_ADDR"], "0.0.0.0:9000");
        assert_eq!(
            read("addr = \"127.0.0.1:8080\"", true)["TENDLY_ADDR"],
            "0.0.0.0:8080"
        );
        assert_eq!(read("", true)["TENDLY_ADDR"], "0.0.0.0:28789");
        // Saying nothing asks for nothing: the defaults downstream still apply.
        assert!(!read("token = \"x\"", false).contains_key("TENDLY_ADDR"));
    }

    #[test]
    fn a_key_names_a_variable_in_this_gateways_namespace_and_no_other() {
        let settings = read(
            r#"
            shell = "/bin/bash"
            claude-bin = "/usr/local/bin/claude"
            TENDLY_CODEX_BIN = "/usr/local/bin/codex"
            path = "not the one you are thinking of"
            "#,
            false,
        );
        assert_eq!(settings["TENDLY_SHELL"], "/bin/bash");
        // Dashes read better in a config file than underscores do.
        assert_eq!(settings["TENDLY_CLAUDE_BIN"], "/usr/local/bin/claude");
        // A full name is accepted as written, for anyone copying a unit file.
        assert_eq!(settings["TENDLY_CODEX_BIN"], "/usr/local/bin/codex");
        // Everything lands under the prefix, so this file can never reach a
        // variable that is not this gateway's to set.
        assert_eq!(settings["TENDLY_PATH"], "not the one you are thinking of");
        assert!(!settings.contains_key("PATH"));
    }

    #[test]
    fn a_list_of_folders_is_joined_the_way_the_system_splits_it() {
        let (first, second) = if cfg!(windows) {
            (r"E:\code", r"C:\Users\me")
        } else {
            ("/srv/code", "/home/me")
        };
        let settings = read(
            &format!("workspace-roots = ['{first}', '{second}']\nbrowse_roots = '{first}'"),
            false,
        );
        let folders: Vec<_> = std::env::split_paths(&settings["TENDLY_WORKSPACE_ROOTS"]).collect();
        assert_eq!(
            folders,
            [
                std::path::PathBuf::from(first),
                std::path::PathBuf::from(second)
            ]
        );
        // One folder written as text is that folder, as it always was.
        assert_eq!(settings["TENDLY_BROWSE_ROOTS"], first);
        // Other lists keep the commas their variables expect.
        assert_eq!(
            read("allowed-origins = ['http://a', 'http://b']", false)["TENDLY_ALLOWED_ORIGINS"],
            "http://a,http://b"
        );
    }

    #[test]
    fn a_file_that_cannot_be_meant_is_reported_rather_than_half_applied() {
        assert!(parse("lan = \"yes\"", false).is_err());
        assert!(parse("port = 70000", false).is_err());
        assert!(parse("token = { value = \"x\" }", false).is_err());
        assert!(parse("this is not toml", false).is_err());
        // An absent file is not an error; it is the ordinary case.
        assert!(parse("", false).unwrap().is_empty());
    }
}
