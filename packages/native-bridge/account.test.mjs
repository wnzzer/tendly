import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { executeAccount, isolatedEnvironment } from './account.mjs';
import { RpcError } from './account-common.mjs';
import { claudeStatusPath, normalizeClaudeProfile, normalizeClaudeLimits, retryAfterSeconds, UsageQueryError, usageFailure } from './account-claude.mjs';
import { normalizeLimits, safeLoginUrl } from './account-codex.mjs';

function fixture(overrides = {}) {
  const calls = [], events = [];
  let closed = false;
  const client = {
    async request(method, params) {
      calls.push({ method, params });
      if (overrides[method]) return overrides[method](params);
      if (method === 'account/read') return { account: { type: 'chatgpt', email: 'fixture@example.invalid', planType: 'plus', accessToken: 'NEVER_RETURN_FAKE_TOKEN' } };
      if (method === 'account/rateLimits/read') return { rateLimits: { primary: { usedPercent: 25, windowDurationMins: 300, resetsAt: 1900000000 }, secondary: null }, rateLimitResetCredits: { availableCount: 2, credits: [{ id: 'credit-fixture', status: 'available' }] } };
      if (method === 'account/login/start') return { loginId: 'login-fixture', verificationUrl: 'https://auth.openai.com/codex/device', userCode: 'ABCD-1234' };
      if (method === 'account/rateLimitResetCredit/consume') return { outcome: 'reset' };
      return {};
    },
    notify(method) { calls.push({ method }); },
    async waitLogin(id, signal) { return overrides.waitLogin ? overrides.waitLogin(id, signal) : { loginId: id, success: true }; },
    async close() { closed = true; },
  };
  return { client, calls, events, options: { emit: event => events.push(event), clientFactory: () => client }, get closed() { return closed; } };
}
const job = { provider: 'codex', config_dir: '/fixture/account' };

