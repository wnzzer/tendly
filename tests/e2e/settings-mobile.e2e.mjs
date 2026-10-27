// Settings and the app's own pages, endpoint profiles, signing in, and the
// whole app on a phone. Endpoints are a local HTTP server started here, so no
// test reaches the internet; Tailscale and updates are only looked at, never
// switched, since they would act on this machine.
import { after, afterEach, before, test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { openBrowser, paneFor, row, seed, sendMessage, setLayout, startGateway, until } from './harness.mjs';

let gateway, browser, endpoint;
const extra = [];
before(async () => {
  gateway = await startGateway();
  browser = await openBrowser(gateway);
  endpoint = await fakeEndpoint();
});
afterEach(async () => {
  for (const context of extra.splice(0)) await context.close();
  await browser.reset();
});
after(async () => { await browser?.close(); await gateway?.stop(); await endpoint?.close(); });

/**
 * A stand-in endpoint: Anthropic's model list and messages, and a balance.
 * It records what it was sent, so a test can see which key arrived.
 */
async function fakeEndpoint() {
  const seen = [];
  const server = createServer((request, response) => {
    let body = '';
    request.on('data', chunk => { body += chunk; });
    request.on('end', () => {
      seen.push({ method: request.method, url: request.url, headers: request.headers, body });
      response.setHeader('content-type', 'application/json');
      if (request.url.startsWith('/v1/models')) return response.end(JSON.stringify({ data: [{ id: 'mock-sonnet', type: 'model', display_name: 'Mock Sonnet' }, { id: 'mock-haiku', type: 'model', display_name: 'Mock Haiku' }], has_more: false }));
      if (request.url.startsWith('/v1/messages')) return response.end(JSON.stringify({ id: 'msg_1', type: 'message', role: 'assistant', model: 'mock-sonnet', content: [{ type: 'text', text: 'ok' }], stop_reason: 'end_turn', usage: { input_tokens: 1, output_tokens: 1 } }));
      if (request.url.startsWith('/balance')) return response.end(JSON.stringify({ balance: '12.34' }));
      response.statusCode = 404; response.end('{}');
    });
  });
  await new Promise(done => server.listen(0, '127.0.0.1', done));
  return { url: `http://127.0.0.1:${server.address().port}`, seen, close: () => new Promise(done => server.close(done)) };
}

/**
 * The page's errors, less one known artefact: opening or reloading a page's
 * own address (/settings/…, /usage) is answered with index.html under a 404
 * status when the page is served from disk, as it is here (see the todo test
 * below). Everything else still counts.
 */
const ROUTE_404 = /^404 GET \/(settings(\/\w+)?|usage|system|sessions)$/;
const pageErrors = (errors = browser.errors) => errors.filter(error => !ROUTE_404.test(error));

const profileInput = (overrides = {}) => ({ provider: 'claude_code', endpoint_url: endpoint.url, model: 'mock-sonnet', permission_mode: 'native', secret_ref: null, proxy_url: null, effort: null, model_aliases: {}, ...overrides });
const unique = name => `${name} ${Math.random().toString(36).slice(2, 7)}`;

/** One Claude Code conversation on the canvas, and the page open on it. */
async function canvasPage(providers = ['claude_code']) {
  const { workspace, sessions } = await seed(browser.api, gateway, providers);
  await setLayout(browser.api, row(...sessions.map((session, index) => paneFor(`p${index}`, session))));
  const page = await browser.newPage();
  await page.goto(gateway.base);
  await page.locator('[data-pane-id="p0"]').waitFor();
  return { page, workspace, sessions };
}
async function prefix(page, key) {
  await page.keyboard.press('Control+b');
  await page.locator('.prefix-hint').waitFor();
  await page.keyboard.press(key);
}
const path = page => new URL(page.url()).pathname;
async function openSettings(page, section) {
  await page.getByRole('button', { name: '设置', exact: true }).first().click();
  await page.locator('.settings-nav').waitFor();
  if (section) await page.locator('.settings-nav-item', { hasText: section }).click();
}

/** A phone, signed in like the desktop browser, with its own error list. */
async function phone(viewport = { width: 390, height: 844 }) {
  const context = await browser.browser.newContext({ viewport, isMobile: true, hasTouch: true, deviceScaleFactor: 2, locale: 'zh-CN' });
  extra.push(context);
  const login = await context.request.post(gateway.base + '/api/auth', { headers: { 'X-Tendly-Client': 'web' }, data: { token: gateway.token } });
  assert.ok(login.ok(), 'phone signs in');
  const errors = [];
  const page = await context.newPage();
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => { if (message.type() === 'error' && !message.text().startsWith('Failed to load resource')) errors.push(message.text()); });
  page.on('response', response => { if (response.status() >= 400) errors.push(`${response.status()} ${response.request().method()} ${new URL(response.url()).pathname}`); });
  return { context, page, errors };
}
/** Rendered elements that cross the left or right edge of the screen, other than inside something that scrolls or clips them. */
const offscreen = page => page.evaluate(() => {
  const width = innerWidth, out = [];
  for (const element of document.querySelectorAll('body *')) {
    const box = element.getBoundingClientRect();
    if (!box.width || !box.height || (box.right <= width + 1 && box.left >= -1)) continue;
    // A closed <details> keeps its content laid out but never paints it.
    if (element.closest('details:not([open]) > :not(summary)')) continue;
    let clipped = false;
    for (let parent = element.parentElement; parent && !clipped; parent = parent.parentElement) {
      if (!/(auto|scroll|hidden|clip)/.test(getComputedStyle(parent).overflowX)) continue;
      const frame = parent.getBoundingClientRect();
      clipped = frame.right <= width + 1 && frame.left >= -1;
    }
    if (!clipped) out.push(`${element.tagName}.${element.className?.baseVal ?? element.className} ${Math.round(box.left)}–${Math.round(box.right)}`);
  }
  return { scrollWidth: document.scrollingElement.scrollWidth, out };
});
const inside = (box, viewport) => !!box && box.x >= 0 && box.y >= 0 && box.x + box.width <= viewport.width + 0.5 && box.y + box.height <= viewport.height + 0.5;

