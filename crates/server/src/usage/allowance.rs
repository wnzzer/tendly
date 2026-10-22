//! How much of a subscription window is left: the window sizes learned from
//! what each account spent, and the /api/usage/allowance handler.

use super::*;

/// How much of a subscription window is likely left, in tokens and in what
/// they would cost through the API.
///
/// The provider reports only a percentage of the window used. Dividing what
/// this host's logs show the account spent in the same window by that
/// percentage gives the window's size -- too small when the account is also
/// used elsewhere, since the percentage counts that use and the logs do not.
/// Every such reading is therefore a lower bound, and the largest of the last
/// few windows is the best one: it is remembered, and what is left is that
/// size times the share not yet used, whoever used the rest.
#[derive(Serialize)]
pub struct Allowance {
    pub account_id: String,
    pub account: String,
    pub provider: ProviderKind,
    /// The subscription, as the provider names it (Max, Pro, Plus…).
    pub plan: Option<String>,
    pub window: &'static str,
    pub window_minutes: f64,
    pub used_percent: f64,
    pub window_start: i64,
    /// None when the window has run out and the next has not begun.
    pub resets_at: Option<i64>,
    /// Logged in the window, and only since the plan last changed.
    pub used_tokens: u64,
    pub used_cost: f64,
    pub estimated_total_tokens: Option<u64>,
    pub estimated_remaining_tokens: Option<u64>,
    pub estimated_total_cost: Option<f64>,
    pub estimated_remaining_cost: Option<f64>,
    /// Too little used yet, or too little seen here, to extrapolate from.
    pub low_confidence: bool,
    /// Set when the reading came from the client's own session log rather
    /// than an account check: when the client last reported it.
    pub observed_at: Option<i64>,
}

pub(crate) struct LoggedLimits {
    pub(crate) limits: crate::accounts::Limits,
    pub(crate) plan: Option<String>,
    pub(crate) observed_at: i64,
}

pub(super) fn epoch(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_f64().map(|n| {
            if n > 1e12 {
                (n / 1000.0) as i64
            } else {
                n as i64
            }
        }),
        Value::String(text) => DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|at| at.timestamp())
            .or_else(|| text.parse::<f64>().ok().map(|n| n as i64)),
        _ => None,
    }
}

/// What one window was seen to hold at least, in tokens and in API cost.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(super) struct Learned {
    resets_at: i64,
    tokens: u64,
    cost: f64,
}

/// Learned sizes by account, window and plan: a new plan starts over.
type Capacities = HashMap<String, Vec<Learned>>;

/// Windows remembered per account: enough to have seen one used alone, few
/// enough that a provider's change of limits is forgotten within a month.
pub(super) const LEARNED_WINDOWS: usize = 4;

fn capacities_path(state_dir: &Path) -> PathBuf {
    state_dir.join("allowance-capacity.json")
}

/// The plan each account was last read on, and since when.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct PlanSeen {
    tier: String,
    since: i64,
}

type Plans = HashMap<String, PlanSeen>;

fn plans_path(state_dir: &Path) -> PathBuf {
    state_dir.join("allowance-plans.json")
}

/// When the account's current plan began, as far as this host can tell. Use
/// from before a change of plan does not count against the new plan's
/// percentage, so a window's logs are only counted from the change on: what
/// that misses only makes the size a lower bound, as every reading is. A plan
/// read for the first time is taken as long held, unless the account has
/// learned sizes only under other plans -- it changed before this was kept.
pub(super) fn plan_since(
    plans: &mut Plans,
    learned: &Capacities,
    account: &str,
    tier: &str,
    read_at: i64,
) -> i64 {
    if let Some(seen) = plans.get_mut(account) {
        if seen.tier != tier {
            *seen = PlanSeen {
                tier: tier.to_owned(),
                since: read_at,
            };
        }
        return seen.since;
    }
    let prefix = format!("{account}/");
    let mut tiers = learned
        .keys()
        .filter_map(|key| key.strip_prefix(&prefix)?.split_once('/'))
        .map(|(_, tier)| tier)
        .filter(|tier| !tier.is_empty());
    let changed = tiers.clone().next().is_some() && tiers.all(|seen| seen != tier);
    let since = if changed { read_at } else { 0 };
    plans.insert(
        account.to_owned(),
        PlanSeen {
            tier: tier.to_owned(),
            since,
        },
    );
    since
}