test('official account reads normalize only display fields and preserve epoch-second quota windows', async () => {
  const f = fixture();
  const view = await executeAccount({ ...job, action: 'read', refresh_token: true }, f.options);
  assert.equal(view.status, 'signed_in');
  assert.equal(view.email, 'fixture@example.invalid');
  assert.deepEqual(view.limits.primary, { used_percent: 25, window_minutes: 300, window_kind: 'five_hour', mapping_basis: 'duration_minutes', resets_at: 1900000000 });
  assert.equal(view.limits.reset_credits, 2);
  assert.equal(view.limits.reset_credits_source, 'official');
  assert.equal(view.capabilities.reset_quota, true);
  assert.equal(f.calls.find(call => call.method === 'account/read').params.refreshToken, true);
  assert.ok(!JSON.stringify(f.events).includes('NEVER_RETURN_FAKE_TOKEN'));
  assert.deepEqual(f.calls.slice(0, 2).map(call => call.method), ['initialize', 'initialized']);
  assert.ok(f.closed);
});
test('device login holds the official app-server until login completion, without external token exchange', async () => {
  let complete;
  const f = fixture({ waitLogin: () => new Promise(resolve => { complete = resolve; }) });
  const running = executeAccount({ ...job, action: 'login', mode: 'device' }, f.options);
  while (!complete) await new Promise(resolve => setImmediate(resolve));
  assert.equal(f.closed, false);
  assert.equal(f.events[0].type, 'login');
  assert.equal(f.events[0].login.url, 'https://auth.openai.com/codex/device');
  assert.deepEqual(f.calls.find(call => call.method === 'account/login/start').params, { type: 'chatgptDeviceCode' });
  complete({ loginId: 'login-fixture', success: true });
  await running;
  assert.equal(f.events.at(-1).view.status, 'signed_in');
  assert.equal(f.closed, true);
  assert.ok(!JSON.stringify(f.calls).includes('chatgptAuthTokens'));
});
test('cancellation uses the official login cancel method instead of claiming local logout', async () => {
  const f = fixture({ waitLogin: async () => ({ cancelled: true }), 'account/read': async () => ({ account: null }) });
  const result = await executeAccount({ ...job, action: 'login', mode: 'browser' }, f.options);
  assert.deepEqual(f.calls.find(call => call.method === 'account/login/cancel').params, { loginId: 'login-fixture' });
  assert.equal(result.status, 'signed_out');
  assert.equal(f.calls.some(call => call.method === 'account/logout'), false);
});
test('reset requires confirmation and official credits, preserves idempotency key, and re-reads official limits', async () => {
  const f = fixture();
  await executeAccount({ ...job, action: 'reset_quota', confirmed: true, idempotency_key: 'same-retry-key', credit_id: 'credit-fixture' }, f.options);
  const consumption = f.calls.find(call => call.method === 'account/rateLimitResetCredit/consume');
  assert.deepEqual(consumption.params, { idempotencyKey: 'same-retry-key', creditId: 'credit-fixture' });
  assert.equal(f.calls.filter(call => call.method === 'account/rateLimits/read').length, 2);
  assert.equal(f.events.find(event => event.type === 'reset').outcome, 'reset');
  assert.equal(f.events.at(-1).view.limits.primary.used_percent, 25, 'Never locally clear quota counters');
  for (const confirmed of [false, undefined]) {
    const denied = fixture();
    await assert.rejects(executeAccount({ ...job, action: 'reset_quota', confirmed, idempotency_key: 'key' }, denied.options));
    assert.equal(denied.calls.some(call => call.method.endsWith('/consume')), false);
  }
  const empty = fixture({ 'account/rateLimits/read': async () => ({ rateLimitResetCredits: { availableCount: 0 } }) });
  await assert.rejects(executeAccount({ ...job, action: 'reset_quota', confirmed: true, idempotency_key: 'new-key' }, empty.options), /No official/);
  assert.equal(empty.calls.some(call => call.method.endsWith('/consume')), false);
});
test('a previously authorized retry may ask the official API for alreadyRedeemed with zero remaining credits', async () => {
  const f = fixture({ 'account/rateLimits/read': async () => ({ rateLimitResetCredits: { availableCount: 0 } }), 'account/rateLimitResetCredit/consume': async () => ({ outcome: 'alreadyRedeemed' }) });
  await executeAccount({ ...job, action: 'reset_quota', confirmed: true, idempotency_key: 'original-key', retry_authorized: true }, f.options);
  assert.equal(f.events.find(event => event.type === 'reset').outcome, 'alreadyRedeemed');
  assert.equal(f.events.at(-1).view.limits.reset_credits, 0);
});
test('a reset credit is never consumed before the parent acknowledges durable retry authorization', async () => {
  const f = fixture();
  let acknowledge;
  const running = executeAccount({ ...job, action: 'reset_quota', confirmed: true, idempotency_key: 'durable-key' }, { ...f.options, authorizeReset: () => new Promise(resolve => { acknowledge = resolve; }) });
  while (!acknowledge) await new Promise(resolve => setImmediate(resolve));
  assert.equal(f.calls.some(call => call.method.endsWith('/consume')), false);
  assert.equal(f.events.at(-1).type, 'reset_authorized');
  acknowledge(); await running;
  assert.equal(f.calls.some(call => call.method.endsWith('/consume')), true);
  const denied = fixture();
  await assert.rejects(executeAccount({ ...job, action: 'reset_quota', confirmed: true, idempotency_key: 'failed-write' }, { ...denied.options, authorizeReset: async () => { throw new Error('Metadata write failed'); } }));
  assert.equal(denied.calls.some(call => call.method.endsWith('/consume')), false);
});
test('unsupported quota APIs are unavailable, never represented as zero usage or reset capability', async () => {
  const f = fixture({ 'account/rateLimits/read': async () => { throw new RpcError(-32601); } });
  const result = await executeAccount({ ...job, action: 'read' }, f.options);
  assert.equal(result.capabilities.quota, false); assert.equal(result.capabilities.reset_quota, false);
  assert.equal(result.limits, undefined);
  assert.ok(result.error.includes('unavailable'));
});
test('unrecognized account schemas are unsupported instead of guessed signed-in or signed-out states', async () => {
  for (const response of [{}, { account: {} }, { account: false }]) {
    const f = fixture({ 'account/read': async () => response });
    await assert.rejects(executeAccount({ ...job, action: 'read' }, f.options), error => error instanceof RpcError && error.code === -32601);
  }
  assert.equal(normalizeLimits({ rateLimitResetCredits: { availableCount: 1.5 } }).reset_credits, undefined);
  assert.equal(normalizeLimits({ rateLimitResetCredits: { availableCount: 1, credits: null } }).reset_credit_details, undefined);
  assert.deepEqual(normalizeLimits({ rateLimitResetCredits: { availableCount: 3, credits: [
    { id: 'a', status: 'available', expiresAt: null, title: 'Forever' },
    { id: 'b', status: 'redeemed', expiresAt: 1800000000 },
    { id: 'c', status: 'available', expiresAt: 1900000000, title: null },
    { id: 'd', status: 'available', expiresAt: 1850000000, grantedAt: 1700000000 },
  ] } }).reset_credit_details, [{ expires_at: 1850000000, granted_at: 1700000000 }, { expires_at: 1900000000 }, { expires_at: null, title: 'Forever' }]);
});
test('quota windows remain unmapped when the native duration is not an exact known window', () => {
  const limits = normalizeLimits({ rateLimits: { primary: { usedPercent: 10, windowDurationMins: 301 }, secondary: { usedPercent: 20, windowDurationMins: 10079 } } });
  assert.equal(limits.primary.window_kind, undefined);
  assert.equal(limits.primary.mapping_basis, undefined);
  assert.equal(limits.secondary.window_kind, undefined);
  assert.equal(limits.reset_credits_source, undefined);
});
test('login URLs allow only official HTTPS domains and account environments drop other account credentials', () => {
  for (const url of ['javascript:alert(1)', 'https://auth.openai.com.evil.invalid/login', 'http://auth.openai.com/login', 'https://user:password@auth.openai.com/login', 'https://evilopenai.com/login']) assert.equal(safeLoginUrl(url), undefined);
  assert.equal(safeLoginUrl('https://chatgpt.com/auth/login'), 'https://chatgpt.com/auth/login');
  assert.deepEqual(isolatedEnvironment({ PATH: '/fixture/bin', HTTP_PROXY: 'http://proxy.invalid', OPENAI_API_KEY: 'fake-a', ANTHROPIC_AUTH_TOKEN: 'fake-b', TENDLY_TOKEN: 'fake-c', CLAUDE_CONFIG_DIR: '/other', CODEX_HOME: '/other', CLAUDECODE: '1' }, 'codex', '/fixture/account'), { PATH: '/fixture/bin', HTTP_PROXY: 'http://proxy.invalid', CODEX_HOME: '/fixture/account' });
  assert.deepEqual(isolatedEnvironment({ PATH: '/fixture/bin', CODEX_HOME: '/default', OPENAI_API_KEY: 'fake-a' }, 'codex', '/fixture/account', null), { PATH: '/fixture/bin' });
  assert.deepEqual(normalizeLimits({}), {});
});
test('logout requires explicit consent and Claude managed login/quota remain unsupported without spawning anything', async () => {
  const f = fixture();
  await assert.rejects(executeAccount({ ...job, action: 'logout', confirmed: false }, f.options));
  assert.equal(f.calls.some(call => call.method === 'account/logout'), false);
  await assert.rejects(executeAccount({ ...job, provider: 'claude_code', action: 'login', mode: 'browser' }, f.options), error => error instanceof RpcError && error.code === -32601);
  await assert.rejects(executeAccount({ ...job, provider: 'claude_code', action: 'read', refresh_token: true }, f.options), error => error instanceof RpcError && error.code === -32601);
});

