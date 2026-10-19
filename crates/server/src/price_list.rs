//! The published price of each model, kept as data rather than code.
//!
//! `data/prices.json` is generated from LiteLLM's community price list by a
//! daily workflow (`scripts/prices/`). A build carries the copy it was built
//! with, so there is always a price list, offline included; the server then
//! fetches the current one from a CDN or the GitCode mirror (reachable from
//! mainland China), keeps it on disk, and checks again once a day. Fetching
//! reads a public file and sends nothing about this host's usage.
//!
//! Whichever list is newer wins: a remote copy older than the one a fresh
//! release carries is ignored. A price the user set wins over both
//! (`usage::Prices`).
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::Duration,
};

const EMBEDDED: &str = include_str!("../../../data/prices.json");
const SCHEMA_VERSION: u32 = 1;
/// Tried in order: a CDN, the GitCode mirror for mainland China, GitHub itself.
/// GitCode's own `/raw/` pages answer with its web app (HTML) rather than the
/// file, so the mirror is read through its API, which serves the bytes as they
/// are and without signing in.
const MIRRORS: [&str; 3] = [
    "https://cdn.jsdelivr.net/gh/wnzzer/tendly@main/data/prices.json",
    "https://api.gitcode.com/api/v5/repos/wnzzer/tendly/raw/data%2Fprices.json?ref=main",
    "https://raw.githubusercontent.com/wnzzer/tendly/main/data/prices.json",
];
const REFRESH_AFTER: Duration = Duration::from_secs(24 * 3600);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
/// The real file is a few dozen kilobytes; anything far larger is not it.
const MOST_BYTES: usize = 2 * 1024 * 1024;

/// Dollars per million tokens. A cache write is priced only where the
/// provider charges for one.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListPrice {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "cacheWrite1h"
    )]
    pub cache_write_1h: Option<f64>,
    /// The most a model takes in, and writes out, in tokens: a fallback for
    /// showing how full a context is, never handed to a client on its own
    /// (a published window can be one the account has not been given).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceList {
    pub schema_version: u32,
    /// RFC 3339; lists compare by it.
    pub updated_at: String,
    pub models: BTreeMap<String, ListPrice>,
}

impl PriceList {
    fn parse(bytes: &[u8]) -> Option<Self> {
        let list: Self = serde_json::from_slice(bytes).ok()?;
        let sane = |value: f64| value.is_finite() && (0.0..=5_000.0).contains(&value);
        let valid = list.schema_version == SCHEMA_VERSION
            && chrono::DateTime::parse_from_rfc3339(&list.updated_at).is_ok()
            && !list.models.is_empty()
            && list.models.values().all(|price| {
                [price.input, price.output, price.cache_read]
                    .into_iter()
                    .chain(price.cache_write)
                    .chain(price.cache_write_1h)
                    .all(sane)
                    && [price.context_window, price.max_output]
                        .into_iter()
                        .flatten()
                        .all(|tokens| (1..=100_000_000).contains(&tokens))
            });
        valid.then_some(list)
    }

    fn newer_than(&self, other: &Self) -> bool {
        let at = |list: &Self| chrono::DateTime::parse_from_rfc3339(&list.updated_at).ok();
        at(self) > at(other)
    }

    /// The price of a model as a client names it in its logs: its own entry,
    /// or the entry it is a dated or rebuilt variant of (`gpt-5.5-codex` is
    /// `gpt-5.5`, `claude-opus-4-1-20250805` is `claude-opus-4-1`). Anything
    /// else after a listed name makes it another model, which has no price.
    pub fn lookup(&self, model: &str) -> Option<ListPrice> {
        let model = normalize(model);
        let mut name = model.as_str();
        loop {
            if let Some(price) = self.models.get(name) {
                return Some(*price);
            }
            let cut = name.rfind('-')?;
            // Only a variant of the same model is priced as it. A gateway
            // that prefixes every model it serves with a Claude name
            // (`claude-fable-5-dd-…`) is not serving Fable 5.
            if !variant_suffix(&model[cut + 1..]) {
                return None;
            }
            name = &name[..cut];
        }
    }
}

/// A tail that names a dated or rebuilt release of the same model, at the
/// same price: `20260901`, `2025-08-07`, `v2`, `latest`, `preview`, `codex`;
/// or the thinking depth a gateway spells into the name (`-thinking`,
/// `-high`), which bills the same tokens at the same rate. `mini`, `nano`,
/// `pro`, `max` and the rest are other models, priced differently.
fn variant_suffix(tail: &str) -> bool {
    tail.split('-').all(|segment| {
        !segment.is_empty() && segment.bytes().all(|b| b.is_ascii_digit())
            || segment
                .strip_prefix('v')
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            || matches!(
                segment,
                "latest"
                    | "preview"
                    | "codex"
                    | "thinking"
                    | "minimal"
                    | "low"
                    | "medium"
                    | "high"
                    | "xhigh"
            )
    })
}

