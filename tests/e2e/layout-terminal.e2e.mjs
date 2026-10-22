// The canvas arranged by hand and by Ctrl+B, and the terminals in it.
// canvas.e2e.mjs covers moving between panes, h/l/o/;/z/%/x/?/: and echo;
// this file covers the rest of the layout keys, the mouse, persistence, two
// pages on one canvas, narrow windows, and what a terminal does in a pane.
//
// Terminals run a plain bash (TENDLY_SHELL, set before the gateway starts) so
// the prompt is "$ " whatever the person's own shell prints.
import { after, afterEach, before, test } from 'node:test';
import assert from 'node:assert/strict';
import { chmod, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { openBrowser, paneFor, row, seed, setLayout, startGateway, until } from './harness.mjs';

let gateway, browser, shellDir;
before(async () => {
  shellDir = await mkdtemp(join(tmpdir(), 'tendly-e2e-shell-'));
  const shell = join(shellDir, 'bash.sh');
  await writeFile(shell, '#!/bin/sh\nPS1="$ " BASH_SILENCE_DEPRECATION_WARNING=1 exec /bin/bash --norc --noprofile "$@"\n');
  await chmod(shell, 0o755);
  process.env.TENDLY_SHELL = shell;
  gateway = await startGateway();
  browser = await openBrowser(gateway, { width: 2400, height: 1000 });
});
afterEach(() => browser.reset());
after(async () => { await browser?.close(); await gateway?.stop(); await rm(shellDir, { recursive: true, force: true }); });

// ---- helpers ---------------------------------------------------------------

const pane = (page, id) => page.locator(`[data-pane-id="${id}"]`);
const host = (page, id) => page.locator(`[data-pane-id="${id}"] .terminal-host`);
const tab = (page, id) => page.locator(`[data-pane-tab-id="${id}"]`);
const selected = page => page.locator('.dock-pane.is-selected').getAttribute('data-pane-id');
/** The pane holding keyboard focus, and whether it is the input of that pane (a terminal's or a composer's). */
const focus = page => page.evaluate(() => {
  const element = document.activeElement;
  return { pane: element?.closest('[data-pane-id]')?.getAttribute('data-pane-id') ?? null, input: element?.tagName === 'TEXTAREA' || element?.tagName === 'INPUT' };
});
/** A terminal's screen, as lines without trailing blanks. */
const screen = async (page, id) => (await host(page, id).innerText()).split('\n').map(line => line.trimEnd()).filter(Boolean);
/** Waits for the shell prompt, so typing is not lost to a terminal still loading. */
const ready = (page, id) => until(async () => (await screen(page, id).catch(() => [])).some(line => line.startsWith('$')), 15000, `the prompt in ${id}`);
async function run(page, id, command, expect) {
  await page.keyboard.type(command); await page.keyboard.press('Enter');
  if (expect) await until(async () => (await screen(page, id)).some(line => line === expect), 10000, `"${expect}" in ${id}`);
}
/** Lets terminals that (re)mounted finish loading: one that loads takes the focus (see the todo tests below). */
const settle = page => page.waitForTimeout(600);
async function prefix(page, key) {
  await page.keyboard.press('Control+b');
  await page.locator('.prefix-hint').waitFor();
  if (key) await page.keyboard.press(key);
}
/** The saved canvas, reduced to its shape: ids, tab groups and ratios. */
function shape(node) {
  if (node.type === 'pane') return node.id;
  if (node.type === 'stack') return { tabs: node.panes.map(item => item.id), ...(node.activePaneId ? { active: node.activePaneId } : {}) };
  return { [node.direction === 'horizontal' ? 'row' : 'column']: Math.round(node.ratio * 100) / 100, first: shape(node.first), second: shape(node.second) };
}
const saved = async () => shape((await browser.api('/canvas/layout')).layout.root);
/** Waits until the saved canvas has the given shape (a save follows a change by ~450 ms). */
const savedAs = (expected, what = 'the saved layout') => until(async () => JSON.stringify(await saved()) === JSON.stringify(expected), 8000, what).catch(async error => { assert.deepEqual(await saved(), expected); throw error; });
let sizes = 0;
/** What `stty size` says in a terminal now, as "rows cols". */
async function sttySize(page, id) {
  const tag = `size-${++sizes}:`;
  await page.keyboard.type(`echo ${tag}$(stty size)`); await page.keyboard.press('Enter');
  let found;
  await until(async () => !!(found = (await screen(page, id)).find(line => line.startsWith(tag))), 5000, tag);
  return found.slice(tag.length);
}
const stack = (id, panes, active) => ({ type: 'stack', kind: 'stack', id, panes, activePaneId: active });

/** Terminals named `ids`, laid out by `layout(panes)`, on a page with the first one focused. */
async function canvasOf(ids, layout = panes => row(...panes), providers = ids.map(() => 'terminal')) {
  const { sessions } = await seed(browser.api, gateway, providers);
  const panes = ids.map((id, index) => paneFor(id, sessions[index]));
  await setLayout(browser.api, layout(panes));
  const page = await browser.newPage();
  await page.goto(gateway.base);
  await page.locator('.dock-pane').first().waitFor();
  // Only a group's front tab is drawn; each drawn terminal is waited for, then the first is clicked.
  const shown = [];
  for (const [index, id] of ids.entries()) if (providers[index] === 'terminal' && await pane(page, id).count()) { await ready(page, id); shown.push(id); }
  await settle(page);
  if (shown.length) await host(page, shown[0]).click();
  return { page, sessions };
}
/** Records the lowest opacity each pane reaches over `ms`, frame by frame. */
const sampleOpacity = (page, ms = 1500) => page.evaluate(ms => {
  window.lowest = {};
  const started = performance.now();
  const sample = () => {
    for (const element of document.querySelectorAll('.dock-pane[data-pane-id]')) {
      const id = element.dataset.paneId;
      window.lowest[id] = Math.min(window.lowest[id] ?? 1, Number(getComputedStyle(element).opacity));
    }
    if (performance.now() - started < ms) requestAnimationFrame(sample);
  };
  requestAnimationFrame(sample);
}, ms);
const lowestOpacity = page => page.evaluate(() => window.lowest);

// ---- layout keys -----------------------------------------------------------

test('Ctrl+B " and - split top and bottom, focus the new pane and are saved', async () => {
  const { page } = await canvasOf(['one', 'two']);
  await prefix(page, '"');
  await savedAs({ row: 0.5, first: { column: 0.5, first: 'one', second: { tabs: [] } }, second: 'two' });
  assert.ok(await page.evaluate(() => !!document.activeElement?.closest('.dock-empty')), 'the empty pane is focused');
  await host(page, 'two').click();
  await prefix(page, '-');
  assert.equal(await page.locator('.dock-split.is-vertical').count(), 2);
  assert.equal(await page.locator('.dock-pane').count(), 4);
  assert.deepEqual(browser.errors, []);
});

test('Ctrl+B H J K L move the divider, again without the prefix while the key repeats', async () => {
  const { page } = await canvasOf(['one', 'two']);
  await prefix(page, 'L'); await page.keyboard.press('L'); await page.keyboard.press('L');
  await savedAs({ row: 0.65, first: 'one', second: 'two' }, 'three steps right');
  await page.waitForTimeout(1000); // past the repeat window: a Ctrl+B inside it goes to the shell (todo below)
  await prefix(page, 'H');
  await savedAs({ row: 0.6, first: 'one', second: 'two' }, 'one step left');
  // Past the repeat window a plain L is the shell's again.
  await page.waitForTimeout(1200);
  await run(page, 'one', 'echo after-$((1+1))', 'after-2');
  assert.deepEqual(await saved(), { row: 0.6, first: 'one', second: 'two' });
  const [left, right] = await Promise.all([pane(page, 'one').boundingBox(), pane(page, 'two').boundingBox()]);
  assert.ok(left.width > right.width * 1.3, `the left pane is wider (${left.width} vs ${right.width})`);
  assert.deepEqual(browser.errors, []);
});

test('Ctrl+B { and } swap pane groups; a conversation keeps its composer focused', async () => {
  const { page } = await canvasOf(['chat', 'one', 'two'], panes => row(...panes), ['claude_code', 'terminal', 'terminal']);
  await pane(page, 'chat').locator('textarea[data-pane-input]').click();
  await prefix(page, '}');
  await savedAs({ row: 0.33, first: 'one', second: { row: 0.5, first: 'chat', second: 'two' } }, 'chat moved right');
  await prefix(page, '}');
  await savedAs({ row: 0.33, first: 'one', second: { row: 0.5, first: 'two', second: 'chat' } }, 'chat moved right again');
  await prefix(page, '{');
  await savedAs({ row: 0.33, first: 'one', second: { row: 0.5, first: 'chat', second: 'two' } }, 'chat moved back');
  await page.waitForTimeout(500);
  assert.equal(await selected(page), 'chat');
  assert.deepEqual(await focus(page), { pane: 'chat', input: true });
  assert.deepEqual(browser.errors, []);
});

test('Ctrl+B Space walks the presets and keeps every pane', async () => {
  const { page } = await canvasOf(['one', 'two', 'three']);
  await prefix(page, ' ');
  await savedAs({ row: 0.5, first: { tabs: ['one', 'three'], active: 'one' }, second: 'two' }, 'preset 1:1');
  await prefix(page, ' ');
  // Panes are dealt out in the order the canvas holds them: one, three, two.
  await savedAs({ row: 0.5, first: { column: 0.5, first: 'one', second: 'three' }, second: { column: 0.5, first: 'two', second: { tabs: [] } } }, 'preset 2x2');
  await prefix(page, ' ');
  await savedAs({ row: 0.25, first: 'one', second: { row: 0.67, first: 'three', second: 'two' } }, 'preset 1:2:1');
  assert.equal(await page.locator('.dock-pane[data-pane-id]').count(), 3);
  assert.deepEqual(browser.errors, []);
});

test('Ctrl+B n, p and a number move through tabs; a conversation tab gets its composer', async () => {
  const { page } = await canvasOf(['one', 'chat', 'two'], ([one, chat, two]) => row(stack('tabs', [one, chat], 'one'), two), ['terminal', 'claude_code', 'terminal']);
  await prefix(page, 'n');
  await savedAs({ row: 0.5, first: { tabs: ['one', 'chat'], active: 'chat' }, second: 'two' });
  assert.equal(await selected(page), 'chat');
  assert.deepEqual(await focus(page), { pane: 'chat', input: true });
  await prefix(page, 'p'); await until(async () => await selected(page) === 'one', 3000, 'p back to one');
  await prefix(page, '2'); await until(async () => await selected(page) === 'chat', 3000, '2 to chat');
  await prefix(page, '9'); await page.waitForTimeout(300);
  assert.equal(await selected(page), 'chat', 'no ninth tab, nothing moves');
  assert.deepEqual(browser.errors, []);
});

test('Ctrl+B b, e, f go to the sidebar, the files and their filter; Esc and Ctrl+B i come back', async () => {
  const { page } = await canvasOf(['one', 'two']);
  await prefix(page, 'b');
  assert.ok(await page.evaluate(() => !!document.activeElement?.closest('.workspace-sidebar')), 'b: in the sidebar');
  await page.keyboard.press('Escape');
  await until(async () => (await focus(page)).pane === 'one', 3000, 'Esc back to the pane');
  await prefix(page, 'e');
  assert.ok(await page.evaluate(() => !!document.activeElement?.closest('.explorer-panel')), 'e: in the file panel');
  await page.keyboard.press('Escape');
  await prefix(page, 'f');
  assert.ok(await page.evaluate(() => document.activeElement?.matches('[data-explorer-filter]')), 'f: in the file filter');
  await prefix(page, 'i');
  await until(async () => JSON.stringify(await focus(page)) === JSON.stringify({ pane: 'one', input: true }), 3000, 'i: back in the terminal');
  await run(page, 'one', 'echo back-$((2*3))', 'back-6');
  assert.deepEqual(browser.errors, []);
});

test('Ctrl+B B, E, S, W, w and s show and hide the app around the canvas', async () => {
  const { page } = await canvasOf(['one', 'two']);
  const explorer = () => page.locator('.explorer-panel').count();
  const shown = await explorer();
  await prefix(page, 'E'); await until(async () => await explorer() !== shown, 3000, 'E toggles the files');
  await prefix(page, 'E'); await until(async () => await explorer() === shown, 3000, 'E toggles them back');
  const sidebar = () => page.locator('.workspace-sidebar').isVisible();
  const open = await sidebar();
  await prefix(page, 'B'); await until(async () => await sidebar() !== open, 3000, 'B toggles the sidebar');
  await prefix(page, 'B'); await until(async () => await sidebar() === open, 3000, 'B toggles it back');
  await prefix(page, 'S'); await page.waitForURL(/\/sessions$/);
  await prefix(page, 'W'); await page.waitForURL(url => new URL(url).pathname === '/');
  await pane(page, 'two').waitFor();
  await prefix(page, 'w'); await page.locator('.palette').waitFor();
  assert.equal(await page.locator('.palette [role="option"]').count(), 2, 'w lists the two open views');
  await page.keyboard.press('Escape');
  await prefix(page, 's'); await page.locator('.palette').waitFor();
  await page.keyboard.press('Escape');
  assert.deepEqual(browser.errors, []);
});

test('Ctrl+B t opens a terminal in the group and types into it; g opens Git changes', async () => {
  const { page } = await canvasOf(['one', 'two']);
  await prefix(page, 't');
  await until(async () => (await page.locator('.dock-pane.is-selected [data-pane-tab-id]').count()) === 2, 8000, 'a second tab in the group');
  const created = await selected(page);
  assert.notEqual(created, 'one');
  await ready(page, created);
  await until(async () => (await focus(page)).pane === created, 5000, 'the new terminal focused');
  await run(page, created, 'echo fresh-$((5*5))', 'fresh-25');
  await prefix(page, 'g');
  await page.locator('.dock-tab[data-pane-tab-id^="changes-"]').waitFor();
  assert.deepEqual(browser.errors, []);
});

// ---- the mouse ---------------------------------------------------------------

test('a tab dragged to the middle of a pane joins its tabs; to an edge, splits it', async () => {
  const { page } = await canvasOf(['one', 'two', 'three', 'four'], ([one, two, three, four]) => row(one, stack('tabs', [two, three, four], 'two')));
  await tab(page, 'three').dragTo(pane(page, 'one').locator('.dock-content'));
  await savedAs({ row: 0.5, first: { tabs: ['one', 'three'], active: 'three' }, second: { tabs: ['two', 'four'], active: 'two' } }, 'three stacked on one');
  const box = await pane(page, 'two').boundingBox();
  await tab(page, 'four').dragTo(pane(page, 'two'), { targetPosition: { x: box.width / 2, y: box.height - 20 } });
  await savedAs({ row: 0.5, first: { tabs: ['one', 'three'], active: 'three' }, second: { column: 0.5, first: { tabs: ['two'], active: 'two' }, second: 'four' } }, 'four below two');
  await ready(page, 'four');
  assert.deepEqual(browser.errors, []);
});

test('the divider follows a drag, resets on double-click and moves with the arrow keys', async () => {
  const { page } = await canvasOf(['one', 'two']);
  const divider = page.locator('.dock-separator');
  const bounds = await divider.boundingBox();
  await page.mouse.move(bounds.x + 3, bounds.y + 200);
  await page.mouse.down();
  await page.mouse.move(bounds.x + 3 - 300, bounds.y + 200, { steps: 8 });
  await page.mouse.up();
  const dragged = (await saved()).row;
  await until(async () => (await saved()).row < 0.45, 5000, 'the drag saved');
  await divider.dblclick();
  await savedAs({ row: 0.5, first: 'one', second: 'two' }, 'double-click resets to 1:1');
  await divider.focus(); await page.keyboard.press('ArrowRight');
  await until(async () => (await saved()).row > 0.5, 5000, 'arrow key moves the divider');
  assert.ok(dragged <= 0.5);
  assert.deepEqual(browser.errors, []);
});

test('the tab menu closes tabs to the right, to the left, the others and all; sessions keep running', async () => {
  const ids = ['a', 'b', 'c', 'd', 'e', 'f'];
  const { page, sessions } = await canvasOf(['one', ...ids], ([one, ...tabs]) => row(one, stack('tabs', tabs, 'c')));
  const menu = async (id, item) => {
    await tab(page, id).click({ button: 'right' });
    await page.getByRole('menuitem', { name: '关闭此标签页' }).hover();
    await page.getByRole('menuitem', { name: item }).click();
  };
  await menu('e', '关闭右侧标签页');
  await savedAs({ row: 0.5, first: 'one', second: { tabs: ['a', 'b', 'c', 'd', 'e'], active: 'c' } }, 'f closed');
  await menu('b', '关闭左侧标签页');
  await savedAs({ row: 0.5, first: 'one', second: { tabs: ['b', 'c', 'd', 'e'], active: 'c' } }, 'a closed');
  await menu('d', '关闭其他标签页');
  await savedAs({ row: 0.5, first: 'one', second: { tabs: ['d'], active: 'd' } }, 'only d left');
  await tab(page, 'one').click();
  await tab(page, 'd').locator('.dock-tab-close').click();
  await savedAs('one', 'd closed by its cross');
  await menu('one', '关闭全部标签页');
  await until(async () => (await browser.api('/canvas/layout')).layout.root.type === 'stack', 5000, 'the canvas emptied');
  const statuses = await Promise.all(sessions.map(async session => (await browser.api(`/sessions/${session.id}`)).status));
  assert.deepEqual(statuses, sessions.map(() => 'running'), 'closing a tab never ends its session');
  assert.deepEqual(browser.errors, []);
});

test('maximize and restore from the pane header; the layout survives a reload and a reopened session', async () => {
  const { page, sessions } = await canvasOf(['one', 'two', 'three']);
  await pane(page, 'two').getByRole('button', { name: '最大化窗格' }).click();
  assert.equal(await page.locator('.dock-pane').count(), 1);
  await page.getByRole('button', { name: '恢复布局' }).click();
  assert.equal(await page.locator('.dock-pane').count(), 3);
  await tab(page, 'three').locator('.dock-tab-close').click();
  await savedAs({ row: 0.33, first: 'one', second: 'two' }, 'three closed');
  await page.waitForTimeout(600);
  await page.reload();
  await pane(page, 'two').waitFor();
  assert.equal(await pane(page, 'three').count(), 0, 'still closed after a reload');
  // Opened again from the sidebar, it comes back as a tab of the selected pane.
  await host(page, 'two').click();
  await page.locator(`.workspace-sidebar [data-sidebar-session-id="${sessions[2].id}"] button.workspace-session`).click();
  await until(async () => (await page.locator('[data-pane-tab-id]').count()) === 3, 5000, 'the session reopened');
  assert.deepEqual(browser.errors, []);
});

// ---- two pages, narrow windows ----------------------------------------------

test('two pages on one canvas: the later save asks, and each answer keeps the other layout recoverable', async () => {
  const { sessions: [one, two] } = await seed(browser.api, gateway, ['terminal', 'terminal']);
  await setLayout(browser.api, row(paneFor('one', one), paneFor('two', two)));
  const open = async () => { const page = await browser.newPage(); await page.goto(gateway.base); await ready(page, 'two'); await host(page, 'one').click(); return page; };
  const first = await open(), second = await open();
  const status = page => page.locator('footer.statusbar span', { hasText: '布局：' }).first().innerText();
  await prefix(first, '%');
  await until(async () => await status(first) === '布局：已保存' && (await saved()).first.row === 0.5, 5000, 'the first page saved');
  await prefix(second, '}');
  await second.locator('.confirmation-bar[role="alert"]').waitFor();
  assert.equal(await status(second), '布局：保存冲突');
  await second.getByRole('button', { name: '加载服务端布局' }).click();
  await until(async () => await second.locator('.dock-pane').count() === 3, 5000, 'the server layout loaded');
  await second.getByRole('button', { name: '恢复本地布局' }).click();
  await savedAs({ row: 0.5, first: 'two', second: 'one' }, 'the swapped layout restored and saved');
  await prefix(first, 'L');
  await first.locator('.confirmation-bar[role="alert"]').waitFor();
  await first.getByRole('button', { name: '改为保存当前布局' }).click();
  await savedAs({ row: 0.5, first: { row: 0.55, first: 'one', second: { tabs: [] } }, second: 'two' }, 'the first page kept its own');
  // The only failures are the two refused saves that raised the bar.
  assert.deepEqual(browser.errors, ['409 PUT /api/canvas/layout', '409 PUT /api/canvas/layout']);
});

test('a narrow window shows the splits as tabs and gives them back, with the terminal resized each way', async () => {
  const { page } = await canvasOf(['one', 'two', 'three']);
  const before = await saved();
  await page.setViewportSize({ width: 900, height: 700 });
  await until(async () => await page.locator('.dock-pane').count() === 1, 5000, 'collapsed into one tab group');
  assert.deepEqual(await page.locator('[data-pane-tab-id]').evaluateAll(tabs => tabs.map(item => item.dataset.paneTabId)), ['one', 'two', 'three']);
  await tab(page, 'two').click();
  await ready(page, 'two'); await settle(page); await host(page, 'two').click();
  const narrow = await sttySize(page, 'two');
  await page.setViewportSize({ width: 2400, height: 1000 });
  await until(async () => await page.locator('.dock-pane').count() === 3, 5000, 'the splits back');
  await ready(page, 'two'); await settle(page);
  await host(page, 'two').click();
  const wide = await sttySize(page, 'two');
  assert.ok(Number(wide.split(' ')[0]) > Number(narrow.split(' ')[0]), `more rows in the taller window (${narrow} → ${wide})`);
  assert.deepEqual(await saved(), before, 'collapsing never rewrites the saved layout');
  assert.deepEqual(browser.errors, []);
});

// ---- terminals ----------------------------------------------------------------

test('a terminal takes Chinese and emoji, and is told its size when the pane grows', async () => {
  const { page } = await canvasOf(['one', 'two']);
  await run(page, 'one', 'echo 你好世界-$((6*7)) 🦊', '你好世界-42 🦊');
  await page.keyboard.insertText('echo 中文输入-ok'); await page.keyboard.press('Enter');
  await until(async () => (await screen(page, 'one')).includes('中文输入-ok'), 5000, 'text inserted as an IME does');
  const half = Number((await sttySize(page, 'one')).split(' ')[1]);
  await prefix(page, 'z');
  await ready(page, 'one'); await settle(page);
  await host(page, 'one').click();
  const full = Number((await sttySize(page, 'one')).split(' ')[1]);
  assert.ok(full > half * 1.6, `more columns once maximized (${half} → ${full})`);
  assert.deepEqual(browser.errors, []);
});

test('find, copy and paste in a terminal, through a long output', async () => {
  await browser.context.grantPermissions(['clipboard-read', 'clipboard-write'], { origin: gateway.base });
  const { page } = await canvasOf(['one']);
  await run(page, 'one', 'for i in $(seq 1 1500); do echo line-$i; done', 'line-1500');
  await page.keyboard.press('Meta+f');
  const search = pane(page, 'one').locator('.terminal-search');
  await search.locator('input').waitFor();
  await page.keyboard.type('line-17');
  // line-17 and line-170…179: found in the history far above the rows on screen.
  await until(async () => (await search.locator('.terminal-search-count').innerText().catch(() => '')) === '1 / 11', 5000, 'all 11 matches');
  await page.keyboard.press('Escape');
  await until(async () => await search.count() === 0 && (await focus(page)).input, 3000, 'Esc closes find and returns to the terminal');
  await page.evaluate(() => navigator.clipboard.writeText('echo pasted-$((1+1))'));
  await page.keyboard.press('Meta+v'); await page.keyboard.press('Enter');
  await until(async () => (await screen(page, 'one')).includes('pasted-2'), 5000, 'the paste ran');
  await run(page, 'one', 'echo copy-me-123', 'copy-me-123');
  await host(page, 'one').getByText('copy-me-123', { exact: true }).last().selectText();
  await page.keyboard.press('Meta+c');
  assert.equal((await page.evaluate(() => navigator.clipboard.readText())).trim(), 'copy-me-123');
  assert.deepEqual(browser.errors, []);
});

test('a reload, or a tab switch while output runs, reconnects with the history intact', async () => {
  const { page } = await canvasOf(['one', 'two'], ([one, two]) => stack('tabs', [one, two], 'one'));
  await run(page, 'one', 'for i in $(seq 1 30); do echo tick-$i; sleep 0.05; done; echo ticks-done');
  await tab(page, 'two').click();
  await ready(page, 'two');
  await tab(page, 'one').click();
  await until(async () => (await screen(page, 'one').catch(() => [])).includes('ticks-done'), 10000, 'the loop finished while away');
  const lines = await screen(page, 'one');
  for (let i = 1; i <= 30; i++) assert.ok(lines.includes(`tick-${i}`), `tick-${i} kept`);
  await page.reload();
  await ready(page, 'one');
  assert.ok((await screen(page, 'one')).includes('ticks-done'), 'the history replays after a reload');
  assert.deepEqual(browser.errors, []);
});

test('a terminal whose shell exits keeps its screen and starts again with a tap', async () => {
  const { page, sessions: [one] } = await canvasOf(['one']);
  await run(page, 'one', 'echo before-exit', 'before-exit');
  await run(page, 'one', 'exit');
  await pane(page, 'one').locator('.terminal-exited-bar').waitFor();
  assert.equal((await browser.api(`/sessions/${one.id}`)).status, 'stopped');
  assert.ok((await screen(page, 'one')).includes('before-exit'), 'the last screen stays');
  await host(page, 'one').click();
  await until(async () => (await browser.api(`/sessions/${one.id}`)).status === 'running', 8000, 'started again');
  await ready(page, 'one');
  await host(page, 'one').click();
  await run(page, 'one', 'echo again-$((3*3))', 'again-9');
  assert.deepEqual(browser.errors, []);
});

test('moving between panes, dragging and closing tabs never fades a pane that stays', async () => {
  const { page } = await canvasOf(['one', 'two', 'three', 'four'], ([one, two, three, four]) => row(one, two, stack('tabs', [three, four], 'three')));
  await sampleOpacity(page);
  await prefix(page, 'l'); await prefix(page, 'l'); await prefix(page, 'h'); await prefix(page, 'o');
  await page.waitForTimeout(1600);
  assert.deepEqual(await lowestOpacity(page), { one: 1, two: 1, three: 1 });
  await sampleOpacity(page);
  await tab(page, 'four').locator('.dock-tab-close').click({ force: true });
  await page.waitForTimeout(1600);
  assert.deepEqual(await lowestOpacity(page), { one: 1, two: 1, three: 1 });
  assert.deepEqual(browser.errors, []);
});

// ---- bugs found -------------------------------------------------------------

// Regression (found by the exploratory run): swapping two terminals leaves focus (and typing) in the other terminal
test('after Ctrl+B } the keys still go to the pane that moved', async () => {
  const { page } = await canvasOf(['one', 'two', 'three']);
  await prefix(page, '}');
  await savedAs({ row: 0.33, first: 'two', second: { row: 0.5, first: 'one', second: 'three' } });
  await page.waitForTimeout(1000);
  await page.keyboard.type('echo moved-$((4+4))'); await page.keyboard.press('Enter');
  await page.waitForTimeout(1000);
  assert.equal(await selected(page), 'one');
  assert.ok((await screen(page, 'one')).includes('moved-8'), 'typed into the pane that moved');
  assert.ok(!(await screen(page, 'two')).includes('moved-8'), 'not into the one it swapped with');
});

// Regression (found by the exploratory run): switching to a background terminal tab leaves focus on the tab header; typing goes nowhere
test('Ctrl+B n into a terminal tab puts the keys in that terminal', async () => {
  const { page } = await canvasOf(['one', 'two', 'three'], ([one, two, three]) => row(one, stack('tabs', [two, three], 'two')));
  await host(page, 'two').click();
  await prefix(page, 'n');
  await ready(page, 'three');
  await page.waitForTimeout(500);
  assert.deepEqual(await focus(page), { pane: 'three', input: true });
  await run(page, 'three', 'echo tabbed-$((1+2))', 'tabbed-3');
});

// Regression (found by the exploratory run): Ctrl+B x after Ctrl+B % / " closes the original pane, not the focused empty one
test('Ctrl+B x right after a split closes the new empty pane', async () => {
  const { page } = await canvasOf(['one', 'two']);
  await prefix(page, '%');
  await savedAs({ row: 0.5, first: { row: 0.5, first: 'one', second: { tabs: [] } }, second: 'two' }, 'split saved');
  assert.ok(await page.evaluate(() => !!document.activeElement?.closest('.dock-empty')), 'the empty pane is focused');
  await prefix(page, 'x');
  await page.waitForTimeout(1000);
  await savedAs({ row: 0.5, first: 'one', second: 'two' }, 'the empty pane gone, both terminals kept');
});

// Regression (found by the exploratory run): closing the selected pane with Ctrl+B x leaves focus on <body> (unless a terminal happens to remount)
test('after Ctrl+B x the keys go to the pane that is selected next', async () => {
  const { page } = await canvasOf(['chat', 'one'], panes => row(...panes), ['claude_code', 'terminal']);
  await prefix(page, 'x');
  await savedAs('chat');
  await page.waitForTimeout(500);
  assert.deepEqual(await focus(page), { pane: 'chat', input: true });
});

// Regression (found by the exploratory run): Ctrl+B within ~900 ms of H/J/K/L is sent to the shell and the next key is typed
test('Ctrl+B right after a repeating resize key starts a new prefix', async () => {
  const { page } = await canvasOf(['one', 'two']);
  await prefix(page, 'L');
  await page.waitForTimeout(200);
  await page.keyboard.press('Control+b');
  await page.keyboard.press('l');
  await until(async () => await selected(page) === 'two', 3000, 'Ctrl+B l moved right');
});

// Regression (found by the exploratory run): "Process exited · tap or type" — a key never restarts it (wterm stops keydown before the pane sees it)
test('typing into an exited terminal starts it again, as its bar says', async () => {
  const { page, sessions: [one] } = await canvasOf(['one']);
  await run(page, 'one', 'exit');
  await pane(page, 'one').locator('.terminal-exited-bar').waitFor();
  await page.keyboard.press('Enter');
  await until(async () => (await browser.api(`/sessions/${one.id}`)).status === 'running', 5000, 'restarted by a key');
});

// Regression (found by the exploratory run): Ctrl+B %, " and z re-create the pane: it replays its entrance from opacity 0 and its terminal reconnects
test('splitting or maximizing a pane does not fade it back in', async () => {
  const { page } = await canvasOf(['one', 'two']);
  await sampleOpacity(page);
  await prefix(page, 'z');
  await page.waitForTimeout(1600);
  assert.equal((await lowestOpacity(page)).one, 1, 'maximized pane never fades');
  await prefix(page, 'z');
  await page.waitForTimeout(300);
  await sampleOpacity(page);
  await prefix(page, '%');
  await page.waitForTimeout(1600);
  assert.equal((await lowestOpacity(page)).one, 1, 'the pane split never fades');
});

// Regression (found by the exploratory run): on load the last terminal to finish loading takes focus and selection, not the first pane
test('on load the selected pane holds the keyboard', async () => {
  const { sessions: [one, two, three] } = await seed(browser.api, gateway, ['terminal', 'terminal', 'terminal']);
  await setLayout(browser.api, row(paneFor('one', one), paneFor('two', two), paneFor('three', three)));
  const page = await browser.newPage();
  await page.goto(gateway.base);
  for (const id of ['one', 'two', 'three']) await ready(page, id);
  await page.waitForTimeout(500);
  const where = await focus(page);
  assert.equal(where.pane, await selected(page), 'focus and selection agree');
  assert.equal(where.pane, 'one', 'the first pane, as the page selects on load');
});