test('Claude status line usage keeps official window names and never invents a percentage', () => {
  const limits = normalizeClaudeLimits({ rate_limits: { five_hour: { used_percentage: 42, resets_at: 1900000000 }, seven_day: { used_percentage: 11 } } });
  assert.deepEqual(limits.primary, { used_percent: 42, resets_at: 1900000000, window_minutes: 300, window_kind: 'five_hour', mapping_basis: 'status_line_window' });
  assert.deepEqual(limits.secondary, { used_percent: 11, window_minutes: 10080, window_kind: 'weekly', mapping_basis: 'status_line_window' });
  assert.equal(limits.spend, undefined);
  // An absent, malformed or out-of-range capture stays unreported rather than
  // becoming a zero or a clamped estimate.
  for (const payload of [undefined, {}, { rate_limits: null }, { rate_limits: [] }, { rate_limits: { five_hour: { used_percentage: 101 } } }, { rate_limits: { five_hour: { used_percentage: -1 } } }, { rate_limits: { five_hour: { used_percentage: 'high' } } }, { rate_limits: { five_hour: null } }]) {
    assert.deepEqual(normalizeClaudeLimits(payload), {}, JSON.stringify(payload));
  }
  assert.equal(normalizeClaudeLimits({ rate_limits: { spend_limit: { used_percentage: 5 } } }).spend.used_percent, 5);
  const cpaLimits = normalizeClaudeLimits({ five_hour: { utilization: 36, resets_at: '2026-09-15T12:00:00Z' }, seven_day: { utilization: 72, resets_at: '2026-09-20T12:00:00Z' } }, 'oauth_usage');
  assert.equal(cpaLimits.primary.used_percent, 36); assert.equal(cpaLimits.primary.resets_at, 1789473600);
  assert.equal(cpaLimits.secondary.used_percent, 72); assert.equal(cpaLimits.secondary.window_kind, 'weekly'); assert.equal(cpaLimits.primary.mapping_basis, undefined);
});
test('Claude usage reads only the local status line capture path beside the account configuration', async () => {
  assert.equal(claudeStatusPath('/fixture/.claude', null), join('/fixture/.claude', 'tendly', 'statusline.json'));
  assert.equal(claudeStatusPath('/fixture/.claude', '/explicit/dir'), join('/explicit/dir', 'tendly', 'statusline.json'));
  const directory = await mkdtemp(join(tmpdir(), 'tendly-usage-'));
  const program = join(directory, 'claude-fixture.mjs');
  await writeFile(program, '#!/usr/bin/env node\nprocess.stdout.write(\'{"loggedIn":true,"email":"fixture@example.invalid","subscriptionType":"pro"}\');\n', { mode: 0o755 });
  const usageJob = { provider: 'claude_code', action: 'usage', config_dir: directory, config_env: directory, program };
  const missing = await executeAccount(usageJob, {});
  assert.equal(missing.usage.availability, 'capture_missing');
  assert.deepEqual(missing.limits, {});
  assert.equal(missing.capabilities.usage, true);
  assert.equal(missing.capabilities.quota, false);
  assert.equal(missing.capabilities.reset_quota, false);
  await mkdir(join(directory, 'tendly'), { recursive: true });
  const capture = join(directory, 'tendly', 'statusline.json');
  await writeFile(capture, JSON.stringify({ rate_limits: { five_hour: { used_percentage: 42, resets_at: 1900000000 } }, captured_at: 1899000000 }));
  const available = await executeAccount(usageJob, {});
  assert.equal(available.usage.availability, 'available');
  assert.equal(available.usage.captured_at, 1899000000);
  assert.equal(available.limits.primary.used_percent, 42);
  // A capture whose window already reset is stale, not presented as current.
  await writeFile(capture, JSON.stringify({ rate_limits: { five_hour: { used_percentage: 42, resets_at: 1000 } } }));
  assert.equal((await executeAccount(usageJob, {})).usage.availability, 'stale');
  await writeFile(capture, JSON.stringify({ session_id: 'no-limits-reported' }));
  const unreported = await executeAccount(usageJob, {});
  assert.equal(unreported.usage.availability, 'not_reported');
  assert.deepEqual(unreported.limits, {});
  await writeFile(capture, 'not json');
  assert.equal((await executeAccount(usageJob, {})).usage.availability, 'capture_missing');
  await rm(directory, { recursive: true, force: true });
});

