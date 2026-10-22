// The structured conversation pane, driven as a person uses it: tool calls,
// requests waiting on you, interrupting and steering a turn, the composer's
// chips, attachments, endpoint and client changes, error turns, history and
// two views of one session. What the page shows is checked against what the
// server stored, and against what the synthetic client was sent.
//
// Tests marked `todo` reproduce bugs found while writing this file; they
// fail today without failing the suite.
import { after, afterEach, before, test } from 'node:test';
import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { chatError, openBrowser, paneFor, sendMessage, setLayout, startGateway, until } from './harness.mjs';

let gateway, browser, workspace;
before(async () => {
  gateway = await startGateway();
  browser = await openBrowser(gateway, { width: 1400, height: 900 });
  workspace = await browser.api('/workspaces', { name: 'E2E', root_path: gateway.workspace });
  await browser.context.grantPermissions(['clipboard-read', 'clipboard-write'], { origin: gateway.base });
});
// A gateway runs at most 32 conversation clients at once: each test's are ended after it.
afterEach(async () => {
  await browser.reset();
  const listed = await browser.api('/sessions');
  for (const session of Array.isArray(listed) ? listed : listed.sessions ?? []) {
    if (!['stopped', 'failed'].includes(session.status)) await browser.api(`/sessions/${session.id}/stop`, {}).catch(() => {});
  }
});
after(async () => { await browser?.close(); await gateway?.stop(); });

const LABEL = { claude_code: 'Claude Code', codex: 'Codex', pi: 'Pi' };

/** A new conversation with `provider`, alone on the canvas, in a page of its own. */
async function open(provider, id = 'chat') {
  const session = await browser.api(`/workspaces/${workspace.id}/sessions`, { provider, title: `${provider} e2e`, interaction_mode: 'structured' });
  await setLayout(browser.api, paneFor(id, session));
  const page = await browser.newPage();
  await page.goto(gateway.base);
  const pane = page.locator(`[data-pane-id="${id}"]`);
  await pane.locator('textarea[data-pane-input]').waitFor();
  return { session, page, pane, input: pane.locator('textarea[data-pane-input]') };
}
/** What the server stored for `session`. */
async function stored(session) { return (await browser.api(`/sessions/${session.id}/conversation`)).events; }
/** Everything the synthetic clients were sent, in order. */
async function clientLog() {
  const text = await readFile(join(gateway.home, 'clients.log'), 'utf8').catch(() => '');
  return text.split('\n').filter(Boolean).map(line => JSON.parse(line));
}
/** What the client was sent from now on. */
async function logFrom() { const start = (await clientLog()).length; return async () => (await clientLog()).slice(start); }
const lastTurn = events => events.filter(event => event.type === 'turn').at(-1);
const toolStatus = (events, id) => events.filter(event => event.type === 'tool' && event.id === id).at(-1)?.status;
/** Waits until the reply `nth` (from 0) has arrived in `pane`. */
const reply = (pane, nth = 0, text = 'Hello 🦊') => pane.locator('article.chat-message.assistant').filter({ hasText: text }).nth(nth).waitFor();
const idle = pane => pane.locator('.chat-working').waitFor({ state: 'detached' });
/** Opens a chip in the composer and returns it. */
async function chip(pane, name) { const menu = pane.locator(`.chat-${name}[data-locked="false"]`); await menu.locator('summary').click(); return menu; }

test('tool calls show their outcome and output, and the reply its cost, across a reload', async () => {
  const { session, page, pane } = await open('claude_code');
  await sendMessage(page, 'chat', 'tools');
  await reply(pane);
  const read = pane.locator('[data-tool-id="read-1"]'), bash = pane.locator('[data-tool-id="bash-1"]');
  await read.locator('.tool-indicator.completed').waitFor();
  await bash.locator('.tool-indicator.failed').waitFor();
  assert.match(await read.locator('summary').innerText(), /Read · src\/main\.rs/);
  await read.locator('summary').click();
  assert.equal((await read.locator('pre').innerText()).trim(), 'fn main() {}');
  await bash.locator('summary').click();
  assert.equal((await bash.locator('pre').innerText()).trim(), 'ls: denied');
  assert.match(await pane.locator('.chat-turn-usage').innerText(), /42.*4/);
  assert.match(await pane.locator('.chat-usage').innerText(), /44\/200k/);
  const events = await stored(session);
  assert.equal(toolStatus(events, 'read-1'), 'completed');
  assert.equal(toolStatus(events, 'bash-1'), 'failed');
  assert.equal(lastTurn(events).status, 'completed');
  await page.reload();
  await reply(pane);
  await pane.locator('[data-tool-id="bash-1"] .tool-indicator.failed').waitFor();
  assert.match(await pane.locator('.chat-usage').innerText(), /44\/200k/);
  assert.deepEqual(browser.errors, []);
});

