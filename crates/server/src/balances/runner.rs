//! A fresh QuickJS VM for each query: no filesystem, process, modules or ambient network.
use super::{Auth, Config, KEY_PLACEHOLDER, RESERVED_HEADERS, Unit};
use rquickjs::{Context, Function, Promise, Runtime};
use serde_json::{Value, json};
use std::{
    io::Read,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use url::Url;

const MAX_RESPONSE: u64 = 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(10);

pub(super) fn allowed_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|_| "Balance query URL is invalid")?;
    if !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || matches!(
            url.host_str(),
            Some("169.254.169.254" | "metadata.google.internal" | "[fd00:ec2::254]")
        )
    {
        return Err("Use an HTTP(S) balance URL without credentials or fragments".into());
    }
    Ok(url)
}

/// Keep decimal text exact for money. Integers and numbers are accepted too;
/// infinity, exponents in strings and arbitrary provider output are rejected.
pub(super) fn normalize(value: &Value, unit: &Unit) -> Result<String, String> {
    let value = value
        .get("value")
        .ok_or("Script must return { value: numberOrDecimalString }")?;
    let raw = match value {
        Value::String(s) => s.trim().to_owned(),
        Value::Number(n) => {
            let text = n.to_string();
            if text.contains(['e', 'E']) {
                let number = n.as_f64().ok_or("Invalid balance number")?;
                let fixed = format!("{number:.12}");
                let fixed = fixed.trim_end_matches('0').trim_end_matches('.');
                if fixed.parse::<f64>().ok() != Some(number) {
                    return Err("Balance has more than 12 fractional digits".into());
                }
                fixed.to_owned()
            } else {
                text
            }
        }
        _ => return Err("Balance value must be a number or decimal string".into()),
    };
    let digits = raw.strip_prefix('-').unwrap_or(&raw);
    let mut parts = digits.split('.');
    let whole = parts.next().unwrap_or("");
    let fraction = parts.next();
    if whole.is_empty()
        || whole.len() > 18
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fraction
            .is_some_and(|f| f.is_empty() || f.len() > 12 || !f.bytes().all(|b| b.is_ascii_digit()))
        || parts.next().is_some()
    {
        return Err(
            "Return a decimal with at most 18 integer and 12 fractional digits (no exponent)"
                .into(),
        );
    }
    let number: f64 = raw.parse().map_err(|_| "Invalid balance value")?;
    if !number.is_finite()
        || (matches!(unit, Unit::Percentage {}) && !(0.0..=100.0).contains(&number))
    {
        return Err("Remaining percentage must be between 0 and 100".into());
    }
    Ok(raw)
}

