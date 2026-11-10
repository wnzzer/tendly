import { reactive } from 'vue';
import { ApiError, errorMessage, json, request } from './api';
import { backendCapabilities } from './backend-capabilities';
import { accountAutoRefreshOn, balanceAutoRefreshOn, balanceDue, officialDue, officialQuotaSource, OFFICIAL_AUTO_REFRESH_MS, shouldAutoRefresh, type BalanceView, type EndpointBalance } from './balance-model';
import { usageBackoffSeconds, type AccountView } from './account-ui';
import { preferences } from './preferences';

/**
 * One copy of the configured endpoint balances for the whole page, so the
 * message box, the endpoint menu and the Usage page never disagree about a
 * figure, and a refresh in one place shows everywhere.
 */
export const balances = reactive({ rows: [] as EndpointBalance[], busy: [] as string[], error: '' });
const lastAttempt = new Map<string, number>();
let loading: Promise<void> | undefined;

export function balanceFor(profileId?: string | null): EndpointBalance | undefined {
  return profileId ? balances.rows.find(row => row.id === profileId) : undefined;
}

export function loadBalances(): Promise<void> {
  if (!backendCapabilities.customBalances) return Promise.resolve();
  loading ??= request<EndpointBalance[]>('/endpoint-balances')
    .then(rows => { balances.rows = rows; balances.error = ''; })
    .catch(cause => { if (!(cause instanceof ApiError && cause.status === 404)) balances.error = errorMessage(cause); })
    .finally(() => { loading = undefined; });
  return loading;
}

export async function refreshBalance(profileId: string): Promise<void> {
  if (balances.busy.includes(profileId)) return;
  balances.busy.push(profileId);
  lastAttempt.set(profileId, Date.now());
  try {
    const view = await request<BalanceView>(`/endpoint-profiles/${encodeURIComponent(profileId)}/balance/query`, json('POST'));
    applyView(profileId, view);
  } catch (cause) {
    const row = balanceFor(profileId);
    if (row) { row.error = errorMessage(cause); row.stale = true; }
  } finally {
    balances.busy = balances.busy.filter(id => id !== profileId);
  }
}

/** Queries again only when the figure is old and this page has not just asked; the server keeps its own cooldown too. */
export function refreshBalanceIfStale(profileId?: string | null): void {
  const row = balanceFor(profileId);
  if (row && shouldAutoRefresh(row, lastAttempt.get(row.id), Date.now())) void refreshBalance(row.id);
  if (accountFor(profileId) && Date.now() - accountsReadAt > 60_000) void loadAccountQuotas();
}

/**
 * Official accounts by the endpoint profile each one signs in through, so an
 * endpoint chip can show the account's allowance the way it shows a custom
 * balance. Reading the list asks Tendly for what it last heard, never the
 * provider, so it is cheap to read again after a turn.
 */
export const accountQuotas = reactive({ byProfile: {} as Record<string, AccountView> });
let accountsReadAt = 0, accountsLoading: Promise<void> | undefined;

export function accountFor(profileId?: string | null): AccountView | undefined {
  return profileId ? accountQuotas.byProfile[profileId] : undefined;
}

export function loadAccountQuotas(): Promise<void> {
  if (!backendCapabilities.accounts) return Promise.resolve();
  accountsReadAt = Date.now();
  accountsLoading ??= request<AccountView[]>('/accounts')
    .then(items => { accountQuotas.byProfile = Object.fromEntries(items.map(account => [account.profile_id, account])); })
    // The chips simply show no allowance; the accounts dialog reports the failure.
    .catch(() => {})
    .finally(() => { accountsLoading = undefined; });
  return accountsLoading;
}

/** The editor saved or queried a configuration: take its view, or drop the row when the query was disabled. */
export function applyView(profileId: string, view: BalanceView, name?: string): void {
  const index = balances.rows.findIndex(row => row.id === profileId);
  if (!view.config) { if (index >= 0) balances.rows.splice(index, 1); return; }
  const next = { id: profileId, name: name ?? balances.rows[index]?.name ?? '', auto_refresh: view.config.auto_refresh ?? null, reading: view.reading, error: view.error, stale: view.stale, attempted_at: view.attempted_at };
  if (index >= 0) Object.assign(balances.rows[index], next); else void loadBalances();
}

/**
 * The timer behind automatic refresh (balance-model.ts): one per page, looking
 * once a minute and whenever the page comes back into view, and asking nothing
 * while it is hidden. The last official check is kept in this browser's
 * storage, so several tabs share one ration rather than each spending it.
 */
let autoTimer: ReturnType<typeof setInterval> | undefined, autoRunning = false;
const officialRefusals = new Map<string, number>();
const OFFICIAL_KEY = 'tendly.official-auto-refresh';
function officialState(): Record<string, { at?: number; notBefore?: number }> {
  try { return JSON.parse(localStorage.getItem(OFFICIAL_KEY) ?? '{}') ?? {}; } catch { return {}; }
}
function recordOfficial(id: string, value: { at?: number; notBefore?: number }) {
  try { const all = officialState(); all[id] = { ...all[id], ...value }; localStorage.setItem(OFFICIAL_KEY, JSON.stringify(all)); } catch { /* Private mode keeps this page's own timer only. */ }
}

export function startAutoRefresh(): void {
  if (autoTimer || typeof window === 'undefined') return;
  autoTimer = setInterval(() => void autoRefreshTick(), 60_000);
  document.addEventListener('visibilitychange', () => { if (document.visibilityState === 'visible') void autoRefreshTick(); });
}

export async function autoRefreshTick(now = Date.now()): Promise<void> {
  if (autoRunning || typeof document !== 'undefined' && document.visibilityState === 'hidden') return;
  autoRunning = true;
  try {
    for (const row of balances.rows) {
      if (balanceAutoRefreshOn(row, preferences.value) && balanceDue(row, lastAttempt.get(row.id), now)) void refreshBalance(row.id);
    }
    if (backendCapabilities.accounts) {
      let asked = false;
      // One at a time: each is a request to the provider.
      for (const account of Object.values(accountQuotas.byProfile)) {
        if (!accountAutoRefreshOn(account.id, preferences.value)) continue;
        const kept = officialState()[account.id] ?? {};
        if (!officialDue(account, kept.at, kept.notBefore, now)) continue;
        asked = true;
        await refreshOfficial(account, now);
      }
      if (asked) await loadAccountQuotas();
    }
  } finally { autoRunning = false; }
}

async function refreshOfficial(account: AccountView, now: number): Promise<void> {
  recordOfficial(account.id, { at: now });
  const path = '/accounts/' + encodeURIComponent(account.id);
  try {
    if (officialQuotaSource(account) === 'refresh') { await request<AccountView>(path + '/refresh', json('POST', {})); return; }
    const view = await request<AccountView>(path + '/usage', json('POST', {}));
    if (view.usage?.availability === 'rate_limited') {
      // The provider said wait: wait longer each time it says so again.
      const refusals = (officialRefusals.get(account.id) ?? 0) + 1;
      officialRefusals.set(account.id, refusals);
      recordOfficial(account.id, { notBefore: now + usageBackoffSeconds(refusals, view.usage.retry_after_seconds) * 1000 });
    } else officialRefusals.delete(account.id);
  } catch {
    // A failure is reported where the account is shown; the timer just waits a round.
    recordOfficial(account.id, { notBefore: now + OFFICIAL_AUTO_REFRESH_MS });
  }
}