/** Contrast of each piece of visible text against what is painted under it. */
const contrasts = (page, root) => page.evaluate(root => {
  const parse = value => { const match = value.match(/rgba?\(([^)]+)\)/); if (!match) return null; const parts = match[1].split(/[,\s/]+/).filter(Boolean).map(Number); return [parts[0], parts[1], parts[2], parts[3] ?? 1]; };
  const blend = (top, under) => [0, 1, 2].map(i => top[i] * top[3] + under[i] * (1 - top[3])).concat(1);
  const luminance = rgb => { const f = v => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(rgb[0]) + 0.7152 * f(rgb[1]) + 0.0722 * f(rgb[2]); };
  function backdrop(element) {
    const layers = [];
    for (let at = element; at; at = at.parentElement) { const color = parse(getComputedStyle(at).backgroundColor); if (color && color[3] > 0) { layers.push(color); if (color[3] >= 1) break; } }
    let base = parse(getComputedStyle(document.body).backgroundColor) ?? [255, 255, 255, 1];
    if (base[3] < 1) base = [255, 255, 255, 1];
    for (const layer of layers.reverse()) base = blend(layer, base);
    return base;
  }
  const out = [];
  for (const element of document.querySelector(root)?.querySelectorAll('*') ?? []) {
    const text = [...element.childNodes].filter(node => node.nodeType === 3).map(node => node.textContent.trim()).join('');
    const box = element.getBoundingClientRect();
    if (!text || !box.width || !box.height || element.closest('[aria-hidden="true"], .sr-only, :disabled, details:not([open]) > :not(summary)')) continue;
    const style = getComputedStyle(element);
    if (style.visibility === 'hidden' || Number(style.opacity) === 0) continue;
    const under = backdrop(element), color = blend(parse(style.color), under);
    const [a, b] = [luminance(color), luminance(under)];
    out.push({ ratio: (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05), text: text.slice(0, 30), heading: element.matches('h1, h2, h3, strong, label, button, a') });
  }
  return out;
}, root);

// ---------------------------------------------------------------- Desktop

test('Ctrl+B opens each page and Esc goes back; ? and : open the help and the palette', async () => {
  const { page } = await canvasPage();
  for (const [key, where, heading] of [[',', '/settings/preferences', '设置'], ['U', '/usage', '用量'], ['M', '/system', undefined], ['S', '/sessions', '全部会话']]) {
    await prefix(page, key);
    await page.locator('.page-shell').waitFor();
    assert.equal(path(page), where, `Ctrl+B ${key}`);
    if (heading) assert.equal(await page.locator('.page-shell h1').first().innerText(), heading);
    await page.keyboard.press('Escape');
    await page.locator('.page-shell').waitFor({ state: 'detached' });
    assert.equal(path(page), '/', `Esc leaves ${where}`);
  }
  await prefix(page, '?');
  await page.getByRole('dialog', { name: '键盘快捷键' }).waitFor();
  await page.keyboard.press('Escape');
  assert.equal(await page.locator('[aria-modal="true"]').count(), 0, 'Esc closes the shortcuts');
  await prefix(page, ':');
  await page.locator('.palette input').fill('用量');
  await page.locator('.palette-item', { hasText: '用量' }).first().waitFor();
  await page.keyboard.press('Enter');
  await page.locator('.page-shell').waitFor();
  assert.equal(path(page), '/usage', 'the palette goes to a page');
  await page.locator('.page-back').click();
  await page.locator('.page-shell').waitFor({ state: 'detached' });
  assert.deepEqual(browser.errors, []);
});

