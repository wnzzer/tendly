//! Codex: `codex`, configured through `CODEX_HOME`, a generated `config.toml`
//! and `-c` overrides.
use super::{AccountOperations, CatalogStep, ClientAdapter, EndpointLaunch, encode};
use crate::ApiError;
use crate::usage::{
    Call, FileState, LoggedLimits, Outcome, Source, Tokens, jsonl_files, number, timestamp,
};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tendly_domain::ProviderKind;

pub struct Codex;

impl ClientAdapter for Codex {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Codex
    }
    fn npm_package(&self) -> &'static str {
        "@openai/codex"
    }
    fn command(&self) -> &'static str {
        "codex"
    }
    fn bin_override(&self) -> &'static str {
        "TENDLY_CODEX_BIN"
    }
    fn config_key(&self) -> &'static str {
        "CODEX_HOME"
    }
    fn home_dir(&self) -> &'static str {
        ".codex"
    }
    fn history_source(&self) -> (&'static str, &'static str) {
        ("codex-default", "TENDLY_CODEX_HISTORY_DIR")
    }
    // Listed first, as it always has been.
    fn source_order(&self) -> u8 {
        0
    }
    fn protected_names(&self) -> &'static [&'static str] {
        &[".codex"]
    }
    fn inherited_keys(&self) -> &'static [&'static str] {
        &["OPENAI_API_KEY", "OPENAI_BASE_URL", "CODEX_API_KEY"]
    }
    fn account_prefixes(&self) -> &'static [&'static str] {
        &["OPENAI_", "CODEX_", "AZURE_OPENAI_"]
    }
    fn resume_args(&self, native_id: &str) -> Vec<String> {
        vec!["resume".into(), native_id.to_owned()]
    }
    fn effort_args(&self, effort: &str) -> Vec<String> {
        vec![
            "-c".into(),
            format!("model_reasoning_effort={}", encode(effort)),
        ]
    }
    // Plan mode is Claude Code's; Codex keeps its native approval model.
    fn plan_mode(&self) -> bool {
        false
    }
    // Codex approves per command, so only the two ends of the range translate.
    fn permission_modes(&self) -> &'static [&'static str] {
        &["ask", "danger"]
    }
    fn configure(&self, launch: EndpointLaunch<'_>) -> Result<(), ApiError> {
        let EndpointLaunch {
            state,
            profile,
            mode,
            secret,
            home,
            args,
            environment,
        } = launch;
        let mut config = "cli_auth_credentials_store = \"file\"\n".to_owned();
        if mode == "interactive" || mode == "trusted" {
            // Same sandbox for both intents; no bypass or fictitious mapping.
            args.extend([
                "-c".into(),
                "approval_policy=\"on-request\"".into(),
                "-c".into(),
                "sandbox_mode=\"workspace-write\"".into(),
            ]);
        }
        if let Some(effort) = profile.and_then(|p| p.effort.as_deref()) {
            args.extend(self.effort_args(effort));
        }
        // Only a window the user set (model_limits.rs): a published one may
        // exceed what the account has.
        if let Some(window) = profile
            .and_then(crate::providers::resolve_model)
            .and_then(|model| crate::model_limits::context_override(&state.state_dir, &model))
        {
            args.extend(["-c".into(), format!("model_context_window={window}")]);
        }
        if profile.is_some_and(|p| p.endpoint_url.is_some() || p.secret_ref.is_some()) {
            config.push_str("model_provider = \"tendly\"\n\n[model_providers.tendly]\nname = \"Tendly\"\nwire_api = \"responses\"\nrequires_openai_auth = false\n");
            let base = profile
                .and_then(|p| p.endpoint_url.as_deref())
                .unwrap_or("https://api.openai.com/v1");
            config.push_str(&format!("base_url = {}\n", encode(base)));
            if let Some(secret) = secret {
                config.push_str("env_key = \"OPENAI_API_KEY\"\n");
                environment.insert("OPENAI_API_KEY".into(), secret);
            }
        }
        // Immutable generated config. CLI auth/history/settings remain its own.
        crate::providers::write_private(&home.join("config.toml"), &config)
    }

    fn default_endpoint(&self) -> &'static str {
        "https://api.openai.com/v1"
    }

    // The same server as `-c` overrides, first, so they are global options
    // before any subcommand.
    fn equip_tools(
        &self,
        _state: &crate::AppState,
        _session: tendly_domain::SessionId,
        program: &str,
        identity: &[(String, String)],
        args: &mut Vec<String>,
    ) -> Result<(), ApiError> {
        let table = identity
            .iter()
            .map(|(key, value)| format!("{key}={}", encode(value)))
            .collect::<Vec<_>>()
            .join(",");
        let overrides = [
            format!("mcp_servers.tendly.command={}", encode(program)),
            r#"mcp_servers.tendly.args=["mcp"]"#.to_owned(),
            format!("mcp_servers.tendly.env={{{table}}}"),
            // A call waiting on the person's confirmation outlasts the default.
            "mcp_servers.tendly.tool_timeout_sec=600".to_owned(),
        ];
        let mut equipped: Vec<String> = overrides
            .into_iter()
            .flat_map(|value| ["-c".to_owned(), value])
            .collect();
        equipped.append(args);
        *args = equipped;
        Ok(())
    }

    fn transcripts(&self) -> &'static [(&'static str, usize)] {
        &[("sessions", 4), ("archived_sessions", 4)]
    }
    fn read_transcript_line(&self, line: &str, source: &Source, state: &mut FileState) {
        read_line(line, source, state);
    }
    fn logged_limits(&self, home: &Path) -> Option<LoggedLimits> {
        logged_limits(home)
    }

    // Its app-server signs in, refreshes and signs out; rate limits come with
    // each refresh rather than a separate usage query.
    fn account_operations(&self) -> AccountOperations {
        AccountOperations {
            managed_login: true,
            usage: false,
        }
    }
    fn account_files(&self) -> &'static [(&'static str, &'static [u8])] {
        &[("config.toml", b"cli_auth_credentials_store = \"file\"\n")]
    }
    fn authorize(
        &self,
        _profile: &tendly_domain::EndpointProfile,
        request: reqwest::RequestBuilder,
        secret: Option<&str>,
    ) -> reqwest::RequestBuilder {
        match secret {
            Some(key) => request.bearer_auth(key),
            None => request,
        }
    }
    fn lists_models_unconfigured(&self) -> bool {
        true
    }
    /// One Responses call, one token, `store: false` so a check leaves nothing
    /// behind on the account (model_probe.rs). Codex itself is not started.
    fn probe_call(
        &self,
        _profile: &tendly_domain::EndpointProfile,
        model: &str,
    ) -> Option<(String, Value)> {
        Some((
            "/v1/responses".into(),
            serde_json::json!({
                "model": model,
                "input": "ping",
                "max_output_tokens": 1,
                "store": false,
            }),
        ))
    }
    // Codex speaks the Responses API, whatever the endpoint.
    fn probe_api(&self, _profile: &tendly_domain::EndpointProfile) -> &'static str {
        "openai-responses"
    }
    fn catalog_args(&self) -> &'static [&'static str] {
        &["app-server"]
    }
    fn unconfigured_catalog_args(&self) -> &'static [&'static str] {
        &["-c", "cli_auth_credentials_store=\"file\""]
    }
    fn catalog_opening(&self) -> &'static str {
        "{\"id\":1,\"method\":\"initialize\",\"params\":{\"clientInfo\":{\"name\":\"tendly_models\",\"version\":\"0.1.0\"}}}\n"
    }
    fn catalog_step(&self, packet: &Value, stage: &mut u8) -> CatalogStep {
        if packet.get("id") == Some(&Value::from(1)) && *stage == 0 {
            if packet.get("error").is_some() {
                return CatalogStep::Done(Err(ApiError::bad(
                    "Codex app-server initialization failed",
                )));
            }
            *stage = 1;
            return CatalogStep::Send(
                "{\"method\":\"initialized\",\"params\":{}}\n{\"id\":2,\"method\":\"model/list\",\"params\":{\"limit\":100,\"includeHidden\":false}}\n",
            );
        }
        if packet.get("id") != Some(&Value::from(2)) {
            return CatalogStep::Wait;
        }
        CatalogStep::Done(
            packet
                .get("result")
                .ok_or_else(|| ApiError::bad("This Codex version cannot list models"))
                .and_then(|body| {
                    let mut catalog = crate::model_catalog::parse_models(
                        body.clone(),
                        "codex://model/list".into(),
                    )?;
                    catalog.has_more = body.get("nextCursor").is_some_and(|c| !c.is_null());
                    Ok(catalog)
                }),
        )
    }
}

