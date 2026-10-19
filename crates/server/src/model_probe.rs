//! Which of an endpoint's models actually run.
//!
//! Model *discovery* (`model_catalog.rs`) never infers: it reads a list and
//! stops there. This module is the other half, and it runs only because a
//! person pressed a button in the profile form: for each model it sends one
//! request that asks for a single token and throws the answer away. What comes
//! back is whether the endpoint accepted that model, how long it took, and —
//! in fixed words, never the endpoint's own — why it did not.
//!
//! A check is billed by the endpoint as one very short call per model, which is
//! why the interface says so before it runs and why the batch is capped.
use crate::{ApiError, providers};
use futures_util::StreamExt;
use serde::Serialize;
use std::{
    collections::BTreeSet,
    path::Path,
    time::{Duration, Instant},
};
use tendly_domain::EndpointProfile;
use url::Url;

/// Models one request may check. A gateway can serve hundreds; each one costs
/// the person a request, so the interface checks the ones they chose.
const MOST_MODELS: usize = 24;
/// Checks in flight at once, so a batch finishes in seconds without looking
/// like an attack on the endpoint.
const PARALLEL: usize = 4;
/// One check's budget, from connect to the answer.
const TIMEOUT: Duration = Duration::from_secs(20);
/// Enough to hold any answer to a one-token request, and no more.
const MAX_BYTES: usize = 64 * 1024;
/// `ok` the model ran, `unauthorized` the key was refused (a credential
/// problem, not a model one), `not_found` the endpoint has no such model,
/// `rate_limited` it asked to be left alone, `timeout` it did not answer, and
/// `failed` everything else.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProbeResult {
    pub model: String,
    pub status: &'static str,
    /// The wire protocol the check used, so the interface can say which one a
    /// model answered on: the same model can work on one API and fail on
    /// another, and Pi speaks whichever its profile names.
    pub api: &'static str,
    /// Time to the first byte of the answer, so a slow model reads as slow.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    /// Fixed words for the interface. Never the endpoint's own text, which may
    /// echo the credential back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// The wire protocol a check of this profile would use, and the path it would
/// be sent to. No request is made: the interface asks this as the form
/// changes, so it can show which API the endpoint is about to be spoken to.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProbePlan {
    /// The client that would send it.
    pub client: &'static str,
    /// The protocol, or empty when the client speaks only one of its own.
    pub api: &'static str,
    /// The path under the endpoint, e.g. `/v1/messages`.
    pub path: String,
    /// Whether the profile names no API and this one was guessed from the URL.
    pub guessed: bool,
}

/// What a check would send, without sending it.
pub fn plan(profile: &EndpointProfile) -> Result<ProbePlan, ApiError> {
    providers::validate_profile(profile)?;
    if profile.native_config.is_some() {
        return Err(ApiError::bad(
            "An official account's models are checked by its own client; open a session instead",
        ));
    }
    let client = crate::adapters::agent(profile.provider)?;
    let api = client.probe_api(profile);
    // The path is the one a check would use; a placeholder model names it
    // without implying anything about a real one.
    let Some((suffix, _)) = client.probe_call(profile, "model") else {
        return Err(ApiError::bad(format!(
            "{} cannot be checked without starting it",
            client.kind().label()
        )));
    };
    Ok(ProbePlan {
        client: client.kind().label(),
        api,
        path: probe_url(profile, &suffix)?.path().to_owned(),
        // A client with a choice of API (Pi) guesses when the profile is silent.
        guessed: !client.apis().is_empty() && profile.api.is_none(),
    })
}