/// The window size a reading proves, if it is one to trust. Below a tenth
/// used, a percentage rounded to the unit is too coarse to divide by; half a
/// point is added so that the rounding never makes the bound too large.
pub(super) fn window_size(
    used_percent: f64,
    used_tokens: u64,
    used_cost: f64,
) -> Option<(u64, f64)> {
    if used_percent < 10.0 || used_tokens == 0 {
        return None;
    }
    let share = ((used_percent + 0.5) / 100.0).min(1.0);
    Some(((used_tokens as f64 / share) as u64, used_cost / share))
}

/// Keep the largest size seen for each window, and only the latest windows.
/// A window's reset time drifts by seconds between readings.
pub(super) fn learn(history: &mut Vec<Learned>, resets_at: i64, (tokens, cost): (u64, f64)) {
    match history
        .iter_mut()
        .find(|seen| (seen.resets_at - resets_at).abs() < 600)
    {
        Some(seen) => {
            seen.tokens = seen.tokens.max(tokens);
            seen.cost = seen.cost.max(cost);
        }
        None => history.push(Learned {
            resets_at,
            tokens,
            cost,
        }),
    }
    history.sort_by_key(|seen| seen.resets_at);
    let excess = history.len().saturating_sub(LEARNED_WINDOWS);
    history.drain(..excess);
}

pub(super) fn capacity(history: &[Learned]) -> Option<(u64, f64)> {
    history
        .iter()
        .map(|seen| (seen.tokens, seen.cost))
        .reduce(|a, b| (a.0.max(b.0), a.1.max(b.1)))
}

/// The window's size and what is left of it. With a learned size, what is
/// left is its unused share; without one, the current reading is all there
/// is, and it is extrapolated from the use this host saw, flagged as unsure.
pub(crate) fn estimate(
    used_percent: f64,
    used_tokens: u64,
    used_cost: f64,
    learned: Option<(u64, f64)>,
) -> (Option<u64>, Option<u64>, Option<f64>, Option<f64>, bool) {
    if let Some((tokens, cost)) = learned {
        let left = (1.0 - used_percent / 100.0).clamp(0.0, 1.0);
        let cost = (cost > 0.0).then_some(cost);
        return (
            Some(tokens),
            Some((tokens as f64 * left) as u64),
            cost,
            cost.map(|cost| cost * left),
            false,
        );
    }
    if used_percent <= 0.0 || used_tokens == 0 {
        return (None, None, None, None, true);
    }
    let share = (used_percent / 100.0).min(1.0);
    let total = (used_tokens as f64 / share) as u64;
    let total_cost = (used_cost > 0.0).then(|| used_cost / share);
    (
        Some(total),
        Some(total.saturating_sub(used_tokens)),
        total_cost,
        total_cost.map(|cost| (cost - used_cost).max(0.0)),
        true,
    )
}

