//! Claude Code: `claude`, configured through `CLAUDE_CONFIG_DIR`, flags and
//! `ANTHROPIC_*` variables.
use super::{AccountOperations, CatalogStep, ClientAdapter, EndpointLaunch};
use crate::usage::{
    Call, FileState, Outcome, Source, Tokens, empty, number, timestamp, timestamp_ms,
};
use crate::{ApiError, AppState};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};
use tendly_domain::{EndpointProfile, ProviderKind};

pub struct ClaudeCode;

impl ClientAdapter for ClaudeCode {
    fn kind(&self) -> ProviderKind {
        ProviderKind::ClaudeCode
    }
    fn npm_package(&self) -> &'static str {
        "@anthropic-ai/claude-code"
    }
    fn command(&self) -> &'static str {
        "claude"
    }
    fn bin_override(&self) -> &'static str {
        "TENDLY_CLAUDE_BIN"
    }
    fn config_key(&self) -> &'static str {
        "CLAUDE_CONFIG_DIR"
    }
    fn home_dir(&self) -> &'static str {
        ".claude"
    }
    fn history_source(&self) -> (&'static str, &'static str) {
        ("claude-default", "TENDLY_CLAUDE_HISTORY_DIR")
    }
    fn source_order(&self) -> u8 {
        1
    }
    fn protected_names(&self) -> &'static [&'static str] {
        &[".claude", ".claude.json"]
    }
    fn inherited_keys(&self) -> &'static [&'static str] {
        &[
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "ANTHROPIC_BASE_URL",
            "CLAUDE_CODE_OAUTH_TOKEN",
            // Which model each of Claude Code's slots runs comes from the
            // session's environment, where it is shown, never from the host's
            // shell.
            "ANTHROPIC_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "ANTHROPIC_SMALL_FAST_MODEL",
        ]
    }
    fn account_prefixes(&self) -> &'static [&'static str] {
        &["ANTHROPIC_", "CLAUDE_"]
    }
    fn resume_args(&self, native_id: &str) -> Vec<String> {
        vec!["--resume".into(), native_id.to_owned()]
    }
    // Claude Code owns the native session title.
    fn title_args(&self, title: &str) -> Vec<String> {
        vec!["--name".into(), title.to_owned()]
    }
    fn effort_args(&self, effort: &str) -> Vec<String> {
        vec!["--effort".into(), effort.to_owned()]
    }
    fn plan_mode(&self) -> bool {
        true
    }
    fn permission_modes(&self) -> &'static [&'static str] {
        &["ask", "plan", "accept_edits", "danger"]
    }
    // Record the session's own usage payload locally, when the operator opted
    // in, so the account page can answer an explicit usage check later.
    fn launch_extras(
        &self,
        state: &AppState,
        config_env: Option<&str>,
    ) -> (Vec<String>, BTreeMap<String, String>) {
        crate::native_config::claude_statusline(state, config_env)
    }
    fn configure(&self, launch: EndpointLaunch<'_>) -> Result<(), ApiError> {
        let EndpointLaunch {
            state,
            profile,
            mode,
            secret,
            args,
            environment,
            ..
        } = launch;
        if let Some(url) = profile.and_then(|p| p.endpoint_url.as_ref()) {
            environment.insert("ANTHROPIC_BASE_URL".into(), url.clone());
        }
        if let Some(secret) = secret {
            environment.insert("ANTHROPIC_API_KEY".into(), secret);
        }
        if mode == "trusted" {
            args.extend(["--permission-mode".into(), "acceptEdits".into()]);
        }
        if mode == "plan" {
            args.extend(["--permission-mode".into(), "plan".into()]);
        }
        // Current CLI uses "manual"; older clients called it "default".
        // Native mode omits this flag entirely.
        if mode == "interactive" {
            let manual = state.claude_manual_mode;
            args.extend([
                "--permission-mode".into(),
                if manual { "manual" } else { "default" }.into(),
            ]);
        }
        if let Some(effort) = profile.and_then(|p| p.effort.as_deref()) {
            args.extend(self.effort_args(effort));
        }
        let config_env = environment.get(self.config_key()).cloned();
        let (overlay, capture) = self.launch_extras(state, config_env.as_deref());
        args.extend(overlay);
        environment.extend(capture);
        Ok(())
    }

    fn default_endpoint(&self) -> &'static str {
        "https://api.anthropic.com"
    }

    // A small private file named by `--mcp-config`, and the tools allowed up
    // front: what needs the person's word is asked in Tendly itself.
    fn equip_tools(
        &self,
        state: &AppState,
        session: tendly_domain::SessionId,
        program: &str,
        identity: &[(String, String)],
        args: &mut Vec<String>,
    ) -> Result<(), ApiError> {
        let directory = crate::providers::private_dir(&state.state_dir.join("agent-mcp"))?;
        let path = directory.join(format!("{session}.json"));
        let environment: serde_json::Map<String, Value> = identity
            .iter()
            .map(|(key, value)| (key.clone(), Value::from(value.as_str())))
            .collect();
        let config = serde_json::json!({ "mcpServers": { "tendly": {
            "type": "stdio", "command": program, "args": ["mcp"], "env": environment,
        } } });
        crate::security::write_owner_only(&path, &config.to_string())
            .map_err(ApiError::internal)?;
        // `=` form: both flags take several values and would swallow the
        // argument after them.
        args.push(format!("--mcp-config={}", path.to_string_lossy()));
        args.push("--allowedTools=mcp__tendly".into());
        Ok(())
    }

    fn transcripts(&self) -> &'static [(&'static str, usize)] {
        &[("projects", 3)]
    }
    fn read_transcript_line(&self, line: &str, source: &Source, state: &mut FileState) {
        read_line(line, source, state);
    }
    // A cache write without a price of its own costs what Anthropic charges:
    // 1.25x input for five minutes and 2x for an hour.
    fn cache_write_rates(&self, input: f64) -> (f64, f64) {
        (input * 1.25, input * 2.0)
    }

    // Sign-in stays with its own CLI; usage is read on request.
    fn account_operations(&self) -> AccountOperations {
        AccountOperations {
            managed_login: false,
            usage: true,
        }
    }
    // An account that owns its directory has to be told to sign in *there*:
    // without the override the command writes the credentials into the host's
    // own configuration, and the account never works while appearing to exist.
    fn login_guidance(&self, login_home: Option<&str>) -> Option<String> {
        let config_prefix = login_home
            .map(|value| format!(" CLAUDE_CONFIG_DIR='{}'", value.replace('\'', "'\\''")))
            .unwrap_or_default();
        Some(format!(
            "env -u ANTHROPIC_API_KEY -u ANTHROPIC_AUTH_TOKEN -u CLAUDE_CODE_OAUTH_TOKEN{} claude auth login",
            config_prefix
        ))
    }
    fn versioned_api(&self, _profile: &EndpointProfile) -> bool {
        true
    }
    fn authorize(
        &self,
        _profile: &EndpointProfile,
        request: reqwest::RequestBuilder,
        secret: Option<&str>,
    ) -> reqwest::RequestBuilder {
        let request = request.header("anthropic-version", "2023-06-01");
        match secret {
            Some(key) => request.header("x-api-key", key),
            None => request,
        }
    }
    // One message, one token, discarded: what a model check asks
    // (model_probe.rs). Claude Code itself is never started for it.
    fn probe_call(&self, _profile: &EndpointProfile, model: &str) -> Option<(String, Value)> {
        Some((
            "/v1/messages".into(),
            serde_json::json!({
                "model": model,
                "max_tokens": 1,
                "messages": [{ "role": "user", "content": "ping" }],
            }),
        ))
    }
    // Claude Code has one wire protocol, whatever the endpoint.
    fn probe_api(&self, _profile: &EndpointProfile) -> &'static str {
        "anthropic-messages"
    }
    // The same read-only handshake a chat session performs.
    fn catalog_args(&self) -> &'static [&'static str] {
        &[
            "--print",
            "--verbose",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--permission-prompt-tool",
            "stdio",
        ]
    }
    fn catalog_opening(&self) -> &'static str {
        "{\"type\":\"control_request\",\"request_id\":\"tendly-models\",\"request\":{\"subtype\":\"initialize\"}}\n"
    }
    fn catalog_step(&self, packet: &Value, _stage: &mut u8) -> CatalogStep {
        let Some(response) = packet.get("response") else {
            return CatalogStep::Wait;
        };
        if response.get("request_id").and_then(Value::as_str) != Some("tendly-models") {
            return CatalogStep::Wait;
        }
        CatalogStep::Done(
            response
                .get("response")
                .and_then(|body| body.get("models"))
                .ok_or_else(|| {
                    ApiError::bad("This Claude Code version does not publish its models")
                })
                .and_then(|models| crate::model_catalog::parse_claude_models(models.clone())),
        )
    }
}