test('the theme is kept by the server: it survives a reload, reaches another browser and the palette can change it', async () => {
  const { page } = await canvasPage();
  try {
    await openSettings(page);
    await page.getByLabel('明暗模式').selectOption('dark');
    await until(async () => (await browser.api('/preferences')).appearance === 'dark', 5000, 'dark saved');
    assert.equal(await page.evaluate(() => document.documentElement.dataset.theme), 'dark');
    await page.reload();
    await page.getByLabel('明暗模式').waitFor();
    assert.equal(await page.getByLabel('明暗模式').inputValue(), 'dark');
    assert.equal(await page.evaluate(() => document.documentElement.dataset.theme), 'dark');
    // A browser that has never seen it gets it from the server.
    const { page: other } = await phone();
    await other.goto(gateway.base);
    await other.waitForFunction(() => document.documentElement.dataset.theme === 'dark');
    // The palette's own command switches back.
    await page.keyboard.press('Escape');
    await page.locator('.page-shell').waitFor({ state: 'detached' });
    await page.keyboard.press('Meta+k');
    await page.locator('.palette input').fill('浅色');
    await page.locator('.palette-item', { hasText: '浅色' }).first().waitFor();
    await page.keyboard.press('Enter');
    await until(async () => (await browser.api('/preferences')).appearance === 'light', 5000, 'light saved');
    assert.equal(await page.evaluate(() => document.documentElement.dataset.theme), 'light');
    assert.deepEqual(pageErrors(), []);
  } finally {
    const prefs = await browser.api('/preferences'); delete prefs.appearance;
    await browser.api('/preferences', prefs, 'PUT');
  }
});

test('the language is this browser\'s: English survives a reload, and Chinese comes back', async () => {
  const { page } = await canvasPage();
  try {
    await openSettings(page);
    await page.getByLabel('语言').selectOption('en');
    await page.locator('.settings-nav-item', { hasText: 'Endpoint profiles' }).waitFor();
    assert.equal(await page.evaluate(() => document.documentElement.lang), 'en');
    await page.reload();
    await page.locator('.settings-nav-item', { hasText: 'Endpoint profiles' }).waitFor();
    assert.deepEqual(await page.locator('.settings-nav-item').allInnerTexts(), ['Preferences', 'Agent clients', 'Endpoint profiles', 'Official accounts', 'Remote access', 'Machines', 'Updates']);
    await page.getByLabel('Language').selectOption('zh-CN');
    await page.locator('.settings-nav-item', { hasText: '端点配置' }).waitFor();
    assert.deepEqual(pageErrors(), []);
  } finally {
    await page.evaluate(() => { for (const key of Object.keys(localStorage)) if (/lang|locale/i.test(key)) localStorage.removeItem(key); });
  }
});

test('in the dark theme every settings section, Usage and System are readable', async () => {
  const { page } = await canvasPage();
  const prefs = await browser.api('/preferences');
  await browser.api('/preferences', { ...prefs, appearance: 'dark' }, 'PUT');
  try {
    await page.reload();
    await page.waitForFunction(() => document.documentElement.dataset.theme === 'dark');
    await openSettings(page);
    const pages = [];
    for (const section of ['首选项', '智能体客户端', '端点配置', '官方账号', '远程访问', '更新']) {
      await page.locator('.settings-nav-item', { hasText: section }).click();
      await page.locator('.settings-nav-item.selected', { hasText: section }).waitFor();
      await page.waitForTimeout(400);
      pages.push([section, await contrasts(page, '.page-shell')]);
    }
    for (const key of ['U', 'M']) {
      await prefix(page, key);
      await page.waitForTimeout(800);
      pages.push([key, await contrasts(page, '.page-shell')]);
    }
    for (const [name, samples] of pages) {
      assert.ok(samples.length > 3, `${name} has text`);
      // The faint metadata tone sits near 2.5:1 in both themes by design; nothing may fall below that.
      assert.deepEqual(samples.filter(sample => sample.ratio < 2.2), [], `${name}: no text close to invisible`);
      assert.deepEqual(samples.filter(sample => sample.heading && sample.ratio < 4.5 && !/^\d/.test(sample.text)).map(sample => sample.text), [], `${name}: titles and controls read clearly`);
    }
    assert.deepEqual(pageErrors(), []);
  } finally {
    const now = await browser.api('/preferences'); delete now.appearance;
    await browser.api('/preferences', now, 'PUT');
  }
});