pub(super) fn run(
    config: Config,
    key: Option<String>,
    proxy: Option<String>,
) -> Result<String, String> {
    run_with_deadline(config, key, proxy, DEADLINE)
}
fn run_with_deadline(
    config: Config,
    key: Option<String>,
    proxy: Option<String>,
    duration: Duration,
) -> Result<String, String> {
    let deadline = Instant::now() + duration;
    let base = allowed_url(&config.query_url)?;
    let mut builder = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(duration)
        .no_proxy();
    if let Some(proxy) = proxy {
        builder = builder.proxy(reqwest::Proxy::all(&proxy).map_err(|_| "Invalid profile proxy")?);
    }
    let client = builder
        .build()
        .map_err(|_| "Cannot initialize balance HTTP client")?;
    let runtime = Runtime::new().map_err(|_| "Cannot initialize JavaScript")?;
    runtime.set_memory_limit(16 * 1024 * 1024);
    runtime.set_max_stack_size(512 * 1024);
    runtime.set_interrupt_handler(Some(Box::new(move || Instant::now() >= deadline)));
    let context = Context::full(&runtime).map_err(|_| "Cannot initialize JavaScript context")?;
    let calls = Arc::new(AtomicUsize::new(0));
    let network_error = Arc::new(Mutex::new(None::<String>));
    let last_error = network_error.clone();
    let auth = config.auth;
    let script = config.script;
    let result = context.with(|ctx| -> rquickjs::Result<String> {
        let request = Function::new(ctx.clone(), move |raw: String| -> String {
            let query = || -> Result<Value, String> {
                if calls.fetch_add(1, Ordering::Relaxed) >= 4 {
                    return Err("At most four requests are allowed per query".into());
                }
                let args: Value =
                    serde_json::from_str(&raw).map_err(|_| "Invalid request options")?;
                let mut url = match args.get("url").and_then(Value::as_str) {
                    Some(path) => base.join(path).map_err(|_| "Invalid request URL")?,
                    None => base.clone(),
                };
                allowed_url(url.as_str())?;
                if url.origin() != base.origin() {
                    return Err("Balance requests must stay on the configured query origin".into());
                }
                let remaining = deadline
                    .checked_duration_since(Instant::now())
                    .ok_or("Balance script timed out")?;
                let method = match args.get("method").and_then(Value::as_str).unwrap_or("GET") {
                    "GET" => reqwest::Method::GET,
                    "POST" => reqwest::Method::POST,
                    _ => return Err("Balance requests support only GET and POST".into()),
                };
                if let (Auth::Query { name }, Some(key)) = (&auth, &key) {
                    let kept: Vec<(String, String)> = url
                        .query_pairs()
                        .filter(|(pair, _)| pair != name.as_str())
                        .map(|(k, v)| (k.into_owned(), v.into_owned()))
                        .collect();
                    url.query_pairs_mut()
                        .clear()
                        .extend_pairs(kept)
                        .append_pair(name, key);
                }
                let mut request = client.request(method, url).timeout(remaining);
                if let Some(headers) = args.get("headers") {
                    let headers = headers
                        .as_object()
                        .ok_or("Request headers must be an object")?;
                    if headers.len() > 16 {
                        return Err("Too many request headers".into());
                    }
                    for (name, value) in headers {
                        let lower = name.to_ascii_lowercase();
                        if RESERVED_HEADERS.contains(&lower.as_str())
                            || matches!(lower.as_str(), "authorization" | "x-api-key")
                            || matches!(&auth, Auth::Header { name: own, .. } if own.eq_ignore_ascii_case(name))
                        {
                            return Err(
                                "Authentication headers are managed by the balance configuration"
                                    .into(),
                            );
                        }
                        request = request
                            .header(name, value.as_str().ok_or("Headers must contain text")?);
                    }
                }
                if let Some(body) = args.get("body") {
                    if body.to_string().len() > 32_768 {
                        return Err("Request body is too large".into());
                    }
                    request = request.json(body);
                }
                if let (Auth::Header { name, value }, Some(key)) = (&auth, &key) {
                    request = request.header(name, value.replacen(KEY_PLACEHOLDER, key, 1));
                }
                let response = request
                    .send()
                    .map_err(|_| "Balance request failed or timed out")?;
                if !response.status().is_success() {
                    return Err(format!(
                        "Balance endpoint returned HTTP {}",
                        response.status().as_u16()
                    ));
                }
                let mut bytes = Vec::new();
                response
                    .take(MAX_RESPONSE + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| "Cannot read balance response")?;
                if bytes.len() as u64 > MAX_RESPONSE {
                    return Err("Balance response exceeds 1 MiB".into());
                }
                serde_json::from_slice(&bytes)
                    .map_err(|_| "Balance endpoint did not return JSON".into())
            };
            match query() {
                Ok(data) => json!({"ok":true,"data":data}).to_string(),
                Err(error) => {
                    *last_error.lock().unwrap() = Some(error.clone());
                    json!({"ok":false,"error":error}).to_string()
                }
            }
        })?;
        ctx.globals().set("__balanceRequest", request)?;
        // Scripts receive a frozen request API;
        // credentials, Node globals, imports and fetch are never installed.
        ctx.eval::<(), _>(
            r#"
            globalThis.ctx = Object.freeze({
                request: async (options = {}) => {
                    const response = JSON.parse(__balanceRequest(JSON.stringify(options)));
                    if (!response.ok) throw new Error(response.error);
                    return response.data;
                }
            });
        "#,
        )?;
        let source =
            format!("(async () => {{\n{script}\n}})().then(value => JSON.stringify(value))");
        let promise: Promise = ctx.eval(source)?;
        promise.finish::<String>()
    });
    let raw = result.map_err(|_| {
        if Instant::now() >= deadline {
            "Balance script timed out".into()
        } else {
            network_error.lock().unwrap().clone().unwrap_or_else(|| {
                "Balance script failed: check JavaScript syntax and return { value: ... }".into()
            })
        }
    })?;
    let result: Value =
        serde_json::from_str(&raw).map_err(|_| "Script must return { value: ... }")?;
    normalize(&result, &config.unit)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config(script: &str) -> Config {
        Config {
            unit: Unit::Percentage {},
            query_url: "https://example.invalid/balance".into(),
            auth: Auth::None {},
            secret_ref: None,
            script: script.into(),
            auto_refresh: None,
        }
    }
    #[test]
    fn executes_async_js_without_node_or_ambient_network() {
        let script = "if (typeof process !== 'undefined' || typeof fetch !== 'undefined' || typeof require !== 'undefined') throw Error(); return {value: await Promise.resolve(72)};";
        assert_eq!(run(config(script), None, None).unwrap(), "72");
    }
    #[test]
    fn infinite_loop_is_interrupted() {
        assert!(
            run_with_deadline(
                config("while(true) {}"),
                None,
                None,
                Duration::from_millis(20)
            )
            .unwrap_err()
            .contains("timed out")
        );
    }
    #[test]
    fn currency_is_exact_and_percentages_are_bounded() {
        let unit = Unit::Money {
            currency: "CNY".into(),
        };
        assert_eq!(
            normalize(&json!({"value":"123456789012345678.12"}), &unit).unwrap(),
            "123456789012345678.12"
        );
        for value in [
            json!(null),
            json!(true),
            json!("NaN"),
            json!("1e20"),
            json!("USD 10"),
        ] {
            assert!(normalize(&json!({"value":value}), &unit).is_err());
        }
        assert_eq!(
            normalize(&json!({"value": 0.0000001}), &unit).unwrap(),
            "0.0000001"
        );
        for value in [-1, 101] {
            assert!(normalize(&json!({"value":value}), &Unit::Percentage {}).is_err());
        }
        assert_eq!(
            normalize(&json!({"value":0}), &Unit::Percentage {}).unwrap(),
            "0"
        );
    }
    #[test]
    fn request_cannot_cross_configured_origin() {
        let error = run(
            config("return { value: await ctx.request({url:'https://other.invalid/'}) };"),
            Some("secret".into()),
            None,
        )
        .unwrap_err();
        assert!(error.contains("origin"));
        assert!(!error.contains("secret"));
    }
    fn server(response: String) -> (String, std::thread::JoinHandle<String>) {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/balance", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut byte = [0u8; 1];
            while !bytes.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                bytes.push(byte[0]);
            }
            stream.write_all(response.as_bytes()).unwrap();
            String::from_utf8(bytes).unwrap()
        });
        (url, worker)
    }
    #[test]
    fn host_request_injects_key_and_script_converts_real_json() {
        let body = r#"{"used_percent":28}"#;
        let (url, worker) = server(format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ));
        let mut cfg =
            config("const data=await ctx.request(); return {value:100-data.used_percent};");
        cfg.query_url = url;
        cfg.auth = Auth::Header {
            name: "Authorization".into(),
            value: "Bearer {key}".into(),
        };
        assert_eq!(run(cfg, Some("fixture-secret".into()), None).unwrap(), "72");
        assert!(
            worker
                .join()
                .unwrap()
                .to_lowercase()
                .contains("authorization: bearer fixture-secret")
        );
    }
    fn ok(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }
    #[test]
    fn a_header_template_carries_the_key_and_the_script_cannot_replace_it() {
        let (url, worker) = server(ok(r#"{"left":40}"#));
        let mut cfg = config("return {value:(await ctx.request()).left};");
        cfg.query_url = url.clone();
        cfg.auth = Auth::Header {
            name: "X-Token".into(),
            value: "Token {key}".into(),
        };
        assert_eq!(
            run(cfg.clone(), Some("fixture-secret".into()), None).unwrap(),
            "40"
        );
        assert!(
            worker
                .join()
                .unwrap()
                .to_lowercase()
                .contains("x-token: token fixture-secret")
        );
        cfg.script = "return {value:await ctx.request({headers:{'x-token':'mine'}})};".into();
        let error = run(cfg, Some("fixture-secret".into()), None).unwrap_err();
        assert!(error.contains("managed by the balance configuration"));
    }
    #[test]
    fn a_query_parameter_carries_the_key_over_any_the_script_wrote() {
        let (url, worker) = server(ok(r#"{"left":"9.50"}"#));
        let mut cfg =
            config("return {value:(await ctx.request({url:'?api_key=stale&page=1'})).left};");
        cfg.unit = Unit::Money {
            currency: "USD".into(),
        };
        cfg.query_url = url;
        cfg.auth = Auth::Query {
            name: "api_key".into(),
        };
        assert_eq!(
            run(cfg, Some("fixture-secret".into()), None).unwrap(),
            "9.50"
        );
        let request = worker.join().unwrap();
        let line = request.lines().next().unwrap();
        assert!(
            line.contains("page=1") && line.contains("api_key=fixture-secret"),
            "{line}"
        );
        assert!(!line.contains("stale"), "{line}");
    }
    #[test]
    fn redirects_are_not_followed_with_credentials() {
        let (url,worker)=server("HTTP/1.1 302 Found\r\nLocation: https://other.invalid/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into());
        let mut cfg = config("return {value:await ctx.request()};");
        cfg.query_url = url;
        cfg.auth = Auth::Header {
            name: "x-api-key".into(),
            value: "{key}".into(),
        };
        let error = run(cfg, Some("fixture-secret".into()), None).unwrap_err();
        assert!(error.contains("HTTP 302"));
        assert!(!error.contains("fixture-secret"));
        assert!(worker.join().unwrap().contains("x-api-key: fixture-secret"));
    }
}
