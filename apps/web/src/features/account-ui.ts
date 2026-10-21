import { randomId } from "./random-id";
import { clientInfo } from "./clients";
import type { AgentProviderKind } from '@tendly/protocol';

export type AccountWindowKind = 'five_hour' | 'weekly';
export type AccountWindowMappingBasis = 'duration_minutes' | 'status_line_window' | 'oauth_usage';
export interface AccountLimit {
  used_percent: number;
  window_minutes?: number;
  resets_at?: string | number;
  /** Present only when the native client reported an exact known duration. */
  window_kind?: AccountWindowKind;
  mapping_basis?: AccountWindowMappingBasis;
}
export type AccountUsageAvailability = 'available' | 'stale' | 'not_reported' | 'capture_missing' | 'capture_disabled' | 'credential_unavailable' | 'query_failed' | 'rate_limited' | 'unauthorized' | 'cached';
/** Explicit Claude usage snapshot; source identifies OAuth or status-line data. */
export interface AccountUsage {
  availability: AccountUsageAvailability;
  capture_path?: string;
  captured_at?: number;
  source?: 'claude_oauth_usage' | 'status_line_capture';
  /** Provider-supplied wait hint for a rate-limited query. */
  retry_after_seconds?: number;
  /** Set when a failed query kept the previous figures; stamps when they were read. */
  retained_checked_at?: string;
}
/**
 * How long to wait before asking the official usage endpoint again. It
 * answers roughly once per window and refuses the rest; asking again while
 * refused does not help and appears to push the allowance further out. So an
 * answer waits a minute, and each refusal in a row doubles the wait from five
 * minutes up to thirty, or longer if the endpoint said so.
 */
export function usageBackoffSeconds(refusals: number, retryAfter?: number | null): number {
  if (refusals <= 0) return 60;
  return Math.max(retryAfter || 0, Math.min(300 * 2 ** (refusals - 1), 1800));
}

/** Human wait hint for a rate-limited usage query, in whole minutes or seconds. */
export function usageRetryHint(seconds?: number): { unit: 'seconds' | 'minutes'; value: number } | undefined {
  if (seconds === undefined || !Number.isFinite(seconds) || seconds <= 0) return undefined;
  return seconds < 90 ? { unit: 'seconds', value: Math.ceil(seconds) } : { unit: 'minutes', value: Math.ceil(seconds / 60) };
}
export type AccountResetOutcome = 'reset' | 'alreadyRedeemed' | 'nothingToReset' | 'noCredit';
export interface AccountView {
  id: string; name: string; provider: AgentProviderKind; profile_id: string; storage_path: string;
  /** The account points at a configuration the host already had, so anything
   * else editing that directory changes this account too. */
  shared_configuration?: boolean;
  native_source_id?: string | null;
  status: 'unknown' | 'signed_out' | 'signed_in' | 'login_pending' | 'error';
  email?: string; plan?: string; checked_at?: string; error?: string; login_command?: string; guidance?: string;
  login?: { url: string; user_code?: string; id?: string };
  limits?: {
    primary?: AccountLimit;
    secondary?: AccountLimit;
    reset_credits?: number;
    reset_credits_source?: 'official';
    /** Available credits the official client itemised, soonest expiry first;
     * may be capped below `reset_credits`. A null expiry never expires. */
    reset_credit_details?: { expires_at: number | null; granted_at?: number; title?: string }[];
  };
  reset_outcome?: AccountResetOutcome;
  usage?: AccountUsage;
  /** Proxy for this account's own sign-in and usage requests; never carries credentials. */
  proxy_url?: string;
  capabilities: { login: boolean; refresh_token: boolean; quota: boolean; reset_quota: boolean; logout: boolean; usage?: boolean };
}
export function accountStatusLabel(status: AccountView['status']): string {
  return { unknown: 'Not checked', signed_out: 'Signed out', signed_in: 'Signed in', login_pending: 'Sign-in in progress', error: 'Account needs attention' }[status];
}
/**
 * A window whose reset time has passed: the figure read before it describes a
 * window that is over. A 5-hour window starts again with the next message, so
 * until then nothing of it is used.
 */