test('a permission request is accepted or declined from the dock above the composer', async () => {
  const { session, page, pane } = await open('claude_code');
  const dock = pane.locator('.chat-approval-dock');
  await sendMessage(page, 'chat', 'approve');
  await dock.waitFor();
  assert.match(await dock.innerText(), /Allow Bash to remove output\?/);
  assert.match(await dock.locator('.chat-approval-scope').innerText(), /\/tmp\/fixture-outside[\s\S]*rm -rf \/tmp\/fixture-outside/);
  assert.equal(await pane.locator('.chat-approval-record.pending').count(), 1);
  await dock.locator('button.allow').click();
  await dock.waitFor({ state: 'detached' });
  await reply(pane);
  await pane.locator('.chat-approval-record:not(.pending)').waitFor();
  assert.equal(toolStatus(await stored(session), 'tool-1'), 'completed');

  await sendMessage(page, 'chat', 'approve');
  await dock.waitFor();
  await dock.locator('button.deny').click();
  await dock.waitFor({ state: 'detached' });
  await reply(pane, 1);
  const events = await stored(session);
  assert.equal(toolStatus(events, 'tool-2'), 'failed');
  assert.equal(events.filter(event => event.type === 'approval_resolved').length, 2);
  assert.deepEqual(browser.errors, []);
});

test('the keyboard answers a request: Esc leaves it waiting, mod+Enter accepts, mod+Backspace declines', async () => {
  const { session, page, pane, input } = await open('claude_code');
  const dock = pane.locator('.chat-approval-dock');
  await sendMessage(page, 'chat', 'approve');
  await dock.waitFor();
  await input.focus();
  await input.press('Escape');
  await page.waitForTimeout(300);
  assert.equal(await dock.count(), 1, 'Esc does not interrupt a turn waiting on you');
  await input.focus();
  await page.keyboard.press('ControlOrMeta+Enter');
  await dock.waitFor({ state: 'detached' });
  await reply(pane);
  await sendMessage(page, 'chat', 'approve');
  await dock.waitFor();
  await input.focus();
  await page.keyboard.press('ControlOrMeta+Backspace');
  await dock.waitFor({ state: 'detached' });
  await reply(pane, 1);
  const events = await stored(session);
  assert.equal(toolStatus(events, 'tool-1'), 'completed');
  assert.equal(toolStatus(events, 'tool-2'), 'failed');
  assert.deepEqual(browser.errors, []);
});

test('a question with options, an answer of your own and several choices reaches the client as answered', async () => {
  const { page, pane } = await open('claude_code');
  const sent = await logFrom();
  await sendMessage(page, 'chat', 'ask');
  const dock = pane.locator('.chat-approval-dock'), submit = dock.locator('button.allow');
  await dock.locator('.chat-question').nth(1).waitFor();
  assert.ok(await submit.isDisabled(), 'nothing answered yet');
  const [format, parts] = [dock.locator('.chat-question').nth(0), dock.locator('.chat-question').nth(1)];
  await format.locator('label.chat-answer').filter({ hasText: 'Brief' }).click();
  // Writing an answer of your own replaces the option picked.
  await format.locator('input[type=text]').fill('Medium');
  assert.equal(await format.locator('input[value="Brief"]').isChecked(), false);
  assert.ok(await submit.isDisabled(), 'the second question is still open');
  await parts.locator('label.chat-answer').filter({ hasText: 'UI' }).click();
  await parts.locator('input[type=text]').fill('Docs');
  await submit.click();
  await dock.waitFor({ state: 'detached' });
  await reply(pane);
  const answer = (await sent()).find(message => message.type === 'control_response');
  assert.deepEqual(answer.response.response.updatedInput.answers, { 'Which format?': 'Medium', 'Which parts?': 'UI, Docs' });
  assert.deepEqual(browser.errors, []);
});