test('Claude usage query uses the native OAuth credential only after an explicit request', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'tendly-usage-oauth-'));
  const program = join(directory, 'claude-fixture.mjs');
  await writeFile(program, '#!/usr/bin/env node\nprocess.stdout.write(\'{"loggedIn":true,"email":"fixture@example.invalid","subscriptionType":"max"}\');\n', { mode: 0o755 });
  await writeFile(join(directory, '.credentials.json'), JSON.stringify({ claudeAiOauth: { accessToken: 'fixture-oauth-token' } }), { mode: 0o600 });
  const originalFetch = globalThis.fetch;
  let request;
  globalThis.fetch = async (url, init) => {
    if (url === 'https://api.anthropic.com/api/oauth/profile') return Response.json({ organization: { organization_type: 'claude_pro', rate_limit_tier: 'default_claude_pro' } });
    request = { url, init };
    return Response.json({ five_hour: { utilization: 36, resets_at: '2026-09-15T12:00:00Z' }, seven_day: { utilization: 72, resets_at: '2026-09-20T12:00:00Z' } });
  };
  try {
    const view = await executeAccount({ provider: 'claude_code', action: 'usage', config_dir: directory, config_env: directory, program }, {});
    assert.equal(view.plan, 'pro');
    assert.equal(view.quota_tier, JSON.stringify(['claude_pro', 'default_claude_pro']));
    assert.equal(view.usage.source, 'claude_oauth_usage');
    assert.equal(view.usage.availability, 'available');
    assert.equal(view.limits.primary.used_percent, 36);
    assert.equal(view.limits.secondary.window_kind, 'weekly'); assert.equal(view.limits.primary.resets_at, 1789473600);
    assert.equal(request.url, 'https://api.anthropic.com/api/oauth/usage');
    assert.equal(request.init.headers.Authorization, 'Bearer fixture-oauth-token');
    assert.equal(request.init.headers['anthropic-beta'], 'oauth-2025-04-20');
    // Without Claude Code's user agent the endpoint answers 429 every time.
    assert.match(request.init.headers['User-Agent'], /^claude-code\/\d+\.\d+\.\d+$/);
    globalThis.fetch = async url => url.endsWith('/profile')
      ? new Response('{}', { status: 503 })
      : Response.json({ five_hour: { utilization: 36 } });
    const withoutProfile = await executeAccount({ provider: 'claude_code', action: 'usage', config_dir: directory, config_env: directory, program }, {});
    assert.equal(withoutProfile.limits.primary.used_percent, 36);
    assert.equal(withoutProfile.quota_tier, undefined);

  } finally { globalThis.fetch = originalFetch; await rm(directory, { recursive: true, force: true }); }
});

