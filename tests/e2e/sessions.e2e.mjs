// Workspaces, the sidebar and sessions: made from the page's own menus,
// renamed, archived, restored, deleted, found again, and kept across a reload
// and across two pages. The page runs in zh-CN, so labels are matched in Chinese.
import { after, afterEach, before, test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdir, realpath, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { openBrowser, paneFor, seed, sendMessage, setLayout, startGateway, until } from './harness.mjs';

let gateway, browser;
before(async () => { gateway = await startGateway(); browser = await openBrowser(gateway, { width: 2400, height: 1000 }); });
afterEach(() => browser.reset());
after(async () => { await browser?.close(); await gateway?.stop(); });

/** The fixture workspace (registering the same directory again returns it). */
const workspace = () => browser.api('/workspaces', { name: 'E2E', root_path: gateway.workspace });
const listed = () => browser.api('/sessions');
const onServer = async id => (await listed()).find(session => session.id === id);
/** Each test starts from a canvas holding one empty pane, whatever the last one left. */
const blankCanvas = () => setLayout(browser.api, { type: 'pane', id: 'blank', kind: 'agent_chat', title: 'Agent session' });
async function openApp() {
  const page = await browser.newPage();
  await page.goto(gateway.base);
  const group = page.locator('section.workspace-group[aria-label="E2E"]');
  await group.waitFor();
  // The branch arrives a moment later and pushes the rows down; act once they have settled.
  await group.getByText('main').first().waitFor({ timeout: 5000 }).catch(() => {});
  return page;
}
const sidebarRow = (page, id) => page.locator(`.workspace-groups [data-sidebar-session-id="${id}"]`);
/** A row's buttons, shown while it is hovered or holds the focus; focused here, so a shifting list cannot hide them. */
async function rowButton(page, id, button) {
  const row = sidebarRow(page, id);
  await row.locator('.workspace-session').focus();
  await row.locator(button).click();
  return row;
}
async function rowMenu(page, id) {
  const row = await rowButton(page, id, '.session-more-button');
  await row.locator('.sidebar-session-menu').waitFor();
  return row;
}
/** The workspace's ··· menu, then "New" → `label` in its flyout. */
async function newFromWorkspaceMenu(page, label) {
  const group = page.locator('section.workspace-group[aria-label="E2E"]');
  await group.locator('.workspace-more-button').click();
  await group.locator('.workspace-more-panel .menu-flyout-trigger').hover();
  await page.locator('nav.menu-flyout button', { hasText: label }).click();
}
/** Waits for the one session the last gesture made, and returns it. */
async function createdSince(before) {
  const known = new Set(before.map(session => session.id));
  let made;
  await until(async () => { made = (await listed()).find(session => !known.has(session.id)); return !!made; }, 10000, 'a new session');
  return made;
}
async function prefix(page, key) {
  await page.keyboard.press('Control+b');
  await page.locator('.prefix-hint').waitFor();
  await page.keyboard.press(key);
}

test('each kind of session is made from the workspace menu and opens in a pane', async () => {
  await workspace(); await blankCanvas();
  const page = await openApp();
  const made = [];
  for (const [label, provider] of [['新建 Claude Code 会话', 'claude_code'], ['新建 Codex 会话', 'codex'], ['新建 Pi 会话', 'pi'], ['新建终端', 'terminal']]) {
    const before = await listed();
    await newFromWorkspaceMenu(page, label);
    const session = await createdSince(before);
    assert.equal(session.provider, provider);
    assert.equal(session.ephemeral, false);
    await sidebarRow(page, session.id).waitFor();
    await page.locator(`[data-pane-tab-id="session-${session.id}"]`).waitFor();
    made.push(session);
  }
  // Each one started as it was made: the gesture that creates a session is the one that opens it.
  await until(async () => (await listed()).filter(session => made.some(item => item.id === session.id)).every(session => session.status === 'running'), 15000, 'every new session running');
  assert.deepEqual(browser.errors, []);
});

test('the draft tab creates a configured session in its place', async () => {
  await workspace(); await blankCanvas();
  const page = await openApp();
  await page.locator('.new-session-more > summary').click();
  await page.locator('.new-session-menu button', { hasText: '自定义新建会话' }).click();
  const draft = page.locator('.session-draft');
  await draft.waitFor();
  const title = draft.locator('input[maxlength="120"]'), create = draft.locator('.dialog-actions .primary-button');
  await title.fill('');
  assert.ok(await create.isDisabled(), 'an empty name cannot be created');
  await draft.locator('select').first().selectOption('codex');
  await title.fill('起草的会话');
  const before = await listed();
  await create.click();
  const session = await createdSince(before);
  assert.equal(session.title, '起草的会话'); assert.equal(session.provider, 'codex');
  await page.locator(`[data-pane-tab-id="session-${session.id}"]`).waitFor();
  assert.equal(await page.locator('.session-draft').count(), 0, 'the draft became the session');
  assert.equal((await sidebarRow(page, session.id).locator('.workspace-session-title').innerText()).trim(), '起草的会话');
  assert.deepEqual(browser.errors, []);
});

test('the first message names a session, a rename in the sidebar outlasts the next one, and both survive a reload', async () => {
  await workspace(); await blankCanvas();
  const page = await openApp();
  const before = await listed();
  await newFromWorkspaceMenu(page, '新建 Claude Code 会话');
  const session = await createdSince(before);
  const paneId = `session-${session.id}`;
  await sendMessage(page, paneId, '帮我看看这个仓库的结构');
  await page.locator(`[data-pane-id="${paneId}"]`).getByText('Hello 🦊').first().waitFor({ timeout: 15000 });
  await until(async () => (await onServer(session.id)).title === '帮我看看这个仓库的结构', 10000, 'a title from the first message');
  const row = sidebarRow(page, session.id), tab = page.locator(`[data-pane-tab-id="${paneId}"]`);
  await until(async () => (await row.innerText()).includes('帮我看看这个仓库的结构') && (await tab.innerText()).includes('帮我看看这个仓库的结构'), 10000, 'the title in the sidebar and the tab');

  await (await rowMenu(page, session.id)).locator('.session-rename-action').click();
  const input = row.locator('.sidebar-session-rename input');
  await input.fill('   ');
  await input.press('Enter');
  assert.equal(await input.count(), 1, 'a blank name is not taken');
  await input.fill('我的会话 🦊');
  await input.press('Enter');
  await until(async () => (await onServer(session.id)).title === '我的会话 🦊', 5000, 'renamed');
  await until(async () => (await tab.innerText()).includes('我的会话 🦊'), 5000, 'the tab renamed');

  await sendMessage(page, paneId, 'second message');
  await page.locator(`[data-pane-id="${paneId}"]`).getByText('second message').first().waitFor();
  await page.waitForTimeout(1500);
  assert.equal((await onServer(session.id)).title, '我的会话 🦊', 'a name given by hand is kept');

  await page.reload();
  await row.waitFor();
  assert.equal((await row.locator('.workspace-session-title').innerText()).trim(), '我的会话 🦊');
  await until(async () => (await tab.innerText()).includes('我的会话 🦊'), 5000, 'the tab after a reload');
  assert.deepEqual(browser.errors, []);
});

test('F2 on a pane renames its session through the dialog', async () => {
  const { sessions: [session] } = await seed(browser.api, gateway, ['codex']);
  await setLayout(browser.api, paneFor('one', session));
  const page = await openApp();
  await page.locator('[data-pane-id="one"]').click({ position: { x: 300, y: 300 } });
  await page.keyboard.press('F2');
  const dialog = page.locator('.session-rename-dialog');
  await dialog.locator('input').fill('对话框里改的名字');
  await dialog.locator('input').press('Enter');
  await until(async () => (await onServer(session.id)).title === '对话框里改的名字', 5000, 'renamed');
  await dialog.waitFor({ state: 'detached' });
  assert.match(await page.locator('[data-pane-tab-id="one"]').innerText(), /对话框里改的名字/);
  assert.equal((await sidebarRow(page, session.id).locator('.workspace-session-title').innerText()).trim(), '对话框里改的名字');
  assert.deepEqual(browser.errors, []);
});

test('archive from the row, undo, archive from the menu, then restore and delete from All sessions', async () => {
  const { sessions: [session] } = await seed(browser.api, gateway, ['codex']);
  await browser.api(`/sessions/${session.id}`, { title: '归档测试' }, 'PATCH');
  await blankCanvas();
  const page = await openApp();
  const row = sidebarRow(page, session.id);
  await rowButton(page, session.id, '.session-archive-quick');
  await until(async () => !!(await onServer(session.id)).archived_at, 5000, 'archived');
  await row.waitFor({ state: 'detached' });
  await page.locator('.toast .toast-action', { hasText: '撤销' }).last().click();
  await until(async () => !(await onServer(session.id)).archived_at, 5000, 'restored by undo');
  await row.waitFor();

  await (await rowMenu(page, session.id)).locator('.session-archive-action').click();
  await until(async () => !!(await onServer(session.id)).archived_at, 5000, 'archived from the menu');
  await row.waitFor({ state: 'detached' });

  await page.locator('.workspace-global-nav .nav-item').nth(1).click();
  await page.waitForURL(/\/sessions/);
  const entry = page.locator('.sessions-row:not(.head)', { hasText: '归档测试' });
  assert.equal(await entry.count(), 0, 'current sessions leave out an archived one');
  await page.locator('.sessions-segments button', { hasText: '已归档会话' }).click();
  await entry.locator('button.icon-button').click();
  await page.getByRole('menuitem', { name: '恢复' }).click();
  await until(async () => !(await onServer(session.id)).archived_at, 5000, 'restored from the page');
  await entry.waitFor({ state: 'detached' });

  await page.locator('.sessions-segments button', { hasText: '未归档会话' }).click();
  await entry.locator('button.icon-button').click();
  await page.getByRole('menuitem', { name: '归档' }).click();
  await until(async () => !!(await onServer(session.id)).archived_at, 5000, 'archived from the page');
  await page.locator('.sessions-segments button', { hasText: '已归档会话' }).click();
  await entry.locator('button.icon-button').click();
  await page.getByRole('menuitem', { name: '删除', exact: true }).click();
  await page.getByRole('menuitem', { name: '永久删除' }).click();
  await until(async () => !(await onServer(session.id)), 5000, 'deleted');
  await entry.waitFor({ state: 'detached' });
  assert.deepEqual(browser.errors, []);
});

test('deleting an archived session closes the pane that showed it', async () => {
  const { sessions: [session] } = await seed(browser.api, gateway, ['terminal']);
  await browser.api(`/sessions/${session.id}`, { title: '要删除的终端' }, 'PATCH');
  await setLayout(browser.api, paneFor('doomed', session));
  const page = await openApp();
  await page.locator('[data-pane-id="doomed"]').waitFor();
  const row = sidebarRow(page, session.id);
  await rowButton(page, session.id, '.session-archive-quick');
  await row.waitFor({ state: 'detached' });
  assert.equal(await page.locator('[data-pane-id="doomed"]').count(), 1, 'archiving leaves the pane open');
  await page.locator('.workspace-global-nav .nav-item').nth(1).click();
  await page.locator('.sessions-segments button', { hasText: '已归档会话' }).click();
  await page.locator('.sessions-row:not(.head)', { hasText: '要删除的终端' }).locator('input[type=checkbox]').click();
  await page.locator('.sessions-bulk .small-button.danger').click();
  await page.locator('.sessions-bulk .small-button.danger', { hasText: '永久删除' }).click();
  await until(async () => !(await onServer(session.id)), 5000, 'deleted');
  await page.locator('.workspace-global-nav .nav-item').first().click();
  await page.locator('[data-pane-tab-id="doomed"]').waitFor({ state: 'detached' });
  assert.deepEqual(browser.errors, []);
});

test('a temporary window is marked as one, and closing it discards the session', async () => {
  await workspace(); await blankCanvas();
  const page = await openApp();
  // A terminal: a conversation asks before it is discarded (see the todo below).
  const first = await listed();
  await newFromWorkspaceMenu(page, '新建终端');
  await createdSince(first);
  const before = await listed();
  const group = page.locator('section.workspace-group[aria-label="E2E"]');
  await group.locator('.workspace-more-button').click();
  await group.locator('.workspace-more-panel .menu-flyout-trigger').hover();
  await page.locator('nav.menu-flyout .workspace-scratch-action').click();
  const temporary = await createdSince(before);
  assert.equal(temporary.ephemeral, true);
  assert.equal(temporary.provider, 'terminal', 'the temporary window is of the kind made last');
  await sidebarRow(page, temporary.id).locator('.session-temporary-badge').waitFor();
  const tab = page.locator(`[data-pane-tab-id="session-${temporary.id}"]`);
  await tab.locator('.dock-tab-ephemeral').waitFor();
  await tab.locator('.dock-tab-close').click();
  await until(async () => !(await onServer(temporary.id)), 5000, 'discarded');
  await sidebarRow(page, temporary.id).waitFor({ state: 'detached' });
  assert.deepEqual(browser.errors, []);
});

test('a kept temporary window stays when its tab is closed', async () => {
  await workspace(); await blankCanvas();
  const page = await openApp();
  const before = await listed();
  const group = page.locator('section.workspace-group[aria-label="E2E"]');
  await group.locator('.workspace-more-button').click();
  await group.locator('.workspace-more-panel .menu-flyout-trigger').hover();
  await page.locator('nav.menu-flyout .workspace-scratch-action').click();
  const temporary = await createdSince(before);
  await (await rowMenu(page, temporary.id)).locator('.session-keep-action').click();
  await until(async () => (await onServer(temporary.id)).ephemeral === false, 5000, 'kept');
  await sidebarRow(page, temporary.id).locator('.session-temporary-badge').waitFor({ state: 'detached' });
  await page.locator(`[data-pane-tab-id="session-${temporary.id}"] .dock-tab-close`).click();
  await page.locator(`[data-pane-tab-id="session-${temporary.id}"]`).waitFor({ state: 'detached' });
  assert.equal(await page.locator('.confirmation-bar').count(), 0);
  await page.waitForTimeout(500);
  assert.ok(await onServer(temporary.id), 'the session outlives its tab');
  await sidebarRow(page, temporary.id).waitFor();
  assert.deepEqual(browser.errors, []);
});

test('sessions are found from the sidebar search, Ctrl+B s and the All sessions page', async () => {
  const { workspace: ws } = await seed(browser.api, gateway, []);
  const titles = ['查找 甲 alpha', '查找 乙 beta', '查找 丙 gamma'];
  const sessions = [];
  for (const title of titles) sessions.push(await browser.api(`/workspaces/${ws.id}/sessions`, { provider: 'codex', title, interaction_mode: 'structured' }));
  await blankCanvas();
  const page = await openApp();
  const search = page.locator('[data-sidebar-search]');
  await search.fill('乙 beta');
  await until(async () => (await page.locator('.workspace-groups .workspace-session-title').allInnerTexts()).map(text => text.trim()).join() === '查找 乙 beta', 5000, 'one match');
  await search.fill('完全不存在的会话 zzzz');
  await page.locator('.workspace-nav-empty').waitFor();
  await page.locator('.workspace-nav-empty button').click();
  assert.equal(await search.inputValue(), '');
  await sidebarRow(page, sessions[0].id).waitFor();

  await page.locator('[data-pane-id="blank"]').click({ position: { x: 300, y: 300 } });
  await prefix(page, 's');
  await page.locator('.palette').waitFor();
  await page.keyboard.type('丙 gamma');
  await page.locator('.palette [role=option]', { hasText: '查找 丙 gamma' }).first().waitFor();
  await page.keyboard.press('Enter');
  await page.locator(`[data-pane-tab-id="session-${sessions[2].id}"]`).waitFor();

  await prefix(page, 'Shift+S');
  await page.waitForURL(/\/sessions/);
  await page.locator('input[type=search]').fill('甲 alpha');
  await until(async () => (await page.locator('.sessions-row:not(.head)').count()) === 1, 5000, 'one row');
  await page.locator('.sessions-row:not(.head) .sessions-title').click();
  await page.waitForURL(url => !url.pathname.startsWith('/sessions'));
  await page.locator(`[data-pane-tab-id="session-${sessions[0].id}"]`).waitFor();
  assert.deepEqual(browser.errors, []);
});

test('opening sessions from the sidebar switches between them without duplicating tabs', async () => {
  const { sessions } = await seed(browser.api, gateway, ['claude_code', 'codex', 'pi']);
  await blankCanvas();
  const page = await openApp();
  for (const session of [...sessions, sessions[0]]) {
    await sidebarRow(page, session.id).locator('.workspace-session').click();
    await page.locator(`[data-pane-id="session-${session.id}"]`).waitFor();
    assert.equal(await page.locator('.workspace-session[aria-current="true"]').count(), 1);
    assert.equal(await sidebarRow(page, session.id).locator('.workspace-session[aria-current="true"]').count(), 1, 'the open session is marked in the list');
  }
  for (const session of sessions) assert.equal(await page.locator(`[data-pane-tab-id="session-${session.id}"]`).count(), 1, 'one tab each');
  assert.deepEqual(browser.errors, []);
});

test('many sessions: the list shows them all and the page archives a filtered set at once', async () => {
  const { workspace: ws } = await seed(browser.api, gateway, []);
  const many = [];
  for (let i = 0; i < 40; i++) many.push(await browser.api(`/workspaces/${ws.id}/sessions`, { provider: 'claude_code', title: `批量 ${i} ${'很长的标题'.repeat(i ? 1 : 12)}`, interaction_mode: 'structured' }));
  await blankCanvas();
  const page = await openApp();
  for (const session of many) await sidebarRow(page, session.id).waitFor({ state: 'attached' });
  const overflow = await page.locator('.workspace-groups').evaluate(element => element.scrollWidth - element.clientWidth);
  assert.equal(overflow, 0, 'a long title does not widen the sidebar');
  await page.locator('.workspace-global-nav .nav-item').nth(1).click();
  await page.locator('input[type=search]').fill('批量');
  await until(async () => (await page.locator('.sessions-row:not(.head)').count()) === 40, 5000, '40 rows');
  await page.locator('.sessions-row.head input[type=checkbox]').check();
  assert.match(await page.locator('.sessions-bulk strong').innerText(), /40/);
  await page.locator('.sessions-bulk .small-button', { hasText: '归档' }).click();
  await until(async () => (await listed()).filter(session => many.some(item => item.id === session.id)).every(session => session.archived_at), 15000, 'all archived');
  await page.locator('.sessions-empty').waitFor();
  await page.locator('.workspace-global-nav .nav-item').first().click();
  assert.equal(await sidebarRow(page, many[0].id).count(), 0);
  assert.deepEqual(browser.errors, []);
});

test('a rename on one page reaches another page open beside it', async () => {
  const { sessions: [session] } = await seed(browser.api, gateway, ['codex']);
  await setLayout(browser.api, paneFor('shared', session));
  const one = await openApp(), two = await openApp();
  await (await rowMenu(one, session.id)).locator('.session-rename-action').click();
  const input = sidebarRow(one, session.id).locator('.sidebar-session-rename input');
  await input.fill('两个页面');
  await input.press('Enter');
  await until(async () => (await sidebarRow(two, session.id).innerText()).includes('两个页面'), 12000, 'the other sidebar');
  await until(async () => (await two.locator('[data-pane-tab-id="shared"]').innerText()).includes('两个页面'), 12000, 'the other tab');
  // Its canvas save may conflict (see the todo below); only the rename is checked here.
});

// ---- Bugs found while exploring: each reproduces one, and fails until it is fixed.

// Regression (found by the exploratory run): every open page rewrites the shared canvas when a session title changes, so the second gets 409 and a "another page updated the canvas" banner
test('a session renamed while two pages are open leaves the other page without a canvas conflict', async () => {
  const { sessions: [session] } = await seed(browser.api, gateway, ['codex']);
  await setLayout(browser.api, paneFor('both', session));
  const one = await openApp(), two = await openApp();
  await one.locator('[data-pane-id="both"]').waitFor(); await two.locator('[data-pane-id="both"]').waitFor();
  await browser.api(`/sessions/${session.id}`, { title: '服务器改的名字' }, 'PATCH');
  await until(async () => (await one.locator('[data-pane-tab-id="both"]').innerText()).includes('服务器改的名字') && (await two.locator('[data-pane-tab-id="both"]').innerText()).includes('服务器改的名字'), 12000, 'both tabs renamed');
  await one.waitForTimeout(2000);
  assert.equal(await one.locator('.confirmation-bar').count() + await two.locator('.confirmation-bar').count(), 0, 'no conflict banner');
  assert.deepEqual(browser.errors, []);
});

// Regression (found by the exploratory run): "删除会话…" swaps the focused button out, focusout closes the menu, so the confirmation never shows
test('a temporary session is deleted from its sidebar menu', async () => {
  const ws = await workspace();
  const temporary = await browser.api(`/workspaces/${ws.id}/sessions`, { provider: 'codex', title: '临时待删', interaction_mode: 'structured', ephemeral: true });
  await blankCanvas();
  const page = await openApp();
  const row = await rowMenu(page, temporary.id);
  await row.locator('.session-delete-action').click();
  await row.locator('.session-delete-confirm').waitFor({ timeout: 2000 });
  await row.locator('.session-delete-action').click();
  await until(async () => !(await onServer(temporary.id)), 5000, 'deleted');
  assert.deepEqual(browser.errors, []);
});

// Regression (found by the exploratory run): the server limits names to 200 bytes, not characters (crates/server/src/app.rs bounded_name), so 67+ CJK characters are refused
test('a 70-character Chinese title is accepted, as the 200-character field allows', async () => {
  const { sessions: [session] } = await seed(browser.api, gateway, ['codex']);
  await blankCanvas();
  const page = await openApp();
  const title = '中'.repeat(70);
  await sidebarRow(page, session.id).locator('.workspace-session').focus();
  await page.keyboard.press('F2');
  await sidebarRow(page, session.id).locator('.sidebar-session-rename input').fill(title);
  await page.keyboard.press('Enter');
  await page.waitForTimeout(1000);
  assert.equal((await onServer(session.id)).title, title);
  assert.deepEqual(browser.errors, []);
});

// Regression (found by the exploratory run): renameSession returns false for an unchanged title, so App keeps the dialog open and Save does nothing
test('saving the rename dialog without a change closes it', async () => {
  const { sessions: [session] } = await seed(browser.api, gateway, ['codex']);
  await setLayout(browser.api, paneFor('same', session));
  const page = await openApp();
  await page.locator('[data-pane-id="same"]').click({ position: { x: 300, y: 300 } });
  await page.keyboard.press('F2');
  const dialog = page.locator('.session-rename-dialog');
  await dialog.waitFor();
  await dialog.locator('.primary-button').click();
  await dialog.waitFor({ state: 'detached', timeout: 2000 });
});

// Regression (found by the exploratory run): asksBeforeDiscard reads status "running", which the server reports for any open client, so every started temporary chat says it is "still running"
test('closing an idle temporary conversation discards it without asking', async () => {
  await workspace(); await blankCanvas();
  const page = await openApp();
  const before = await listed();
  await newFromWorkspaceMenu(page, '新建 Codex 会话');
  await createdSince(before);
  const again = await listed();
  const group = page.locator('section.workspace-group[aria-label="E2E"]');
  await group.locator('.workspace-more-button').click();
  await group.locator('.workspace-more-panel .menu-flyout-trigger').hover();
  await page.locator('nav.menu-flyout .workspace-scratch-action').click();
  const temporary = await createdSince(again);
  const paneId = `session-${temporary.id}`;
  await sendMessage(page, paneId, 'hello');
  await page.locator(`[data-pane-id="${paneId}"]`).getByText('Hello 🦊').first().waitFor({ timeout: 15000 });
  await page.locator(`[data-pane-id="${paneId}"] button.chat-send:not(.chat-stop)`).waitFor();
  await page.locator(`[data-pane-tab-id="${paneId}"] .dock-tab-close`).click();
  await page.waitForTimeout(500);
  assert.equal(await page.locator('.confirmation-bar').count(), 0, 'the turn is over: nothing is lost');
  await until(async () => !(await onServer(temporary.id)), 5000, 'discarded');
});

// Regression (found by the exploratory run): archiveSession in SidebarSessionRow refocuses its own menu button (closeMenu(true)), which then leaves the DOM, so focus falls to <body>
test('Delete on a sidebar row archives it and moves focus to the next row', async () => {
  const { sessions } = await seed(browser.api, gateway, ['codex', 'codex', 'codex']);
  await blankCanvas();
  const page = await openApp();
  const middle = sessions[1];
  await sidebarRow(page, middle.id).locator('.workspace-session').focus();
  await page.keyboard.press('Delete');
  await until(async () => !!(await onServer(middle.id)).archived_at, 5000, 'archived');
  await sidebarRow(page, middle.id).waitFor({ state: 'detached' });
  assert.ok(await page.evaluate(() => !!document.activeElement?.closest('[data-sidebar-session-id]')), 'focus stays in the list');
});

// Regression (found by the exploratory run): revealSession still treats temporary sessions as absent from the sidebar (WorkspaceSidebar.vue) and jumps to the All sessions page
test('locating a focused temporary window finds its row in the sidebar', async () => {
  const ws = await workspace();
  const temporary = await browser.api(`/workspaces/${ws.id}/sessions`, { provider: 'terminal', title: '定位临时', ephemeral: true });
  await browser.api(`/sessions/${temporary.id}/start`, {});
  await setLayout(browser.api, paneFor('scratch', temporary));
  const page = await openApp();
  await page.locator('[data-pane-id="scratch"] .terminal-host').click();
  await page.locator('.workspace-locate-button').click();
  await page.waitForTimeout(500);
  assert.equal(new URL(page.url()).pathname, '/', 'stays on the canvas');
  assert.ok(await page.evaluate(id => document.activeElement?.closest('[data-sidebar-session-id]')?.dataset.sidebarSessionId === id, temporary.id), 'its row is focused');
});

// Last: it leaves a second workspace selected, which the tests above do not expect.
test('a workspace is added from the sidebar; adding its folder again opens it, and a missing folder is refused', async () => {
  await workspace(); await blankCanvas();
  const folder = join(gateway.home, '第二个项目');
  await mkdir(folder);
  const git = (...args) => execFileSync('git', args, { cwd: folder, stdio: 'ignore' });
  git('init', '-q', '-b', 'main'); await writeFile(join(folder, 'README.md'), '# 二\n');
  git('add', '.'); git('-c', 'user.name=e2e', '-c', 'user.email=e2e@example.invalid', 'commit', '-qm', 'Initial');
  const page = await openApp();
  const add = async path => {
    await page.locator('.workspace-group-label button[aria-label="添加工作区"]').click();
    const form = page.locator('.workspace-form');
    await form.waitFor();
    if (!(await form.locator('.advanced-directory').evaluate(details => details.open))) await form.locator('.advanced-directory summary').click();
    await form.locator('.advanced-directory input').fill(path);
    return form;
  };
  let form = await add(folder);
  assert.equal(await form.locator(':scope > label input').inputValue(), '第二个项目', 'named after its folder');
  await form.locator('.dialog-actions .primary-button').click();
  await page.locator('section.workspace-group[aria-label="第二个项目"].selected').waitFor();
  const count = (await browser.api('/workspaces')).length;

  // Typed as the server stores it (resolved: /var is /private/var on macOS), the dialog knows it.
  form = await add(await realpath(folder));
  await until(async () => (await form.locator('.dialog-actions .primary-button').innerText()).includes('打开工作区'), 5000, 'the folder is known');
  await form.locator('.dialog-actions .primary-button').click();
  await form.waitFor({ state: 'detached' });
  assert.equal((await browser.api('/workspaces')).length, count, 'no second workspace for one folder');

  form = await add(join(gateway.home, 'missing'));
  await form.locator('.dialog-actions .primary-button').click();
  await form.locator('.inline-error').waitFor();
  await form.locator('.dialog-actions .secondary-button').click();
  assert.equal((await browser.api('/workspaces')).length, count);
  assert.deepEqual(browser.errors, ['400 POST /api/workspaces'], 'only the refused folder failed');
});
