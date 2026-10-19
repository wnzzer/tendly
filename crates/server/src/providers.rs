//! Endpoint validation and private configuration-file helpers.
use crate::ApiError;
use std::{
    fs,
    path::{Path, PathBuf},
};
use tendly_domain::{EndpointProfile, KeyHeader, ProviderKind};
use url::Url;

/// Levels both native clients accept. Tendly forwards one of these or none.
pub const EFFORT_LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

pub fn validate_effort(value: Option<&str>) -> Result<(), ApiError> {
    if value.is_some_and(|value| !EFFORT_LEVELS.contains(&value)) {
        return Err(ApiError::bad(
            "Reasoning effort must be low, medium, high, xhigh or max",
        ));
    }
    Ok(())
}

/// Where a profile's key goes on each request. Two headers each client
/// already knows how to send are told apart from any other, which a client
/// sends next to a placeholder in its own (`KEY_STAND_IN`).
pub enum KeyPlacement<'a> {
    /// The protocol's own header: no key header set.
    Protocol,
    XApiKey,
    Bearer,
    Custom(&'a KeyHeader),
}

pub fn key_placement(p: &EndpointProfile) -> KeyPlacement<'_> {
    match &p.key_header {
        None => KeyPlacement::Protocol,
        Some(h) if h.name.eq_ignore_ascii_case("x-api-key") && h.value == KEY_SLOT => {
            KeyPlacement::XApiKey
        }
        Some(h) if h.name.eq_ignore_ascii_case("authorization") && h.value == "Bearer {key}" => {
            KeyPlacement::Bearer
        }
        Some(h) => KeyPlacement::Custom(h),
    }
}

/// Where a key header's value takes the key.
pub const KEY_SLOT: &str = "{key}";
/// What a client that will not send a request without a key of its own gets
/// for it, when the real key travels in another header. Never a credential.
pub const KEY_STAND_IN: &str = "tendly";

/// Holds the key itself where a client sends it inside a header of the
/// profile's own: read by no client, it is there so a session's output that
/// names the key is redacted like any other key (native_chat/values.rs).
pub const KEY_VARIABLE: &str = "TENDLY_ENDPOINT_API_KEY";

/// The header's value with the key in it.
pub fn key_value(header: &KeyHeader, key: &str) -> String {
    header.value.replacen(KEY_SLOT, key, 1)
}

/// A header name and a one-line value with `{key}` once. No `$`, and no
/// leading `!`: Pi reads either in its configuration as a variable or a
/// command to run.
fn validate_key_header(header: &KeyHeader) -> Result<(), ApiError> {
    let token = |b: u8| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b);
    let name = header.name.as_str();
    if name.is_empty() || name.len() > 64 || !name.bytes().all(token) {
        return Err(ApiError::bad("Use a valid HTTP header name for the key"));
    }
    if [
        "host",
        "cookie",
        "proxy-authorization",
        "content-length",
        "content-type",
        "transfer-encoding",
        "connection",
        "upgrade",
    ]
    .contains(&name.to_ascii_lowercase().as_str())
    {
        return Err(ApiError::bad("That header cannot carry the key"));
    }
    let value = header.value.as_str();
    if value.len() > 256
        || value.matches(KEY_SLOT).count() != 1
        || value.chars().any(char::is_control)
        || value.contains('$')
        || value.starts_with('!')
    {
        return Err(ApiError::bad(
            "The header value must contain {key} exactly once, on one line, without $ or a leading !",
        ));
    }
    Ok(())
}