test('a subagent card holds the calls it made, nested, with what it spent', async () => {
  const { session, page, pane } = await open('claude_code');
  await sendMessage(page, 'chat', 'subagent');
  await reply(pane);
  const task = pane.locator('.chat-tool[data-tool-id="task-1"]');
  await until(async () => toolStatus(await stored(session), 'task-1') !== 'running', 5000, 'the task to end');
  await task.locator('.chat-tool-stats').waitFor();
  assert.match(await task.locator('summary').first().innerText(), /Task · Survey the code/);
  await task.locator('summary').first().click();
  const grep = task.locator('.chat-tool-children [data-tool-id="grep-1"]');
  await grep.locator('.tool-indicator.completed').waitFor();
  assert.match(await grep.locator('summary').innerText(), /Grep · TODO · src\/lib\.rs/);
  await grep.locator('summary').click();
  assert.match(await grep.innerText(), /lib\.rs:3: TODO/);
  assert.deepEqual(browser.errors, []);
});

test('Esc in the composer and the stop button each interrupt a running turn', async () => {
  const { session, page, pane, input } = await open('claude_code');
  await sendMessage(page, 'chat', 'hold');
  await pane.locator('.chat-working').waitFor();
  await input.press('Escape');
  await idle(pane);
  assert.equal(lastTurn(await stored(session)).status, 'interrupted');
  await sendMessage(page, 'chat', 'hold');
  await pane.locator('button.chat-stop').click();
  await idle(pane);
  assert.equal(lastTurn(await stored(session)).status, 'interrupted');
  await sendMessage(page, 'chat', 'after two interrupts');
  await reply(pane);
  assert.equal(await chatError(page, 'chat'), '');
  assert.deepEqual(browser.errors, []);
});

test('Enter while the agent works steers the running turn', async () => {
  const { session, page, pane, input } = await open('claude_code');
  await sendMessage(page, 'chat', 'hold');
  await pane.locator('.chat-working').waitFor();
  assert.match(await input.getAttribute('placeholder'), /Enter/);
  await input.fill('join');
  await input.press('Enter');
  await reply(pane);
  await idle(pane);
  const steered = pane.locator('article.chat-message.user').filter({ hasText: 'join' });
  await steered.locator('.chat-steered').waitFor();
  assert.equal(await input.inputValue(), '');
  const events = await stored(session);
  assert.ok(events.some(event => event.type === 'message' && event.text === 'join' && event.steered === true));
  assert.equal(events.filter(event => event.type === 'turn' && event.status === 'running').length, 1, 'read inside the one turn');
  assert.deepEqual(browser.errors, []);
});

test('queued messages can be removed, edited back and sent now; "interrupt and send" sends at once', async () => {
  const { session, page, pane, input } = await open('claude_code');
  await sendMessage(page, 'chat', 'hold');
  await pane.locator('.chat-working').waitFor();
  for (const text of ['q1', 'q2', 'q3']) { await input.fill(text); await input.press('Alt+Enter'); }
  const queued = pane.locator('.chat-queued');
  assert.deepEqual(await queued.locator('p').allInnerTexts(), ['q1', 'q2', 'q3']);
  await queued.nth(2).locator('button.chat-queued-remove').click();
  await queued.nth(1).locator('button').nth(2).click(); // Edit
  assert.equal(await input.inputValue(), 'q2');
  assert.deepEqual(await queued.locator('p').allInnerTexts(), ['q1']);
  await input.fill('');
  await queued.nth(0).locator('button').nth(1).click(); // Send now
  await reply(pane);
  await pane.locator('.chat-queue').waitFor({ state: 'detached' });
  let events = await stored(session);
  assert.equal(events.filter(event => event.type === 'turn')[1].status, 'interrupted');
  assert.ok(events.some(event => event.type === 'message' && event.role === 'user' && event.text === 'q1' && !event.steered));

  await sendMessage(page, 'chat', 'hold');
  await pane.locator('.chat-working').waitFor();
  await input.fill('tools');
  await pane.locator('.chat-steer-more > summary').click();
  await pane.locator('.chat-steer-more nav button.danger').click();
  await pane.locator('[data-tool-id="read-4"]').waitFor();
  await reply(pane, 1);
  events = await stored(session);
  assert.deepEqual(events.filter(event => event.type === 'turn' && event.status !== 'running').map(event => event.status), ['interrupted', 'completed', 'interrupted', 'completed']);
  assert.deepEqual(browser.errors, []);
});