export function accountLimitExpired(limit?: AccountLimit, now = Date.now()): boolean {
  const resets = accountResetDate(limit?.resets_at);
  return !!resets && resets.getTime() <= now;
}
export function accountLimitPercent(limit?: AccountLimit, now = Date.now()): number | undefined {
  if (!limit || !Number.isFinite(limit.used_percent) || limit.used_percent < 0 || limit.used_percent > 100) return undefined;
  return accountLimitExpired(limit, now) ? 0 : limit.used_percent;
}
export function accountResetCreditState(account: Pick<AccountView, 'status' | 'capabilities' | 'limits'>):
  'unsupported' | 'signed_out' | 'not_reported' | 'available' | 'empty' {
  if (!account.capabilities.quota) return 'unsupported';
  if (account.status !== 'signed_in') return 'signed_out';
  const count = account.limits?.reset_credits;
  if (count === undefined || account.limits?.reset_credits_source !== 'official') return 'not_reported';
  return count > 0 ? 'available' : 'empty';
}
export function accountResetDate(value?: string | number): Date | undefined {
  if (value === undefined) return undefined;
  const date = new Date(typeof value === 'number' && value < 1e12 ? value * 1000 : value);
  return Number.isFinite(date.getTime()) ? date : undefined;
}
export function accountLoginUrl(account: Pick<AccountView, 'provider' | 'login'>): string | undefined {
  if (!account.login?.url) return undefined;
  try {
    const url = new URL(account.login.url), domains = clientInfo(account.provider).account.loginDomains;
    return url.protocol === 'https:' && !url.username && !url.password && domains.some(domain => url.hostname === domain || url.hostname.endsWith('.' + domain)) ? url.href : undefined;
  } catch { return undefined; }
}
/**
 * Which sign-in actions an account's state makes sense to offer.
 *
 * `unknown` means the official status has never been read — not that the
 * account is signed out — so everything stays on offer there: hiding sign-in
 * from an account nobody has checked yet would strand it. Once a status is
 * known the half that cannot apply goes away, because signing in again while
 * signed in does nothing, and there is no token to refresh or session to end
 * while signed out.
 *
 * Every account operation re-reads the official status afterwards, so a sign-in
 * or sign-out flips these in the same round trip rather than leaving the button
 * that undoes it hidden.
 */
export function accountSignInActions(account: Pick<AccountView, 'status' | 'capabilities'>): {
  signIn: boolean;
  refreshToken: boolean;
  signOut: boolean;
} {
  const out = account.status === 'signed_out' || account.status === 'error';
  return {
    signIn: account.capabilities.login && account.status !== 'login_pending' && account.status !== 'signed_in',
    refreshToken: account.capabilities.refresh_token && !out,
    signOut: account.capabilities.logout && !out,
  };
}
export function canResetAccountQuota(account: AccountView): boolean {
  return account.status === 'signed_in' && account.capabilities.reset_quota && account.capabilities.quota && Number.isFinite(account.limits?.reset_credits) && Number(account.limits?.reset_credits) > 0;
}
export function quotaResetOutcomeNotice(outcome?: AccountResetOutcome): string {
  return outcome === 'reset' ? 'Official quota reset completed.' : outcome === 'alreadyRedeemed' ? 'This reset credit was already redeemed. No new quota reset was performed.' : outcome === 'nothingToReset' ? 'The official provider reports nothing to reset. No quota reset was performed.' : outcome === 'noCredit' ? 'No official reset credit is available. No quota reset was performed.' : 'The reset request completed. The official status below is the source of truth.';
}
export interface QuotaResetAttempt { idempotency_key: string; credit_id?: string; outcome: 'pending' | 'unknown' }
export const QUOTA_RESET_STORAGE_KEY = 'tendly.pending-quota-resets.v1';
export const QUOTA_RESET_STORAGE_WARNING = 'Pending reset protection is unavailable in this browser. Do not repeat a quota reset; refresh official quota first. New reset requests are blocked.';
type ResetStorage = Pick<Storage, 'getItem' | 'setItem'>;
export function createQuotaResetStore(uuid: () => string = randomId, storage?: ResetStorage | (() => ResetStorage | undefined), requirePersistence = false) {
  const attempts = new Map<string, QuotaResetAttempt>(), durable = new Set<string>();
  let hydrated = false, storageFailed = false;
  function resolveStorage() { try { return typeof storage === 'function' ? storage() : storage; } catch { return undefined; } }
  function hydrate() {
    if (hydrated) return;
    const source = resolveStorage();
    if (!source) { storageFailed = requirePersistence; hydrated = true; return; }
    try {
      const raw = source.getItem(QUOTA_RESET_STORAGE_KEY), values: unknown = raw ? JSON.parse(raw) : [];
      if (!Array.isArray(values) || values.length > 1000 || values.some(item => !item || typeof item.account_id !== 'string' || !/^[A-Za-z0-9_-]{1,200}$/.test(item.account_id) || typeof item.idempotency_key !== 'string' || !/^[A-Za-z0-9_-]{1,200}$/.test(item.idempotency_key))) throw Error('Invalid pending reset metadata');
      for (const item of values) { attempts.set(item.account_id, { idempotency_key: item.idempotency_key, outcome: 'unknown' }); durable.add(item.account_id); }
    } catch { storageFailed = true; }
    hydrated = true;
  }
  function persist(omit?: string) {
    const source = resolveStorage();
    if (!source) { storageFailed = requirePersistence; return !requirePersistence; }
    try {
      // Only opaque account/request identifiers survive a page reload. Never
      // store account details, login URLs, tokens, quotas or secret values.
      source.setItem(QUOTA_RESET_STORAGE_KEY, JSON.stringify([...attempts].filter(([id]) => id !== omit).map(([account_id, attempt]) => ({ account_id, idempotency_key: attempt.idempotency_key }))));
      storageFailed = false; return true;
    } catch { storageFailed = true; return false; }
  }
  return {
    get(id: string) { hydrate(); return attempts.get(id); },
    storageAvailable() { hydrate(); return !storageFailed; },
    begin(account: AccountView, confirmed: boolean, creditId?: string) {
      if (!confirmed) throw Error('Confirm that this reset may consume an official reset credit.');
      hydrate();
      const existing = attempts.get(account.id);
      if (existing) { if (requirePersistence && !durable.has(account.id) && !persist()) throw Error(QUOTA_RESET_STORAGE_WARNING); durable.add(account.id); return existing; }
      if (!canResetAccountQuota(account)) throw Error('The official client does not currently offer a quota reset credit.');
      // A denied/corrupt/read-only storage cannot silently generate a new key
      // after a reload and charge for a second, unknowingly repeated request.
      if (requirePersistence && (storageFailed || !persist())) throw Error(QUOTA_RESET_STORAGE_WARNING);
      const attempt: QuotaResetAttempt = { idempotency_key: uuid(), outcome: 'pending', ...(creditId ? { credit_id: creditId } : {}) };
      attempts.set(account.id, attempt);
      if (requirePersistence && !persist()) throw Error(QUOTA_RESET_STORAGE_WARNING);
      durable.add(account.id); return attempt;
    },
    unknown(id: string) { const attempt = attempts.get(id); if (attempt) attempt.outcome = 'unknown'; },
    complete(id: string) { hydrate(); if (requirePersistence && !persist(id)) { const attempt = attempts.get(id); if (attempt) attempt.outcome = 'unknown'; return false; } attempts.delete(id); durable.delete(id); return true; },
  };
}
export const quotaResetAttempts = createQuotaResetStore(randomId, () => typeof window === 'undefined' ? undefined : window.sessionStorage, true);

