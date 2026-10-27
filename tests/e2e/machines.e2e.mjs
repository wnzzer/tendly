// Two machines paired through their pages: an invite made on one, redeemed on
// the other, each one's sessions seen and opened from the other, an agent's
// call sent across, a grant changed and the pairing removed. Both gateways run
// here, one opened as 127.0.0.1 and one as localhost, so their sign-in cookies
// stay apart as two machines' would; each says where the other reaches it.
import { after, before, test } from 'node:test';
import assert from 'node:assert/strict';
import { openBrowser, seed, startGateway, until } from './harness.mjs';

let here, there, mine, theirs, remote;
before(async () => {
  const machine = name => base => ({ TENDLY_INSTANCE_LABEL: name, TENDLY_PUBLIC_URL: `${base}/` });
  [here, there] = await Promise.all([
    startGateway({ env: machine('mac-here') }),
    startGateway({ host: 'localhost', env: machine('box-there') }),
  ]);
  [mine, theirs] = await Promise.all([openBrowser(here), openBrowser(there)]);
  remote = await seed(theirs.api, there, ['claude_code']);
});
after(async () => {
  await mine?.close(); await theirs?.close();
  await here?.stop(); await there?.stop();
});

/** Settings → Machines on a gateway, loaded. */
async function machinesPage(browser, gateway) {
  const page = await browser.newPage();
  await page.goto(gateway.base + '/settings/machines');
  await page.locator('.machines-page').waitFor();
  await page.locator('.machine-card .machine-hint, .machine-row').first().waitFor();
  return page;
}
const row = (page, name) => page.locator('.machine-row').filter({ has: page.locator('.machine-name strong', { hasText: name }) });
/** Page errors, less the 404 a page address gets when the page is served from disk (see settings-mobile). */
const pageErrors = errors => errors.filter(error => !/^404 GET \/settings(\/\w+)?$/.test(error));

test('an invite made on one machine pairs it with another, each granting its own', async () => {
  const invitePage = await machinesPage(theirs, there);
  const invite = invitePage.locator('.machine-pair .machine-card').nth(0);
  await invite.locator('select').selectOption('control');
  await invite.getByRole('button', { name: '生成邀请链接' }).click();
  const link = (await invite.locator('.machine-invite code').innerText()).trim();
  assert.match(link, new RegExp(`^${there.base}/#tendly-pair=[0-9A-F]{12}$`));

  const page = await machinesPage(mine, here);
  const add = page.locator('.machine-pair .machine-card').nth(1);
  const pair = add.getByRole('button', { name: '配对' });
  await add.locator('input.machine-link').fill('https://example.com/not-an-invite');
  assert.equal(await pair.isDisabled(), true, 'a link without a code is not offered');
  await add.locator('input.machine-link').fill(link);
  await add.locator('select').selectOption('spawn');
  await pair.click();
  const peer = row(page, 'box-there');
  await peer.waitFor();
  await until(async () => (await peer.innerText()).includes('可访问'), 10000, 'the paired machine to answer');
  assert.match(await peer.innerText(), /你在那边可以\s*:\s*完全控制/);
  assert.equal(await peer.locator('.machine-grants select').inputValue(), 'spawn');

  // The other side shows this machine, with the opposite grants, and can reach it too.
  await invitePage.reload();
  const back = row(invitePage, 'mac-here');
  await back.waitFor();
  await until(async () => (await back.innerText()).includes('可访问'), 10000, 'this machine to answer there');
  assert.equal(await back.locator('.machine-grants select').inputValue(), 'control');
  assert.match(await back.innerText(), /你在那边可以\s*:\s*也可启动会话/);

  // The same link pairs nothing else.
  await add.locator('input.machine-link').fill(link);
  await pair.click();
  await add.locator('.inline-error').waitFor();
  assert.equal(await page.locator('.machine-row').count(), 1);
  assert.deepEqual(pageErrors(mine.errors).filter(error => !error.startsWith('409 POST /api/peers')), []);
  assert.deepEqual(pageErrors(theirs.errors), []);
});

