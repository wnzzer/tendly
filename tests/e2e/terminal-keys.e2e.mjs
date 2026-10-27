// The on-screen keys a phone gets in a terminal, pressed by a fingertip, and
// the bytes the program behind them actually receives. Claude Code and Codex
// are stood in for by a program that prints each read in hex, so what is
// checked is what the client would read, not what the page meant to send.
import { after, afterEach, before, test } from 'node:test';
import assert from 'node:assert/strict';
import { chmod, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { openBrowser, paneFor, row, setLayout, startGateway, until } from './harness.mjs';

const ECHO = `#!/usr/bin/env node
process.stdin.setRawMode?.(true);
process.stdout.write('KEYS READY\\r\\n');
process.stdin.on('data', data => process.stdout.write('KEY ' + data.toString('hex') + '\\r\\n'));
`;
let dir, gateway, browser, workspace;
const contexts = [];
before(async () => {
  dir = await mkdtemp(join(tmpdir(), 'tendly-keys-'));
  const echo = join(dir, 'key-echo.mjs');
  await writeFile(echo, ECHO); await chmod(echo, 0o755);
  gateway = await startGateway({ env: () => ({ TENDLY_CLAUDE_BIN: echo, TENDLY_CODEX_BIN: echo }) });
  browser = await openBrowser(gateway);
  workspace = await browser.api('/workspaces', { name: 'Keys', root_path: gateway.workspace });
});
afterEach(async () => { for (const context of contexts.splice(0)) await context.close(); });
after(async () => { await browser?.close(); await gateway?.stop(); if (dir) await rm(dir, { recursive: true, force: true }); });

/** A phone showing one running session of `provider` in a terminal. */
async function phoneTerminal(provider) {
  const session = await browser.api(`/workspaces/${workspace.id}/sessions`, { provider, title: `${provider} keys`, interaction_mode: 'pty' });
  await browser.api(`/sessions/${session.id}/start`, {});
  await setLayout(browser.api, row(paneFor('keys', session)));
  const context = await browser.browser.newContext({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true, deviceScaleFactor: 2, locale: 'zh-CN' });
  contexts.push(context);
  await context.request.post(gateway.base + '/api/auth', { headers: { 'X-Tendly-Client': 'web' }, data: { token: gateway.token } });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(gateway.base);
  // An agent's session opens on its conversation; its terminal is one step away.
  if (provider !== 'terminal') await page.getByText('打开原生客户端视图').filter({ visible: true }).first().tap();
  const bar = page.locator('.terminal-keys');
  await bar.waitFor();
  return { page, bar, errors, session };
}
const screen = async page => (await page.locator('.terminal-host').innerText()).split('\n').map(line => line.trim()).filter(Boolean);
const reads = async page => (await screen(page)).filter(line => line.startsWith('KEY ')).map(line => line.slice(4));
const mainKeys = bar => bar.locator('.terminal-keys-row').last().locator('[data-key]').evaluateAll(keys => keys.map(key => key.dataset.key));
/** Taps a key on the bar and returns the reads the program saw because of it. */
async function tap(page, bar, key, expected = 1) {
  const before = (await reads(page)).length;
  await bar.locator(`[data-key="${key}"]`).tap();
  let seen = [];
  await until(async () => (seen = (await reads(page)).slice(before)).length >= expected, 5000, `the program to read ${key}`);
  await page.waitForTimeout(150);
  return (await reads(page)).slice(before);
}

test('Claude Code gets its own keys first, each sending what the key would', async () => {
  const { page, bar, errors } = await phoneTerminal('claude_code');
  await until(async () => (await screen(page)).includes('KEYS READY'), 15000, 'the stand-in to start');
  assert.equal(await bar.getAttribute('data-client'), 'claude_code');
  assert.deepEqual(await mainKeys(bar), ['esc', 'shiftTab', 'ctrlC', 'slash', 'left', 'up', 'down', 'right', 'enter']);
  assert.equal(await bar.getByRole('button', { name: '键盘' }).count(), 0, 'no key for the keyboard');
  assert.deepEqual(await tap(page, bar, 'esc'), ['1b']);
  assert.deepEqual(await tap(page, bar, 'shiftTab'), ['1b5b5a']);
  assert.deepEqual(await tap(page, bar, 'ctrlC'), ['03']);
  assert.deepEqual(await tap(page, bar, 'slash'), ['2f']);
  assert.match((await tap(page, bar, 'up'))[0], /^1b(5b|4f)41$/);
  assert.deepEqual(await tap(page, bar, 'enter'), ['0d']);

  await bar.getByRole('button', { name: '更多按键' }).tap();
  const more = await bar.locator('.terminal-keys-row').first().locator('[data-key]').evaluateAll(keys => keys.map(key => key.dataset.key));
  assert.deepEqual(more, ['newline', 'escEsc', 'ctrlO', 'ctrlT', 'tab', 'ctrl', 'shift']);
  assert.equal((await bar.locator('[data-key="escEsc"]').getAttribute('title')), '连按两次 Esc · 清空输入');
  assert.deepEqual(await tap(page, bar, 'newline'), ['0a']);
  assert.deepEqual(await tap(page, bar, 'escEsc', 2), ['1b', '1b'], 'two presses, not Alt+Esc');
  assert.deepEqual(await tap(page, bar, 'ctrlO'), ['0f']);
  // Shift armed, then Tab, is still Shift+Tab.
  await bar.locator('[data-key="shift"]').tap();
  assert.equal(await bar.locator('[data-key="shift"]').getAttribute('aria-pressed'), 'true');
  assert.deepEqual(await tap(page, bar, 'tab'), ['1b5b5a']);
  assert.equal(await bar.locator('[data-key="shift"]').getAttribute('aria-pressed'), 'false', 'released after one key');
  assert.deepEqual(errors, []);
});

test('Codex keeps the same first row and has its own behind more', async () => {
  const { page, bar, errors } = await phoneTerminal('codex');
  await until(async () => (await screen(page)).includes('KEYS READY'), 15000, 'the stand-in to start');
  assert.deepEqual(await mainKeys(bar), ['esc', 'shiftTab', 'ctrlC', 'slash', 'left', 'up', 'down', 'right', 'enter']);
  await bar.getByRole('button', { name: '更多按键' }).tap();
  assert.equal(await bar.locator('[data-key="escEsc"]').getAttribute('title'), '连按两次 Esc · 编辑上一条消息');
  assert.deepEqual(await tap(page, bar, 'ctrlR'), ['12']);
  assert.deepEqual(await tap(page, bar, 'ctrlT'), ['14']);
  assert.deepEqual(errors, []);
});

test('a shell gets shell keys, and a tap on the terminal brings the keyboard up', async () => {
  const { page, bar, errors } = await phoneTerminal('terminal');
  await until(async () => (await screen(page).catch(() => [])).length > 0, 15000, 'the prompt');
  await page.waitForTimeout(500);
  assert.equal(await bar.getAttribute('data-client'), 'shell');
  assert.deepEqual(await mainKeys(bar), ['tab', 'ctrlC', 'ctrlR', 'ctrlD', 'left', 'up', 'down', 'right', 'enter']);
  await page.evaluate(() => document.activeElement?.blur());
  await page.locator('.terminal-host').tap();
  await until(() => page.evaluate(() => {
    const element = document.activeElement;
    return !!element?.closest('.terminal-host') && (element.tagName === 'TEXTAREA' || element.isContentEditable || element.tabIndex >= 0);
  }), 5000, 'the terminal to take the keyboard');
  // Typed on the soft keyboard, then sent with the bar's own keys.
  await page.keyboard.type('echo bar-$((40+2))');
  await bar.locator('[data-key="enter"]').tap();
  await until(async () => (await screen(page)).includes('bar-42'), 5000, 'the command to run');
  assert.deepEqual(errors, []);
});