/** How close a window is to running out, for its colour: amber from 70%, red from 90%. */
export function quotaLevel(percent?: number): 'ok' | 'warn' | 'danger' | 'unknown' {
  if (percent === undefined) return 'unknown';
  return percent >= 90 ? 'danger' : percent >= 70 ? 'warn' : 'ok';
}

/**
 * The window's name as a person would say it. A known kind wins; otherwise an
 * exact duration is named in days or hours rather than "10080 minutes", and a
 * window that reported nothing keeps its native slot name.
 */
export function quotaWindowTitle(slot: 'primary' | 'secondary', limit?: AccountLimit): { key: string; values: Record<string, number> } {
  if (limit?.window_kind === 'five_hour') return { key: '5-hour window', values: {} };
  if (limit?.window_kind === 'weekly') return { key: 'Weekly window', values: {} };
  const minutes = limit?.window_minutes;
  if (minutes === 10080) return { key: 'Weekly window', values: {} };
  if (minutes === 300) return { key: '5-hour window', values: {} };
  if (minutes && minutes % 1440 === 0) return { key: '{count}-day window', values: { count: minutes / 1440 } };
  if (minutes && minutes % 60 === 0) return { key: '{count}-hour window', values: { count: minutes / 60 } };
  return { key: slot === 'primary' ? 'Primary window' : 'Secondary window', values: {} };
}

/** Time until a reset as a translation key and its values, coarsest units first. */
export function untilLabel(ms: number): { soon: boolean; key: string; values: Record<string, number> } {
  if (!Number.isFinite(ms) || ms < 60_000) return { soon: true, key: '', values: {} };
  const minutes = Math.floor(ms / 60_000), days = Math.floor(minutes / 1440), hours = Math.floor((minutes % 1440) / 60), rest = minutes % 60;
  if (days) return { soon: false, key: hours ? '{d}d {h}h' : '{d}d', values: { d: days, h: hours } };
  if (hours) return { soon: false, key: rest ? '{h}h {m}m' : '{h}h', values: { h: hours, m: rest } };
  return { soon: false, key: '{m}m', values: { m: rest } };
}

/** The window's name in the few characters a list row has room for. */
export function quotaShortTitle(slot: 'primary' | 'secondary', limit?: AccountLimit): { key: string; values: Record<string, number> } {
  const full = quotaWindowTitle(slot, limit);
  return { key: ({ '5-hour window': '5h', 'Weekly window': 'Week', '{count}-day window': '{count}d', '{count}-hour window': '{count}h', 'Primary window': 'Main', 'Secondary window': 'Other' } as Record<string, string>)[full.key] ?? full.key, values: full.values };
}

/** An official account's allowance as one figure: the window with the least left, since that is the one that stops work first. */
export interface AccountQuota { remaining: number; slot: 'primary' | 'secondary'; limit: AccountLimit }

/**
 * What an endpoint chip shows for an official account: the remaining share of
 * its tightest window. Nothing for an account that is not signed in or has
 * reported no window, rather than a figure that looks like an empty allowance.
 */
export function accountQuota(account: Pick<AccountView, 'status' | 'limits'>, now = Date.now()): AccountQuota | undefined {
  if (account.status !== 'signed_in') return undefined;
  let tightest: AccountQuota | undefined;
  for (const slot of ['primary', 'secondary'] as const) {
    const limit = account.limits?.[slot], used = accountLimitPercent(limit, now);
    if (!limit || used === undefined) continue;
    const remaining = Math.round((100 - used) * 10) / 10;
    if (!tightest || remaining < tightest.remaining) tightest = { remaining, slot, limit };
  }
  return tightest;
}
