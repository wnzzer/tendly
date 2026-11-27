//! Gemini, through Google's Antigravity CLI (`agy`): signed in with a Google
//! account, or run against the Gemini API with a key.
//!
//! agy has no variable naming its configuration home: it reads `$HOME/.gemini`
//! and nothing else, except `agy acp`, which takes `--gemini-dir`. Tendly
//! records a session's home in [`GEMINI_DIR`] like any client's, and a launch
//! that must find it there gets a home of its own whose `.gemini` is that
//! directory and whose every other entry is the person's own ([`home_for`]),
//! so the commands agy runs keep their git, ssh and tool configuration. The
//! chat (gemini_chat.rs) runs `agy acp --gemini-dir` instead and keeps HOME.
//!
//! agy stores its conversations in SQLite databases of its own encoding, so
//! the usage page does not count its calls; a chat reports each turn's tokens
//! as it ends.
use super::{AccountOperations, CatalogStep, ClientAdapter, EndpointLaunch};
use crate::model_catalog::ModelCatalog;
use crate::usage::{FileState, Source};
use crate::{ApiError, AppState};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use tendly_domain::{EndpointProfile, ProviderKind};

pub struct Gemini;

/// Where Tendly records the configuration home (`.gemini`) a launch uses.
pub const GEMINI_DIR: &str = "TENDLY_GEMINI_DIR";

/// The reasoning levels agy names its model variants with: `gemini-4-neon-high`
/// is Gemini 4 Neon at high.
pub const EFFORTS: [&str; 4] = ["low", "medium", "high", "xhigh"];

/// A home for a launch that has to find `gemini_dir` as `$HOME/.gemini`: its
/// `.gemini` is that directory, and each other entry of the person's own home
/// is linked in, so a command agy runs sees the same configuration it would
/// anywhere else. Entries added to the real home later are linked on the next
/// launch. `None` where no link can be made.
pub fn home_for(state_dir: &Path, gemini_dir: &Path) -> Option<PathBuf> {
    let digest = Sha256::digest(gemini_dir.to_string_lossy().as_bytes());
    let name: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
    let home = state_dir.join("clients").join("gemini-homes").join(name);
    std::fs::create_dir_all(&home).ok()?;
    let own = home.join(".gemini");
    if std::fs::read_link(&own).ok().as_deref() != Some(gemini_dir) {
        let _ = std::fs::remove_file(&own);
        link(gemini_dir, &own).ok()?;
    }
    let real = std::env::var_os("HOME").map(PathBuf::from)?;
    if let Ok(entries) = std::fs::read_dir(&real) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let target = home.join(&name);
            if name == ".gemini" || target.symlink_metadata().is_ok() {
                continue;
            }
            let _ = link(&entry.path(), &target);
        }
    }
    Some(home)
}

#[cfg(unix)]
fn link(target: &Path, at: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, at)
}
#[cfg(not(unix))]
fn link(_target: &Path, _at: &Path) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

/// Whether a launch with this profile reaches the Gemini API with a key or an
/// endpoint of its own, rather than a signed-in account.
fn configures_endpoint(profile: &EndpointProfile) -> bool {
    profile.endpoint_url.is_some() || profile.secret_ref.is_some()
}