test('a profile is created from the form: its key is stored, its models are read from the endpoint, and it is listed', async () => {
  const { page } = await canvasPage();
  await openSettings(page, '端点配置');
  await page.getByRole('button', { name: '新建配置' }).click();
  const form = page.locator('form.profile-form');
  const create = form.getByRole('button', { name: '创建配置' });
  assert.ok(await create.isDisabled(), 'nothing to create without a name');
  const name = unique('Fake Claude');
  await form.locator('.setting-row', { hasText: '名称' }).locator('input').fill(name);
  await form.locator('.setting-row', { hasText: '端点 URL' }).locator('input').fill(endpoint.url);
  await form.locator('input[type=password]').fill('sk-e2e-key');
  const before = endpoint.seen.length;
  await form.getByRole('button', { name: '从端点拉取模型' }).click();
  await form.getByText('已返回 2 个模型').waitFor();
  const asked = endpoint.seen.slice(before).find(entry => entry.url.startsWith('/v1/models'));
  assert.equal(asked?.headers['x-api-key'], 'sk-e2e-key', 'the pasted key reaches the endpoint');
  await create.click();
  await page.locator('.profile-card', { hasText: name }).waitFor();
  const saved = (await browser.api('/endpoint-profiles')).find(profile => profile.name === name);
  assert.equal(saved.endpoint_url, endpoint.url);
  assert.match(saved.secret_ref, /^env:TENDLY_SECRET_/, 'only a reference is kept on the profile');
  assert.ok(!JSON.stringify(saved).includes('sk-e2e-key'), 'the key itself never comes back');
  assert.equal(await form.locator('.profile-save-state').innerText(), '修改会自动保存', 'it opens for editing');
  assert.deepEqual(browser.errors, []);
});

test('an existing profile saves as it is edited, refuses an empty name, checks its model and can be deleted', async () => {
  const name = unique('Editable');
  const profile = await browser.api('/endpoint-profiles', profileInput({ name }));
  const { page } = await canvasPage();
  await openSettings(page, '端点配置');
  await page.locator('.profile-card', { hasText: name }).click();
  const form = page.locator('form.profile-form');
  const nameField = form.locator('.setting-row', { hasText: '名称' }).locator('input');
  const status = form.locator('.profile-save-state');
  await nameField.fill(name + ' renamed');
  assert.equal(await status.innerText(), '离开输入框后保存', 'typing is not saved yet');
  await nameField.press('Tab');
  await until(async () => (await browser.api('/endpoint-profiles')).some(item => item.id === profile.id && item.name === name + ' renamed'), 5000, 'rename saved');
  await status.getByText('已保存').waitFor();
  // An empty name is not saved, and leaving asks first.
  await nameField.fill(''); await nameField.press('Tab');
  await status.getByText('未保存：需要填写名称').waitFor();
  await page.getByRole('button', { name: '新建配置' }).click();
  await page.getByRole('button', { name: '继续编辑' }).click();
  await nameField.fill(name); await nameField.press('Tab');
  await until(async () => (await browser.api('/endpoint-profiles')).some(item => item.id === profile.id && item.name === name), 5000, 'name restored');
  // One very short request to the endpoint says whether its model runs.
  const before = endpoint.seen.length;
  await form.locator('.setting-check button', { hasText: '检测模型' }).click();
  await form.locator('.setting-check').getByText('可用').first().waitFor();
  assert.ok(endpoint.seen.slice(before).some(entry => entry.method === 'POST' && entry.url.startsWith('/v1/messages')), 'the model was asked');
  await form.locator('header').getByRole('button', { name: '删除' }).click();
  await page.getByRole('button', { name: '确认删除' }).click();
  await page.locator('.profile-card', { hasText: name }).waitFor({ state: 'detached' });
  assert.ok(!(await browser.api('/endpoint-profiles')).some(item => item.id === profile.id));
  assert.deepEqual(browser.errors, []);
});

