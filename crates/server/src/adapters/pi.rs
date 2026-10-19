//! Pi (`pi`, @earendil-works/pi-coding-agent): configured through
//! `PI_CODING_AGENT_DIR`, a generated `models.json` and flags.
//!
//! Pi has no approval prompts and no MCP: it runs its tools without asking,
//! so it offers no permission modes, and Tendly's own tools do not reach
//! it. Its chat runs over `pi --mode rpc` (chat-pi.mjs).
use super::{AccountOperations, CatalogStep, ClientAdapter, EndpointLaunch};
use crate::providers::KEY_STAND_IN;
use crate::usage::{
    Call, FileState, Outcome, Source, Tokens, empty, number, timestamp, timestamp_ms,
};
use crate::{ApiError, AppState};
use serde_json::{Value, json};
use std::sync::Arc;
use tendly_domain::{EndpointProfile, ProviderKind};

pub struct Pi;

/// The provider a profile's endpoint becomes in the generated `models.json`.
const PROVIDER: &str = "tendly";
/// The variable the generated `models.json` reads the key from, so the key
/// itself is never written to disk.
const KEY_VARIABLE: &str = "TENDLY_PI_API_KEY";

/// Whether a launch with this profile reaches its own endpoint through the
/// generated provider, rather than through Pi's own configuration.
fn configures_endpoint(profile: &EndpointProfile) -> bool {
    profile.endpoint_url.is_some() || profile.secret_ref.is_some()
}

/// The profile's key header, unless it is the one the API sends anyway:
/// `x-api-key` for Anthropic Messages, `x-goog-api-key` for Google, Bearer
/// for the OpenAI APIs.
fn other_header(profile: &EndpointProfile) -> Option<&tendly_domain::KeyHeader> {
    let (name, value) = match api_of(profile) {
        "anthropic-messages" => ("x-api-key", "{key}"),
        "google-generative-ai" => ("x-goog-api-key", "{key}"),
        _ => ("authorization", "Bearer {key}"),
    };
    profile
        .key_header
        .as_ref()
        .filter(|h| !(h.name.eq_ignore_ascii_case(name) && h.value == value))
}

/// The APIs Pi speaks to a custom provider (its docs, models.md).
const APIS: [&str; 4] = [
    "openai-completions",
    "openai-responses",
    "anthropic-messages",
    "google-generative-ai",
];

/// Which API a profile's endpoint speaks: the one it names, else a guess from
/// its URL -- an Anthropic host speaks Anthropic Messages, and anything else
/// is taken as OpenAI Chat Completions, which nearly every gateway serves.
fn api_of(profile: &EndpointProfile) -> &'static str {
    // A named API is one of APIS, whose entries are static, so the borrow is
    // narrowed to the client's own vocabulary rather than the profile's string.
    match profile.api.as_deref() {
        Some(named) => APIS
            .iter()
            .find(|api| **api == named)
            .copied()
            .unwrap_or("openai-completions"),
        None => guess_api(profile.endpoint_url.as_deref().unwrap_or_default()),
    }
}

fn guess_api(url: &str) -> &'static str {
    let host = url::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .unwrap_or_default();
    if host.contains("anthropic") {
        "anthropic-messages"
    } else {
        "openai-completions"
    }
}

/// Pi downloads the tools it searches with (`fd`, `rg`) into `bin/` under its
/// home. Every session has a home of its own, so each would download them
/// again; its `bin/` is one directory all sessions share instead. Where a link
/// cannot be made, a session simply downloads its own, as Pi would anyway.
fn share_tools(state_dir: &std::path::Path, home: &std::path::Path) {
    let shared = state_dir.join("clients").join("pi-bin");
    let bin = home.join("bin");
    if bin.symlink_metadata().is_ok() || std::fs::create_dir_all(&shared).is_err() {
        return;
    }
    #[cfg(unix)]
    let _ = std::os::unix::fs::symlink(&shared, &bin);
    #[cfg(windows)]
    let _ = std::os::windows::fs::symlink_dir(&shared, &bin);
}