/// Check `models` against the endpoint `profile` describes. The profile is a
/// draft: nothing here is saved, and no check touches a stored profile.
pub async fn probe(
    state_dir: &Path,
    profile: &EndpointProfile,
    models: &[String],
) -> Result<Vec<ProbeResult>, ApiError> {
    providers::validate_profile(profile)?;
    // An official account's models run through its own client, with its own
    // sign-in; a bare endpoint request would answer a question nobody asked.
    if profile.native_config.is_some() {
        return Err(ApiError::bad(
            "An official account's models are checked by its own client; open a session instead",
        ));
    }
    let models = clean(models)?;
    let client = crate::adapters::agent(profile.provider)?;
    let secret = profile
        .secret_ref
        .as_deref()
        .map(|reference| crate::secrets::resolve_reference(state_dir, reference))
        .transpose()?;
    // What each client would send, per model: a check is only worth running if
    // it is the request that client makes.
    let mut plan = Vec::with_capacity(models.len());
    for model in models {
        let Some((suffix, body)) = client.probe_call(profile, &model) else {
            return Err(ApiError::bad(format!(
                "{} cannot be checked without starting it",
                client.kind().label()
            )));
        };
        plan.push((model, probe_url(profile, &suffix)?, body));
    }
    let api = client.probe_api(profile);
    let connection = connection(profile)?;
    let mut results: Vec<(usize, ProbeResult)> =
        futures_util::stream::iter(plan.into_iter().enumerate())
            .map(|(index, (model, url, body))| {
                let run = Run {
                    connection: &connection,
                    client,
                    profile,
                    secret: secret.as_deref(),
                    api,
                };
                async move {
                    let result = one(&run, model, url, body).await;
                    (index, result)
                }
            })
            .buffer_unordered(PARALLEL)
            .collect()
            .await;
    // Answered in the order asked, so a row does not jump while it is read.
    results.sort_by_key(|(index, _)| *index);
    Ok(results.into_iter().map(|(_, result)| result).collect())
}

/// Where a check goes: the profile's endpoint, then the path its client would
/// use. A base URL that already ends in `/v1` does not get a second one.
pub fn probe_url(profile: &EndpointProfile, suffix: &str) -> Result<Url, ApiError> {
    let client = crate::adapters::agent(profile.provider)?;
    let mut url = Url::parse(
        profile
            .endpoint_url
            .as_deref()
            .unwrap_or(client.default_endpoint()),
    )
    .map_err(|_| ApiError::bad("Invalid endpoint URL"))?;
    if matches!(
        url.host_str(),
        Some("169.254.169.254" | "metadata.google.internal")
    ) {
        return Err(ApiError::bad(
            "Cloud metadata addresses cannot be used as model endpoints",
        ));
    }
    let base = url.path().trim_end_matches('/');
    let suffix = match suffix.strip_prefix("/v1/") {
        Some(rest) if base.ends_with("/v1") => format!("/{rest}"),
        _ => suffix.to_owned(),
    };
    url.set_path(&format!("{base}{suffix}"));
    Ok(url)
}

/// What every check in a run shares: the connection, the client whose request
/// shape is used, the profile that decides headers and proxy, the credential
/// and the protocol being spoken.
struct Run<'a> {
    connection: &'a reqwest::Client,
    client: &'static dyn crate::adapters::ClientAdapter,
    profile: &'a EndpointProfile,
    secret: Option<&'a str>,
    api: &'static str,
}

