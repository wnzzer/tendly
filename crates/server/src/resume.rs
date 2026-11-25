//! `tendly resume <session>`: continue a Tendly session in this terminal.
//!
//! The running server owns the database, the configuration homes and the keys,
//! so this asks it for the session's launch (`launch-plan`, built by the same
//! launch layer as Tendly's own terminals) and then becomes that client
//! here: same program, arguments, checkout and environment, and so the same
//! endpoint, model and native conversation. Nothing is written into the
//! command line or the clipboard but the Tendly session ID.
use serde::Deserialize;
use std::{collections::BTreeMap, path::PathBuf, process::Command};

#[derive(Deserialize)]
struct Plan {
    title: String,
    provider: String,
    program: String,
    args: Vec<String>,
    cwd: PathBuf,
    env: BTreeMap<String, String>,
    env_remove: Vec<String>,
}

const USAGE: &str = "Usage: tendly resume <Tendly session ID> [--force]\n\nThe ID is the Tendly session ID (Copy resume command in a session's menu),\nnot the client's own session ID. --force opens it even while Tendly runs it.";

pub async fn command(arguments: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    let force = arguments.iter().any(|argument| argument == "--force");
    let ids: Vec<&String> = arguments
        .iter()
        .filter(|argument| !argument.starts_with('-'))
        .collect();
    if arguments
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        println!("{USAGE}");
        return Ok(());
    }
    let [id] = ids.as_slice() else {
        return Err(USAGE.into());
    };
    if uuid::Uuid::parse_str(id).is_err() {
        return Err(format!("{id:?} is not a Tendly session ID.\n\n{USAGE}").into());
    }
    let plan = fetch(id, force).await?;
    eprintln!(
        "Resuming \u{201c}{}\u{201d} with {} in {}",
        plan.title,
        client_label(&plan.provider),
        plan.cwd.display()
    );
    run(plan)
}

async fn fetch(id: &str, force: bool) -> Result<Plan, Box<dyn std::error::Error>> {
    let url = crate::mcp::local_url();
    let mut request = reqwest::Client::new()
        .post(format!(
            "{url}/api/sessions/{id}/launch-plan{}",
            if force { "?force=true" } else { "" }
        ))
        .header("x-tendly-client", "cli");
    if let Some(token) = crate::security::existing_token() {
        request = request.bearer_auth(token);
    }
    let response = request
        .send()
        .await
        .map_err(|_| format!("Tendly is not answering at {url}. Start it with `tendly`."))?;
    let status = response.status();
    if !status.is_success() {
        let body: serde_json::Value = response.json().await.unwrap_or_default();
        return Err(body["error"]
            .as_str()
            .map_or_else(|| format!("Tendly answered {status}"), str::to_owned)
            .into());
    }
    Ok(response.json().await?)
}

fn client_label(provider: &str) -> &str {
    match provider {
        "claude_code" => "Claude Code",
        "codex" => "Codex",
        "pi" => "Pi",
        "gemini" => "Gemini",
        "terminal" => "a shell",
        other => other,
    }
}

/// Become the client: on Unix this process is replaced, so the terminal's
/// signals and exit status are the client's own; elsewhere it waits for it.
fn run(plan: Plan) -> Result<(), Box<dyn std::error::Error>> {
    let mut command = Command::new(&plan.program);
    command.args(&plan.args).current_dir(&plan.cwd);
    for key in &plan.env_remove {
        command.env_remove(key);
    }
    command.envs(&plan.env);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = command.exec();
        Err(format!("Could not start {}: {error}", plan.program).into())
    }
    #[cfg(not(unix))]
    {
        let status = command
            .status()
            .map_err(|error| format!("Could not start {}: {error}", plan.program))?;
        std::process::exit(status.code().unwrap_or(1));
    }
}