test('a failed usage query reports why instead of silently showing nothing', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'tendly-usage-failure-'));
  const program = join(directory, 'claude-fixture.mjs');
  await writeFile(program, '#!/usr/bin/env node\nprocess.stdout.write(\'{"loggedIn":true,"email":"fixture@example.invalid","subscriptionType":"max"}\');\n', { mode: 0o755 });
  await writeFile(join(directory, '.credentials.json'), JSON.stringify({ claudeAiOauth: { accessToken: 'fixture-oauth-token' } }), { mode: 0o600 });
  const originalFetch = globalThis.fetch;
  const job = { provider: 'claude_code', action: 'usage', config_dir: directory, config_env: directory, program };
  try {
    // A rate limit clears by itself, so the wait hint is passed through rather
    // than the page claiming the endpoint was unreachable.
    globalThis.fetch = async () => new Response('{"error":{"message":"Rate limited."}}', { status: 429, headers: { 'retry-after': '45' } });
    const limited = await executeAccount(job, {});
    assert.equal(limited.usage.availability, 'rate_limited');
    assert.equal(limited.usage.retry_after_seconds, 45);
    // The provider's diagnostic body is never carried into the account view.
    assert.ok(!JSON.stringify(limited).includes('Rate limited.'));
    assert.equal(limited.usage.status, undefined);

    // An expired credential needs a native sign-in, not a network check.
    globalThis.fetch = async () => new Response('{}', { status: 401 });
    assert.equal((await executeAccount(job, {})).usage.availability, 'unauthorized');

    // Anything else stays the generic transport failure.
    globalThis.fetch = async () => { throw Error('socket hang up'); };
    const failed = await executeAccount(job, {});
    assert.equal(failed.usage.availability, 'query_failed');
    assert.equal(failed.usage.retry_after_seconds, undefined);
  } finally { globalThis.fetch = originalFetch; await rm(directory, { recursive: true, force: true }); }
});