test('a custom balance is tried, saved, and shown beside the endpoint in a conversation', async () => {
  const name = unique('Balance');
  const profile = await browser.api('/endpoint-profiles', profileInput({ name }));
  const { page } = await canvasPage();
  await openSettings(page, '端点配置');
  await page.locator('.profile-card', { hasText: name }).click();
  const editor = page.locator('.balance-editor');
  await editor.getByRole('radio', { name: '法币余额' }).click();
  await editor.locator('.setting-row', { hasText: '余额查询地址' }).locator('input').fill(endpoint.url + '/balance');
  await editor.locator('input.balance-currency').fill('USD');
  await editor.locator('select').filter({ has: page.locator('option[value=none]') }).selectOption('none');
  await editor.getByRole('button', { name: '试运行脚本' }).click();
  await editor.locator('.balance-preview').getByText('12.34').waitFor();
  await page.locator('.page-shell').click({ position: { x: 4, y: 4 } });
  await until(async () => (await browser.api(`/endpoint-profiles/${profile.id}/balance`)).config?.query_url === endpoint.url + '/balance', 5000, 'balance saved');
  // A session on that endpoint shows what is left in its endpoint chip.
  const { workspace } = await seed(browser.api, gateway, []);
  const session = await browser.api(`/workspaces/${workspace.id}/sessions`, { provider: 'claude_code', title: 'balance e2e', interaction_mode: 'structured', endpoint_profile_id: profile.id });
  await setLayout(browser.api, paneFor('b', session));
  await page.goto(gateway.base);
  const chip = page.locator('[data-pane-id="b"] .chat-endpoint > summary');
  await chip.getByText(name).waitFor();
  await chip.getByText('12.34').waitFor();
  assert.deepEqual(browser.errors, []);
});

test('a new session given an endpoint profile keeps it and launches the client on its model', async () => {
  const name = unique('Session profile');
  const profile = await browser.api('/endpoint-profiles', profileInput({ name }));
  const { page } = await canvasPage();
  await prefix(page, 'C');
  const draft = page.getByRole('dialog', { name: '新建会话' });
  await draft.locator('label', { hasText: '端点配置' }).locator('select').selectOption(profile.id);
  await draft.getByText(endpoint.url).waitFor();
  await draft.getByRole('button', { name: '创建会话' }).click();
  await draft.waitFor({ state: 'detached' });
  const created = (await browser.api('/sessions')).find(session => session.endpoint_profile_id === profile.id);
  assert.ok(created, 'the session names the profile');
  const id = await page.locator('.dock-pane.is-selected').getAttribute('data-pane-id');
  assert.equal((await page.locator(`[data-pane-id="${id}"] .chat-endpoint > summary`).innerText()).trim(), name);
  await sendMessage(page, id, 'hello');
  await page.locator(`[data-pane-id="${id}"]`).getByText('Hello 🦊').first().waitFor();
  const launches = (await readFile(join(gateway.home, 'clients.log'), 'utf8')).split('\n').filter(Boolean).map(line => JSON.parse(line)).filter(line => line.event === 'args');
  const args = launches.at(-1).args;
  assert.equal(args[args.indexOf('--model') + 1], 'mock-sonnet', 'the profile\'s model is passed to Claude Code');
  assert.deepEqual(browser.errors, []);
});

test('preferences for Claude Code are saved, survive a reload, and a one-click session starts on the default endpoint', async () => {
  const name = unique('Default');
  const profile = await browser.api('/endpoint-profiles', profileInput({ name }));
  const before = await browser.api('/preferences');
  await browser.api('/preferences', { ...before, claude_code: {} }, 'PUT');
  const { page } = await canvasPage();
  try {
    await openSettings(page);
    const card = page.locator('.preference-card', { hasText: 'Claude Code' });
    await card.locator('.preference-row', { hasText: '默认端点' }).locator('select').selectOption(profile.id);
    await card.locator('.preference-row', { hasText: '默认思考深度' }).locator('select').selectOption('high');
    await until(async () => (await browser.api('/preferences')).claude_code.effort === 'high', 5000, 'effort saved');
    assert.equal((await browser.api('/preferences')).claude_code.endpoint_profile_id, profile.id);
    await page.reload();
    await card.locator('select').first().waitFor();
    assert.deepEqual((await card.locator('select').evaluateAll(selects => selects.map(select => select.value))).slice(0, 2), [profile.id, 'high']);
    await page.keyboard.press('Escape');
    await page.locator('.page-shell').waitFor({ state: 'detached' });
    const known = new Set((await browser.api('/sessions')).map(session => session.id));
    await page.locator('.new-session-top').click();
    await until(async () => (await browser.api('/sessions')).some(session => !known.has(session.id)), 5000, 'a new session');
    const created = (await browser.api('/sessions')).find(session => !known.has(session.id));
    assert.equal(created.provider, 'claude_code');
    assert.equal(created.endpoint_profile_id, profile.id, 'the default endpoint is used');
    assert.deepEqual(pageErrors(), []);
  } finally { await browser.api('/preferences', before, 'PUT'); }
});

