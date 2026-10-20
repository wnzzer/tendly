import { test } from 'node:test';
import assert from 'node:assert/strict';
import { placeKey, placeOf, placeUrl } from './checkout-place.ts';
import { changesPane, filePane, paneCheckout } from './pane-context.ts';

test('a place is a workspace directory, or a worktree of it, and reads back the same', () => {
  assert.equal(placeKey('ws'), 'ws');
  assert.equal(placeKey('ws', null), 'ws');
  assert.deepEqual(placeOf(placeKey('ws', '/repo.worktrees/feat')), { workspaceId: 'ws', checkout: '/repo.worktrees/feat' });
  assert.deepEqual(placeOf('ws'), { workspaceId: 'ws', checkout: null });
});

test('a place in a worktree names it on every request, after any query of its own', () => {
  assert.equal(placeUrl('ws', '/git/status'), '/workspaces/ws/git/status');
  const tree = placeKey('ws', '/repo.worktrees/a b');
  assert.equal(placeUrl(tree, '/git/status'), '/workspaces/ws/git/status?checkout=%2Frepo.worktrees%2Fa%20b');
  assert.equal(placeUrl(tree, '/file?path=x.ts'), '/workspaces/ws/file?path=x.ts&checkout=%2Frepo.worktrees%2Fa%20b');
});

test('the same file in a worktree is another pane, and the pane remembers its checkout', () => {
  const home = filePane('ws', 'src/a.ts'), tree = filePane('ws', 'src/a.ts', '/repo.worktrees/feat');
  assert.notEqual(home.id, tree.id);
  assert.equal(paneCheckout(home, []), null);
  assert.equal(paneCheckout(tree, []), '/repo.worktrees/feat');
  assert.notEqual(changesPane('ws').id, changesPane('ws', '/repo.worktrees/feat').id);
  assert.equal(paneCheckout(changesPane('ws', '/repo.worktrees/feat'), []), '/repo.worktrees/feat');
});

test('a session pane looks where its session works', () => {
  const session = { id: 's', workspace_id: 'ws', provider: 'codex', checkout_path: '/repo.worktrees/feat' };
  const pane = { type: 'pane', id: 'session-s', kind: 'agent_chat', title: 's', metadata: { workspace_id: 'ws', session_id: 's' } };
  assert.equal(paneCheckout(pane, [session]), '/repo.worktrees/feat');
  assert.equal(paneCheckout(pane, [{ ...session, checkout_path: null }]), null);
});
