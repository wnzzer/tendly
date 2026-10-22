// Files and Git as a person uses them: the file panel and editor, the Git
// pane, the workspace branch control, and a session's own branch chip.
// Each test gets a repository of its own, so no state leaks between them.
import { after, afterEach, before, test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { access, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { openBrowser, paneFor, row, sendMessage, setLayout, startGateway, until } from './harness.mjs';

let gateway, browser;
before(async () => { gateway = await startGateway(); browser = await openBrowser(gateway, { width: 2400, height: 1000 }); });
afterEach(() => browser.reset());
after(async () => { await browser?.close(); await gateway?.stop(); });

let repos = 0;
/** A fresh repository on `main`, one commit holding `files`, registered as a workspace. */
async function repository(files = { 'README.md': '# Fixture\n' }) {
  const root = join(gateway.home, `repo-${++repos}`);
  await mkdir(root);
  const git = (...args) => execFileSync('git', ['-c', 'user.name=e2e', '-c', 'user.email=e2e@example.invalid', ...args], { cwd: root, encoding: 'utf8' });
  git('init', '-q', '-b', 'main');
  for (const [path, content] of Object.entries(files)) { await mkdir(dirname(join(root, path)), { recursive: true }); await writeFile(join(root, path), content); }
  git('add', '.'); git('commit', '-qm', 'Initial');
  const workspace = await browser.api('/workspaces', { name: `Repo ${repos}`, root_path: root });
  return { root, git, workspace };
}
const editorPane = (workspace, path, id = 'file') => ({ type: 'pane', id, kind: 'editor', title: path.split('/').pop(), metadata: { workspace_id: workspace.id, path } });
const gitPane = (workspace, id = 'git') => ({ type: 'pane', id, kind: 'git_diff', title: 'Changes', metadata: { workspace_id: workspace.id } });
const chatSession = (workspace, title = 'claude e2e') => browser.api(`/workspaces/${workspace.id}/sessions`, { provider: 'claude_code', title, interaction_mode: 'structured' });
const exists = path => access(path).then(() => true, () => false);
const read = path => readFile(path, 'utf8');
async function open(root) {
  await setLayout(browser.api, root);
  const page = await browser.newPage();
  await page.goto(gateway.base);
  await page.locator('.dock-pane').first().waitFor();
  return page;
}
async function prefix(page, key) {
  await page.keyboard.press('Control+b');
  await page.locator('.prefix-hint').waitFor();
  await page.keyboard.press(key);
}
const explorerOf = page => page.locator('.explorer-panel');
const treeRow = (page, name) => explorerOf(page).locator('.tree-row', { has: page.locator(`span:text-is("${name}")`) });
const selected = page => page.locator('.dock-pane.is-selected');
const tabs = page => page.locator('.dock-tab').allInnerTexts().then(names => names.map(name => name.trim()));
/** Waits for `file` to read `content` on disk. */
const onDisk = (path, content, what) => until(async () => (await read(path).catch(() => null)) === content, 10000, what ?? `${path} to be saved`);

// ------------------------------------------------------------------ files

test('a file opened from the tree is edited and saved with mod+S', async () => {
  const { root, workspace } = await repository({ 'README.md': '# Fixture\n', 'src/app.js': 'export const x = 1;\n' });
  const page = await open(editorPane(workspace, 'README.md'));
  await treeRow(page, 'src').click();
  await treeRow(page, 'app.js').click();
  const editor = selected(page).locator('textarea.code-editor');
  await until(async () => (await editor.count()) && (await editor.inputValue()) === 'export const x = 1;\n', 10000, 'app.js to open');
  assert.deepEqual(await tabs(page), ['README.md', 'app.js']);
  await editor.click(); await page.keyboard.press('ControlOrMeta+End'); await page.keyboard.type('export const y = 2;\n');
  await selected(page).locator('.dirty-indicator').waitFor();
  await page.keyboard.press('ControlOrMeta+s');
  await onDisk(join(root, 'src/app.js'), 'export const x = 1;\nexport const y = 2;\n');
  await selected(page).locator('.inline-success').waitFor();
  assert.equal(await selected(page).locator('.dirty-indicator').count(), 0, 'no longer marked unsaved');
  assert.deepEqual(browser.errors, []);
});

test('an open file follows a change on disk, and a draft is kept rather than overwritten', async () => {
  const { root, workspace } = await repository({ 'notes.txt': 'first\n' });
  const page = await open(editorPane(workspace, 'notes.txt'));
  const editor = selected(page).locator('textarea.code-editor');
  await until(async () => (await editor.inputValue()) === 'first\n', 10000, 'file to load');
  // Untouched: the editor takes the new version by itself.
  await writeFile(join(root, 'notes.txt'), 'second\n');
  await until(async () => (await editor.inputValue()) === 'second\n', 10000, 'editor to follow the disk');
  // With a draft: the draft stays, a warning says why, and saving does not overwrite the host.
  await editor.click(); await page.keyboard.press('ControlOrMeta+End'); await page.keyboard.type('mine');
  await writeFile(join(root, 'notes.txt'), 'third\n');
  await selected(page).locator('.inline-error').waitFor();
  assert.equal(await editor.inputValue(), 'second\nmine');
  await page.keyboard.press('ControlOrMeta+s');
  await page.waitForTimeout(500);
  assert.equal(await read(join(root, 'notes.txt')), 'third\n', 'the host version is not overwritten');
  assert.equal(await editor.inputValue(), 'second\nmine', 'the draft is kept');
  // The refused save is the 409 the warning describes.
  assert.deepEqual(browser.errors.filter(error => !/^409 PUT \/api\/workspaces\/[^/]+\/file$/.test(error)), []);
});

test('the file panel creates, renames and deletes files', async () => {
  const { root, workspace } = await repository();
  const page = await open(editorPane(workspace, 'README.md'));
  const explorer = explorerOf(page);
  await treeRow(page, 'README.md').waitFor();
  await explorer.getByRole('button', { name: '新建文件', exact: true }).click();
  // A path makes its folder too, and the new file opens to be written in.
  await explorer.locator('.tree-compose input').fill('made/new.txt');
  await explorer.locator('.tree-compose input').press('Enter');
  await treeRow(page, 'new.txt').waitFor();
  assert.ok(await exists(join(root, 'made/new.txt')));
  await until(async () => (await tabs(page)).includes('new.txt'), 5000, 'new file to open');
  // F2 renames in place.
  await treeRow(page, 'new.txt').focus();
  await page.keyboard.press('F2');
  await explorer.locator('.tree-rename input').fill('renamed.txt');
  await explorer.locator('.tree-rename input').press('Enter');
  await treeRow(page, 'renamed.txt').waitFor();
  assert.deepEqual([await exists(join(root, 'made/renamed.txt')), await exists(join(root, 'made/new.txt'))], [true, false]);
  // Deleting asks first.
  await treeRow(page, 'renamed.txt').click({ button: 'right' });
  await page.getByRole('menuitem', { name: '删除…' }).click();
  await page.getByRole('menuitem', { name: '永久删除' }).click();
  await until(async () => !(await exists(join(root, 'made/renamed.txt'))), 5000, 'file to be deleted');
  await until(async () => (await treeRow(page, 'renamed.txt').count()) === 0, 5000, 'row to go');
  // The tab new.txt opened in stays on its old path (see the rename bug below)
  // and asks for it when the tree changes: that 404 is the bug's, not this test's.
  assert.deepEqual(browser.errors.filter(error => !/^404 GET \/api\/workspaces\/[^/]+\/file$/.test(error)), []);
});

test('a file created on disk appears in the tree without a refresh', async () => {
  const { root, workspace } = await repository();
  const page = await open(editorPane(workspace, 'README.md'));
  await treeRow(page, 'README.md').waitFor();
  await writeFile(join(root, 'agent-wrote.txt'), 'by an agent\n');
  await treeRow(page, 'agent-wrote.txt').waitFor({ timeout: 10000 });
  await rm(join(root, 'agent-wrote.txt'));
  await until(async () => (await treeRow(page, 'agent-wrote.txt').count()) === 0, 10000, 'row to go');
  assert.deepEqual(browser.errors, []);
});

test('Ctrl+B f finds a file among the loaded ones and opens it', async () => {
  // Workspace-wide search needs a real Codex app-server; the fixture has
  // none, so this covers the filter over what the tree has loaded.
  const { workspace } = await repository({ 'README.md': '# Fixture\n', 'guide.md': '# Guide\n', 'other.txt': 'x\n' });
  const page = await open(editorPane(workspace, 'README.md'));
  await treeRow(page, 'guide.md').waitFor();
  await selected(page).locator('textarea.code-editor').click();
  await prefix(page, 'f');
  await until(() => page.evaluate(() => document.activeElement?.hasAttribute('data-explorer-filter')), 5000, 'filter to take focus');
  await page.keyboard.type('guid');
  await until(async () => (await explorerOf(page).locator('.tree-row').allInnerTexts()).map(text => text.split('\n')[0]).join() === 'guide.md', 5000, 'only guide.md to match');
  await treeRow(page, 'guide.md').click();
  await until(async () => (await tabs(page)).includes('guide.md'), 5000, 'guide.md to open');
  assert.equal(await selected(page).locator('textarea.code-editor').inputValue(), '# Guide\n');
  assert.deepEqual(browser.errors, []);
});

test('markdown switches between preview, split and source; images and binaries preview', async () => {
  const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==', 'base64');
  const { root, workspace } = await repository({ 'notes.md': '# Notes\n\n- one\n' });
  await writeFile(join(root, 'pic.png'), png);
  await writeFile(join(root, 'blob.bin'), Buffer.from([0, 1, 2, 3, 255, 0, 0, 7]));
  const page = await open(editorPane(workspace, 'notes.md'));
  const pane = selected(page);
  await pane.locator('.md-modes').waitFor();
  const mode = name => pane.locator('.md-modes button', { hasText: name }).click();
  await mode('双栏');
  await pane.locator('.markdown-preview h1', { hasText: 'Notes' }).waitFor();
  assert.equal(await pane.locator('textarea.code-editor').count(), 1, 'split shows the source too');
  await mode('预览');
  assert.deepEqual([await pane.locator('textarea.code-editor').count(), await pane.locator('.markdown-preview').count()], [0, 1]);
  await mode('源码');
  assert.deepEqual([await pane.locator('textarea.code-editor').count(), await pane.locator('.markdown-preview').count()], [1, 0]);
  await mode('双栏');
  await treeRow(page, 'pic.png').click();
  await selected(page).locator('img').first().waitFor();
  await until(() => selected(page).locator('img').first().evaluate(image => image.complete && image.naturalWidth === 1), 5000, 'image to load');
  await treeRow(page, 'blob.bin').click();
  await until(async () => (await selected(page).locator('.content-footer').innerText()).includes('无法预览'), 5000, 'binary to say no preview');
  assert.deepEqual(browser.errors, []);
});

test('open files come back after a reload, and a closed tab keeps its draft', async () => {
  const { root, workspace } = await repository({ 'README.md': '# Fixture\n', 'notes.md': '# Notes\n' });
  const page = await open(editorPane(workspace, 'README.md'));
  await treeRow(page, 'notes.md').click();
  const editor = selected(page).locator('textarea.code-editor');
  await until(async () => (await tabs(page)).includes('notes.md') && (await editor.inputValue()) === '# Notes\n', 5000, 'notes.md to open');
  await editor.click(); await page.keyboard.press('ControlOrMeta+End'); await page.keyboard.type('draft line');
  // Closing the tab neither asks nor loses the draft: opening the file again shows it.
  await page.locator('.dock-tab', { hasText: 'notes.md' }).locator('button').click();
  await until(async () => !(await tabs(page)).includes('notes.md'), 5000, 'tab to close');
  await treeRow(page, 'notes.md').click();
  await until(async () => (await selected(page).locator('textarea.code-editor').inputValue()) === '# Notes\ndraft line', 5000, 'draft to come back');
  await selected(page).locator('textarea.code-editor').press('ControlOrMeta+s');
  await onDisk(join(root, 'notes.md'), '# Notes\ndraft line');
  await until(async () => JSON.stringify(await browser.api('/canvas/layout')).includes('notes.md'), 10000, 'layout to be saved');
  await page.reload();
  await until(async () => JSON.stringify(await tabs(page)) === JSON.stringify(['README.md', 'notes.md']), 10000, 'both tabs after reload');
  await page.locator('.dock-tab', { hasText: 'notes.md' }).click();
  await until(async () => (await selected(page).locator('textarea.code-editor').inputValue()) === '# Notes\ndraft line', 5000, 'notes.md to load');
  assert.deepEqual(browser.errors, []);
});

// -------------------------------------------------------------------- Git

test('Ctrl+B g lists modified, deleted and new files; stage, unstage and discard work', async () => {
  const { root, git, workspace } = await repository({ 'README.md': '# Fixture\n', 'a.txt': 'one\ntwo\n', 'gone.txt': 'bye\n' });
  await writeFile(join(root, 'a.txt'), 'one\nTWO\n');
  await writeFile(join(root, 'new.txt'), 'fresh\n');
  await rm(join(root, 'gone.txt'));
  const page = await open(editorPane(workspace, 'README.md'));
  await selected(page).locator('textarea.code-editor').click();
  await prefix(page, 'g');
  const pane = page.locator('.git-pane');
  await pane.waitFor();
  await until(async () => (await pane.locator('.git-file').count()) === 3, 5000, 'three changes');
  assert.deepEqual((await pane.locator('.git-file-open').allInnerTexts()).map(text => text.replace(/\s+/g, ' ').trim()), ['M a.txt', 'D gone.txt', '? new.txt']);
  // The first change's diff opens by itself.
  await pane.locator('.diff-view', { hasText: '+TWO' }).waitFor();
  assert.match(await pane.locator('.diff-view').innerText(), /-two/);
  await pane.getByRole('button', { name: '暂存 a.txt', exact: true }).click();
  await until(() => git('status', '--porcelain').startsWith('M  a.txt'), 5000, 'a.txt staged');
  await pane.getByRole('button', { name: '取消暂存 a.txt', exact: true }).click();
  await until(() => git('status', '--porcelain').startsWith(' M a.txt'), 5000, 'a.txt unstaged');
  await pane.getByRole('button', { name: '丢弃 gone.txt 的改动', exact: true }).click();
  await pane.locator('.git-discard-confirm .danger').click();
  await onDisk(join(root, 'gone.txt'), 'bye\n', 'gone.txt to be restored');
  await until(async () => (await pane.locator('.git-file').count()) === 2, 5000, 'two changes left');
  assert.deepEqual(browser.errors, []);
});

test('one hunk of a file is staged on its own', async () => {
  const lines = Array.from({ length: 20 }, (_, index) => String(index + 1));
  const { root, git, workspace } = await repository({ 'a.txt': lines.join('\n') + '\n' });
  await writeFile(join(root, 'a.txt'), lines.map(line => line === '2' ? 'TWO' : line === '19' ? 'NINETEEN' : line).join('\n') + '\n');
  const page = await open(gitPane(workspace));
  const pane = page.locator('.git-pane');
  await until(async () => (await pane.getByRole('button', { name: '暂存片段' }).count()) === 2, 5000, 'two hunks');
  await pane.getByRole('button', { name: '暂存片段' }).first().click();
  await until(() => git('diff', '--cached').includes('+TWO'), 5000, 'first hunk staged');
  assert.ok(!git('diff', '--cached').includes('NINETEEN'), 'the second hunk is not staged');
  assert.ok(git('diff').includes('+NINETEEN'), 'the second hunk stays in the working tree');
  assert.deepEqual(browser.errors, []);
});

test('mod+Enter commits what is staged, and History shows it', async () => {
  const { root, git, workspace } = await repository();
  await writeFile(join(root, 'README.md'), '# Changed\n');
  await writeFile(join(root, 'added.txt'), 'new\n');
  const page = await open(gitPane(workspace));
  const pane = page.locator('.git-pane');
  await until(async () => (await pane.locator('.git-file').count()) === 2, 5000, 'two changes');
  const commit = pane.locator('.commit-form button.primary-button').first();
  await pane.locator('.commit-form textarea').fill('Second commit');
  assert.ok(await commit.isDisabled(), 'nothing staged, nothing to commit');
  await pane.getByRole('button', { name: '全部暂存', exact: true }).click();
  await until(async () => !(await commit.isDisabled()), 5000, 'commit to be possible');
  await pane.locator('.commit-form textarea').press('ControlOrMeta+Enter');
  await until(() => git('log', '-1', '--format=%s').trim() === 'Second commit', 5000, 'commit');
  assert.equal(git('status', '--porcelain'), '');
  await pane.locator('.git-message', { hasText: 'Second commit' }).waitFor();
  assert.equal(await pane.locator('.commit-form textarea').inputValue(), '', 'message box emptied');
  await pane.getByRole('tab', { name: '历史' }).click();
  await pane.getByText('Second commit').first().waitFor();
  assert.deepEqual(browser.errors, []);
});

test('a merge that conflicts is resolved and continued from the Git pane', async () => {
  const { root, git, workspace } = await repository({ 'c.txt': 'base\n' });
  git('checkout', '-qb', 'other'); await writeFile(join(root, 'c.txt'), 'theirs\n'); git('commit', '-qam', 'theirs');
  git('checkout', '-q', 'main'); await writeFile(join(root, 'c.txt'), 'ours\n'); git('commit', '-qam', 'ours');
  const page = await open(gitPane(workspace));
  const pane = page.locator('.git-pane');
  await pane.getByRole('button', { name: '更多 Git 操作' }).click();
  await page.getByRole('menuitem', { name: '将分支合并到当前分支…' }).click();
  await page.locator('.git-prompt button.primary-button').click();
  await pane.locator('.git-conflicts .git-file', { hasText: 'c.txt' }).waitFor();
  await pane.locator('.git-operation').waitFor();
  await pane.locator('.diff-view', { hasText: '<<<<<<<' }).waitFor();
  await pane.locator('.git-conflicts .git-file').click({ button: 'right' });
  await page.getByRole('menuitem', { name: '保留当前（ours）' }).click();
  await onDisk(join(root, 'c.txt'), 'ours\n');
  const proceed = pane.locator('.git-operation button', { hasText: '继续' });
  await until(async () => !(await proceed.isDisabled()), 5000, 'continue to be possible');
  await proceed.click();
  await until(() => git('log', '-1', '--format=%s').trim() === "Merge branch 'other'", 5000, 'merge commit');
  await until(async () => (await pane.locator('.git-operation').count()) === 0, 5000, 'operation banner to go');
  assert.deepEqual(browser.errors, []);
});

test('a directory that is not a repository can be made one from the Git pane', async () => {
  const plain = join(gateway.home, `plain-${++repos}`);
  await mkdir(plain); await writeFile(join(plain, 'x.txt'), 'x\n');
  const workspace = await browser.api('/workspaces', { name: 'Plain', root_path: plain });
  const page = await open(gitPane(workspace));
  const pane = page.locator('.git-pane');
  await pane.getByRole('button', { name: '初始化仓库' }).click();
  await until(async () => (await pane.locator('.git-file', { hasText: 'x.txt' }).count()) === 1, 5000, 'x.txt as a new file');
  assert.ok(await exists(join(plain, '.git')));
  // Before it existed the pane read a status that could not be had: those are the expected 4xx.
  assert.deepEqual(browser.errors.filter(error => !/^40[09] GET \/api\/workspaces\/[^/]+\/git\/(status|remotes|stashes)$/.test(error)), []);
});

// --------------------------------------------------------------- branches

test('the branch control creates and switches branches, and every place shows the same one', async () => {
  const { git, workspace } = await repository();
  const page = await open(gitPane(workspace));
  const pane = page.locator('.git-pane');
  const shown = async () => [
    (await page.locator('footer.statusbar .ws-branch').innerText()).trim(),
    (await pane.locator('.git-branch-bar .ws-branch').innerText()).trim(),
    (await page.locator('.workspace-sidebar .ws-branch').last().innerText()).trim(),
  ];
  await until(async () => (await page.locator('footer.statusbar .ws-branch').count()) === 1, 10000, 'status bar branch');
  await page.locator('footer.statusbar .ws-branch').click();
  const panel = page.locator('.ws-branch-panel');
  await panel.getByText('新分支…').click();
  await panel.locator('.ws-create input').fill('feature/one');
  await panel.locator('.ws-create input').press('Enter');
  await until(() => git('branch', '--show-current').trim() === 'feature/one', 5000, 'switch to the new branch');
  await until(async () => (await shown()).every(name => name === 'feature/one'), 10000, 'every place to show feature/one');
  await pane.locator('.git-branch-bar .ws-branch').click();
  await panel.locator('.ws-row-main', { hasText: 'main' }).click();
  await until(() => git('branch', '--show-current').trim() === 'main', 5000, 'switch back');
  await until(async () => (await shown()).every(name => name === 'main'), 10000, 'every place to show main');
  // A checkout made outside the app is followed too.
  git('checkout', '-q', 'feature/one');
  await until(async () => (await shown()).every(name => name === 'feature/one'), 15000, 'an outside checkout to show');
  assert.deepEqual(browser.errors, []);
});

test('switching the workspace branch under a running session asks first, then ends it', async () => {
  const { git, workspace } = await repository();
  git('branch', 'feature/a');
  const session = await chatSession(workspace);
  const page = await open(paneFor('chat', session));
  await sendMessage(page, 'chat', 'hello');
  await page.locator('[data-pane-id="chat"]').getByText('Hello 🦊').first().waitFor();
  await until(async () => (await page.locator('footer.statusbar .ws-branch').count()) === 1, 10000, 'status bar branch');
  await page.locator('footer.statusbar .ws-branch').click();
  const panel = page.locator('.ws-branch-panel');
  await panel.locator('.ws-row-main', { hasText: 'feature/a' }).click();
  await panel.locator('.ws-blocking').waitFor();
  assert.equal(git('branch', '--show-current').trim(), 'main', 'nothing switched yet');
  await panel.getByRole('button', { name: '结束并切换' }).click();
  await until(() => git('branch', '--show-current').trim() === 'feature/a', 10000, 'the switch');
  assert.notEqual((await browser.api(`/sessions/${session.id}`)).status, 'running');
  // The question was asked through the 409 the first switch got.
  assert.deepEqual(browser.errors.filter(error => !/^409 POST \/api\/workspaces\/[^/]+\/git\/switch$/.test(error)), []);
});

test('a session moves into its own worktree from its branch chip, works there, and moves back', async () => {
  const { root, git, workspace } = await repository();
  const session = await chatSession(workspace);
  const page = await open(paneFor('chat', session));
  const chat = page.locator('[data-pane-id="chat"]');
  await sendMessage(page, 'chat', 'hello');
  await chat.getByText('Hello 🦊').first().waitFor();
  const chip = chat.locator('.chat-branch');
  await until(async () => (await chip.innerText()).trim() === 'main', 5000, 'chip to say main');
  // A new branch, typed in the chip; with a conversation under way it asks first.
  await chip.locator('summary').click();
  await page.locator('.chat-branch-body input').fill('feat/wt');
  await page.locator('.chat-branch-body input').press('Enter');
  await chat.locator('.chat-endpoint-confirm button').first().click();
  const tree = `${root}.worktrees/feat-wt`;
  await until(async () => (await browser.api(`/sessions/${session.id}`)).checkout_path?.endsWith('.worktrees/feat-wt'), 10000, 'the move');
  assert.equal(git('branch', '--show-current').trim(), 'main', 'the workspace directory stays on main');
  assert.equal(execFileSync('git', ['branch', '--show-current'], { cwd: tree, encoding: 'utf8' }).trim(), 'feat/wt');
  await until(async () => (await chip.innerText()).trim() === 'feat/wt', 5000, 'chip to say feat/wt');
  // The file panel follows the session into its worktree.
  await writeFile(join(tree, 'only-here.txt'), 'x\n');
  await chat.locator('textarea[data-pane-input]').click();
  await treeRow(page, 'only-here.txt').waitFor({ timeout: 10000 });
  await sendMessage(page, 'chat', 'after the move');
  await chat.getByText('Hello 🦊').nth(1).waitFor();
  assert.equal(await chat.locator('.chat-alert').count(), 0);
  // And back to the workspace directory.
  await chip.locator('summary').click();
  await page.locator('.chat-branch-body button', { hasText: 'main' }).click();
  await chat.locator('.chat-endpoint-confirm button').first().click();
  await until(async () => (await browser.api(`/sessions/${session.id}`)).checkout_path === null, 10000, 'the move back');
  await until(async () => (await chip.innerText()).trim() === 'main', 5000, 'chip to say main again');
  assert.deepEqual(browser.errors, []);
});

test('Git changes opened from a session in a worktree commit on its branch only', async () => {
  const { git, workspace } = await repository();
  const session = await chatSession(workspace);
  const moved = await browser.api(`/sessions/${session.id}/checkout`, { branch: 'feat/x', create: true });
  const page = await open(paneFor('chat', session));
  await writeFile(join(moved.checkout_path, 'wt-only.txt'), 'in the worktree\n');
  await page.locator('[data-pane-id="chat"] textarea[data-pane-input]').click();
  await prefix(page, 'g');
  const pane = selected(page).locator('.git-pane');
  await pane.waitFor();
  await until(async () => (await pane.locator('.git-branch-bar').innerText()).includes('feat/x'), 5000, 'the worktree branch');
  await until(async () => (await pane.locator('.git-file').allInnerTexts()).some(text => text.includes('wt-only.txt')), 5000, 'the worktree change');
  await pane.locator('.commit-form textarea').fill('wt commit');
  await pane.getByRole('button', { name: '更多提交操作' }).click();
  await page.getByRole('menuitem', { name: '全部暂存并提交' }).click();
  const log = (cwd = undefined) => execFileSync('git', ['log', '-1', '--format=%s'], { cwd: cwd ?? moved.checkout_path, encoding: 'utf8' }).trim();
  await until(() => log() === 'wt commit', 5000, 'commit in the worktree');
  assert.equal(git('log', '-1', '--format=%s').trim(), 'Initial', 'main is untouched');
  assert.equal(git('status', '--porcelain'), '');
  assert.deepEqual(browser.errors, []);
});

// ------------------------------------------------------------------- bugs

// Regression (found by the exploratory run): Esc is only heard inside the teleported panel, but opening it leaves focus on the chip, so Esc does nothing
test('Esc closes the workspace branch menu', async () => {
  const { workspace } = await repository();
  const page = await open(gitPane(workspace));
  await until(async () => (await page.locator('footer.statusbar .ws-branch').count()) === 1, 10000, 'status bar branch');
  await page.locator('footer.statusbar .ws-branch').click();
  await page.locator('.ws-branch-panel').waitFor();
  await page.keyboard.press('Escape');
  await page.waitForTimeout(300);
  assert.equal(await page.locator('.ws-branch-panel').count(), 0);
});

// Regression (found by the exploratory run): a just-created session branch counts as merged; "Clean up merged" deletes it with its worktree and moves the session back
test('a session\'s new branch is not offered for clean-up as merged', async () => {
  const { git, workspace } = await repository();
  const session = await chatSession(workspace);
  const moved = await browser.api(`/sessions/${session.id}/checkout`, { branch: 'feat/fresh', create: true });
  const page = await open(gitPane(workspace));
  await page.locator('.git-pane .ws-branch').click();
  const panel = page.locator('.ws-branch-panel');
  await panel.locator('.ws-row', { hasText: 'feat/fresh' }).waitFor();
  // What can be cleaned up is read after the list; give it time to arrive.
  await until(async () => (await panel.innerText()).includes('可清理'), 5000).catch(() => {});
  const offered = await panel.getByRole('button', { name: '清理已合并的' }).count();
  if (offered) {
    await panel.getByRole('button', { name: '清理已合并的' }).click();
    await panel.locator('.ws-confirm .danger').click();
    await page.waitForTimeout(1500);
  }
  assert.equal(offered, 0, 'the session branch is listed as cleanable');
  assert.ok(await exists(moved.checkout_path), 'the worktree is gone');
  assert.match(git('branch'), /feat\/fresh/);
});

// Regression (found by the exploratory run): the tab stays on the old path, still saying "saved · in sync"; saving then reports a host change
test('renaming an open file moves its tab with it', async () => {
  const { workspace } = await repository({ 'README.md': '# Fixture\n', 'old.txt': 'x\n' });
  const page = await open(editorPane(workspace, 'old.txt'));
  await treeRow(page, 'old.txt').focus();
  await page.keyboard.press('F2');
  await explorerOf(page).locator('.tree-rename input').fill('new.txt');
  await explorerOf(page).locator('.tree-rename input').press('Enter');
  await treeRow(page, 'new.txt').waitFor();
  await page.waitForTimeout(1500);
  assert.deepEqual(await tabs(page), ['new.txt']);
});

// Regression (found by the exploratory run): until the first status arrives the status bar reads "Git 不可用" (and "分离的 HEAD" right after a session moves into a worktree)
test('the status bar never says Git is unavailable for a repository', async () => {
  const { workspace } = await repository();
  await setLayout(browser.api, gitPane(workspace));
  const page = await browser.newPage();
  await page.addInitScript(() => {
    window.seen = new Set();
    new MutationObserver(() => { const bar = document.querySelector('footer.statusbar'); if (bar) window.seen.add(bar.innerText); })
      .observe(document, { subtree: true, childList: true, characterData: true });
  });
  await page.goto(gateway.base);
  await until(async () => (await page.locator('footer.statusbar .ws-branch').count()) === 1, 10000, 'status bar branch');
  const seen = await page.evaluate(() => [...window.seen]);
  assert.ok(!seen.some(text => text.includes('Git 不可用')), 'said Git is unavailable');
});