test('Usage, System and the clients, remote and update sections open without errors', async () => {
  const { page } = await canvasPage();
  await prefix(page, 'U');
  await page.locator('.page-shell h1', { hasText: '用量' }).waitFor();
  await page.getByRole('button', { name: '7D' }).click();
  await prefix(page, 'M');
  await page.locator('.page-shell').getByText('CPU').first().waitFor();
  await page.getByRole('button', { name: '24h', exact: true }).click();
  await openSettings(page, '智能体客户端');
  await page.locator('.client-card').first().waitFor();
  assert.deepEqual(await page.locator('.client-card strong').evaluateAll(items => items.map(item => item.firstChild.textContent.trim())), ['Claude Code', 'Codex', 'Pi']);
  await page.locator('.settings-nav-item', { hasText: '远程访问' }).click();
  await page.getByText('Tailscale').first().waitFor();
  await page.locator('.settings-nav-item', { hasText: '更新' }).click();
  await page.getByText('当前版本').waitFor();
  assert.deepEqual(browser.errors, []);
});

test('signing in: a browser without the cookie is asked for the token, refused a wrong one and let in with the right one', async () => {
  await seed(browser.api, gateway, ['claude_code']);
  const context = await browser.browser.newContext({ viewport: { width: 1280, height: 800 }, locale: 'zh-CN' });
  extra.push(context);
  const page = await context.newPage();
  const crashes = [];
  page.on('pageerror', error => crashes.push(error.message));
  await page.goto(gateway.base + '/settings/endpoints');
  const dialog = page.getByRole('dialog', { name: '连接 Tendly' });
  await dialog.waitFor();
  const connect = dialog.getByRole('button', { name: '安全连接' });
  assert.ok(await connect.isDisabled());
  await page.keyboard.press('Escape');
  await page.keyboard.press('Meta+k');
  assert.ok(await dialog.isVisible(), 'Esc does not dismiss it');
  assert.equal(await page.locator('.palette').count(), 0, 'shortcuts wait until signed in');
  const token = dialog.locator('input[name=password]');
  await token.fill('not-the-token'); await token.press('Enter');
  await dialog.locator('.inline-error').waitFor();
  await token.fill(gateway.token); await token.press('Enter');
  await dialog.waitFor({ state: 'detached' });
  await page.locator('.settings-nav-item.selected', { hasText: '端点配置' }).waitFor();
  assert.equal(path(page), '/settings/endpoints', 'the address asked for is kept');
  await page.reload();
  await page.locator('.settings-nav').waitFor();
  assert.equal(await page.locator('[aria-modal="true"]').count(), 0, 'the cookie keeps it signed in');
  assert.deepEqual(crashes, []);
});

// ------------------------------------------------------------------ Phone

test('phone: one view at a time, switched from the title sheet and by swiping, stopping at either end', async () => {
  await seed(browser.api, gateway, []);
  const { sessions } = await seed(browser.api, gateway, ['claude_code', 'codex', 'terminal']);
  await setLayout(browser.api, row(paneFor('a', sessions[0]), paneFor('b', sessions[1]), paneFor('c', sessions[2])));
  const { context, page, errors } = await phone();
  await page.goto(gateway.base);
  await page.locator('.app-shell.is-mobile').waitFor();
  const title = page.locator('.mobile-title strong').first();
  await title.getByText('claude_code e2e').waitFor();
  assert.equal(await page.locator('.mobile-title-count').innerText(), '3');
  assert.equal(await page.locator('.statusbar').count(), 0, 'no status bar on a phone');
  await page.locator('.mobile-title').tap();
  const sheet = page.locator('.switcher-list');
  assert.deepEqual(await sheet.locator('.sheet-row strong').allInnerTexts(), ['claude_code e2e', 'codex e2e', 'terminal e2e']);
  await sheet.locator('.sheet-row', { hasText: 'codex e2e' }).tap();
  await title.getByText('codex e2e').waitFor();
  const touch = await context.newCDPSession(page);
  async function swipe(from, to) {
    const y = 420;
    await touch.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: from, y }] });
    for (let step = 1; step <= 8; step++) {
      await touch.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x: from + (to - from) * step / 8, y }] });
      await page.waitForTimeout(16);
    }
    await touch.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
    await page.waitForTimeout(500);
  }
  await swipe(320, 60); await title.getByText('terminal e2e').waitFor();
  await swipe(320, 60); assert.equal(await title.innerText(), 'terminal e2e', 'the last view stays');
  await swipe(60, 320); await title.getByText('codex e2e').waitFor();
  await swipe(60, 320); await title.getByText('claude_code e2e').waitFor();
  await swipe(60, 320); assert.equal(await title.innerText(), 'claude_code e2e', 'the first view stays');
  assert.deepEqual(errors, []);
});