/// `us.anthropic.claude-opus-5-5-20260901-v1:0`, `claude-opus-5-5[1m]` and
/// a gateway's `anthropic/claude-opus-5-5` all become `claude-opus-5-5`; a
/// display name, `DeepSeek V4 Flash`, becomes `deepseek-v4-flash`.
fn normalize(model: &str) -> String {
    let mut name = model
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
        .to_ascii_lowercase();
    if let Some(at) = name.find('[') {
        name.truncate(at);
    }
    if let Some(at) = name.rfind('/') {
        name.drain(..=at);
    }
    for prefix in ["us.", "eu.", "apac.", "global."] {
        if let Some(rest) = name.strip_prefix(prefix) {
            name = rest.to_owned();
        }
    }
    if let Some(rest) = name.strip_prefix("anthropic.") {
        name = rest.to_owned();
    }
    if let Some(at) = name.find(':') {
        name.truncate(at);
    }
    if let Some(rest) = name.strip_suffix("-v1") {
        name = rest.to_owned();
    }
    name
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// The copy this build carries.
    Built,
    /// Fetched from a mirror, possibly on an earlier run.
    Fetched,
}

struct Current {
    list: Arc<PriceList>,
    origin: Origin,
}

fn current() -> &'static RwLock<Current> {
    static CURRENT: std::sync::OnceLock<RwLock<Current>> = std::sync::OnceLock::new();
    CURRENT.get_or_init(|| {
        RwLock::new(Current {
            list: Arc::new(
                PriceList::parse(EMBEDDED.as_bytes()).expect("data/prices.json is valid"),
            ),
            origin: Origin::Built,
        })
    })
}

pub fn list() -> Arc<PriceList> {
    current().read().expect("price list").list.clone()
}

/// When the list in use was published, and where it came from.
pub fn status() -> (String, Origin) {
    let current = current().read().expect("price list");
    (current.list.updated_at.clone(), current.origin)
}

/// Use `list` if it is newer than the one in use.
fn adopt(list: PriceList) -> bool {
    let mut current = current().write().expect("price list");
    if !list.newer_than(&current.list) {
        return false;
    }
    *current = Current {
        list: Arc::new(list),
        origin: Origin::Fetched,
    };
    true
}

fn cache_path(state_dir: &Path) -> PathBuf {
    state_dir.join("price-list.json")
}

async fn fetch() -> Option<(Vec<u8>, PriceList)> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("tendly/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()?;
    for url in MIRRORS {
        let Ok(response) = client.get(url).send().await else {
            continue;
        };
        if !response.status().is_success() {
            continue;
        }
        let Ok(bytes) = response.bytes().await else {
            continue;
        };
        if bytes.len() > MOST_BYTES {
            continue;
        }
        if let Some(list) = PriceList::parse(&bytes) {
            return Some((bytes.to_vec(), list));
        }
        tracing::debug!(url, "price list from this mirror is not valid");
    }
    None
}

