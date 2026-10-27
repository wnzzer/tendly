// A throwaway gateway for the browser suite: its own state directory, its own
// port and synthetic clients (packages/native-bridge/fixtures) in place of
// Claude Code, Codex and Pi, so no account, model or network is touched.
// It is stopped by its PID only: a real gateway may be running beside it.
import { execFileSync, spawn } from 'node:child_process';
import { chmod, copyFile, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { chromium } from 'playwright-core';

const ROOT = resolve(import.meta.dirname, '../..');
const FIXTURES = join(ROOT, 'packages/native-bridge/fixtures');
const TOKEN = 'e2e-local-token';

function freePort() {
  return new Promise((done, fail) => {
    const server = createServer().listen(0, '127.0.0.1', () => { const { port } = server.address(); server.close(() => done(port)); });
    server.on('error', fail);
  });
}

async function fixture(dir, name) {
  const path = join(dir, name);
  await copyFile(join(FIXTURES, name), path); await chmod(path, 0o755);
  return path;
}

/**
 * Starts a gateway; `stop()` ends it and removes what it wrote. `host` is the
 * name it is opened at (two gateways on one machine need two, or their cookies
 * would be one), and `env(base)` adds to its environment.
 */
export async function startGateway({ host = '127.0.0.1', env = () => ({}) } = {}) {
  const home = await mkdtemp(join(tmpdir(), 'tendly-e2e-'));
  const bin = join(home, 'bin'), workspace = join(home, 'workspace');
  await mkdir(bin); await mkdir(workspace); await mkdir(join(home, 'home'));
  // An empty home makes zsh open its first-run setup instead of a prompt.
  await writeFile(join(home, 'home', '.zshrc'), '');
  // Commits made through the page need a name, now that the gateway's home is its own.
  await writeFile(join(home, 'home', '.gitconfig'), '[user]\n\tname = e2e\n\temail = e2e@example.invalid\n[init]\n\tdefaultBranch = main\n');
  // A repository, as most workspaces are: the Git pane and branch chips have something to show.
  const git = (...args) => execFileSync('git', args, { cwd: workspace, stdio: 'ignore' });
  git('init', '-q', '-b', 'main'); await writeFile(join(workspace, 'README.md'), '# Fixture\n');
  git('add', '.'); git('-c', 'user.name=e2e', '-c', 'user.email=e2e@example.invalid', 'commit', '-qm', 'Initial');
  const port = await freePort(), base = `http://${host}:${port}`;
  const server = process.env.TENDLY_E2E_SERVER ?? join(ROOT, 'target/debug/tendly-server');
  const output = [];
  const child = spawn(server, ['serve'], {
    cwd: home,
    stdio: ['ignore', 'pipe', 'pipe'],
    env: {
      ...process.env,
      // Its own home too: the clients' logs and settings it reads are not this machine's.
      HOME: join(home, 'home'),
      TENDLY_HOME: join(home, 'state'),
      TENDLY_ADDR: `127.0.0.1:${port}`,
      TENDLY_TOKEN: TOKEN,
      // A given server is tested as shipped, with the page and bridge built into it.
      ...(process.env.TENDLY_E2E_SERVER ? {} : { TENDLY_WEB_DIR: join(ROOT, 'apps/web/dist'), TENDLY_NATIVE_BRIDGE: join(ROOT, 'packages/native-bridge') }),
      ...(process.env.TENDLY_E2E_WEB_DIR ? { TENDLY_WEB_DIR: process.env.TENDLY_E2E_WEB_DIR } : {}),
      TENDLY_CLAUDE_BIN: await fixture(bin, 'claude-cli.mjs'),
      TENDLY_PI_BIN: await fixture(bin, 'pi-cli.mjs'),
      TENDLY_CODEX_BIN: await fixture(bin, 'chat-cli.mjs'),
      TENDLY_CHAT_TEST_LOG: join(home, 'clients.log'),
      TENDLY_CHAT_TEST_INHERITED: 'kept',
      TENDLY_NO_FANS: '1',
      ...env(base),
    },
  });
  child.stdout.on('data', chunk => output.push(String(chunk)));
  child.stderr.on('data', chunk => output.push(String(chunk)));
  const exited = new Promise(done => child.once('exit', done));
  // A test process that dies without stopping it must not leave the gateway behind.
  const orphan = () => { if (child.exitCode === null) child.kill('SIGKILL'); };
  process.once('exit', orphan);
  for (let i = 0; ; i++) {
    try { if ((await fetch(base + '/api/auth')).status < 500) break; } catch {}
    if (child.exitCode !== null || i > 300) throw Error(`gateway did not start:\n${output.join('')}`);
    await new Promise(done => setTimeout(done, 100));
  }
  return {
    base, home, workspace, token: TOKEN, output,
    async stop() {
      process.off('exit', orphan);
      if (child.exitCode === null) { child.kill('SIGTERM'); await Promise.race([exited, new Promise(done => setTimeout(done, 5000))]); }
      if (child.exitCode === null) child.kill('SIGKILL');
      if (!process.env.TENDLY_E2E_KEEP) await rm(home, { recursive: true, force: true });
    },
  };
}

/**
 * A headless browser signed in to `gateway`. It never takes a window: the
 * suite runs while the person keeps working. `TENDLY_E2E_CHROME` names
 * another Chrome-compatible executable.
 */
export async function openBrowser(gateway, viewport = { width: 1600, height: 1000 }) {
  const executablePath = process.env.TENDLY_E2E_CHROME ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
  const browser = await chromium.launch({ headless: true, executablePath });
  const context = await browser.newContext({ viewport, locale: 'zh-CN' });
  const errors = [];
  const headers = { 'X-Tendly-Client': 'web' };
  const login = await context.request.post(gateway.base + '/api/auth', { headers, data: { token: gateway.token } });
  if (!login.ok()) throw Error(`sign-in failed: ${login.status()} ${await login.text()}`);
  /** The gateway's HTTP API, as the page calls it. */
  async function api(path, data, method = data === undefined ? 'GET' : 'POST') {
    const response = await context.request.fetch(gateway.base + '/api' + path, { method, headers, data });
    const text = await response.text();
    if (!response.ok()) throw Error(`${method} ${path}: ${response.status()} ${text}`);
    return text ? JSON.parse(text) : undefined;
  }
  async function newPage() {
    const page = await context.newPage();
    page.on('pageerror', error => errors.push(error.message));
    // A failed request is named by its URL; the console line that repeats it says less.
    page.on('console', message => { if (message.type() === 'error' && !message.text().startsWith('Failed to load resource')) errors.push(message.text()); });
    page.on('response', response => { if (response.status() >= 400) errors.push(`${response.status()} ${response.request().method()} ${new URL(response.url()).pathname}`); });
    return page;
  }
  /** Between tests: pages from the last one closed, its errors forgotten. A page left open keeps saving the canvas. */
  async function reset() { for (const page of context.pages()) await page.close(); errors.length = 0; }
  return { browser, context, api, newPage, reset, errors, close: () => browser.close() };
}

/** A pane showing `session` on the canvas. */
export function paneFor(id, session) {
  return { type: 'pane', id, kind: session.provider === 'terminal' ? 'terminal' : 'agent_chat', title: session.title, metadata: { workspace_id: session.workspace_id, session_id: session.id } };
}
/** Panes side by side, left to right. */
export function row(...panes) {
  return panes.reduceRight((second, first, index) => ({ type: 'split', id: `row-${index}`, direction: 'horizontal', ratio: 1 / (panes.length - index), first, second }));
}
/** Replaces the canvas with `root`, as a saved layout the page loads. */
export async function setLayout(api, root) {
  const prior = await api('/canvas/layout');
  await api('/canvas/layout', { expected_revision: prior.revision, layout: { version: 1, root } }, 'PUT');
}
/** A workspace on the gateway's fixture repository, and sessions in it. */
export async function seed(api, gateway, providers) {
  const workspace = await api('/workspaces', { name: 'E2E', root_path: gateway.workspace });
  const sessions = [];
  for (const provider of providers) {
    const session = await api(`/workspaces/${workspace.id}/sessions`, { provider, title: `${provider} e2e`, ...(provider === 'terminal' ? {} : { interaction_mode: 'structured' }) });
    // The page starts a terminal it creates; one made here is started here.
    // A conversation starts when its composer is first used.
    if (provider === 'terminal') await api(`/sessions/${session.id}/start`, {});
    sessions.push(session);
  }
  return { workspace, sessions };
}
/**
 * Types `text` into a conversation pane and sends it. Focusing the composer
 * starts a stopped client; Enter is pressed once the send button takes it.
 */
export async function sendMessage(page, paneId, text) {
  const pane = page.locator(`[data-pane-id="${paneId}"]`);
  const input = pane.locator('textarea[data-pane-input]');
  await input.click(); await input.fill(text);
  await pane.locator('button.chat-send:not(.chat-stop):not([disabled])').waitFor();
  await input.press('Enter');
}
/** What the conversation error banner says, if anything. */
export async function chatError(page, paneId) {
  const banner = page.locator(`[data-pane-id="${paneId}"] .chat-alert`);
  return await banner.count() ? (await banner.first().innerText()).trim() : '';
}
/** Waits until `check` holds, polling; fails after `ms`. */
export async function until(check, ms = 10000, what = 'condition') {
  const deadline = Date.now() + ms;
  while (!(await check())) {
    if (Date.now() > deadline) throw Error(`timed out waiting for ${what}`);
    await new Promise(done => setTimeout(done, 100));
  }
}