test('phone: composer cards, the session menu and the palette open on screen and close again', async () => {
  for (const viewport of [{ width: 390, height: 844 }, { width: 320, height: 640 }]) {
    const { sessions } = await seed(browser.api, gateway, ['claude_code']);
    await setLayout(browser.api, paneFor('a', sessions[0]));
    const { page, errors } = await phone(viewport);
    await page.goto(gateway.base);
    await page.locator('.chat-composer.is-phone').waitFor();
    assert.deepEqual(await offscreen(page), { scrollWidth: viewport.width, out: [] }, `${viewport.width}: canvas`);
    const chips = page.locator('.chat-composer .chip-menu > summary:visible');
    assert.ok(await chips.count() >= 2, 'endpoint and thinking-depth chips');
    for (let index = 0; index < await chips.count(); index++) {
      await chips.nth(index).tap();
      const card = page.locator('.chip-menu[open] .chip-menu-panel');
      await card.waitFor();
      await until(async () => inside(await card.boundingBox(), viewport), 3000, `${viewport.width}: card ${index} on screen`);
      // A tap on the conversation, clear of the top bar's own branch chip.
      await page.mouse.click(8, viewport.height / 3);
      await page.locator('.chip-menu[open]').waitFor({ state: 'detached' });
    }
    await page.locator('details.chat-floating-menu > summary:visible').tap();
    const menu = page.locator('details.chat-floating-menu[open] nav');
    await menu.waitFor();
    // It slides up as it opens; where it settles is what counts.
    await until(async () => inside(await menu.boundingBox(), viewport), 3000, `${viewport.width}: session menu on screen`);
    await page.mouse.click(8, viewport.height / 3);
    await page.locator('details.chat-floating-menu[open]').waitFor({ state: 'detached' });
    await page.getByLabel('搜索与命令').first().tap();
    await page.locator('.palette').waitFor();
    await until(async () => inside(await page.locator('.palette').boundingBox(), viewport), 3000, `${viewport.width}: palette on screen`);
    await page.keyboard.press('Escape');
    await page.locator('.palette').waitFor({ state: 'detached' });
    assert.deepEqual(await offscreen(page), { scrollWidth: viewport.width, out: [] }, `${viewport.width}: nothing left over`);
    assert.deepEqual(errors, []);
    await page.context().close();
  }
});

test('phone: an endpoint picked in the composer card is confirmed, then used by the session', async () => {
  const name = unique('Phone endpoint');
  const profile = await browser.api('/endpoint-profiles', profileInput({ name }));
  const { sessions } = await seed(browser.api, gateway, ['claude_code']);
  await setLayout(browser.api, paneFor('a', sessions[0]));
  const { page, errors } = await phone();
  await page.goto(gateway.base);
  await page.locator('.chat-composer .chat-model > summary').tap();
  await page.locator('.chip-menu[open] button', { hasText: name }).tap();
  await page.getByRole('button', { name: '确认切换端点' }).tap();
  await until(async () => (await browser.api('/sessions')).find(session => session.id === sessions[0].id)?.endpoint_profile_id === profile.id, 5000, 'endpoint applied');
  // The chip names the endpoint's model: its id at once, its display name once the endpoint's list arrives.
  await page.locator('.chat-composer .chat-model > summary').getByText(/mock[- ]sonnet/i).waitFor();
  assert.deepEqual(errors, []);
});

