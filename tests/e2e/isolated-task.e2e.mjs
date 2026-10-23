// An isolated task from end to end: made from the new-session dialog, its
// worktree prepared as the workspace says, its work reviewed in the branch
// chip's dialog and merged back, leaving no worktree or branch behind. The
// page runs in zh-CN, so labels are matched in Chinese.
import { after, afterEach, before, test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { openBrowser, setLayout, startGateway, until } from './harness.mjs';

let gateway, browser;
before(async () => { gateway = await startGateway(); browser = await openBrowser(gateway); });
afterEach(() => browser.reset());
after(async () => { await browser?.close(); await gateway?.stop(); });

const git = (cwd, ...args) => execFileSync('git', ['-c', 'user.name=e2e', '-c', 'user.email=e2e@example.invalid', ...args], { cwd, encoding: 'utf8' }).trim();

test('an isolated task is prepared, reviewed and merged back from the page', async () => {
  // What a worktree lacks: an ignored .env, prepared by copying it and running a command.
  await writeFile(join(gateway.workspace, '.gitignore'), '.env\n');
  git(gateway.workspace, 'add', '.gitignore'); git(gateway.workspace, 'commit', '-qm', 'Ignore .env');
  await writeFile(join(gateway.workspace, '.env'), 'TOKEN=local\n');
  const workspace = await browser.api('/workspaces', { name: 'E2E', root_path: gateway.workspace });
  await browser.api(`/workspaces/${workspace.id}`, { worktree_setup: { copy: ['.env'], command: 'echo "$TENDLY_BRANCH" > prepared.txt' } }, 'PATCH');
  await setLayout(browser.api, { type: 'pane', id: 'blank', kind: 'agent_chat', title: 'Agent session' });

  const page = await browser.newPage();
  await page.goto(gateway.base);
  await page.locator('section.workspace-group[aria-label="E2E"]').waitFor();
  await page.locator('.new-session-more > summary').click();
  await page.locator('.new-session-menu button', { hasText: '自定义新建会话' }).click();
  const dialog = page.locator('section.dialog[aria-label="新建会话"]');
  await dialog.waitFor();
  await dialog.locator('.session-kinds label', { hasText: 'Codex' }).click();
  await dialog.locator('input[maxlength="120"]').fill('Fix the login page');
  await dialog.locator('.session-place-options label', { hasText: '隔离任务' }).click();
  // The form names the branch before it exists, from the base it will start at, and says how it is prepared.
  await dialog.getByText('tendly/fix-the-login-page').first().waitFor();
  await until(async () => await dialog.locator('.session-place-fields select').inputValue() === 'main', 5000, 'the base to default to the current branch');
  assert.match(await dialog.locator('.session-place-setup').innerText(), /\.env/);
  // A branch checked out already is refused before Create.
  await dialog.locator('.session-place-fields input').fill('main');
  await dialog.locator('.session-place-error').waitFor();
  assert.ok(await dialog.locator('.dialog-actions .primary-button').isDisabled());
  await dialog.locator('.session-place-fields input').fill('');
  const known = new Set((await browser.api('/sessions')).map(session => session.id));
  await dialog.locator('.dialog-actions .primary-button').click();
  let session;
  // Created, then moved into its worktree: the dialog opens it only after both.
  await until(async () => { session = (await browser.api('/sessions')).find(item => !known.has(item.id)); return !!session?.checkout_base; }, 10000, 'the new session in its worktree');
  assert.equal(session.checkout_branch, 'tendly/fix-the-login-page');
  assert.equal(session.checkout_base, 'main');
  const tree = session.checkout_path;
  assert.ok(tree.endsWith('.worktrees/tendly-fix-the-login-page'), tree);

  const pane = page.locator(`[data-pane-id="session-${session.id}"]`);
  await pane.locator('.chat-setup.completed').waitFor({ timeout: 20000 });
  assert.match(await pane.locator('.chat-setup').innerText(), /worktree 已准备好/);
  assert.equal(await readFile(join(tree, '.env'), 'utf8'), 'TOKEN=local\n');
  assert.equal((await readFile(join(tree, 'prepared.txt'), 'utf8')).trim(), 'tendly/fix-the-login-page');

  // The agent's work, committed on the task's branch.
  await writeFile(join(tree, 'login.txt'), 'fixed\n');
  git(tree, 'add', 'login.txt'); git(tree, 'commit', '-qm', 'Fix the login page');

  await pane.locator('.chat-branch > summary').click();
  await pane.locator('.chat-branch-review').click();
  const review = page.locator('section.dialog[aria-label="审阅任务"]');
  await review.waitFor();
  await review.locator('.git-commit-file', { hasText: 'login.txt' }).waitFor();
  await review.locator('.diff-lines').getByText('fixed').waitFor();
  // prepared.txt was made by the setup and never committed: listed, and it holds the merge back.
  assert.ok(await review.locator('.git-commit-file', { hasText: 'prepared.txt' }).count());
  assert.ok(await review.getByRole('button', { name: '合并到 main' }).isDisabled());
  git(tree, 'add', 'prepared.txt'); git(tree, 'commit', '-qm', 'Keep what the setup wrote');
  await page.keyboard.press('Escape');
  await pane.locator('.chat-branch > summary').click();
  await pane.locator('.chat-branch-review').click();
  await review.waitFor();
  await review.getByRole('button', { name: '合并到 main' }).click();
  await review.waitFor({ state: 'detached', timeout: 15000 });

  const done = (await browser.api('/sessions')).find(item => item.id === session.id);
  assert.ok(done.archived_at, 'the finished task is archived');
  assert.equal(done.checkout_path ?? null, null);
  assert.equal(await readFile(join(gateway.workspace, 'login.txt'), 'utf8'), 'fixed\n');
  assert.ok(!existsSync(tree), 'its worktree is gone');
  assert.equal(git(gateway.workspace, 'branch', '--list', 'tendly/*'), '');
  assert.deepEqual(browser.errors, []);
});