fn read_line(line: &str, source: &Source, state: &mut FileState) {
    let interesting = line.contains("\"token_count\"")
        || line.contains("\"task_started\"")
        || line.contains("\"turn_context\"")
        || line.contains("\"session_meta\"");
    if !interesting {
        return;
    }
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return;
    };
    let payload = value.get("payload").unwrap_or(&Value::Null);
    match value.get("type").and_then(Value::as_str) {
        Some("session_meta") => {
            if let Some(id) = payload.get("id").and_then(Value::as_str) {
                state.stated_conversation = Arc::from(id);
            }
            if let Some(cwd) = payload.get("cwd").and_then(Value::as_str) {
                state.stated_cwd = Arc::from(cwd);
            }
        }
        Some("turn_context") => {
            if let Some(model) = payload.get("model").and_then(Value::as_str) {
                state.stated_model = Arc::from(model);
            }
            if let Some(cwd) = payload.get("cwd").and_then(Value::as_str) {
                state.stated_cwd = Arc::from(cwd);
            }
        }
        // Codex starts a task for each message the person sends.
        Some("event_msg")
            if payload.get("type").and_then(Value::as_str) == Some("task_started") =>
        {
            let Some(at) = timestamp(&value) else { return };
            state.calls.push(Call {
                at,
                provider: ProviderKind::Codex,
                model: state.stated_model.clone(),
                conversation: state.stated_conversation.clone(),
                cwd: state.stated_cwd.clone(),
                home_session: source.home_session.clone(),
                dedupe: None,
                user: true,
                tokens: Tokens::default(),
                duration_ms: None,
                outcome: Outcome::Unlogged,
            });
        }
        Some("event_msg") if payload.get("type").and_then(Value::as_str) == Some("token_count") => {
            let Some(info) = payload.get("info").filter(|info| !info.is_null()) else {
                return;
            };
            // The same running total is reported again with a rate-limit update.
            let total = info
                .get("total_token_usage")
                .map(|t| number(t, "total_tokens"))
                .unwrap_or(0);
            if total != 0 && total == state.stated_total {
                return;
            }
            state.stated_total = total;
            let Some(last) = info.get("last_token_usage") else {
                return;
            };
            let Some(at) = timestamp(&value) else { return };
            let cached = number(last, "cached_input_tokens");
            state.calls.push(Call {
                at,
                provider: ProviderKind::Codex,
                model: state.stated_model.clone(),
                conversation: state.stated_conversation.clone(),
                cwd: state.stated_cwd.clone(),
                home_session: source.home_session.clone(),
                dedupe: None,
                user: false,
                tokens: Tokens {
                    // OpenAI counts cached input inside input; split it out.
                    input: number(last, "input_tokens").saturating_sub(cached),
                    cache_read: cached,
                    cache_write_5m: number(last, "cache_write_input_tokens"),
                    cache_write_1h: 0,
                    output: number(last, "output_tokens"),
                    reasoning: number(last, "reasoning_output_tokens"),
                },
                duration_ms: None,
                outcome: Outcome::Unlogged,
            });
        }
        _ => {}
    }
}