test('phone: settings is reached from the drawer, every section fits the screen, and back returns to the conversation', async () => {
  const { sessions } = await seed(browser.api, gateway, ['claude_code']);
  await setLayout(browser.api, paneFor('a', sessions[0]));
  const { page, errors } = await phone();
  await page.goto(gateway.base);
  await page.locator('.chat-composer.is-phone').waitFor();
  await page.locator('.mobile-menu').first().tap();
  await page.locator('.sidebar.drawer-open .drawer-footer').getByRole('button', { name: '设置' }).tap();
  await page.locator('.mobile-page-title', { hasText: '设置' }).waitFor();
  for (const section of ['首选项', '智能体客户端', '端点配置', '官方账号', '远程访问', '更新']) {
    await page.locator('.settings-nav-item', { hasText: section }).tap();
    await page.locator('.settings-nav-item.selected', { hasText: section }).waitFor();
    await page.waitForTimeout(300);
    assert.deepEqual(await offscreen(page), { scrollWidth: 390, out: [] }, section);
  }
  await page.locator('.page-back').tap();
  await page.locator('.page-shell').waitFor({ state: 'detached' });
  await page.locator('.chat-composer.is-phone').waitFor();
  assert.deepEqual(errors, []);
});

// ------------------------------------------------------------- Known bugs

// Regression (found by the exploratory run): AgentClientsPanel loads only on mount, before the backend capabilities arrive, so a reload or link shows an empty panel
test('reloading on Settings → Agent clients lists the clients', async () => {
  const { page } = await canvasPage();
  await openSettings(page, '智能体客户端');
  await page.locator('.client-card').first().waitFor();
  await page.reload();
  await page.locator('.clients-page').waitFor();
  await page.locator('.client-card').first().waitFor({ timeout: 5000 });
});

// Regression (found by the exploratory run): PageShell ignores Esc whenever any details[open] exists, including an expanded tool call on the canvas
test('Esc leaves a page while a tool call is open in a conversation behind it', async () => {
  const { page } = await canvasPage();
  await sendMessage(page, 'p0', 'tools');
  await page.locator('[data-pane-id="p0"]').getByText('Hello 🦊').first().waitFor();
  await page.locator('[data-pane-id="p0"] details.chat-tool > summary').first().click();
  await prefix(page, ',');
  await page.locator('.page-shell').waitFor();
  await page.keyboard.press('Escape');
  await page.locator('.page-shell').waitFor({ state: 'detached', timeout: 3000 });
});

// Regression (found by the exploratory run): ModalDialog focuses the first input, the hidden read-only username, so a typed or pasted token goes nowhere
test('the sign-in dialog puts the cursor in the token field', async () => {
  const context = await browser.browser.newContext({ viewport: { width: 1280, height: 800 }, locale: 'zh-CN' });
  extra.push(context);
  const page = await context.newPage();
  await page.goto(gateway.base);
  await page.getByRole('dialog', { name: '连接 Tendly' }).waitFor();
  await page.keyboard.type(gateway.token);
  assert.equal(await page.locator('input[name=password]').inputValue(), gateway.token);
});

// Regression (found by the exploratory run): HostUsage polls /api/host/resources every 4 s behind the sign-in dialog, each a 401
test('a signed-out page does not poll the API', async () => {
  const context = await browser.browser.newContext({ viewport: { width: 1280, height: 800 }, locale: 'zh-CN' });
  extra.push(context);
  const page = await context.newPage();
  const refused = [];
  page.on('response', response => { if (response.status() === 401) refused.push(new URL(response.url()).pathname); });
  await page.goto(gateway.base);
  await page.getByRole('dialog', { name: '连接 Tendly' }).waitFor();
  await page.waitForTimeout(5000);
  assert.deepEqual(refused, []);
});

// Regression (found by the exploratory run): CommandPalette's (pointer:coarse) rule comes before kbd{display:inline-flex} and loses, so phones show "Ctrl+B S" hints
test('the palette hides its key hints on a touch screen', async () => {
  await seed(browser.api, gateway, []);
  const { page } = await phone();
  await page.goto(gateway.base);
  await page.locator('.app-shell.is-mobile').waitFor();
  await page.getByLabel('搜索与命令').first().tap();
  await page.locator('.palette kbd').first().waitFor({ state: 'attached' });
  assert.deepEqual(await page.locator('.palette kbd').evaluateAll(keys => keys.filter(key => getComputedStyle(key).display !== 'none').map(key => key.textContent)), []);
});

// Regression (found by the exploratory run): served from disk, /settings/… and /usage come back as index.html under a 404 status (ServeDir not_found_service)
test('a page\'s own address is answered with 200', async () => {
  for (const route of ['/settings/preferences', '/usage', '/system', '/sessions']) {
    const response = await fetch(gateway.base + route);
    assert.equal(response.status, 200, route);
  }
});
