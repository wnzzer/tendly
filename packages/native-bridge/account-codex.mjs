// Codex accounts, through the official app-server account RPCs: sign-in,
// logout, quota windows and earned rate-limit resets.
import { AppServerCancelled, AppServerClient, AppServerError, AppServerTimeout } from './app-server.mjs';
import { clean, numeric, RpcError } from './account-common.mjs';

export function safeLoginUrl(value) {
  try { const url = new URL(value); return url.protocol === 'https:' && !url.username && !url.password && ['openai.com', 'chatgpt.com'].some(host => url.hostname === host || url.hostname.endsWith('.' + host)) ? url.href : undefined; } catch { return undefined; }
}
export function normalizeLimits(response) {
  const limits = response?.rateLimitsByLimitId?.codex ?? response?.rateLimits;
  // Codex currently exposes opaque `primary`/`secondary` keys rather than
  // user-facing names. We only attach a friendly kind when the official
  // duration is an exact known window; unknown durations stay unmapped.
  const window = value => {
    if (!value || numeric(value.usedPercent) === undefined) return undefined;
    const minutes = numeric(value.windowDurationMins);
    const windowKind = minutes === 300 ? 'five_hour' : minutes === 10080 ? 'weekly' : undefined;
    return {
      used_percent: value.usedPercent,
      ...(minutes !== undefined ? { window_minutes: minutes } : {}),
      ...(windowKind ? { window_kind: windowKind, mapping_basis: 'duration_minutes' } : {}),
      ...(numeric(value.resetsAt) !== undefined ? { resets_at: value.resetsAt } : {}),
    };
  };
  const primary = window(limits?.primary), secondary = window(limits?.secondary);
  const resetCredits = response?.rateLimitResetCredits;
  const count = resetCredits?.availableCount;
  const available = Number.isSafeInteger(count) && count >= 0 ? count : undefined;
  // A count is only considered official when it came from the native
  // rateLimitResetCredits object. Keeping the provenance lets the UI explain
  // that this is an earned reset credit, never a locally-created reset.
  const resetSource = resetCredits && typeof resetCredits === 'object' && !Array.isArray(resetCredits) ? 'official' : undefined;
  // Detail rows come only when the backend sends them, and may be capped below
  // the count. Only available credits are kept, soonest expiry first; a null
  // expiry is a credit that never expires.
  const details = resetSource && Array.isArray(resetCredits.credits) ? resetCredits.credits
    .filter(credit => credit?.status === 'available')
    .map(credit => ({
      expires_at: numeric(credit.expiresAt) ?? null,
      ...(numeric(credit.grantedAt) !== undefined ? { granted_at: credit.grantedAt } : {}),
      ...(clean(credit.title) ? { title: clean(credit.title) } : {}),
    }))
    .sort((a, b) => (a.expires_at ?? Infinity) - (b.expires_at ?? Infinity)) : undefined;
  return {
    ...(primary ? { primary } : {}),
    ...(secondary ? { secondary } : {}),
    ...(available !== undefined ? { reset_credits: available } : {}),
    ...(resetSource ? { reset_credits_source: resetSource } : {}),
    ...(details ? { reset_credit_details: details } : {}),
  };
}
// The app-server connection for account management. Only the RPC error a
// client states is shown to the user (as RpcError); every other failure keeps
// the account wording it always had.
class RpcClient {
  constructor(job, env) {
    this.notifications = []; this.waiters = new Set();
    this.server = new AppServerClient(job.program || 'codex', ['app-server'], {
      cwd: job.config_dir, env,
      onNotification: message => {
        if (message.method !== 'account/login/completed') return;
        this.notifications.push(message.params); this.notifications = this.notifications.slice(-16);
        for (const wake of this.waiters) wake();
      },
      onClose: () => { for (const wake of this.waiters) wake(); },
    });
  }
  get closed() { return this.server.closed; }
  async request(method, params = {}, signal) {
    if (this.closed) throw new Error('Account operation cancelled.');
    try { return await this.server.request(method, params, { signal }); }
    catch (error) {
      if (error instanceof AppServerError) throw new RpcError(error.code);
      if (error instanceof AppServerCancelled) throw new Error('Account operation cancelled.');
      if (error instanceof AppServerTimeout) throw new Error('Official account request timed out.');
      throw new Error('Native account process closed.');
    }
  }
  notify(method) { this.server.notify(method); }
  waitLogin(id, signal) {
    return new Promise((resolveWait, rejectWait) => {
      const finish = (callback, value) => { clearTimeout(timer); this.waiters.delete(check); signal?.removeEventListener('abort', check); callback(value); };
      const check = () => {
        const found = this.notifications.find(value => value?.loginId === id);
        if (found) finish(resolveWait, found);
        else if (signal?.aborted) finish(resolveWait, { cancelled: true });
        else if (this.closed) finish(rejectWait, new Error('Native account process closed.'));
      };
      const timer = setTimeout(() => finish(resolveWait, { cancelled: true, expired: true }), 30 * 60 * 1000);
      this.waiters.add(check); signal?.addEventListener('abort', check, { once: true }); check();
    });
  }
  close() { return this.server.close(); }
}