test('the model, thinking depth and permission chips change the running client in place', async () => {
  const { session, page, pane } = await open('claude_code');
  await sendMessage(page, 'chat', 'hi');
  await reply(pane);
  const sent = await logFrom();
  const model = await chip(pane, 'model');
  assert.deepEqual(await model.locator('nav button strong').allInnerTexts(), ['Default (Sonnet)', 'Sonnet', 'Opus']);
  assert.equal(await pane.locator('.chat-effort').count(), 0, 'the default model offers no depth');
  await model.locator('nav button').filter({ hasText: 'Opus' }).click();
  await until(async () => (await pane.locator('.chat-model .chip-menu-label').innerText()) === 'Opus', 5000, 'the chip to say Opus');
  await (await chip(pane, 'model')).locator('nav button').filter({ hasText: /^Sonnet/ }).click();
  const effort = pane.locator('.chat-effort');
  await effort.waitFor();
  await (await chip(pane, 'effort')).locator('input[type=range]').fill('3');
  await until(async () => (await sent()).some(message => message.request?.subtype === 'set_model' && message.request.effort === 'high'), 5000, 'the depth to be sent');
  await page.keyboard.press('Escape');
  const permission = await chip(pane, 'permission');
  assert.equal(await permission.locator('nav button').count(), 4);
  await permission.locator('nav button').nth(2).click(); // accept edits
  await until(async () => (await stored(session)).some(event => event.type === 'settings' && event.permission_mode === 'accept_edits'), 5000, 'the mode to be reported');
  await pane.locator('.chat-permission .chip-menu-label').filter({ hasText: /\S/ }).waitFor();
  const requests = (await sent()).filter(message => message.type === 'control_request').map(message => message.request);
  assert.deepEqual(requests.map(request => [request.subtype, request.model ?? request.mode, request.effort]), [
    ['set_model', 'opus', undefined], ['set_model', 'sonnet', undefined], ['set_model', 'sonnet', 'high'], ['set_permission_mode', 'acceptEdits', undefined],
  ]);
  await sendMessage(page, 'chat', 'still answering');
  await reply(pane, 1);
  assert.deepEqual(browser.errors, []);
});

test('typing / offers the commands the client announced, and Tab completes one', async () => {
  const { page, pane, input } = await open('claude_code');
  await sendMessage(page, 'chat', 'hi');
  await reply(pane);
  await input.click();
  await input.pressSequentially('/');
  const menu = pane.locator('.chat-commands[role=listbox]');
  assert.deepEqual(await menu.locator('[role=option]').allInnerTexts(), ['/review', '/compact']);
  await input.pressSequentially('co');
  assert.deepEqual(await menu.locator('[role=option]').allInnerTexts(), ['/compact']);
  await input.press('Tab');
  assert.equal(await input.inputValue(), '/compact ');
  await menu.waitFor({ state: 'detached' });
  await input.fill('/');
  await input.press('End');
  await menu.waitFor();
  await input.press('Escape');
  await menu.waitFor({ state: 'detached' });
  assert.deepEqual(browser.errors, []);
});

test('a failed turn says why, and the next message is answered', async () => {
  const { session, page, pane } = await open('claude_code');
  const alerts = pane.locator('.chat-timeline .chat-alert');
  for (const [scenario, said] of [['fail', /could not complete the turn/], ['fail-auth', /does not support this authentication mode/], ['fail-401', /HTTP 401[\s\S]*401 bad key/]]) {
    const before = await alerts.count();
    await sendMessage(page, 'chat', scenario);
    await alerts.nth(before).waitFor();
    assert.match(await alerts.nth(before).innerText(), said, scenario);
    await idle(pane);
    assert.equal(lastTurn(await stored(session)).status, 'failed', scenario);
  }
  await sendMessage(page, 'chat', 'and now?');
  await pane.locator('article.chat-message.user').filter({ hasText: 'and now?' }).waitFor();
  await until(async () => lastTurn(await stored(session)).status === 'completed', 5000, 'the turn to complete');
  assert.deepEqual(browser.errors, []);
});

// A 1×1 PNG.
const PNG = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==', 'base64');

