//! What differs between the agent clients Tendly drives, in one place.
//!
//! Everything the server does with a client goes through its adapter: where
//! its binary comes from, how a launch reaches an endpoint, which permission
//! modes it offers, how its model list is read, how its account signs in, and
//! where its transcripts are and how they read. The rest of the server asks
//! the adapter and never names a client, so adding one is a file here and an
//! entry in `adapter`.
use crate::{ApiError, AppState};
use std::{collections::BTreeMap, path::Path};
use tendly_domain::{EndpointProfile, ProviderKind};

mod claude_code;
mod codex;
mod pi;

/// A launch Tendly configures itself, as opposed to one that defers to a
/// native configuration: the endpoint, credential and permission choices of a
/// profile, turned into this client's flags, variables and files.
pub struct EndpointLaunch<'a> {
    pub state: &'a AppState,
    pub profile: Option<&'a EndpointProfile>,
    /// The profile's permission mode, `native` without a profile.
    pub mode: &'a str,
    /// The resolved credential, never the reference.
    pub secret: Option<String>,
    /// The session's private configuration home.
    pub home: &'a Path,
    pub args: &'a mut Vec<String>,
    pub environment: &'a mut BTreeMap<String, String>,
}

/// One reply in a client's model-list handshake, and what it calls for.
pub enum CatalogStep {
    /// Not the reply that matters; keep reading.
    Wait,
    /// Write this to the client, then keep reading.
    Send(&'static str),
    /// The handshake is over, with the list or why there is none.
    Done(Result<crate::model_catalog::ModelCatalog, ApiError>),
}

/// Which account operations a client's own tooling supports.
#[derive(Debug, Clone, Copy, Default)]
pub struct AccountOperations {
    /// Sign in, refresh the token and sign out through Tendly.
    pub managed_login: bool,
    /// Read subscription usage on request.
    pub usage: bool,
}

pub trait ClientAdapter: Sync {
    fn kind(&self) -> ProviderKind;

    /// The npm package that installs it, and the command it installs.
    fn npm_package(&self) -> &'static str;
    fn command(&self) -> &'static str;
    /// The server variable that names a binary to use instead.
    fn bin_override(&self) -> &'static str;
    /// The variable that points it at a configuration home.
    fn config_key(&self) -> &'static str;
    /// Its configuration home under the user's own home directory.
    fn home_dir(&self) -> &'static str;
    /// Its default history source: the id sessions record, and the server
    /// variable that names a different directory to read.
    fn history_source(&self) -> (&'static str, &'static str);
    /// Its place among the host's configurations and history sources, lowest
    /// first. The first available one is what those dialogs preselect.
    fn source_order(&self) -> u8;
    /// Names in a workspace that hold its configuration, kept out of listings.
    fn protected_names(&self) -> &'static [&'static str];
    /// Host variables that would route a launch Tendly configures to some
    /// other endpoint, account or model; removed before its own are set.
    fn inherited_keys(&self) -> &'static [&'static str];
    /// Prefixes of every host variable an isolated account launch drops.
    fn account_prefixes(&self) -> &'static [&'static str];

    /// The flags that reopen one of its conversations.
    fn resume_args(&self, native_id: &str) -> Vec<String>;
    /// The flags that name a new conversation, where it keeps a title.
    fn title_args(&self, _title: &str) -> Vec<String> {
        Vec::new()
    }
    /// The flags that choose a model, for a launch with this profile.
    fn model_args(&self, _profile: &EndpointProfile, model: &str) -> Vec<String> {
        vec!["--model".into(), model.to_owned()]
    }
    /// The flags that set a reasoning effort.
    fn effort_args(&self, effort: &str) -> Vec<String>;
    /// Whether a profile may launch it in plan mode.
    fn plan_mode(&self) -> bool;
    /// The permission modes a chat session can switch between.
    fn permission_modes(&self) -> &'static [&'static str];
    /// What every launch adds for this client, given the configuration home
    /// it reads (`None` for the host default).
    fn launch_extras(
        &self,
        _state: &AppState,
        _config_env: Option<&str>,
    ) -> (Vec<String>, BTreeMap<String, String>) {
        (Vec::new(), BTreeMap::new())
    }
    /// A launch Tendly configures: endpoint, credential, permission mode,
    /// effort and context window, as this client takes them.
    fn configure(&self, launch: EndpointLaunch<'_>) -> Result<(), ApiError>;

    /// The APIs a profile's endpoint can speak to it, for a client that
    /// speaks more than one; a profile names one of these or none.
    fn apis(&self) -> &'static [&'static str] {
        &[]
    }
    /// Its vendor's API, for a profile that names no endpoint.
    fn default_endpoint(&self) -> &'static str;
    /// Whether this profile's API base URL leaves out the `/v1` the model
    /// list is under.
    fn versioned_api(&self, _profile: &EndpointProfile) -> bool {
        false
    }
    /// A model-list request with this profile's API credential and headers.
    fn authorize(
        &self,
        profile: &EndpointProfile,
        request: reqwest::RequestBuilder,
        secret: Option<&str>,
    ) -> reqwest::RequestBuilder;
    /// Whether a launch whose key goes in the profile's own header still puts
    /// a stand-in in the client's, because the client sends nothing without
    /// one. A check sends the same, so it passes only where the launch would.
    fn key_stand_in(&self, _profile: &EndpointProfile) -> bool {
        false
    }
    /// Whether, with neither endpoint nor key, the client itself is asked
    /// which models its default account offers.
    fn lists_models_unconfigured(&self) -> bool {
        false
    }
    /// The one request that would tell whether this model runs: the path under
    /// the endpoint and the JSON body, or `None` for a client that can only
    /// answer by being started. It asks for a single token and is sent only
    /// from an explicit model check (model_probe.rs), never from discovery.
    ///
    /// The wire protocol comes with it: a client with a choice of API (Pi)
    /// speaks whichever the profile names, and the interface says which one a
    /// check used, because the same model can answer on one and fail on
    /// another.
    fn probe_call(
        &self,
        _profile: &EndpointProfile,
        _model: &str,
    ) -> Option<(String, serde_json::Value)> {
        None
    }
    /// The wire protocol this client speaks to a profile's endpoint, as a
    /// value the interface knows (`anthropic-messages`, `openai-responses`,
    /// `openai-completions`, `google-generative-ai`).
    fn probe_api(&self, _profile: &EndpointProfile) -> &'static str {
        ""
    }
    /// The arguments that start its read-only model-list handshake, the extra
    /// ones for an unconfigured ask, the line that opens it, and how each
    /// reply moves it on (`stage` starts at zero and is the adapter's own).
    fn catalog_args(&self) -> &'static [&'static str];
    fn unconfigured_catalog_args(&self) -> &'static [&'static str] {
        &[]
    }
    fn catalog_opening(&self) -> &'static str;
    fn catalog_step(&self, packet: &serde_json::Value, stage: &mut u8) -> CatalogStep;

    /// The account operations its tooling supports.
    fn account_operations(&self) -> AccountOperations;
    /// The command a person runs to sign an account in, where Tendly does
    /// not sign in for them; `login_home` is where that sign-in must land.
    fn login_guidance(&self, _login_home: Option<&str>) -> Option<String> {
        None
    }
    /// Files a new account's own directory starts with.
    fn account_files(&self) -> &'static [(&'static str, &'static [u8])] {
        &[]
    }

    /// Where under a configuration home its transcripts are, and how deep.
    fn transcripts(&self) -> &'static [(&'static str, usize)];
    /// What one transcript line adds to the calls read from its file.
    fn read_transcript_line(
        &self,
        line: &str,
        source: &crate::usage::Source,
        state: &mut crate::usage::FileState,
    );
    /// What a cache write costs per million tokens, for five minutes and for
    /// an hour, when its price lists none: plain input unless its vendor
    /// charges more to cache.
    fn cache_write_rates(&self, input: f64) -> (f64, f64) {
        (input, input)
    }
    /// Give a launch Tendly's MCP server (agent.rs): `program` run as
    /// `program mcp` with `identity` as its environment, through this launch's
    /// flags only, never the person's own client configuration.
    fn equip_tools(
        &self,
        state: &AppState,
        session: tendly_domain::SessionId,
        program: &str,
        identity: &[(String, String)],
        args: &mut Vec<String>,
    ) -> Result<(), ApiError>;

    /// The newest rate-limit reading its own logs under `home` carry, for an
    /// account nobody has asked about.
    fn logged_limits(&self, _home: &Path) -> Option<crate::usage::LoggedLimits> {
        None
    }
}