async fn one(run: &Run<'_>, model: String, url: Url, body: serde_json::Value) -> ProbeResult {
    let Run {
        connection,
        client,
        profile,
        secret,
        api,
    } = *run;
    let request = connection
        .post(url)
        .header("Accept", "application/json")
        .json(&body);
    let request = client.authorize(profile, request, secret);
    let started = Instant::now();
    let response = match tokio::time::timeout(TIMEOUT, request.send()).await {
        Ok(Ok(response)) => response,
        Ok(Err(error)) if error.is_timeout() => {
            return answer(
                model,
                "timeout",
                api,
                None,
                "The endpoint did not answer in time",
            );
        }
        Ok(Err(_)) => {
            return answer(
                model,
                "failed",
                api,
                None,
                "Cannot connect to the endpoint. Check the URL, proxy and server network.",
            );
        }
        Err(_) => {
            return answer(
                model,
                "timeout",
                api,
                None,
                "The endpoint did not answer in time",
            );
        }
    };
    let latency = Some(started.elapsed().as_millis() as u64);
    let status = response.status();
    // Redirects are refused before the credential can follow one somewhere
    // else, the same rule model discovery keeps.
    if status.is_redirection() {
        return answer(
            model,
            "failed",
            api,
            latency,
            "Redirects are blocked to protect the key; enter the final base URL",
        );
    }
    let code = status.as_u16();
    if status.is_success() {
        // Some gateways answer 200 with an error object inside; a body that
        // says `error` is not a model that ran.
        let body = read(response).await;
        return match reports_error(&body) {
            true => answer(
                model,
                "failed",
                api,
                latency,
                "The endpoint answered with an error object",
            ),
            false => answer(model, "ok", api, latency, ""),
        };
    }
    let (status, detail) = match code {
        401 | 403 => (
            "unauthorized",
            "The key was refused. Check the key, not the model.",
        ),
        // A 404 does not say which of the two it means, and the difference
        // matters: a client that speaks one protocol (Claude Code, Codex)
        // gets a 404 for every model when the endpoint lacks that route, and
        // reading that as "the models are gone" invites the wrong repair.
        404 => (
            "not_found",
            "The endpoint has no such model, or does not serve this route",
        ),
        408 => ("timeout", "The endpoint did not answer in time"),
        429 => ("rate_limited", "Rate limited; try again shortly"),
        _ => ("failed", "The endpoint rejected the check"),
    };
    answer(
        model,
        status,
        api,
        latency,
        &match code {
            // The code is the diagnostic; the endpoint's own text is not.
            400 | 401 | 403 | 404 | 408 | 429 | 500..=599 => detail.to_owned(),
            _ => format!("{detail} (HTTP {code})"),
        },
    )
}

fn answer(
    model: String,
    status: &'static str,
    api: &'static str,
    latency_ms: Option<u64>,
    detail: &str,
) -> ProbeResult {
    ProbeResult {
        model,
        status,
        api,
        latency_ms,
        detail: (!detail.is_empty()).then(|| detail.to_owned()),
    }
}

/// Whether a 200 body carries an error: gateways in front of a model
/// sometimes answer with one rather than with a status.
fn reports_error(bytes: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return false;
    };
    match value.get("error") {
        None | Some(serde_json::Value::Null) => false,
        Some(serde_json::Value::Bool(reported)) => *reported,
        Some(serde_json::Value::Array(entries)) => !entries.is_empty(),
        Some(serde_json::Value::Object(fields)) => !fields.is_empty(),
        Some(serde_json::Value::String(text)) => !text.trim().is_empty(),
        Some(_) => true,
    }
}

async fn read(mut response: reqwest::Response) -> Vec<u8> {
    let mut bytes = Vec::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        if bytes.len() + chunk.len() > MAX_BYTES {
            break;
        }
        bytes.extend_from_slice(&chunk);
    }
    bytes
}

fn connection(profile: &EndpointProfile) -> Result<reqwest::Client, ApiError> {
    let mut builder = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none());
    if let Some(proxy) = profile.proxy_url.as_deref() {
        builder = builder
            .no_proxy()
            .proxy(reqwest::Proxy::all(proxy).map_err(|_| ApiError::bad("Invalid HTTP(S) proxy"))?);
    }
    builder
        .build()
        .map_err(|_| ApiError::bad("Unable to configure the model-check connection"))
}