/// The newest rate-limit reading Codex wrote under `home`. Every token count
/// it logs carries one, so a Codex account has figures without a query.
fn logged_limits(home: &Path) -> Option<LoggedLimits> {
    let mut files = Vec::new();
    jsonl_files(&home.join("sessions"), &mut files, 4);
    let modified = |file: &PathBuf| std::fs::metadata(file).and_then(|m| m.modified()).ok();
    files.sort_by_key(|file| std::cmp::Reverse(modified(file)));
    // The newest few: a session that never reached a model call has none.
    for file in files.iter().take(5) {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        for line in text.lines().rev() {
            if !line.contains("\"rate_limits\"") {
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let Some(limits) = value
                .pointer("/payload/rate_limits")
                .filter(|v| v.is_object())
            else {
                continue;
            };
            let window = |key: &str| {
                let entry = limits.get(key).filter(|v| v.is_object())?;
                Some(crate::accounts::LimitWindow {
                    used_percent: entry.get("used_percent")?.as_f64()?,
                    window_minutes: entry.get("window_minutes").and_then(Value::as_f64),
                    resets_at: entry.get("resets_at").cloned().filter(|v| !v.is_null()),
                    window_kind: None,
                    mapping_basis: None,
                })
            };
            let (primary, secondary) = (window("primary"), window("secondary"));
            if primary.is_none() && secondary.is_none() {
                continue;
            }
            return Some(LoggedLimits {
                limits: crate::accounts::Limits {
                    primary,
                    secondary,
                    reset_credits: None,
                    reset_credits_source: None,
                },
                plan: limits
                    .get("plan_type")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                observed_at: timestamp(&value)?,
            });
        }
    }
    None
}