/// Load what an earlier run fetched, then keep the list current. Failures
/// are quiet: the list in use stays, and the next check tries again.
pub fn spawn_refresh(state_dir: PathBuf) {
    tokio::spawn(async move {
        let path = cache_path(&state_dir);
        let mut fetched_at = None;
        if let Ok(bytes) = std::fs::read(&path)
            && let Some(list) = PriceList::parse(&bytes)
        {
            adopt(list);
            fetched_at = std::fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .ok();
        }
        loop {
            let due = fetched_at
                .and_then(|at: std::time::SystemTime| at.elapsed().ok())
                .is_none_or(|age| age >= REFRESH_AFTER);
            if due {
                match fetch().await {
                    Some((bytes, list)) => {
                        fetched_at = Some(std::time::SystemTime::now());
                        if let Err(error) = std::fs::write(&path, &bytes) {
                            tracing::debug!(%error, "could not keep the price list");
                        }
                        if adopt(list) {
                            tracing::info!("price list updated");
                        }
                    }
                    None => tracing::debug!("no mirror answered with a price list"),
                }
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(updated_at: &str, models: &[&str]) -> PriceList {
        PriceList {
            schema_version: SCHEMA_VERSION,
            updated_at: updated_at.into(),
            models: models
                .iter()
                .map(|name| {
                    (
                        (*name).to_owned(),
                        ListPrice {
                            input: 1.0,
                            output: 2.0,
                            cache_read: 0.1,
                            cache_write: None,
                            cache_write_1h: None,
                            context_window: None,
                            max_output: None,
                        },
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn the_built_list_parses_and_prices_both_clients() {
        let built = PriceList::parse(EMBEDDED.as_bytes()).expect("data/prices.json");
        let opus = built.lookup("claude-opus-5-5").expect("Claude");
        assert!(
            opus.cache_write.is_some(),
            "Anthropic charges for cache writes"
        );
        let gpt = built.lookup("gpt-5.5").expect("GPT");
        assert_eq!(gpt.cache_write, None, "OpenAI does not");
    }

    #[test]
    fn the_shipped_list_prices_variants_but_not_a_gateways_own_models() {
        let built = PriceList::parse(EMBEDDED.as_bytes()).expect("data/prices.json");
        assert!(built.lookup("claude-sonnet-4-5-20250929").is_some());
        assert!(built.lookup("claude-3-5-haiku-20241022").is_some());
        // Other makers' models, at their own prices.
        for model in ["deepseek-v4-flash", "glm-5.3-flash", "gemini-3.8-flash"] {
            assert!(built.lookup(model).is_some(), "{model}");
        }
        for gateway in [
            "claude-fable-5-dd-lacol-b9-5.3newq",
            "claude-fable-5-dd-lacol-inim-gnil",
            "claude-fable-5-dd-3.5-MLG/gro-iaz",
        ] {
            assert_eq!(built.lookup(gateway), None, "{gateway}");
        }
    }

    #[test]
    fn a_model_finds_its_family_however_a_client_spells_it() {
        let prices = list(
            "2026-10-08T00:00:00Z",
            &["claude-opus-5-5", "gpt-5.5", "gpt-5"],
        );
        for (name, expected) in [
            ("claude-opus-5-5", true),
            ("claude-opus-5-5[1m]", true),
            ("Claude-Opus-5-5-20260901", true),
            ("us.anthropic.claude-opus-5-5-20260901-v1:0", true),
            ("gpt-5.5-codex", true),
            ("gpt-5-2025-08-07", true),
            ("gpt-5.5-latest", true),
            // A thinking depth a gateway spells into the name.
            ("claude-opus-5-5-thinking", true),
            ("gpt-5.5-high", true),
            ("gpt-5.5-xhigh", true),
            // A gateway's maker prefix, and a display name.
            ("anthropic/claude-opus-5-5", true),
            ("openrouter/openai/gpt-5.5", true),
            ("GPT 5.5", true),
            // A smaller or larger model is not priced as its family.
            ("gpt-5-mini", false),
            ("gpt-5.5-pro", false),
            ("gpt-5.5-max", false),
            // A gateway's own models behind a Claude-looking prefix.
            ("claude-opus-5-5-dd-lacol-b9-5.3newq", false),
            // A different family is not the nearest one.
            ("claude-opus-5", false),
            ("gpt-6", false),
            ("glm-5.1", false),
            ("", false),
        ] {
            assert_eq!(prices.lookup(name).is_some(), expected, "{name}");
        }
        assert_eq!(normalize("gpt-5.5-codex"), "gpt-5.5-codex");
    }

    #[test]
    fn a_list_must_be_this_schema_dated_and_sane() {
        let good = list("2026-10-08T00:00:00Z", &["gpt-5.5"]);
        let bytes = |list: &PriceList| serde_json::to_vec(list).unwrap();
        assert!(PriceList::parse(&bytes(&good)).is_some());
        let mut later = good.clone();
        later.schema_version = 2;
        assert!(PriceList::parse(&bytes(&later)).is_none(), "a later schema");
        let mut undated = good.clone();
        undated.updated_at = "yesterday".into();
        assert!(PriceList::parse(&bytes(&undated)).is_none());
        let mut absurd = good.clone();
        absurd.models.get_mut("gpt-5.5").unwrap().output = 1e9;
        assert!(PriceList::parse(&bytes(&absurd)).is_none());
        assert!(PriceList::parse(&bytes(&list("2026-10-08T00:00:00Z", &[]))).is_none());
        assert!(PriceList::parse(b"<html>").is_none());
    }

    #[test]
    fn only_a_newer_list_replaces_the_one_in_use() {
        let older = list("2026-01-01T00:00:00Z", &["a"]);
        let newer = list("2026-10-08T00:00:00Z", &["a"]);
        assert!(newer.newer_than(&older));
        assert!(!older.newer_than(&newer));
        assert!(!newer.newer_than(&newer.clone()));
    }
}
