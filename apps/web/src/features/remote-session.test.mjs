import test from 'node:test';
import assert from 'node:assert/strict';
import { rememberRemote, routed, routedUrl, assetUrl, sessionPath } from './api.ts';
import { remotePane } from './remote-session.ts';
import { acceptsScopedPane, reusablePane } from './pane-context.ts';

test("only what belongs to a paired machine goes there; everything else stays as it was", () => {
  assert.equal(routed('/sessions/mine/conversation'), '/sessions/mine/conversation');
  rememberRemote('m1', ['s-there', 'w-there', 'p-there']);
  assert.equal(routed(`${sessionPath('s-there')}/conversation`), '/machines/m1/sessions/s-there/conversation');
  assert.equal(routed('/workspaces/w-there/attachments?name=a'), '/machines/m1/workspaces/w-there/attachments?name=a');
  assert.equal(routed('/endpoint-profiles/p-there/models'), '/machines/m1/endpoint-profiles/p-there/models');
  assert.equal(routed('/sessions/mine/stop'), '/sessions/mine/stop');
  assert.equal(routed('/peers/sessions'), '/peers/sessions');
  assert.equal(routedUrl('/api/sessions/s-there/chat/ws'), '/api/machines/m1/sessions/s-there/chat/ws');
  assert.equal(routedUrl('/api/sessions/mine/chat/ws'), '/api/sessions/mine/chat/ws');
  assert.equal(assetUrl('w-there', 'a.png'), '/api/machines/m1/workspaces/w-there/asset?path=a.png');
  assert.equal(assetUrl('w-mine', 'a.png'), '/api/workspaces/w-mine/asset?path=a.png');
});

test("a paired machine's tab is never taken over by another session", () => {
  const pane = remotePane({ id: 'm', name: 'box' }, { id: 's', title: 'work', provider: 'terminal' });
  assert.equal(pane.kind, 'terminal');
  assert.equal(pane.metadata.session_id, undefined);
  assert.equal(acceptsScopedPane(pane, [], []), true);
  assert.equal(acceptsScopedPane({ type: 'pane', id: 'n', kind: 'terminal', metadata: {} }, [], []), false);
  assert.equal(reusablePane([pane], { type: 'pane', id: 'n', kind: 'terminal', metadata: {} }), undefined);
});