async function readCodex(client, refresh, signal) {
  const response = await client.request('account/read', { refreshToken: !!refresh }, signal);
  const account = response?.account;
  if (!response || !Object.hasOwn(response, 'account') || (account !== null && (!account || !['chatgpt', 'apiKey', 'amazonBedrock'].includes(account.type)))) throw new RpcError(-32601);
  const view = { status: account ? 'signed_in' : 'signed_out', checked_at: new Date().toISOString(), capabilities: { login: true, refresh_token: true, quota: false, reset_quota: false, logout: true } };
  if (account) { view.email = clean(account.email); view.plan = clean(account.planType, 80); }
  if (account?.type === 'chatgpt') {
    try {
      const limits = await client.request('account/rateLimits/read', {}, signal);
      view.limits = normalizeLimits(limits); view.capabilities.quota = true;
      view.capabilities.reset_quota = (view.limits.reset_credits ?? 0) > 0;
    } catch { view.error = 'Official quota information is unavailable for this client or account.'; }
  }
  return view;
}

/**
 * What an account operation needs from its app-server connection; tests pass a
 * fake with the same shape.
 * @typedef {object} AccountClient
 * @property {(method: string, params?: object, signal?: AbortSignal) => Promise<any>} request
 * @property {(method: string) => void} notify
 * @property {(id: string, signal?: AbortSignal) => Promise<any>} waitLogin
 * @property {() => Promise<void>} close
 */
/**
 * @param {any} job
 * @param {{ emit?: (event: object) => void, clientFactory?: (job: any) => AccountClient, signal?: AbortSignal, authorizeReset?: (key: string) => Promise<void>, env: NodeJS.ProcessEnv }} options
 */
export async function codexAccount(job, { emit = () => {}, clientFactory = value => new RpcClient(value, env), signal, authorizeReset = async () => {}, env }) {
  const client = clientFactory(job);
  try {
    await client.request('initialize', { clientInfo: { name: 'tendly_accounts', title: 'Tendly account management', version: '0.1.0' } }, signal);
    client.notify('initialized');
    if (job.action === 'login') {
      if (!['device', 'browser'].includes(job.mode)) throw new Error('Choose a supported login mode.');
      const login = await client.request('account/login/start', { type: job.mode === 'device' ? 'chatgptDeviceCode' : 'chatgpt' }, signal);
      const url = safeLoginUrl(login?.verificationUrl ?? login?.authUrl);
      if (!url || typeof login.loginId !== 'string' || !login.loginId) throw new Error('Native login did not return a safe official authorization URL.');
      emit({ type: 'login', login: { url, id: clean(login.loginId, 128), ...(login.userCode ? { user_code: clean(login.userCode, 128) } : {}) } });
      const result = await client.waitLogin(login.loginId, signal);
      if (result.cancelled) {
        try { await client.request('account/login/cancel', { loginId: login.loginId }); } catch { /* Closing the native process also closes this attempt. */ }
      } else if (!result.success) throw new Error('Official login did not complete successfully.');
    } else if (job.action === 'logout') {
      if (!job.confirmed) throw new Error('Logout requires explicit confirmation.');
      await client.request('account/logout', {}, signal);
    } else if (job.action === 'reset_quota') {
      if (!job.confirmed || typeof job.idempotency_key !== 'string' || !job.idempotency_key.trim() || job.idempotency_key.length > 200) throw new Error('Quota reset requires confirmation and a stable idempotency key.');
      const available = await client.request('account/rateLimits/read', {}, signal);
      if (!job.retry_authorized && (normalizeLimits(available).reset_credits ?? 0) <= 0) throw new Error('No official earned rate-limit reset credits are available.');
      if (!job.retry_authorized && job.credit_id && !available.rateLimitResetCredits?.credits?.some(credit => credit.id === job.credit_id && credit.status === 'available')) throw new Error('The selected official reset credit is not available.');
      emit({ type: 'reset_authorized', idempotency_key: job.idempotency_key });
      await authorizeReset(job.idempotency_key);
      const result = await client.request('account/rateLimitResetCredit/consume', { idempotencyKey: job.idempotency_key, ...(job.credit_id ? { creditId: job.credit_id } : {}) }, signal);
      if (!['reset', 'alreadyRedeemed', 'nothingToReset', 'noCredit'].includes(result?.outcome)) throw new Error('Unrecognized official reset result.');
      emit({ type: 'reset', outcome: result.outcome });
    }
    const view = await readCodex(client, job.action === 'read' && job.refresh_token, job.action === 'login' && signal?.aborted ? undefined : signal);
    emit({ type: 'view', view }); return view;
  } finally { await client.close(); }
}