impl ClientAdapter for Pi {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Pi
    }
    fn npm_package(&self) -> &'static str {
        "@earendil-works/pi-coding-agent"
    }
    fn command(&self) -> &'static str {
        "pi"
    }
    fn bin_override(&self) -> &'static str {
        "TENDLY_PI_BIN"
    }
    fn config_key(&self) -> &'static str {
        "PI_CODING_AGENT_DIR"
    }
    fn home_dir(&self) -> &'static str {
        ".pi/agent"
    }
    fn history_source(&self) -> (&'static str, &'static str) {
        ("pi-default", "TENDLY_PI_HISTORY_DIR")
    }
    fn source_order(&self) -> u8 {
        2
    }
    fn protected_names(&self) -> &'static [&'static str] {
        &[".pi"]
    }
    // Its session directory, which would move transcripts out of the
    // session's own home.
    fn inherited_keys(&self) -> &'static [&'static str] {
        &["PI_CODING_AGENT_SESSION_DIR", KEY_VARIABLE]
    }
    fn account_prefixes(&self) -> &'static [&'static str] {
        &["PI_"]
    }
    fn resume_args(&self, native_id: &str) -> Vec<String> {
        vec!["--session".into(), native_id.to_owned()]
    }
    fn title_args(&self, title: &str) -> Vec<String> {
        vec!["--name".into(), title.to_owned()]
    }
    // Through the generated provider by name, so an ID that itself has a slash
    // (`zai-org/GLM-5.3`) is not read as a provider of its own.
    fn model_args(&self, profile: &EndpointProfile, model: &str) -> Vec<String> {
        let model = if configures_endpoint(profile) {
            format!("{PROVIDER}/{model}")
        } else {
            model.to_owned()
        };
        vec!["--model".into(), model]
    }
    fn effort_args(&self, effort: &str) -> Vec<String> {
        vec!["--thinking".into(), effort.to_owned()]
    }
    fn plan_mode(&self) -> bool {
        false
    }
    // No approvals to switch between: Pi runs its tools without asking.
    fn permission_modes(&self) -> &'static [&'static str] {
        &[]
    }
    /// The profile's endpoint as a provider in `models.json`, listing the
    /// models the profile offers (Pi offers only models it is told about),
    /// each with the context window set for it.
    fn configure(&self, launch: EndpointLaunch<'_>) -> Result<(), ApiError> {
        let EndpointLaunch {
            state,
            profile,
            secret,
            home,
            args,
            environment,
            ..
        } = launch;
        share_tools(&state.state_dir, home);
        if let Some(effort) = profile.and_then(|p| p.effort.as_deref()) {
            args.extend(self.effort_args(effort));
        }
        let Some(profile) = profile.filter(|p| configures_endpoint(p)) else {
            return Ok(());
        };
        let mut models: Vec<String> = profile.models.clone();
        match crate::providers::resolve_model(profile) {
            Some(model) if !models.contains(&model) => models.insert(0, model),
            Some(_) => {}
            // No default named: the first model offered, rather than Pi's own
            // default, which belongs to some other provider.
            None => match models.first() {
                Some(first) => args.extend(self.model_args(profile, first)),
                None => {
                    return Err(ApiError::bad(
                        "Pi needs to be told which models the endpoint serves: choose a default model or the models this profile offers",
                    ));
                }
            },
        }
        let url = profile
            .endpoint_url
            .as_deref()
            .unwrap_or_else(|| self.default_endpoint());
        let entries: Vec<Value> = models
            .iter()
            .map(|id| {
                let window = crate::model_limits::context_override(&state.state_dir, id);
                match window {
                    Some(window) => json!({ "id": id, "contextWindow": window }),
                    None => json!({ "id": id }),
                }
            })
            .collect();
        // Pi lists a provider's models only once it has some key, so an
        // endpoint without one gets a placeholder, as its docs advise for
        // keyless local servers. A key bound for another header than the
        // API's own goes there, read from the same variable, and the API's
        // own header gets the placeholder.
        let header = secret.as_ref().and(other_header(profile));
        let key = match secret {
            Some(secret) if header.is_none() => {
                environment.insert(KEY_VARIABLE.into(), secret);
                format!("${KEY_VARIABLE}")
            }
            Some(secret) => {
                environment.insert(KEY_VARIABLE.into(), secret);
                KEY_STAND_IN.into()
            }
            None => KEY_STAND_IN.into(),
        };
        let mut provider = json!({
            "baseUrl": url, "api": api_of(profile), "apiKey": key, "models": entries,
        });
        if let Some(header) = header {
            let value = header
                .value
                .replacen("{key}", &format!("${{{KEY_VARIABLE}}}"), 1);
            provider["headers"] = json!({ header.name.clone(): value });
        }
        let config = json!({ "providers": { PROVIDER: provider } });
        crate::providers::write_private(&home.join("models.json"), &config.to_string())
    }

    fn apis(&self) -> &'static [&'static str] {
        &APIS
    }
    fn key_stand_in(&self, profile: &EndpointProfile) -> bool {
        other_header(profile).is_some()
    }
    fn default_endpoint(&self) -> &'static str {
        "https://api.openai.com/v1"
    }
    // Each API lists its models under its own path and credential header.
    fn versioned_api(&self, profile: &EndpointProfile) -> bool {
        api_of(profile) == "anthropic-messages"
    }
    fn authorize(
        &self,
        profile: &EndpointProfile,
        request: reqwest::RequestBuilder,
        secret: Option<&str>,
    ) -> reqwest::RequestBuilder {
        match (api_of(profile), secret) {
            (_, None) => request,
            ("anthropic-messages", Some(key)) => request
                .header("anthropic-version", "2023-06-01")
                .header("x-api-key", key),
            ("google-generative-ai", Some(key)) => request.header("x-goog-api-key", key),
            (_, Some(key)) => request.bearer_auth(key),
        }
    }
    /// Whichever API the profile names, or the one its URL suggests.
    fn probe_api(&self, profile: &EndpointProfile) -> &'static str {
        api_of(profile)
    }
    fn catalog_args(&self) -> &'static [&'static str] {
        &["--mode", "rpc", "--no-session"]
    }
    /// A check speaks whichever API the profile names, with the same path
    /// Pi itself would call (model_probe.rs). One token, nothing stored.
    fn probe_call(&self, profile: &EndpointProfile, model: &str) -> Option<(String, Value)> {
        let (suffix, body) = match api_of(profile) {
            "anthropic-messages" => (
                "/v1/messages".to_owned(),
                json!({
                    "model": model,
                    "max_tokens": 1,
                    "messages": [{ "role": "user", "content": "ping" }],
                }),
            ),
            "google-generative-ai" => (
                format!("/v1beta/models/{model}:generateContent"),
                json!({
                    "contents": [{ "role": "user", "parts": [{ "text": "ping" }] }],
                    "generationConfig": { "maxOutputTokens": 1 },
                }),
            ),
            "openai-responses" => (
                "/v1/responses".to_owned(),
                json!({
                    "model": model,
                    "input": "ping",
                    "max_output_tokens": 1,
                    "store": false,
                }),
            ),
            _ => (
                "/v1/chat/completions".to_owned(),
                json!({
                    "model": model,
                    "messages": [{ "role": "user", "content": "ping" }],
                    "max_tokens": 1,
                    "stream": false,
                }),
            ),
        };
        Some((suffix, body))
    }
    fn catalog_opening(&self) -> &'static str {
        "{\"id\":\"tendly-models\",\"type\":\"get_available_models\"}\n"
    }
    // Each model as `provider/id`, which is how Pi is told to run it.
    fn catalog_step(&self, packet: &Value, _stage: &mut u8) -> CatalogStep {
        if packet.get("id").and_then(Value::as_str) != Some("tendly-models") {
            return CatalogStep::Wait;
        }
        if packet.get("success") != Some(&Value::Bool(true)) {
            return CatalogStep::Done(Err(ApiError::bad("Pi could not list its models")));
        }
        let rows = packet
            .pointer("/data/models")
            .and_then(Value::as_array)
            .map(|models| {
                models
                    .iter()
                    .filter_map(|model| {
                        let provider = model.get("provider")?.as_str()?;
                        let id = model.get("id")?.as_str()?;
                        Some(json!({
                            "id": format!("{provider}/{id}"),
                            "display_name": model.get("name").and_then(Value::as_str).unwrap_or(id),
                            "supportedReasoningEfforts": if model.get("reasoning") == Some(&Value::Bool(true)) {
                                json!(["low", "medium", "high"])
                            } else {
                                json!([])
                            },
                        }))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        CatalogStep::Done(crate::model_catalog::parse_models(
            json!({ "data": rows }),
            "pi://get_available_models".into(),
        ))
    }

    // Sign-in happens in Pi itself, with /login.
    fn account_operations(&self) -> AccountOperations {
        AccountOperations::default()
    }
    fn login_guidance(&self, login_home: Option<&str>) -> Option<String> {
        let prefix = login_home
            .map(|value| format!("PI_CODING_AGENT_DIR='{}' ", value.replace('\'', "'\\''")))
            .unwrap_or_default();
        Some(format!("{prefix}pi   # then /login"))
    }

    // Pi has no MCP: its extensions are its own way to add tools.
    fn equip_tools(
        &self,
        _state: &AppState,
        _session: tendly_domain::SessionId,
        _program: &str,
        _identity: &[(String, String)],
        _args: &mut Vec<String>,
    ) -> Result<(), ApiError> {
        Ok(())
    }

    fn transcripts(&self) -> &'static [(&'static str, usize)] {
        &[("sessions", 2)]
    }
    fn read_transcript_line(&self, line: &str, source: &Source, state: &mut FileState) {
        read_line(line, source, state);
    }
}

/// A Pi session file: a `session` header naming the conversation and its
/// directory, then `message` entries. A user message counts as one sent; an
/// assistant message carries the tokens of its call.
fn read_line(line: &str, source: &Source, state: &mut FileState) {
    if !line.contains("\"type\":\"session\"") && !line.contains("\"type\":\"message\"") {
        return;
    }
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return;
    };
    match value.get("type").and_then(Value::as_str) {
        Some("session") => {
            if let Some(id) = value.get("id").and_then(Value::as_str) {
                state.stated_conversation = Arc::from(id);
            }
            if let Some(cwd) = value.get("cwd").and_then(Value::as_str) {
                state.stated_cwd = Arc::from(cwd);
            }
        }
        Some("message") => {
            let Some(message) = value.get("message") else {
                return;
            };
            let Some(at) = timestamp(&value) else { return };
            let (conversation, cwd) = (state.stated_conversation.clone(), state.stated_cwd.clone());
            let dedupe = value
                .get("id")
                .and_then(Value::as_str)
                .map(|id| Arc::from(format!("pi:{conversation}:{id}")));
            let call = |model: Arc<str>, tokens: Tokens, user: bool| Call {
                at,
                provider: ProviderKind::Pi,
                model,
                conversation: conversation.clone(),
                cwd: cwd.clone(),
                home_session: source.home_session.clone(),
                dedupe: dedupe.clone(),
                tokens,
                user,
                duration_ms: None,
                outcome: Outcome::Unlogged,
            };
            match message.get("role").and_then(Value::as_str) {
                Some("user") => {
                    let typed = match message.get("content") {
                        Some(Value::String(text)) => !text.trim().is_empty(),
                        Some(Value::Array(blocks)) => blocks
                            .iter()
                            .any(|block| block.get("type").and_then(Value::as_str) == Some("text")),
                        _ => false,
                    };
                    if typed {
                        let call = call(empty(), Tokens::default(), true);
                        state.calls.push(call);
                    }
                }
                Some("assistant") => {
                    let Some(usage) = message.get("usage") else {
                        return;
                    };
                    let model = message.get("model").and_then(Value::as_str).unwrap_or("");
                    if model.is_empty() {
                        return;
                    }
                    let tokens = Tokens {
                        input: number(usage, "input"),
                        cache_read: number(usage, "cacheRead"),
                        cache_write_5m: number(usage, "cacheWrite"),
                        cache_write_1h: 0,
                        output: number(usage, "output"),
                        reasoning: 0,
                    };
                    let mut call = call(Arc::from(model), tokens, false);
                    // A stop the person asked for is reported as an error too.
                    let stopped = message.get("stopReason").and_then(Value::as_str)
                        == Some("aborted")
                        || message
                            .get("errorMessage")
                            .and_then(Value::as_str)
                            .is_some_and(|error| error.contains("aborted"));
                    call.outcome = match message.get("stopReason").and_then(Value::as_str) {
                        _ if stopped => Outcome::Unlogged,
                        Some("error") => Outcome::Failed,
                        _ => Outcome::Answered,
                    };
                    // The message is stamped when its request leaves, the
                    // entry when the reply is complete.
                    if call.outcome == Outcome::Answered {
                        call.duration_ms = message
                            .get("timestamp")
                            .and_then(Value::as_i64)
                            .zip(timestamp_ms(&value))
                            .and_then(|(start, end)| u64::try_from(end - start).ok());
                    }
                    state.calls.push(call);
                }
                _ => {}
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pi_session_file_counts_its_messages_and_calls() {
        let lines = [
            r#"{"type":"session","version":3,"id":"conv-1","timestamp":"2026-10-09T01:00:00.000Z","cwd":"/work"}"#,
            r#"{"type":"message","id":"a1","parentId":null,"timestamp":"2026-10-09T01:00:01.000Z","message":{"role":"user","content":"Hello"}}"#,
            r#"{"type":"message","id":"a2","parentId":"a1","timestamp":"2026-10-09T01:00:02.000Z","message":{"role":"assistant","content":[{"type":"text","text":"Hi"}],"provider":"tendly","model":"qwen-local","usage":{"input":100,"output":20,"cacheRead":50,"cacheWrite":5,"totalTokens":175}}}"#,
            r#"{"type":"message","id":"a3","parentId":"a2","timestamp":"2026-10-09T01:00:03.000Z","message":{"role":"toolResult","toolCallId":"c","toolName":"bash","content":[],"isError":false}}"#,
            // A Codex line that happens to sit in a `sessions/` directory too.
            r#"{"type":"response_item","timestamp":"2026-10-09T01:00:04.000Z","payload":{"type":"message"}}"#,
        ];
        let source = crate::usage::test_source(ProviderKind::Pi);
        let mut state = crate::usage::test_state();
        for line in lines {
            read_line(line, &source, &mut state);
        }
        assert_eq!(state.calls.len(), 2);
        assert!(state.calls[0].user && &*state.calls[0].conversation == "conv-1");
        let call = &state.calls[1];
        assert_eq!((&*call.model, &*call.cwd), ("qwen-local", "/work"));
        assert_eq!(
            (
                call.tokens.input,
                call.tokens.output,
                call.tokens.cache_read,
                call.tokens.cache_write_5m
            ),
            (100, 20, 50, 5)
        );
    }

    #[test]
    fn pi_lists_its_models_as_provider_and_id() {
        let packet = json!({"id":"tendly-models","type":"response","command":"get_available_models","success":true,
            "data":{"models":[{"id":"zai-org/GLM-5.3","name":"GLM","provider":"kunlun","reasoning":true},{"id":"plain","provider":"local"}]}});
        let CatalogStep::Done(Ok(catalog)) = Pi.catalog_step(&packet, &mut 0) else {
            panic!("a list");
        };
        let rows: Vec<_> = catalog
            .models
            .iter()
            .map(|m| (m.id.as_str(), m.name.as_str(), m.efforts.len()))
            .collect();
        assert_eq!(
            rows,
            [
                ("kunlun/zai-org/GLM-5.3", "GLM", 3),
                ("local/plain", "plain", 0)
            ]
        );
    }

    #[test]
    fn a_profile_names_its_api_or_an_anthropic_host_is_told_apart() {
        assert_eq!(guess_api("https://api.anthropic.com"), "anthropic-messages");
        assert_eq!(
            guess_api("http://192.168.0.254:8317/v1"),
            "openai-completions"
        );
        assert!(APIS.contains(&guess_api("")));
    }
}
