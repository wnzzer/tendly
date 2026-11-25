// Gemini accounts, through Google's Antigravity CLI (agy). Sign-in happens in
// agy itself; Tendly reads who is signed in and asks agy for the account's
// quota (`agy -p /quota`). The OAuth token is never read out: only the email
// agy stores beside it.
import { spawnNative as spawn } from './native-spawn.mjs';
import { mkdtemp, readFile, rm, symlink } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { clean, noCapabilities } from './account-common.mjs';

const MAX_OUTPUT = 64 * 1024;

/** Who agy holds a sign-in for in this home, by email only. */
export async function signedInEmail(home) {
  try {
    const raw = await readFile(join(home, 'antigravity-cli', 'antigravity-oauth-token'), 'utf8');
    if (raw.length > MAX_OUTPUT) return { signedIn: false };
    const stored = JSON.parse(raw);
    const signedIn = !!stored && typeof stored === 'object' && !!stored.token && typeof stored.token === 'object';
    return { signedIn, email: signedIn ? clean(stored.id_token_email, 200) : undefined };
  } catch { return { signedIn: false }; }
}

const WINDOWS = [
  { match: /five.?hour/i, kind: 'five_hour', minutes: 300, slot: 'primary' },
  { match: /week/i, kind: 'weekly', minutes: 10080, slot: 'secondary' },
];

/**
 * `agy -p /quota` prints one window per line: the model group, the window,
 * what is left of it and when it resets, tab-separated:
 * `Gemini Models<TAB>Five Hour Limit Remaining<TAB>99%<TAB>2026-11-25T16:14:10Z`.
 * The first group, agy's own Gemini models, fills the account's two windows;
 * the Claude and GPT models it also serves are limited separately and not shown.
 */
export function parseQuota(output) {
  const groups = new Map();
  for (const line of String(output).split('\n')) {
    const [group, label, left, reset] = line.split('\t').map(field => field?.trim());
    const window = WINDOWS.find(w => w.match.test(label ?? ''));
    const remaining = Number.parseFloat(left ?? '');
    if (!group || !window || !Number.isFinite(remaining) || remaining < 0 || remaining > 100) continue;
    const resets = Date.parse(reset ?? '');
    if (!groups.has(group)) groups.set(group, {});
    groups.get(group)[window.slot] = {
      used_percent: Math.round((100 - remaining) * 100) / 100,
      window_minutes: window.minutes,
      window_kind: window.kind,
      ...(Number.isFinite(resets) ? { resets_at: Math.floor(resets / 1000) } : {}),
    };
  }
  const [first] = groups.values();
  return first ?? {};
}

/**
 * agy finds its home only as `$HOME/.gemini`, so a home other than the host's
 * is reached through a scratch HOME whose `.gemini` is that directory.
 */
async function withHome(job, env, run) {
  if (!job.config_env) return run(env);
  const home = await mkdtemp(join(tmpdir(), 'tendly-gemini-'));
  try {
    await symlink(job.config_dir, join(home, '.gemini'));
    return await run({ ...env, HOME: home });
  } finally { await rm(home, { recursive: true, force: true }); }
}

function quota(job, env, signal) {
  return withHome(job, env, home => new Promise((resolveQuota, rejectQuota) => {
    let output = '';
    const child = spawn(job.program || 'agy', ['-p', '/quota'], { cwd: home.HOME ?? job.config_dir, env: home, stdio: ['ignore', 'pipe', 'ignore'] });
    const timer = setTimeout(() => { child.kill('SIGKILL'); rejectQuota(new Error('Antigravity quota check timed out.')); }, 45000);
    const abort = () => child.kill('SIGKILL');
    signal?.addEventListener('abort', abort, { once: true });
    child.stdout.setEncoding('utf8');
    child.stdout.on('data', chunk => { output += chunk; if (output.length > MAX_OUTPUT) child.kill('SIGKILL'); });
    child.once('error', () => { clearTimeout(timer); rejectQuota(new Error('Native Antigravity client (agy) is unavailable.')); });
    child.once('close', code => {
      clearTimeout(timer); signal?.removeEventListener('abort', abort);
      if (code === 0) resolveQuota(output); else rejectQuota(new Error('agy could not read this account\'s quota.'));
    });
  }));
}

/** @param {any} job @param {{ emit?: (event: object) => void, env?: Record<string, string|undefined>, signal?: AbortSignal }} options */
export async function geminiAccount(job, { emit = () => {}, env, signal }) {
  const { signedIn, email } = await signedInEmail(job.config_dir);
  const capabilities = { ...noCapabilities(), usage: true };
  /** @type {Record<string, any>} */
  let view = { status: signedIn ? 'signed_in' : 'signed_out', ...(email ? { email } : {}), checked_at: new Date().toISOString(), capabilities };
  if (job.action === 'usage' && signedIn) {
    try {
      const limits = parseQuota(await quota(job, env, signal));
      view = { ...view, limits, usage: { availability: Object.keys(limits).length ? 'available' : 'not_reported', source: 'agy_quota' } };
    } catch {
      view = { ...view, limits: {}, usage: { availability: 'query_failed', source: 'agy_quota' } };
    }
  }
  emit({ type: 'view', view }); return view;
}