/// A message the person typed: text, not a tool result passed back, not a
/// note the client wrote into the conversation itself.
fn claude_user_message(line: &str, source: &Source, state: &mut FileState) {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return;
    };
    if value.get("type").and_then(Value::as_str) != Some("user")
        || value.get("isMeta").and_then(Value::as_bool) == Some(true)
    {
        return;
    }
    let typed = match value
        .get("message")
        .and_then(|message| message.get("content"))
    {
        Some(Value::String(text)) => !text.trim().is_empty(),
        Some(Value::Array(blocks)) => {
            blocks
                .iter()
                .any(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                && !blocks
                    .iter()
                    .any(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
        }
        _ => false,
    };
    let Some(at) = timestamp(&value).filter(|_| typed) else {
        return;
    };
    state.calls.push(Call {
        at,
        provider: ProviderKind::ClaudeCode,
        model: empty(),
        conversation: Arc::from(value.get("sessionId").and_then(Value::as_str).unwrap_or("")),
        cwd: Arc::from(value.get("cwd").and_then(Value::as_str).unwrap_or("")),
        home_session: source.home_session.clone(),
        dedupe: value
            .get("uuid")
            .and_then(Value::as_str)
            .map(|id| Arc::from(format!("user:{id}"))),
        user: true,
        tokens: Tokens::default(),
        duration_ms: None,
        outcome: Outcome::Unlogged,
    });
}

/// A request Claude Code logs before sending it (recent versions): when it
/// left, for the reply that names it to time itself by, and the model it
/// asked for, for a failure that names none.
fn claude_request(value: &Value, state: &mut FileState) {
    if let Some(model) = value
        .get("params")
        .and_then(|params| params.get("model"))
        .and_then(Value::as_str)
    {
        state.stated_model = Arc::from(model);
    }
    if let (Some(id), Some(at)) = (value.get("id").and_then(Value::as_str), timestamp_ms(value)) {
        state.request_starts.insert(Arc::from(id), at);
    }
}

/// An attempt that failed and was retried (`api_error`), or a request that
/// failed for good, logged as a reply the client wrote itself.
fn claude_failure(value: &Value, outcome: Outcome, source: &Source, state: &mut FileState) {
    let (Some(at), Some(id)) = (timestamp(value), value.get("uuid").and_then(Value::as_str)) else {
        return;
    };
    state.calls.push(Call {
        at,
        provider: ProviderKind::ClaudeCode,
        model: state.stated_model.clone(),
        conversation: Arc::from(value.get("sessionId").and_then(Value::as_str).unwrap_or("")),
        cwd: Arc::from(value.get("cwd").and_then(Value::as_str).unwrap_or("")),
        home_session: source.home_session.clone(),
        dedupe: Some(Arc::from(format!("failure:{id}"))),
        user: false,
        tokens: Tokens::default(),
        duration_ms: None,
        outcome,
    });
}

fn read_line(line: &str, source: &Source, state: &mut FileState) {
    if line.contains("\"type\":\"user\"") {
        claude_user_message(line, source, state);
        return;
    }
    let request = line.contains("\"type\":\"api-request\"");
    let retry = line.contains("\"subtype\":\"api_error\"");
    if !request && !retry && !line.contains("\"usage\"") {
        return;
    }
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return;
    };
    match value.get("type").and_then(Value::as_str) {
        Some("api-request") => return claude_request(&value, state),
        Some("system") if retry => {
            return claude_failure(&value, Outcome::Retried, source, state);
        }
        Some("assistant") => {}
        _ => return,
    }
    if value.get("isApiErrorMessage").and_then(Value::as_bool) == Some(true) {
        return claude_failure(&value, Outcome::Failed, source, state);
    }
    let Some(message) = value.get("message") else {
        return;
    };
    let Some(usage) = message.get("usage") else {
        return;
    };
    let model = message.get("model").and_then(Value::as_str).unwrap_or("");
    if model.is_empty() || model == "<synthetic>" {
        return;
    }
    let Some(at) = timestamp(&value) else { return };
    state.stated_model = Arc::from(model);
    // Each content block of a reply is its own line, the last written when
    // the reply ends: the reply took until then.
    let duration_ms = value
        .get("requestRef")
        .and_then(Value::as_str)
        .and_then(|id| state.request_starts.get(id))
        .zip(timestamp_ms(&value))
        .and_then(|(start, end)| u64::try_from(end - start).ok());
    let (write_5m, write_1h) = match usage.get("cache_creation") {
        Some(split) => (
            number(split, "ephemeral_5m_input_tokens"),
            number(split, "ephemeral_1h_input_tokens"),
        ),
        None => (number(usage, "cache_creation_input_tokens"), 0),
    };
    let dedupe = message.get("id").and_then(Value::as_str).map(|id| {
        Arc::from(format!(
            "{id}:{}",
            value.get("requestId").and_then(Value::as_str).unwrap_or("")
        ))
    });
    if let Some(earlier) = state
        .calls
        .iter_mut()
        .rev()
        .take(8)
        .find(|call| dedupe.is_some() && call.dedupe == dedupe)
    {
        earlier.duration_ms = duration_ms.or(earlier.duration_ms);
        return;
    }
    state.calls.push(Call {
        at,
        provider: ProviderKind::ClaudeCode,
        model: Arc::from(model),
        conversation: Arc::from(value.get("sessionId").and_then(Value::as_str).unwrap_or("")),
        cwd: Arc::from(value.get("cwd").and_then(Value::as_str).unwrap_or("")),
        home_session: source.home_session.clone(),
        dedupe,
        user: false,
        tokens: Tokens {
            input: number(usage, "input_tokens"),
            cache_read: number(usage, "cache_read_input_tokens"),
            cache_write_5m: write_5m,
            cache_write_1h: write_1h,
            output: number(usage, "output_tokens"),
            reasoning: usage
                .get("output_tokens_details")
                .map(|details| number(details, "thinking_tokens"))
                .unwrap_or(0),
        },
        duration_ms,
        outcome: Outcome::Answered,
    });
}