/// Whether a profile can be saved: a bounded name, a client that takes
/// profiles, and settings that do not contradict each other. A profile that
/// uses an existing native configuration may not also redirect its endpoint,
/// credential or permissions, since those belong to that configuration.
pub fn validate_profile(p: &EndpointProfile) -> Result<(), ApiError> {
    crate::environment::validate(&p.environment)?;
    if p.name.trim().is_empty() || p.name.len() > 120 {
        return Err(ApiError::bad("Profile name must be 1–120 characters"));
    }
    if matches!(p.provider, ProviderKind::Terminal) {
        return Err(ApiError::bad(
            "Terminal sessions do not use endpoint profiles",
        ));
    }
    if let Some(reference) = &p.native_config {
        // Where the requests go and who they authenticate as belong to the
        // native configuration; Tendly must not redirect either.
        if reference.source_id.is_empty()
            || !Path::new(&reference.config_dir).is_absolute()
            || p.permission_mode != "native"
            || p.endpoint_url.is_some()
            || p.secret_ref.is_some()
            || p.key_header.is_some()
        {
            return Err(ApiError::bad(
                "An existing native configuration cannot be combined with endpoint, credential, proxy or permission overrides",
            ));
        }
        // A proxy is how the account reaches its own endpoint, not a different
        // endpoint, so an official account may carry one. Account settings
        // write it here, and forbidding it left those profiles unvalidatable.
        if let Some(proxy) = p.proxy_url.as_deref() {
            validate_proxy(proxy)?;
        }
        // Mapping an official slot onto another upstream model is something both
        // clients already do themselves — Claude Code through its
        // ANTHROPIC_DEFAULT_*_MODEL variables, Codex through model_providers.
        // A second alias table here would compete with theirs and would apply
        // only to sessions Tendly launched, so it stays refused.
        if !p.model_aliases.is_empty() {
            return Err(ApiError::bad(
                "An official account maps models through its own client configuration, not through aliases here",
            ));
        }
        // A model and a depth are launch choices, not a redirection: the client
        // still resolves and validates them against its own account.
        validate_effort(p.effort.as_deref())?;
        if let Some(model) = p.model.as_deref()
            && (model.trim().is_empty() || model.len() > 200 || model.chars().any(char::is_control))
        {
            return Err(ApiError::bad("Model must be a real model ID"));
        }
        return Ok(());
    }
    if p.permission_mode == "plan" && !crate::adapters::agent(p.provider)?.plan_mode() {
        return Err(ApiError::bad(format!(
            "{} keeps its native approval model; plan mode is not one of its modes",
            p.provider.label()
        )));
    }
    if !matches!(
        p.permission_mode.as_str(),
        "native" | "interactive" | "trusted" | "plan" | "blocked"
    ) {
        return Err(ApiError::bad("Invalid permission mode"));
    }
    if let Some(value) = &p.endpoint_url {
        let url = Url::parse(value)
            .map_err(|_| ApiError::bad("Endpoint must be an absolute HTTP(S) URL"))?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(ApiError::bad(
                "Endpoint must use HTTP(S), without embedded credentials, query or fragment",
            ));
        }
    }
    if let Some(value) = &p.proxy_url {
        validate_proxy(value)?;
    }
    // Reasoning effort is a fixed vocabulary both clients share. Anything else
    // would be forwarded to a client that would simply reject it.
    validate_effort(p.effort.as_deref())?;
    // Only an API the client names as one it speaks.
    if let Some(api) = p.api.as_deref()
        && !crate::adapters::agent(p.provider)?.apis().contains(&api)
    {
        return Err(ApiError::bad(format!(
            "{} does not speak the {api} API",
            p.provider.label()
        )));
    }
    if let Some(header) = &p.key_header {
        validate_key_header(header)?;
    }
    if p.models.len() > 500
        || p.models
            .iter()
            .any(|id| id.trim().is_empty() || id.len() > 200 || id.chars().any(char::is_control))
    {
        return Err(ApiError::bad("Models must be real model IDs (maximum 500)"));
    }
    if p.model_aliases.len() > 100
        || p.model_aliases.iter().any(|(alias, id)| {
            alias.trim().is_empty()
                || id.trim().is_empty()
                || alias.len() > 120
                || id.len() > 200
                || alias.chars().any(char::is_control)
                || id.chars().any(char::is_control)
        })
    {
        return Err(ApiError::bad(
            "Model aliases must map a short name to a real model ID (maximum 100 mappings)",
        ));
    }
    if p.model
        .as_ref()
        .is_some_and(|s| s.len() > 200 || s.chars().any(char::is_control))
    {
        return Err(ApiError::bad("Invalid model identifier"));
    }
    if let Some(reference) = &p.secret_ref {
        let source = reference
            .strip_prefix("env:")
            .ok_or_else(|| ApiError::bad("Use env:VARIABLE_NAME; never enter a secret value"))?;
        if !source.starts_with("TENDLY_SECRET_")
            || !source
                .bytes()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
        {
            return Err(ApiError::bad(
                "Secret references must use env:TENDLY_SECRET_NAME",
            ));
        }
    }
    Ok(())
}

pub fn private_dir(path: &Path) -> Result<PathBuf, ApiError> {
    fs::create_dir_all(path).map_err(ApiError::internal)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(ApiError::internal)?;
    }
    dunce::canonicalize(path).map_err(ApiError::internal)
}