/// The models to check, in the order asked, without repeats.
fn clean(models: &[String]) -> Result<Vec<String>, ApiError> {
    if models.is_empty() {
        return Err(ApiError::bad("Choose at least one model to check"));
    }
    if models.len() > MOST_MODELS {
        return Err(ApiError::bad(format!(
            "Check at most {MOST_MODELS} models at a time"
        )));
    }
    let mut seen = BTreeSet::new();
    let mut clean = Vec::new();
    for model in models {
        let model = model.trim();
        if model.is_empty() || model.len() > 200 || model.chars().any(char::is_control) {
            return Err(ApiError::bad(
                "Model IDs are 1–200 characters without control characters",
            ));
        }
        if seen.insert(model.to_owned()) {
            clean.push(model.to_owned());
        }
    }
    Ok(clean)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tendly_domain::ProviderKind;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    fn profile(url: String) -> EndpointProfile {
        EndpointProfile {
            id: uuid::Uuid::new_v4(),
            name: "fixture".into(),
            provider: ProviderKind::Codex,
            endpoint_url: Some(url),
            model: None,
            permission_mode: "native".into(),
            secret_ref: None,
            proxy_url: None,
            effort: None,
            model_aliases: Default::default(),
            models: Vec::new(),
            api: None,
            native_config: None,
            environment: Default::default(),
            created_at: chrono::Utc::now(),
        }
    }

    /// A server that answers each connection with the next scripted reply and
    /// hands back what it was asked.
    async fn serve(replies: Vec<&'static str>) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let mut requests = Vec::new();
            for reply in replies {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buf = vec![0; 16 * 1024];
                let n = socket.read(&mut buf).await.unwrap();
                requests.push(String::from_utf8_lossy(&buf[..n]).into_owned());
                socket.write_all(reply.as_bytes()).await.unwrap();
            }
            requests
        });
        (format!("http://{address}"), handle)
    }

    fn reply(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    #[test]
    fn a_check_path_does_not_double_the_version_a_base_url_already_has() {
        let mut p = profile("https://gateway.test".into());
        assert_eq!(
            probe_url(&p, "/v1/responses").unwrap().path(),
            "/v1/responses"
        );
        p.endpoint_url = Some("https://gateway.test/v1".into());
        assert_eq!(
            probe_url(&p, "/v1/responses").unwrap().path(),
            "/v1/responses"
        );
        assert_eq!(
            probe_url(&p, "/chat/completions").unwrap().path(),
            "/v1/chat/completions"
        );
        p.endpoint_url = Some("https://gateway.test/prefix/v1/".into());
        assert_eq!(
            probe_url(&p, "/v1/messages").unwrap().path(),
            "/prefix/v1/messages"
        );
        p.endpoint_url = None;
        p.provider = ProviderKind::ClaudeCode;
        assert_eq!(
            probe_url(&p, "/v1/messages").unwrap().as_str(),
            "https://api.anthropic.com/v1/messages"
        );
        // The metadata host is refused here as it is for discovery.
        p.endpoint_url = Some("http://169.254.169.254/latest".into());
        assert!(probe_url(&p, "/v1/messages").is_err());
    }

    #[test]
    fn a_batch_is_bounded_and_each_model_named_once() {
        let models = |values: &[&str]| values.iter().map(|v| (*v).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            clean(&models(&["a", "a", " b "])).unwrap(),
            ["a".to_owned(), "b".to_owned()]
        );
        assert!(clean(&models(&[])).is_err());
        assert!(clean(&models(&["bad\nmodel"])).is_err());
        let many: Vec<String> = (0..MOST_MODELS + 1).map(|i| format!("m{i}")).collect();
        assert!(clean(&many).is_err());
    }

    #[test]
    fn a_body_that_reports_an_error_is_not_a_model_that_ran() {
        assert!(!reports_error(b"{\"id\":\"msg_1\",\"content\":[]}"));
        assert!(!reports_error(b"{\"error\":null}"));
        assert!(!reports_error(b"not json"));
        assert!(reports_error(b"{\"error\":{\"message\":\"nope\"}}"));
        assert!(reports_error(b"{\"error\":\"nope\"}"));
    }

    #[tokio::test]
    async fn a_working_model_is_asked_the_way_its_client_would_and_reports_ok() {
        let (base, server) = serve(vec![Box::leak(
            reply("200 OK", r#"{"id":"resp_1","output":[]}"#).into_boxed_str(),
        )])
        .await;
        let p = profile(format!("{base}/v1"));
        let results = probe(std::env::temp_dir().as_path(), &p, &["gpt-5.5".to_owned()])
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, "ok");
        assert_eq!(results[0].model, "gpt-5.5");
        assert!(results[0].latency_ms.is_some());
        assert_eq!(results[0].detail, None);
        let request = server.await.unwrap().remove(0).to_lowercase();
        assert!(request.starts_with("post /v1/responses "));
        assert!(request.contains("\"max_output_tokens\":1"));
        assert!(request.contains("\"model\":\"gpt-5.5\""));
        assert!(request.contains("\"store\":false"), "no server-side turn");
    }

    #[tokio::test]
    async fn a_refused_key_a_missing_model_and_a_rate_limit_are_told_apart() {
        let (base, server) = serve(vec![
            Box::leak(
                reply(
                    "401 Unauthorized",
                    r#"{"error":{"message":"bad key sk-live-1"}}"#,
                )
                .into_boxed_str(),
            ),
            Box::leak(reply("404 Not Found", r#"{"error":"unknown model"}"#).into_boxed_str()),
            Box::leak(reply("429 Too Many Requests", "").into_boxed_str()),
            Box::leak(reply("200 OK", r#"{"error":{"message":"no quota"}}"#).into_boxed_str()),
        ])
        .await;
        let p = profile(base);
        let models = [
            "a".to_owned(),
            "b".to_owned(),
            "c".to_owned(),
            "d".to_owned(),
        ];
        let results = probe(std::env::temp_dir().as_path(), &p, &models)
            .await
            .unwrap();
        let seen: Vec<(&str, &str)> = results
            .iter()
            .map(|r| (r.model.as_str(), r.status))
            .collect();
        assert_eq!(
            seen,
            [
                ("a", "unauthorized"),
                ("b", "not_found"),
                ("c", "rate_limited"),
                ("d", "failed"),
            ],
            "answered in the order asked"
        );
        // Nothing the endpoint said reaches the interface, where it would
        // echo the credential back to the browser.
        for result in &results {
            let detail = result.detail.clone().unwrap_or_default();
            assert!(!detail.contains("sk-live-1"), "{detail}");
            assert!(!detail.contains("quota"), "{detail}");
        }
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_redirect_is_refused_before_the_key_can_follow_it() {
        let (base, server) = serve(vec![Box::leak(
            reply("302 Found", "")
                .replace(
                    "Content-Length: 0",
                    "Location: http://169.254.169.254/secret\r\nContent-Length: 0",
                )
                .into_boxed_str(),
        )])
        .await;
        let results = probe(
            std::env::temp_dir().as_path(),
            &profile(base),
            &["a".to_owned()],
        )
        .await
        .unwrap();
        assert_eq!(results[0].status, "failed");
        assert!(results[0].detail.as_deref().unwrap().contains("Redirect"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn an_unreachable_endpoint_reports_a_failure_rather_than_an_error_page() {
        // Bound, then released: a port nothing listens on.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let results = probe(
            std::env::temp_dir().as_path(),
            &profile(format!("http://{address}")),
            &["a".to_owned()],
        )
        .await
        .unwrap();
        assert_eq!(results[0].status, "failed");
        assert_eq!(results[0].latency_ms, None);
    }

    #[tokio::test]
    async fn an_official_account_is_told_where_its_models_are_checked() {
        let mut p = profile(String::new());
        p.endpoint_url = None;
        p.native_config = Some(tendly_domain::NativeConfigReference {
            source_id: "claude-default".into(),
            // Absolute on every platform; "/fixture" has no drive on Windows.
            config_dir: std::env::temp_dir()
                .join("claude")
                .to_string_lossy()
                .into_owned(),
            config_env: None,
        });
        let error = probe(std::env::temp_dir().as_path(), &p, &["a".to_owned()])
            .await
            .unwrap_err();
        assert!(error.message.contains("own client"), "{}", error.message);
    }
}
