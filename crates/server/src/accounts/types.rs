//! The stored account record, its proxy setting, and the views the API returns.

use super::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AccountRecord {
    pub(super) id: Uuid,
    pub(super) name: String,
    pub(super) provider: ProviderKind,
    pub(super) profile_id: Uuid,
    /// Optional live reference to an existing native client home. The account
    /// directory still stores only Tendly metadata; credentials stay in
    /// this referenced directory and are never copied.
    #[serde(default)]
    pub(super) native_config: Option<NativeConfigReference>,
    /// Proxy for this account's own operations — sign-in and the usage check.
    /// Session traffic is separate and does not use it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) proxy_url: Option<String>,
}

/// Same rule as a session proxy, so one account setting can be carried to its
/// endpoint profile unchanged. SOCKS stays rejected because the native Claude
/// client does not support it, and credentials are rejected because this URL is
/// displayed in the account page.
pub(super) fn validated_proxy(value: Option<&str>) -> Result<Option<String>> {
    let Some(raw) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if raw.len() > 400 || raw.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(ApiError::bad("Invalid proxy URL"));
    }
    crate::providers::validate_proxy(raw)?;
    Ok(Some(raw.to_owned()))
}
/// Node's `--use-env-proxy` reads the lowercase names; the child CLIs read the
/// uppercase ones. Both are set so one configuration covers fetch and clients.
pub(super) fn proxy_environment(proxy: &str) -> Vec<(&'static str, String)> {
    [
        "http_proxy",
        "https_proxy",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "all_proxy",
    ]
    .into_iter()
    .map(|key| (key, proxy.to_owned()))
    .collect()
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Capabilities {
    pub login: bool,
    pub refresh_token: bool,
    pub quota: bool,
    pub reset_quota: bool,
    pub logout: bool,
    /// A user-triggered usage check exists without making Tendly the OAuth
    /// owner. Claude usage is read from the official native credential context
    /// or the optional local status-line capture, never polled automatically.
    #[serde(default)]
    pub usage: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LoginView {
    pub url: String,
    pub id: Option<String>,
    pub user_code: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LimitWindow {
    pub used_percent: f64,
    pub window_minutes: Option<f64>,
    pub resets_at: Option<serde_json::Value>,
    /// Friendly names are attached only when the native client reports an
    /// exact known duration. `primary`/`secondary` remain the source keys.
    pub window_kind: Option<WindowKind>,
    pub mapping_basis: Option<WindowMappingBasis>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowKind {
    FiveHour,
    Weekly,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowMappingBasis {
    DurationMinutes,
    /// The native client named the window itself (Claude status line payload).
    StatusLineWindow,
    /// The official Claude OAuth usage endpoint named the window.
    OAuthUsage,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Limits {
    pub primary: Option<LimitWindow>,
    pub secondary: Option<LimitWindow>,
    pub reset_credits: Option<u64>,
    /// Provenance is kept separate from the count so an unavailable or
    /// malformed native response is never represented as zero credits.
    pub reset_credits_source: Option<ResetCreditsSource>,
    /// Available credits the official client itemised, soonest expiry first.
    /// The backend may cap the list below `reset_credits`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_credit_details: Option<Vec<ResetCreditDetail>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResetCreditDetail {
    /// Unix seconds; `None` means the credit does not expire.
    pub expires_at: Option<f64>,
    /// Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granted_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}
/// Where a Claude usage figure came from. Queries are explicit and the value
/// remains a bounded display snapshot; it can be absent, stale, or older than
/// a reset.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageView {
    pub availability: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captured_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Present when the provider asked us to wait before querying again. It is
    /// a hint for the page, never a timer Tendly acts on by itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_seconds: Option<u64>,
    /// Set when a transient failure kept the figures from the previous check
    /// rather than blanking them. It stamps when those figures were read, so
    /// the page never presents stale numbers as current.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retained_checked_at: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResetCreditsSource {
    Official,
}
#[derive(Clone, Debug, Serialize)]
pub struct AccountView {
    pub id: Uuid,
    pub name: String,
    pub provider: ProviderKind,
    pub profile_id: Uuid,
    /// Source id is exposed for UI provenance only (never credential data).
    pub native_source_id: Option<String>,
    pub storage_path: String,
    /// True when this account points at a configuration the host already had,
    /// rather than one of its own. Its sign-in is then whatever that directory
    /// currently holds, so anything else editing it — another tool switching
    /// profiles, a native re-login — changes this account too. A user cannot
    /// infer that from a path, and being surprised by it is how an account
    /// stops working for no visible reason.
    pub shared_configuration: bool,
    pub status: String,
    pub email: Option<String>,
    pub plan: Option<String>,
    pub quota_tier: Option<String>,
    pub checked_at: Option<String>,
    pub error: Option<String>,
    pub login: Option<LoginView>,
    pub limits: Option<Limits>,
    pub capabilities: Capabilities,
    pub guidance: Option<String>,
    pub reset_outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<UsageView>,
    /// Proxy used for this account's sign-in and usage check. Never carries
    /// credentials — those are rejected on the way in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_url: Option<String>,
}
#[derive(Deserialize)]
pub(super) struct BridgeView {
    pub(super) status: String,
    pub(super) email: Option<String>,
    pub(super) plan: Option<String>,
    pub(super) quota_tier: Option<String>,
    pub(super) checked_at: Option<String>,
    pub(super) error: Option<String>,
    pub(super) limits: Option<Limits>,
    pub(super) capabilities: Capabilities,
    #[serde(default)]
    pub(super) usage: Option<UsageView>,
}