static CLAUDE_CODE: claude_code::ClaudeCode = claude_code::ClaudeCode;
static CODEX: codex::Codex = codex::Codex;
static PI: pi::Pi = pi::Pi;

/// The adapter for an agent client; a plain terminal has none.
/// A request with the profile's key where its endpoint reads it: the
/// client's own header, or the one the profile names, which replaces any of
/// that name the client set.
pub fn authorize(
    client: &dyn ClientAdapter,
    profile: &EndpointProfile,
    request: reqwest::RequestBuilder,
    secret: Option<&str>,
) -> reqwest::RequestBuilder {
    let (Some(header), Some(key)) = (profile.key_header.as_ref(), secret) else {
        return client.authorize(profile, request, secret);
    };
    let stand_in = client
        .key_stand_in(profile)
        .then_some(crate::providers::KEY_STAND_IN);
    let request = client.authorize(profile, request, stand_in);
    let value = crate::providers::key_value(header, key);
    match (
        reqwest::header::HeaderName::from_bytes(header.name.as_bytes()),
        reqwest::header::HeaderValue::from_str(&value),
    ) {
        (Ok(name), Ok(mut value)) => {
            value.set_sensitive(true);
            request.headers([(name, value)].into_iter().collect())
        }
        // Saved profiles are validated; a draft with a broken header is
        // simply sent without it, and the endpoint says so.
        _ => request,
    }
}

pub fn adapter(kind: ProviderKind) -> Option<&'static dyn ClientAdapter> {
    match kind {
        ProviderKind::ClaudeCode => Some(&CLAUDE_CODE),
        ProviderKind::Codex => Some(&CODEX),
        ProviderKind::Pi => Some(&PI),
        ProviderKind::Terminal => None,
    }
}

/// The adapter, or the error a terminal gets where only a client makes sense.
pub fn agent(kind: ProviderKind) -> Result<&'static dyn ClientAdapter, ApiError> {
    adapter(kind).ok_or_else(|| ApiError::bad("A terminal session has no agent client"))
}

/// Every agent client, in the order the interface offers them.
pub fn all() -> impl Iterator<Item = &'static dyn ClientAdapter> {
    ProviderKind::AGENTS.into_iter().filter_map(adapter)
}

/// Quote a string as TOML and JSON both write one.
pub(crate) fn encode(value: &str) -> String {
    serde_json::to_string(value).expect("string serialization")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_agent_has_an_adapter_that_names_itself() {
        for kind in ProviderKind::AGENTS {
            assert_eq!(adapter(kind).map(|client| client.kind()), Some(kind));
        }
        assert!(adapter(ProviderKind::Terminal).is_none());
        let keys: std::collections::HashSet<_> = all().map(|client| client.config_key()).collect();
        assert_eq!(
            keys.len(),
            ProviderKind::AGENTS.len(),
            "each reads its own home"
        );
    }
}