/// `agy models` prints one model per line, `id<TAB>name`, each reasoning level
/// a model of its own (`gemini-4-neon-high<TAB>Gemini 4 Neon (High)`). A model
/// listed at two or more levels becomes one model with those levels; one
/// listed at a single level keeps its own ID, as agy's own model menu does.
pub fn parse_model_list(output: &str) -> Vec<Value> {
    let rows: Vec<(&str, &str)> = output
        .lines()
        .filter_map(|line| {
            let (id, name) = line.split_once('\t')?;
            let id = id.trim();
            (!id.is_empty() && !id.contains(char::is_whitespace)).then_some((id, name.trim()))
        })
        .collect();
    fn split(id: &str) -> Option<(&str, &'static str)> {
        EFFORTS
            .iter()
            .find_map(|effort| Some((id.strip_suffix(&format!("-{effort}"))?, *effort)))
    }
    let mut models: Vec<(String, String, Vec<&str>)> = Vec::new();
    for (id, name) in &rows {
        let variants = split(id).filter(|(base, _)| {
            rows.iter()
                .filter(|(other, _)| split(other).is_some_and(|(b, _)| b == *base))
                .count()
                >= 2
        });
        let (base, effort) = match variants {
            Some((base, effort)) => (base, Some(effort)),
            None => (*id, None),
        };
        let shown = match effort {
            // "Gemini 4 Neon (High)" names the level; the model is the rest.
            Some(_) => name
                .rsplit_once(" (")
                .map_or(*name, |(model, _)| model)
                .to_owned(),
            None => (*name).to_owned(),
        };
        match models.iter_mut().find(|(id, ..)| id == base) {
            Some((.., efforts)) => efforts.extend(effort),
            None => models.push((base.to_owned(), shown, effort.into_iter().collect())),
        }
    }
    models
        .into_iter()
        .map(|(id, name, mut efforts)| {
            efforts.sort_by_key(|effort| EFFORTS.iter().position(|e| e == effort));
            json!({ "id": id, "display_name": name, "supportedReasoningEfforts": efforts })
        })
        .collect()
}

/// The model agy runs a new conversation on, as `agy models` lists it first,
/// with its level where the list gives the model several.
pub fn listed_default(output: &str) -> Option<(String, Option<String>)> {
    let rows = parse_model_list(output);
    let first = output.lines().find_map(|line| {
        let (id, _) = line.split_once('\t')?;
        let id = id.trim();
        (!id.is_empty() && !id.contains(char::is_whitespace)).then_some(id)
    })?;
    rows.iter().find_map(|row| {
        let base = row["id"].as_str()?;
        if base == first {
            return Some((base.to_owned(), None));
        }
        let level = first.strip_prefix(base)?.strip_prefix('-')?;
        let offered = row["supportedReasoningEfforts"].as_array()?;
        offered
            .iter()
            .any(|e| e.as_str() == Some(level))
            .then(|| (base.to_owned(), Some(level.to_owned())))
    })
}

