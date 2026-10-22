// A conversation with each client: started from the composer, answered,
// and still there after a reload.
import { after, afterEach, before, test } from 'node:test';
import assert from 'node:assert/strict';
import { chatError, openBrowser, until, paneFor, row, seed, sendMessage, setLayout, startGateway } from './harness.mjs';

let gateway, browser;
before(async () => { gateway = await startGateway(); browser = await openBrowser(gateway, { width: 2400, height: 1000 }); });
afterEach(() => browser.reset());
after(async () => { await browser?.close(); await gateway?.stop(); });

test('Claude Code, Codex and Pi each answer a message and keep it across a reload', async () => {
  const { sessions } = await seed(browser.api, gateway, ['claude_code', 'codex', 'pi']);
  const ids = ['claude', 'codex', 'pi'];
  await setLayout(browser.api, row(...sessions.map((session, i) => paneFor(ids[i], session))));
  const page = await browser.newPage();
  await page.goto(gateway.base);
  for (const id of ids) await sendMessage(page, id, `hello from ${id}`);
  for (const id of ids) {
    const pane = page.locator(`[data-pane-id="${id}"]`);
    await pane.getByText('Hello 🦊').first().waitFor();
    assert.equal(await chatError(page, id), '', id);
  }
  await page.reload();
  for (const id of ids) {
    const pane = page.locator(`[data-pane-id="${id}"]`);
    await pane.getByText(`hello from ${id}`).first().waitFor();
    await pane.getByText('Hello 🦊').first().waitFor();
  }
  assert.deepEqual(browser.errors, []);
});

test('an empty session handed to another client opens without an error', async () => {
  // Reported: an empty Pi session continued in Claude Code showed
  // "无法读取对话响应" -- the boundary the handoff stored had no name.
  const { sessions: [pi] } = await seed(browser.api, gateway, ['pi']);
  await setLayout(browser.api, paneFor('pi', pi));
  const page = await browser.newPage();
  await page.goto(gateway.base);
  const pane = page.locator('[data-pane-id="pi"]');
  await pane.locator('.chat-endpoint > summary').click();
  await page.locator('.chat-handoff-choice').filter({ hasText: 'Claude Code' }).first().click();
  await pane.locator('.chat-handoff-confirm button').first().click();
  const claude = page.locator('.dock-pane.is-selected');
  await claude.getByText('Claude Code').first().waitFor();
  await page.waitForFunction(() => document.querySelector('.dock-pane.is-selected')?.dataset.paneId !== 'pi');
  const id = await claude.getAttribute('data-pane-id');
  const opened = page.locator(`[data-pane-id="${id}"]`).locator('.chat-boundary, .chat-alert');
  await opened.first().waitFor();
  assert.equal(await chatError(page, id), '');
  await sendMessage(page, id, 'continue here');
  await page.locator(`[data-pane-id="${id}"]`).getByText('Hello 🦊').first().waitFor();
  // A layout change is saved a moment after it is made; reload once it is.
  await until(async () => JSON.stringify(await browser.api('/canvas/layout')).includes(`"activePaneId":"${id}"`));
  await page.reload();
  await opened.first().waitFor();
  assert.equal(await chatError(page, id), '');
  assert.deepEqual(browser.errors, []);
});