test('a rate-limited query still prefers a usable local capture over an error', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'tendly-usage-fallback-'));
  const program = join(directory, 'claude-fixture.mjs');
  await writeFile(program, '#!/usr/bin/env node\nprocess.stdout.write(\'{"loggedIn":true,"email":"fixture@example.invalid","subscriptionType":"max"}\');\n', { mode: 0o755 });
  await writeFile(join(directory, '.credentials.json'), JSON.stringify({ claudeAiOauth: { accessToken: 'fixture-oauth-token' } }), { mode: 0o600 });
  const capturePath = claudeStatusPath(directory, directory);
  await mkdir(join(directory, 'tendly'), { recursive: true });
  await writeFile(capturePath, JSON.stringify({ captured_at: 1899000000, rate_limits: { five_hour: { used_percentage: 12, resets_at: 4102444800 } } }));
  const originalFetch = globalThis.fetch;
  try {
    globalThis.fetch = async () => new Response('{}', { status: 429 });
    const view = await executeAccount({ provider: 'claude_code', action: 'usage', config_dir: directory, config_env: directory, program }, {});
    assert.equal(view.usage.availability, 'available');
    assert.equal(view.usage.source, 'status_line_capture');
    assert.equal(view.limits.primary.used_percent, 12);
  } finally { globalThis.fetch = originalFetch; await rm(directory, { recursive: true, force: true }); }
});

test('retry hints accept both Retry-After forms and never promise a stale delay', () => {
  assert.equal(retryAfterSeconds('45'), 45);
  const now = Date.parse('2026-09-15T00:00:00Z');
  assert.equal(retryAfterSeconds(new Date(now + 120_000).toUTCString(), now), 120);
  // A past date or negative delta must not render as a future wait.
  assert.equal(retryAfterSeconds('Mon, 01 Jan 2001 00:00:00 GMT'), 0);
  assert.equal(retryAfterSeconds('999999'), 86400);
  assert.equal(retryAfterSeconds(''), undefined);
  assert.equal(retryAfterSeconds('soon'), undefined);
  assert.equal(retryAfterSeconds(undefined), undefined);
  assert.equal(usageFailure(new UsageQueryError(429, '30')).retry_after_seconds, 30);
  assert.equal(usageFailure(new UsageQueryError(403)).availability, 'unauthorized');
  assert.equal(usageFailure(Error('offline')).availability, 'query_failed');
});

test('Claude profile distinguishes capacity tiers and rejects incomplete profiles', () => {
  const profile = tier => normalizeClaudeProfile({ organization: { organization_type: 'claude_max', rate_limit_tier: tier } });
  const five = profile('default_claude_max_5x');
  const twenty = profile('default_claude_max_20x');
  assert.equal(five.plan, 'max 5x');
  assert.equal(twenty.plan, 'max 20x');
  assert.notEqual(five.quota_tier, twenty.quota_tier);
  assert.deepEqual(normalizeClaudeProfile({ organization: { organization_type: 'claude_max' } }), {});
  assert.deepEqual(normalizeClaudeProfile(null), {});
});