impl ClientAdapter for Gemini {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Gemini
    }
    // agy installs from Google's own script, not npm.
    fn npm_package(&self) -> &'static str {
        ""
    }
    fn command(&self) -> &'static str {
        "agy"
    }
    fn bin_override(&self) -> &'static str {
        "TENDLY_GEMINI_BIN"
    }
    fn config_key(&self) -> &'static str {
        GEMINI_DIR
    }
    fn home_dir(&self) -> &'static str {
        ".gemini"
    }
    fn history_source(&self) -> (&'static str, &'static str) {
        ("gemini-default", "TENDLY_GEMINI_HISTORY_DIR")
    }
    fn source_order(&self) -> u8 {
        3
    }
    fn protected_names(&self) -> &'static [&'static str] {
        &[".gemini"]
    }
    // A key or endpoint from the host would take over an account launch.
    fn inherited_keys(&self) -> &'static [&'static str] {
        &[
            "GEMINI_API_KEY",
            "GOOGLE_API_KEY",
            "GOOGLE_GEMINI_BASE_URL",
            "AGY_ADC_AUTH",
        ]
    }
    fn account_prefixes(&self) -> &'static [&'static str] {
        &["GEMINI_", "GOOGLE_GEMINI_", "AGY_", "ANTIGRAVITY_"]
    }
    fn resume_args(&self, native_id: &str) -> Vec<String> {
        vec!["--conversation".into(), native_id.to_owned()]
    }
    fn effort_args(&self, effort: &str) -> Vec<String> {
        vec!["--effort".into(), effort.to_owned()]
    }
    fn plan_mode(&self) -> bool {
        true
    }
    // `auto` approves what agy judges safe; `danger` is everything, which a
    // chat answers itself (gemini_chat.rs).
    fn permission_modes(&self) -> &'static [&'static str] {
        &["ask", "plan", "accept_edits", "danger"]
    }
    /// A launch that reads an account's home finds it as `$HOME/.gemini`.
    fn launch_extras(
        &self,
        state: &AppState,
        config_env: Option<&str>,
    ) -> (Vec<String>, BTreeMap<String, String>) {
        let mut environment = BTreeMap::new();
        if let Some(home) = config_env
            .filter(|dir| !dir.is_empty())
            .and_then(|dir| home_for(&state.state_dir, Path::new(dir)))
        {
            environment.insert("HOME".into(), home.to_string_lossy().into_owned());
        }
        (Vec::new(), environment)
    }
    /// The Gemini API with the profile's key, at its endpoint if it names one,
    /// in the session's own home, where agy is told to use the API rather
    /// than a sign-in.
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
        if let Some(effort) = profile.and_then(|p| p.effort.as_deref()) {
            args.extend(self.effort_args(effort));
        }
        match mode {
            "trusted" => args.extend(["--mode".into(), "accept-edits".into()]),
            "plan" => args.extend(["--mode".into(), "plan".into()]),
            _ => {}
        }
        if let Some(own) = home_for(&state.state_dir, home) {
            environment.insert("HOME".into(), own.to_string_lossy().into_owned());
        }
        let Some(profile) = profile.filter(|p| configures_endpoint(p)) else {
            return Ok(());
        };
        if let Some(url) = &profile.endpoint_url {
            environment.insert("GOOGLE_GEMINI_BASE_URL".into(), url.clone());
        }
        if let Some(secret) = secret {
            environment.insert("GEMINI_API_KEY".into(), secret);
        }
        let settings = home.join("antigravity-cli");
        std::fs::create_dir_all(&settings).map_err(ApiError::internal)?;
        crate::providers::write_private(
            &settings.join("settings.json"),
            &json!({ "modelProvider": "gemini" }).to_string(),
        )
    }

    fn default_endpoint(&self) -> &'static str {
        "https://generativelanguage.googleapis.com"
    }
    // The model list is under `/v1/models` on the API's bare host.
    fn versioned_api(&self, _profile: &EndpointProfile) -> bool {
        true
    }
    fn authorize(
        &self,
        _profile: &EndpointProfile,
        request: reqwest::RequestBuilder,
        secret: Option<&str>,
    ) -> reqwest::RequestBuilder {
        match secret {
            Some(key) => request.header("x-goog-api-key", key),
            None => request,
        }
    }
    fn probe_api(&self, _profile: &EndpointProfile) -> &'static str {
        "google-generative-ai"
    }
    fn probe_call(&self, _profile: &EndpointProfile, model: &str) -> Option<(String, Value)> {
        Some((
            format!("/v1beta/models/{model}:generateContent"),
            json!({
                "contents": [{ "role": "user", "parts": [{ "text": "ping" }] }],
                "generationConfig": { "maxOutputTokens": 1 },
            }),
        ))
    }
    // `agy models` prints the list and exits; nothing is written to it.
    fn catalog_args(&self) -> &'static [&'static str] {
        &["models"]
    }
    fn catalog_opening(&self) -> &'static str {
        ""
    }
    fn catalog_step(&self, _packet: &Value, _stage: &mut u8) -> CatalogStep {
        CatalogStep::Wait
    }
    fn catalog_text(&self, output: &str) -> Option<Result<ModelCatalog, ApiError>> {
        let rows = parse_model_list(output);
        Some(if rows.is_empty() {
            Err(ApiError::bad(
                "agy listed no models. Sign this account in to Google first.",
            ))
        } else {
            crate::model_catalog::parse_models(json!({ "data": rows }), "agy://models".into())
        })
    }

    // Sign-in happens in agy itself; Tendly reads who is signed in and the
    // account's quota (account-gemini.mjs).
    fn account_operations(&self) -> AccountOperations {
        AccountOperations {
            managed_login: false,
            usage: true,
        }
    }
    fn login_guidance(&self, login_home: Option<&str>) -> Option<String> {
        Some(match login_home {
            Some(home) => format!(
                "agy acp --login --gemini-dir '{}'",
                home.replace('\'', "'\\''")
            ),
            None => "agy   # then sign in with Google".into(),
        })
    }

    // Only a chat can be given Tendly's tools: `agy acp` takes MCP servers
    // with each session (gemini_chat.rs), while agy's terminal reads them
    // only from its own configuration, which Tendly never writes.
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
        &[]
    }
    fn read_transcript_line(&self, _line: &str, _source: &Source, _state: &mut FileState) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_listed_at_several_levels_become_one_model_with_those_levels() {
        let output = "Fetching available models...\n\
            gemini-4-neon-high\tGemini 4 Neon (High)\n\
            gemini-4-neon-xhigh\tGemini 4 Neon (XHigh)\n\
            gemini-4-neon-low\tGemini 4 Neon (Low)\n\
            gemini-pro-orchestrator\tOrchestrator (Carbon)\n\
            claude-opus-5-5-medium\tClaude Opus 5.5 (Medium)\n\
            claude-opus-5-5-high\tClaude Opus 5.5 (High)\n\
            gpt-oss-120b-medium\tGPT-OSS 120B (Medium)\n";
        let rows = parse_model_list(output);
        let summary: Vec<(String, String, Vec<String>)> = rows
            .iter()
            .map(|row| {
                (
                    row["id"].as_str().unwrap().to_owned(),
                    row["display_name"].as_str().unwrap().to_owned(),
                    serde_json::from_value(row["supportedReasoningEfforts"].clone()).unwrap(),
                )
            })
            .collect();
        let efforts = |list: &[&str]| list.iter().map(|e| (*e).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            summary,
            [
                (
                    "gemini-4-neon".into(),
                    "Gemini 4 Neon".into(),
                    efforts(&["low", "high", "xhigh"])
                ),
                (
                    "gemini-pro-orchestrator".into(),
                    "Orchestrator (Carbon)".into(),
                    vec![]
                ),
                (
                    "claude-opus-5-5".into(),
                    "Claude Opus 5.5".into(),
                    efforts(&["medium", "high"])
                ),
                (
                    "gpt-oss-120b-medium".into(),
                    "GPT-OSS 120B (Medium)".into(),
                    vec![]
                ),
            ]
        );
    }

    #[test]
    fn the_default_is_the_first_model_listed_at_its_level() {
        let output = "Fetching available models...\n\
            gemini-4-neon-high\tGemini 4 Neon (High)\n\
            gemini-4-neon-low\tGemini 4 Neon (Low)\n";
        assert_eq!(
            listed_default(output),
            Some(("gemini-4-neon".into(), Some("high".into())))
        );
        assert_eq!(
            listed_default("gpt-oss-120b-medium\tGPT-OSS\n"),
            Some(("gpt-oss-120b-medium".into(), None))
        );
        assert_eq!(listed_default("Error: sign in\n"), None);
    }

    #[test]
    fn a_launch_home_holds_the_gemini_directory_and_links_the_rest() {
        let root = std::env::temp_dir().join(format!("tendly-gemini-{}", uuid::Uuid::new_v4()));
        let gemini = root.join("account");
        std::fs::create_dir_all(&gemini).unwrap();
        let home = home_for(&root, &gemini);
        if let Some(home) = &home {
            assert_eq!(&std::fs::read_link(home.join(".gemini")).unwrap(), &gemini);
            // The same directory always gets the same home.
            assert_eq!(home_for(&root, &gemini).as_ref(), Some(home));
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