test('files attached with + and pasted into the composer go with the message, images shown', async () => {
  const { session, page, pane, input } = await open('claude_code');
  await input.click();
  const [chooser] = await Promise.all([page.waitForEvent('filechooser'), pane.locator('button.chat-attach').click()]);
  await chooser.setFiles([{ name: 'notes.txt', mimeType: 'text/plain', buffer: Buffer.from('hello notes') }, { name: 'shot.png', mimeType: 'image/png', buffer: PNG }]);
  const chips = pane.locator('.chat-attachments li');
  await chips.nth(1).waitFor();
  await input.focus();
  await input.evaluate((element, base64) => {
    const data = new DataTransfer();
    data.items.add(new File([Uint8Array.from(atob(base64), c => c.charCodeAt(0))], 'image.png', { type: 'image/png' }));
    element.dispatchEvent(new ClipboardEvent('paste', { clipboardData: data, bubbles: true, cancelable: true }));
  }, PNG.toString('base64'));
  await chips.nth(2).waitFor();
  assert.match(await chips.nth(2).innerText(), /^pasted-.*\.png/);
  await chips.first().locator('button').click(); // drop notes.txt
  await until(async () => await chips.count() === 2, 3000, 'the chip to go');
  await input.fill('look at these');
  await pane.locator('button.chat-send:not([disabled])').click();
  await reply(pane);
  const sent = (await stored(session)).find(event => event.type === 'message' && event.role === 'user').text;
  assert.match(sent, /^Attached files in this workspace:\n- \.tendly-files\/.*\/shot-[^\n]*\.png\n- \.tendly-files\/.*\/pasted-[^\n]*\.png\n\nlook at these$/);
  const images = pane.locator('.chat-attached-images img');
  await until(async () => await images.count() === 2 && (await images.evaluateAll(all => all.every(image => image.naturalWidth === 1))), 5000, 'the images to load');
  assert.equal(await chips.count(), 0);
  assert.deepEqual(browser.errors, []);
});

test('changing the endpoint profile starts a new context that carries the conversation', async () => {
  const profile = await browser.api('/endpoint-profiles', { name: 'Fixture endpoint', provider: 'claude_code', endpoint_url: 'http://127.0.0.1:9/v1', model: 'sonnet' });
  const { session, page, pane } = await open('claude_code');
  await sendMessage(page, 'chat', 'first words');
  await reply(pane);
  const menu = await chip(pane, 'endpoint');
  await menu.locator('nav button').filter({ hasText: 'Fixture endpoint' }).click();
  const confirm = pane.locator('.chat-endpoint-confirm');
  assert.match(await confirm.innerText(), /Fixture endpoint/);
  await confirm.locator('button').first().click();
  await pane.locator('.chat-boundary').filter({ hasText: 'Fixture endpoint' }).waitFor();
  await until(async () => (await pane.locator('.chat-endpoint .chip-menu-label').innerText()) === 'Fixture endpoint', 5000, 'the chip to name the profile');
  const sent = await logFrom();
  await sendMessage(page, 'chat', 'second words');
  await reply(pane, 1);
  const log = await sent();
  assert.deepEqual(log.find(message => message.event === 'args').args.slice(0, 2), ['--model', 'sonnet']);
  const prompt = log.find(message => message.type === 'user').message.content;
  assert.match(prompt, /first words[\s\S]*Hello 🦊[\s\S]*second words$/);
  assert.equal((await browser.api(`/sessions/${session.id}`)).endpoint_profile_id, profile.id);
  // The page asks the unreachable endpoint for its models; it is refused.
  assert.deepEqual(browser.errors.filter(error => !error.includes(`/endpoint-profiles/${profile.id}/models`)), []);
});

test('a conversation moves to every other client and the new one is told what was said', async () => {
  for (const from of ['claude_code', 'codex', 'pi']) {
    for (const to of ['claude_code', 'codex', 'pi'].filter(other => other !== from)) {
      const { page, pane } = await open(from);
      await sendMessage(page, 'chat', `marker ${from} to ${to}`);
      await reply(pane);
      await (await chip(pane, 'endpoint')).locator('.chat-handoff-choice').filter({ hasText: LABEL[to] }).last().click();
      await pane.locator('.chat-handoff-confirm button').first().click();
      await page.waitForFunction(() => document.querySelector('.dock-pane.is-selected')?.dataset.paneId !== 'chat');
      const id = await page.locator('.dock-pane.is-selected').getAttribute('data-pane-id');
      const target = page.locator(`[data-pane-id="${id}"]`);
      await target.locator('.chat-boundary').waitFor();
      assert.equal(await chatError(page, id), '', `${from} → ${to}`);
      await target.locator('article.chat-message.user').filter({ hasText: `marker ${from} to ${to}` }).waitFor();
      const sent = await logFrom();
      await sendMessage(page, id, 'continue');
      await reply(target, 1);
      const prompt = JSON.stringify((await sent()).filter(message => JSON.stringify(message).includes('continue')));
      assert.match(prompt, new RegExp(`marker ${from} to ${to}`), `${from} → ${to}: the earlier conversation goes with the message`);
      await browser.reset();
    }
  }
});

