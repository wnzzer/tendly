import { test } from 'node:test';
import assert from 'node:assert/strict';
import { backTargetFor, draggedSidebarWidth, hasUnsavedWork, rememberPane, resolveReference, storedSidebarWidth } from './app-shell.ts';

const app = { id: 'app', root_path: '/work/app' };
const lib = { id: 'lib', root_path: '/work/app/vendor/lib' };
const win = { id: 'win', root_path: 'C:\\Users\\me\\repo' };
const all = [app, lib, win];

test('a path inside the workspace or its checkout opens relative to it', () => {
  assert.deepEqual(resolveReference({ path: '/work/app/src/main.ts' }, app, all), { kind: 'workspace', workspaceId: 'app', checkout: null, path: 'src/main.ts' });
  assert.deepEqual(resolveReference({ path: '/work/app.worktrees/feat/a.ts', checkout: '/work/app.worktrees/feat' }, app, all), { kind: 'workspace', workspaceId: 'app', checkout: '/work/app.worktrees/feat', path: 'a.ts' });
  // A relative path or a bare name from a session in a worktree is that worktree's.
  assert.deepEqual(resolveReference({ path: 'src/a.ts', checkout: '/work/app.worktrees/feat' }, app, all), { kind: 'workspace', workspaceId: 'app', checkout: '/work/app.worktrees/feat', path: 'src/a.ts' });
  assert.deepEqual(resolveReference({ path: 'a.ts', checkout: '/work/app.worktrees/feat' }, app, all), { kind: 'search', workspaceId: 'app', checkout: '/work/app.worktrees/feat', name: 'a.ts' });
  // An absolute path into the workspace directory opens there, even from a worktree session.
  assert.deepEqual(resolveReference({ path: '/work/app/b.ts', checkout: '/work/app.worktrees/feat' }, app, all), { kind: 'workspace', workspaceId: 'app', checkout: null, path: 'b.ts' });
  assert.deepEqual(resolveReference({ path: './src/a.ts' }, app, all), { kind: 'workspace', workspaceId: 'app', checkout: null, path: 'src/a.ts' });
});

test('Windows paths match whatever their slashes and case', () => {
  assert.deepEqual(resolveReference({ path: 'c:/users/ME/repo/Src/A.ts' }, win, all), { kind: 'workspace', workspaceId: 'win', checkout: null, path: 'Src/A.ts' });
});

test('a path in another workspace opens there, in the most specific one', () => {
  const elsewhere = { id: 'docs', root_path: '/srv/docs' };
  assert.deepEqual(resolveReference({ path: '/work/app/vendor/lib/x.rs' }, elsewhere, all), { kind: 'workspace', workspaceId: 'lib', checkout: null, path: 'x.rs' });
  assert.deepEqual(resolveReference({ path: '/work/app/readme.md' }, elsewhere, all), { kind: 'workspace', workspaceId: 'app', checkout: null, path: 'readme.md' });
});

test('a path in no workspace is shown read-only, and a bare name is looked up', () => {
  assert.deepEqual(resolveReference({ path: '/etc/hosts' }, app, all), { kind: 'host', path: '/etc/hosts' });
  assert.deepEqual(resolveReference({ path: '/work/apple/x' }, app, all), { kind: 'host', path: '/work/apple/x' }, 'a root is a whole directory, not a prefix');
  assert.deepEqual(resolveReference({ path: 'main.ts' }, app, all), { kind: 'search', workspaceId: 'app', checkout: null, name: 'main.ts' });
});

test('Back on a phone goes to the latest other pane still open, from a pushed view only', () => {
  const history = rememberPane(rememberPane(rememberPane([], 'chat', 'files'), 'files', 'chat'), 'chat', 'diff');
  assert.deepEqual(history, ['files', 'chat'], 'each pane once, most recent last');
  assert.equal(backTargetFor({ id: 'diff', kind: 'git_diff' }, history, new Set(['chat', 'files', 'diff'])), 'chat');
  assert.equal(backTargetFor({ id: 'diff', kind: 'git_diff' }, history, new Set(['files', 'diff'])), 'files', 'a closed pane is skipped');
  assert.equal(backTargetFor({ id: 'chat', kind: 'agent_chat' }, history, new Set(['chat', 'files'])), undefined, 'a conversation is not a pushed view');
  assert.equal(rememberPane(Array.from({ length: 20 }, (_, i) => `p${i}`), 'new', undefined).length, 20, 'at most twenty');
});

test('the workspace column keeps within its bounds', () => {
  assert.equal(storedSidebarWidth('240'), 240);
  for (const raw of ['183', '381', '', null, 'wide']) assert.equal(storedSidebarWidth(raw), undefined, String(raw));
  assert.equal(draggedSidebarWidth(500, 100), 380);
  assert.equal(draggedSidebarWidth(150, 100), 184);
  assert.equal(draggedSidebarWidth(350.6, 100), 251);
});

test('leaving is held back while anything would be lost', () => {
  const clean = { fileDrafts: false, gitDrafts: false, layoutPending: false, layoutStatus: 'Saved' };
  assert.equal(hasUnsavedWork(clean), false);
  assert.equal(hasUnsavedWork({ ...clean, layoutStatus: 'Saved in this browser' }), false);
  for (const change of [{ fileDrafts: true }, { gitDrafts: true }, { layoutPending: true }, { layoutStatus: 'Saving…' }, { layoutStatus: 'Memory only' }, { layoutStatus: 'Save conflict' }]) {
    assert.equal(hasUnsavedWork({ ...clean, ...change }), true, JSON.stringify(change));
  }
});