pub fn validate_proxy(value: &str) -> Result<(), ApiError> {
    let url = Url::parse(value).map_err(|_| ApiError::bad("Proxy must be an HTTP(S) URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || (url.path() != "/" && !url.path().is_empty())
    {
        return Err(ApiError::bad(
            "Use an HTTP(S) proxy without credentials or path. Claude Code does not support SOCKS.",
        ));
    }
    Ok(())
}

pub fn resolve_model(profile: &EndpointProfile) -> Option<String> {
    profile
        .model
        .as_ref()
        .map(|model| profile.model_aliases.get(model).unwrap_or(model).clone())
}

pub(crate) fn write_private(path: &Path, content: &str) -> Result<(), ApiError> {
    use std::io::Write;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(path) {
        Ok(mut file) => file
            .write_all(content.as_bytes())
            .map_err(ApiError::internal),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(ApiError::internal(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_rejects_secret_values_and_embedded_auth() {
        let mut p = EndpointProfile {
            id: uuid::Uuid::new_v4(),
            name: "one".into(),
            provider: ProviderKind::Codex,
            endpoint_url: Some("https://name:secret@example.test".into()),
            model: None,
            permission_mode: "native".into(),
            secret_ref: None,
            proxy_url: None,
            effort: None,
            model_aliases: Default::default(),
            models: Vec::new(),
            api: None,
            key_header: None,
            native_config: None,
            environment: Default::default(),
            created_at: chrono::Utc::now(),
        };
        assert!(validate_profile(&p).is_err());
        p.endpoint_url = Some("https://example.test/v1".into());
        p.secret_ref = Some("env:HOME".into());
        assert!(validate_profile(&p).is_err());
        p.secret_ref = Some("env:TENDLY_SECRET_WORK".into());
        assert!(validate_profile(&p).is_ok());
        // A key header: a header name, and one line with the key once.
        let header = |name: &str, value: &str| {
            Some(KeyHeader {
                name: name.into(),
                value: value.into(),
            })
        };
        for (name, value) in [
            ("x-api-key", "{key}"),
            ("Authorization", "Bearer {key}"),
            ("X-Token", "Token {key}"),
        ] {
            p.key_header = header(name, value);
            assert!(validate_profile(&p).is_ok(), "{name}: {value}");
        }
        for (name, value) in [
            ("X Token", "{key}"),
            ("Host", "{key}"),
            ("X-Token", "fixed"),
            ("X-Token", "{key}{key}"),
            ("X-Token", "{key}\nX-Other: 1"),
            // Pi reads these as a variable and as a command to run.
            ("X-Token", "${key}"),
            ("X-Token", "!{key}"),
        ] {
            p.key_header = header(name, value);
            assert!(validate_profile(&p).is_err(), "{name}: {value:?}");
        }
        // The two any client sends by itself are told apart from the rest.
        p.key_header = header("X-API-Key", "{key}");
        assert!(matches!(key_placement(&p), KeyPlacement::XApiKey));
        p.key_header = header("authorization", "Bearer {key}");
        assert!(matches!(key_placement(&p), KeyPlacement::Bearer));
        p.key_header = header("Authorization", "Token {key}");
        assert!(matches!(key_placement(&p), KeyPlacement::Custom(_)));
        p.key_header = None;
        assert!(matches!(key_placement(&p), KeyPlacement::Protocol));
    }

    #[test]
    fn an_official_account_may_choose_a_model_but_never_where_requests_go() {
        let mut p = EndpointProfile {
            id: uuid::Uuid::new_v4(),
            name: "official".into(),
            provider: ProviderKind::ClaudeCode,
            endpoint_url: None,
            model: None,
            permission_mode: "native".into(),
            secret_ref: None,
            proxy_url: None,
            effort: None,
            model_aliases: Default::default(),
            models: Vec::new(),
            api: None,
            key_header: None,
            native_config: Some(tendly_domain::NativeConfigReference {
                source_id: "claude-default".into(),
                config_dir: std::env::temp_dir()
                    .join("tendly-fixture")
                    .to_string_lossy()
                    .into_owned(),
                config_env: None,
            }),
            environment: Default::default(),
            created_at: chrono::Utc::now(),
        };
        assert!(validate_profile(&p).is_ok());
        // A model and a depth are launch choices the client still resolves
        // against its own account, so an official account may carry them.
        p.model = Some("claude-fable-5-1[1m]".into());
        p.effort = Some("high".into());
        assert!(validate_profile(&p).is_ok());
        // The account writes its proxy here; refusing it left those profiles
        // stored but unvalidatable.
        p.proxy_url = Some("http://192.168.0.253:7890".into());
        assert!(validate_profile(&p).is_ok());
        p.proxy_url = Some("http://user:pass@proxy.test".into());
        assert!(validate_profile(&p).is_err());
        p.proxy_url = None;
        // Where the requests go and who they authenticate as stay the native
        // configuration's, and a second alias table would compete with the
        // client's own mapping.
        p.model_aliases = [("fast".to_string(), "haiku".to_string())].into();
        assert!(validate_profile(&p).is_err());
        p.model_aliases = Default::default();
        p.endpoint_url = Some("https://relay.test/v1".into());
        assert!(validate_profile(&p).is_err());
        p.endpoint_url = None;
        p.secret_ref = Some("env:TENDLY_SECRET_WORK".into());
        assert!(validate_profile(&p).is_err());
        p.secret_ref = None;
        p.key_header = Some(KeyHeader {
            name: "X-Token".into(),
            value: "{key}".into(),
        });
        assert!(validate_profile(&p).is_err());
    }

    #[test]
    fn plan_is_forwarded_only_to_claude_code() {
        let mut p = EndpointProfile {
            id: uuid::Uuid::new_v4(),
            name: "plan".into(),
            provider: ProviderKind::ClaudeCode,
            endpoint_url: None,
            model: None,
            permission_mode: "plan".into(),
            secret_ref: None,
            proxy_url: None,
            effort: None,
            model_aliases: Default::default(),
            models: Vec::new(),
            api: None,
            key_header: None,
            native_config: None,
            environment: Default::default(),
            created_at: chrono::Utc::now(),
        };
        assert!(validate_profile(&p).is_ok());
        p.provider = ProviderKind::Codex;
        assert!(validate_profile(&p).is_err());
    }
}