test('Codex: a command request, a question, and depth and approvals taken at the next turn', async () => {
  const { session, page, pane } = await open('codex');
  const dock = pane.locator('.chat-approval-dock');
  await sendMessage(page, 'chat', 'approve');
  await dock.waitFor();
  assert.match(await dock.locator('.chat-approval-scope').innerText(), /printf fixture/);
  await dock.locator('button.allow').click();
  await reply(pane);
  assert.equal(toolStatus(await stored(session), 'tool-1'), 'completed');
  await sendMessage(page, 'chat', 'ask');
  await dock.waitFor();
  await dock.locator('label.chat-answer').filter({ hasText: 'Brief' }).click();
  await dock.locator('button.allow').click();
  await reply(pane, 1);
  // The fixture asserts the answer it was given; a wrong one would fail the turn.
  assert.equal(lastTurn(await stored(session)).status, 'completed');

  const models = await chip(pane, 'model');
  assert.deepEqual(await models.locator('nav button strong').allInnerTexts(), ['Fixture', 'gpt-plain'], 'a hidden model is not offered');
  await page.keyboard.press('Escape');
  await (await chip(pane, 'effort')).locator('input[type=range]').fill('2');
  await until(async () => (await pane.locator('.chat-effort .chip-menu-label').innerText()) !== '', 3000, 'the depth chip');
  await page.keyboard.press('Escape');
  await (await chip(pane, 'permission')).locator('nav button').last().click();
  await until(async () => (await stored(session)).some(event => event.type === 'settings' && event.permission_mode === 'danger'), 5000, 'the mode to be reported');
  const sent = await logFrom();
  await sendMessage(page, 'chat', 'next turn');
  await reply(pane, 2);
  const start = (await sent()).find(message => message.method === 'turn/start');
  assert.equal(start.params.effort, 'high');
  assert.equal(start.params.approvalPolicy, 'never');
  assert.deepEqual(browser.errors, []);
});

test('Pi: an extension confirmation, and a model switch that brings thinking depth', async () => {
  const { session, page, pane } = await open('pi');
  const dock = pane.locator('.chat-approval-dock');
  await sendMessage(page, 'chat', 'confirm');
  await dock.waitFor();
  assert.match(await dock.innerText(), /Delete build output\?[\s\S]*rm -rf dist/);
  await dock.locator('button.allow').click();
  await reply(pane, 0, 'Deleted');
  await pane.locator('[data-tool-id="call-1"] .tool-indicator.completed').waitFor();
  assert.equal(await pane.locator('.chat-effort').count(), 0, 'the local model has no depth');
  const sent = await logFrom();
  await (await chip(pane, 'model')).locator('nav button').filter({ hasText: 'GLM' }).click();
  await pane.locator('.chat-effort').waitFor();
  assert.equal(await pane.locator('.chat-model .chip-menu-label').innerText(), 'GLM');
  await (await chip(pane, 'effort')).locator('input[type=range]').fill('3');
  await until(async () => (await sent()).some(message => message.type === 'set_thinking_level' && message.level === 'high'), 5000, 'the level to be sent');
  await page.keyboard.press('Escape');
  assert.deepEqual((await sent()).filter(message => message.type === 'set_model').map(message => [message.provider, message.modelId]), [['kunlun', 'zai-org/GLM-5.3']]);
  await sendMessage(page, 'chat', 'hi');
  await reply(pane, 0, 'Hello 🦊');
  assert.equal(lastTurn(await stored(session)).status, 'completed');
  assert.deepEqual(browser.errors, []);
});