pub(super) async fn allowance(State(state): State<AppState>) -> Result<Json<Vec<Allowance>>> {
    let Json(accounts) = crate::accounts::list(State(state.clone())).await?;
    let prices = load_prices(&state.state_dir);
    let now = Utc::now().timestamp();
    let path = capacities_path(&state.state_dir);
    let plans_path = plans_path(&state.state_dir);
    let rows = tokio::task::spawn_blocking(move || {
        // One request at a time reads, updates and writes the learned sizes.
        static LEARNING: Mutex<()> = Mutex::new(());
        let _learning = LEARNING
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut learned: Capacities = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let before = learned.clone();
        let mut plans: Plans = std::fs::read(&plans_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let plans_before = plans.clone();
        let mut rows = Vec::new();
        for account in accounts {
            let provider = account.provider;
            let checked = account
                .limits
                .clone()
                .filter(|limits| limits.primary.is_some() || limits.secondary.is_some());
            // A newer session reading supersedes the saved account check,
            // including its plan, so a downgrade cannot retain old capacity.
            let logged = crate::adapters::adapter(provider)
                .and_then(|client| client.logged_limits(Path::new(&account.storage_path)));
            let checked_at = account
                .checked_at
                .as_deref()
                .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
                .map(|at| at.timestamp());
            let prefer_logged = logged.as_ref().is_some_and(|logged| {
                checked.is_none() || checked_at.is_none_or(|at| logged.observed_at > at)
            });
            let (limits, observed_at, logged_plan) = if prefer_logged {
                let logged = logged.unwrap();
                (logged.limits, Some(logged.observed_at), logged.plan)
            } else if let Some(limits) = checked {
                (limits, None, None)
            } else {
                continue;
            };
            let limits = &limits;
            let calls = collect(&[(PathBuf::from(&account.storage_path), None)]);
            let plan = logged_plan.or_else(|| account.plan.clone());
            let tier = if provider == ProviderKind::ClaudeCode {
                account.quota_tier.as_deref()
            } else {
                plan.as_deref()
            };
            let plan_since = tier.map_or(0, |tier| {
                plan_since(
                    &mut plans,
                    &learned,
                    &account.id.to_string(),
                    tier,
                    observed_at.or(checked_at).unwrap_or(now),
                )
            });
            for (window, limit) in [
                ("primary", &limits.primary),
                ("secondary", &limits.secondary),
            ] {
                let Some(limit) = limit else { continue };
                let (Some(minutes), Some(resets_at)) = (
                    // A window named but not measured is still a known length.
                    limit.window_minutes.or(match limit.window_kind {
                        Some(crate::accounts::WindowKind::FiveHour) => Some(300.0),
                        Some(crate::accounts::WindowKind::Weekly) => Some(10_080.0),
                        None => None,
                    }),
                    limit.resets_at.as_ref().and_then(epoch),
                ) else {
                    continue;
                };
                // A window past its reset is over: nothing of the next is used
                // yet, and it starts with the next message, so it has no reset
                // time until then.
                let expired = resets_at <= now;
                let (start, used_percent, resets_at) = if expired {
                    (resets_at, 0.0, None)
                } else {
                    (
                        resets_at - (minutes * 60.0) as i64,
                        limit.used_percent,
                        Some(resets_at),
                    )
                };
                let (mut tokens, mut cost) = (0u64, 0f64);
                for call in calls
                    .iter()
                    .filter(|call| call.provider == provider && call.at >= start.max(plan_since))
                {
                    tokens += call.tokens.total();
                    cost += prices.cost(call).unwrap_or(0.0);
                }
                let history = learned
                    .entry(format!("{}/{window}/{}", account.id, tier.unwrap_or("")))
                    .or_default();
                let can_estimate = provider != ProviderKind::ClaudeCode
                    || (tier.is_some()
                        && account.usage.as_ref().is_some_and(|usage| {
                            usage.availability == "available" && usage.retained_checked_at.is_none()
                        }));
                if can_estimate
                    && let (Some(resets_at), Some(size)) =
                        (resets_at, window_size(used_percent, tokens, cost))
                {
                    learn(history, resets_at, size);
                }
                let (total, remaining, total_cost, remaining_cost, low) = if can_estimate {
                    estimate(used_percent, tokens, cost, capacity(history))
                } else {
                    (None, None, None, None, true)
                };
                rows.push(Allowance {
                    account_id: account.id.to_string(),
                    account: account.name.clone(),
                    provider,
                    plan: plan.clone(),
                    window,
                    window_minutes: minutes,
                    used_percent,
                    window_start: start,
                    resets_at,
                    used_tokens: tokens,
                    used_cost: cost,
                    estimated_total_tokens: total,
                    estimated_remaining_tokens: remaining,
                    estimated_total_cost: total_cost,
                    estimated_remaining_cost: remaining_cost,
                    low_confidence: low,
                    observed_at,
                });
            }
        }
        learned.retain(|_, history| !history.is_empty());
        if learned != before
            && let Ok(bytes) = serde_json::to_vec(&learned)
        {
            let _ = std::fs::write(&path, bytes);
        }
        if plans != plans_before
            && let Ok(bytes) = serde_json::to_vec(&plans)
        {
            let _ = std::fs::write(&plans_path, bytes);
        }
        rows
    })
    .await
    .map_err(crate::ApiError::internal)?;
    Ok(Json(rows))
}