test("the other machine's sessions are listed here and open there, signed in by a ticket", async () => {
  const page = await machinesPage(mine, here);
  const peer = row(page, 'box-there');
  await peer.getByRole('button', { name: '会话' }).click();
  const session = peer.locator('.machine-workspace li', { hasText: 'claude_code e2e' });
  await session.waitFor();
  assert.match(await peer.locator('.machine-workspace-name').innerText(), /E2E/);

  const [popup] = await Promise.all([
    mine.context.waitForEvent('page'),
    session.getByRole('button', { name: '打开' }).click(),
  ]);
  const id = remote.sessions[0].id;
  await popup.waitForURL(url => url.origin === there.base && url.pathname === `/session/${id}`, { timeout: 15000 });
  await until(async () => !new URL(popup.url()).hash, 10000, 'the ticket to leave the address bar');
  const auth = await popup.evaluate(() => fetch('/api/auth').then(response => response.json()));
  assert.equal(auth.authenticated, true, 'signed in there without its token');
  const workspaces = await popup.evaluate(() => fetch('/api/workspaces').then(response => response.status));
  assert.equal(workspaces, 200);
  await popup.close();

  // A ticket is good once: the same link again signs nothing in.
  const reused = await mine.context.request.post(there.base + '/api/auth', { headers: { 'X-Tendly-Client': 'web' }, data: { ticket: 'tdt_not-a-ticket' } });
  assert.equal(reused.status(), 401);
  // This machine's own page is still signed in to this machine.
  assert.equal((await mine.api('/auth')).authenticated, true);
});

test("an agent here lists the other machine's sessions by naming it", async () => {
  const machines = await mine.api('/agent/call', { name: 'tendly_machines' });
  assert.equal(machines.isError, false);
  const listed = JSON.parse(machines.content[0].text);
  assert.deepEqual(listed.machines.map(machine => [machine.name, machine.reachable, machine.you_may]), [['box-there', true, 'control']]);
  const sessions = await mine.api('/agent/call', { name: 'tendly_list', arguments: { what: 'sessions', machine: 'box-there' } });
  assert.equal(sessions.isError, false, sessions.content[0].text);
  const result = JSON.parse(sessions.content[0].text);
  assert.equal(result.machine, 'box-there');
  assert.deepEqual(result.result.map(session => session.title), ['claude_code e2e']);
  // Starting one there takes a session of this machine; a call from outside one is told so.
  const refused = await mine.api('/agent/call', { name: 'tendly_spawn', arguments: { provider: 'codex', prompt: 'hi', machine: 'box-there' } });
  assert.equal(refused.isError, true);
});

test('a grant lowered there is what this machine sees, and removing the pairing ends it on both', async () => {
  const theirPage = await machinesPage(theirs, there);
  const back = row(theirPage, 'mac-here');
  await back.locator('.machine-grants select').selectOption('view');
  await until(async () => (await back.locator('.machine-grants select').inputValue()) === 'view' && !(await back.locator('select:disabled').count()), 10000, 'the grant to save');

  const page = await machinesPage(mine, here);
  const peer = row(page, 'box-there');
  await until(async () => /你在那边可以\s*:\s*查看会话/.test(await peer.innerText()), 10000, 'the lowered grant');
  assert.equal(await peer.getByRole('button', { name: '打开它的窗口' }).count(), 0);
  const opened = await mine.context.request.post(here.base + `/api/peers/${await peerId()}/open`, { headers: { 'X-Tendly-Client': 'web' }, data: {} });
  assert.equal(opened.status(), 403, 'and its window no longer opens');

  await peer.getByRole('button', { name: '移除' }).click();
  await peer.getByRole('button', { name: '移除（两边同时）' }).click();
  await page.getByText('还没有。可以从本机发出邀请').waitFor();
  await theirPage.reload();
  await theirPage.getByText('还没有。可以从本机发出邀请').waitFor();
  assert.deepEqual(pageErrors(theirs.errors), []);
});

async function peerId() {
  const { peers } = await mine.api('/peers');
  return peers.find(peer => peer.name === 'box-there').id;
}