test('replies render as Markdown, and a message and a code block copy as written', async () => {
  const { page, pane } = await open('claude_code');
  await sendMessage(page, 'chat', 'seed');
  await reply(pane, 0, 'That is all for turn 1');
  const message = pane.locator('article.chat-message.assistant').last();
  assert.equal(await message.locator('h2').innerText(), 'Turn 1');
  assert.equal(await message.locator('li').count(), 3);
  assert.equal(await message.locator('strong').innerText(), 'the cache');
  assert.equal(await message.locator('tr').count(), 3);
  assert.equal(await message.locator('.chat-code small').innerText(), 'rust');
  await message.hover();
  await message.locator('.chat-message-actions button').click();
  assert.match(await page.evaluate(() => navigator.clipboard.readText()), /^## Turn 1\n\nHere is what changed:/);
  await message.locator('.chat-code').hover();
  await message.locator('.chat-code-copy button').click();
  assert.match(await page.evaluate(() => navigator.clipboard.readText()), /^fn main\(\) \{\n {2}let value_0 = compute\(0\);/);
  assert.deepEqual(browser.errors, []);
});

test('older history is read in pages as the view nears the top', async () => {
  const { session, page, pane } = await open('claude_code');
  await sendMessage(page, 'chat', 'seed');
  await reply(pane, 0, 'That is all for turn 1');
  await page.close();
  // Enough turns that the latest page (1000 events) no longer holds the first.
  for (let turn = 2; turn <= 80; turn++) {
    const id = randomUUID();
    await browser.api(`/sessions/${session.id}/conversation/message`, { id, content: 'seed', configuration_revision: 0 });
    await until(async () => (await stored(session)).some(event => event.type === 'turn' && event.id === id && event.status === 'completed'), 10000, `turn ${turn}`);
  }
  const latest = await browser.api(`/sessions/${session.id}/conversation`);
  assert.ok(latest.older > 0, 'the first turns are on an older page');
  const view = await browser.newPage();
  await view.goto(gateway.base);
  const shown = view.locator('[data-pane-id="chat"]');
  await shown.locator('h2').filter({ hasText: /^Turn 80$/ }).waitFor();
  assert.equal(await shown.locator('h2').filter({ hasText: /^Turn 1$/ }).count(), 0);
  await shown.locator('.chat-history-older').waitFor({ state: 'attached' });
  await until(async () => {
    await shown.locator('.chat-timeline').evaluate(element => { element.scrollTop = 0; });
    return await shown.locator('h2').filter({ hasText: /^Turn 1$/ }).count() > 0;
  }, 15000, 'the first turn to be read back');
  await shown.locator('.chat-history-older').waitFor({ state: 'detached' });
  assert.deepEqual(browser.errors, []);
});

test('a reload during a running turn or a waiting request picks up where it was', async () => {
  const { session, page, pane, input } = await open('claude_code');
  await sendMessage(page, 'chat', 'hold');
  await pane.locator('.chat-working').waitFor();
  await page.reload();
  await pane.locator('.chat-working').waitFor();
  await pane.locator('button.chat-stop').waitFor();
  await input.click();
  await input.fill('join');
  await input.press('Enter');
  await reply(pane);
  await idle(pane);

  await sendMessage(page, 'chat', 'approve');
  await pane.locator('.chat-approval-dock').waitFor();
  await page.reload();
  await pane.locator('.chat-approval-dock button.allow').click();
  await reply(pane, 1);
  assert.equal(toolStatus(await stored(session), 'tool-2'), 'completed');
  assert.deepEqual(browser.errors, []);
});

test('two pages on one session see each other\'s messages, answers and interrupts', async () => {
  const { session, page: a, pane: paneA } = await open('claude_code');
  const b = await browser.newPage();
  await b.goto(gateway.base);
  const paneB = b.locator('[data-pane-id="chat"]');
  await paneB.locator('textarea[data-pane-input]').waitFor();
  await sendMessage(a, 'chat', 'from page A');
  await reply(paneA);
  await paneB.locator('article.chat-message.user').filter({ hasText: 'from page A' }).waitFor();
  await reply(paneB);

  await sendMessage(b, 'chat', 'approve');
  await paneA.locator('.chat-approval-dock').waitFor();
  await paneB.locator('.chat-approval-dock button.allow').click();
  await paneA.locator('.chat-approval-dock').waitFor({ state: 'detached' });
  await reply(paneA, 1);

  await sendMessage(a, 'chat', 'hold');
  await paneB.locator('button.chat-stop').click();
  await idle(paneA);
  assert.equal(lastTurn(await stored(session)).status, 'interrupted');
  // Two pages each save the one canvas; the later save is refused, as designed.
  assert.deepEqual(browser.errors.filter(error => error !== '409 PUT /api/canvas/layout'), []);
});

// ---- Bugs found while writing this file. Each fails today. ----

// Regression (found by the exploratory run): wheel-scrolling up during a streamed reply snaps back to the bottom (virtualizer scrollToIndex reconcile)
test('scrolling up with the wheel while a reply streams stops following it', async () => {
  const { page, pane } = await open('claude_code');
  await sendMessage(page, 'chat', 'seed');
  await reply(pane, 0, 'That is all for turn 1');
  await sendMessage(page, 'chat', 'stream:1500');
  await page.waitForTimeout(1000);
  const timeline = pane.locator('.chat-timeline'), box = await timeline.boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + 100);
  // One turn of the wheel at a time, back to the bottom in between: each
  // should leave the view where it was scrolled to. Most are undone.
  const kept = [];
  for (let i = 0; i < 4; i++) {
    await page.mouse.wheel(0, -300);
    await page.waitForTimeout(400);
    const gap = await timeline.evaluate(element => element.scrollHeight - element.scrollTop - element.clientHeight);
    kept.push(gap > 100);
    if (await pane.locator('.chat-latest').count()) { await pane.locator('.chat-latest').click(); await page.waitForTimeout(300); }
  }
  assert.deepEqual(kept, [true, true, true, true], 'every scroll up is kept');
});

// Regression (found by the exploratory run): after a handoff the earlier replies are labelled with the new client
test('a conversation carried to another client keeps saying who answered it', async () => {
  const { page, pane } = await open('claude_code');
  await sendMessage(page, 'chat', 'who said this');
  await reply(pane);
  await (await chip(pane, 'endpoint')).locator('.chat-handoff-choice').filter({ hasText: 'Codex' }).last().click();
  await pane.locator('.chat-handoff-confirm button').first().click();
  await page.waitForFunction(() => document.querySelector('.dock-pane.is-selected')?.dataset.paneId !== 'chat');
  const id = await page.locator('.dock-pane.is-selected').getAttribute('data-pane-id');
  const carried = page.locator(`[data-pane-id="${id}"] article.chat-message.assistant`).first();
  await carried.waitFor();
  assert.equal((await carried.locator('.chat-message-label').innerText()).trim(), 'Claude Code');
});

// Regression (found by the exploratory run): the boundary names "Isolated configuration" in English in a Chinese page
test('the boundary of a change back to the isolated configuration is in the page\'s language', async () => {
  const profile = await browser.api('/endpoint-profiles', { name: 'Round trip', provider: 'claude_code', endpoint_url: 'http://127.0.0.1:9/v1' });
  const { page, pane } = await open('claude_code');
  await sendMessage(page, 'chat', 'hi');
  await reply(pane);
  for (const name of ['Round trip', '原生 · 独立配置']) {
    await (await chip(pane, 'endpoint')).locator('nav button').filter({ hasText: name }).first().click();
    await pane.locator('.chat-endpoint-confirm button').first().click();
    await until(async () => (await pane.locator('.chat-endpoint .chip-menu-label').innerText()) === name, 5000, `the chip to say ${name}`);
  }
  const boundary = pane.locator('.chat-boundary').last();
  await boundary.waitFor();
  assert.doesNotMatch(await boundary.innerText(), /Isolated configuration/);
  assert.ok(profile.id);
});

// Regression (found by the exploratory run): a steer the turn ended without reading runs as its own turn but stays labelled "Steer" inside the earlier one
test('a message the running turn never read is not shown as read inside it', async () => {
  const { session, page, pane, input } = await open('claude_code');
  await sendMessage(page, 'chat', 'hold');
  await pane.locator('.chat-working').waitFor();
  // `late` is read only after the held turn ends: it runs as a turn of its own.
  await input.fill('late');
  await input.press('Enter');
  await reply(pane, 1);
  const turns = (await stored(session)).filter(event => event.type === 'turn' && event.status === 'running');
  assert.equal(turns.length, 2, 'it ran as a turn of its own');
  const late = pane.locator('article.chat-message.user').filter({ hasText: 'late' });
  assert.equal(await late.locator('.chat-steered').count(), 0);
});

// Regression (found by the exploratory run): a rejected model ID always offers "start a new context on this model", even when the client refused its format
test('a Pi model ID the client cannot parse is not offered as a new context', async () => {
  const { page, pane } = await open('pi');
  await sendMessage(page, 'chat', 'hi');
  await reply(pane);
  const model = await chip(pane, 'model');
  await model.locator('.chat-model-custom input').fill('not-a-pi-id');
  await model.locator('.chat-model-custom button').click();
  await pane.locator('.chat-alert').first().waitFor();
  await page.waitForTimeout(3000);
  assert.equal(await pane.locator('.chat-endpoint-confirm').count(), 0);
});
