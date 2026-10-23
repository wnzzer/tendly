import { after, before, test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';
import vue from '@vitejs/plugin-vue';
import { createSSRApp } from 'vue';
import { renderToString } from 'vue/server-renderer';

let server, WorkspacePane;
before(async () => {
  server = await createServer({ configFile: false, root: fileURLToPath(new URL('../../', import.meta.url)), plugins: [vue()], server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom', optimizeDeps: { noDiscovery: true } });
  WorkspacePane = (await server.ssrLoadModule('/src/features/WorkspacePane.vue')).default;
});
after(async () => { await server?.close(); });

const workspace = { id: 'w1', name: 'tendly', root_path: '/srv/tendly' };
const render = pane => renderToString(createSSRApp(WorkspacePane, { pane, workspaces: [workspace], sessions: [], profiles: [], gitRefresh: {} }));

test('a draft tab a saved layout still holds keeps its form in the tab', async () => {
  const html = await render({ type: 'pane', id: 'draft-1', kind: 'agent_chat', title: 'New session', metadata: { workspace_id: 'w1', draft: true, provider: 'codex' } });
  assert.match(html, /class="session-draft"/);
  assert.match(html, /<form class="form-stack"/);
  assert.doesNotMatch(html, /modal-backdrop/);
  assert.match(html, /<input checked type="radio" name="session-provider" value="codex"/);
});

test('an unbound agent pane without the draft mark keeps its old picker', async () => {
  const html = await render({ type: 'pane', id: 'p1', kind: 'agent_chat', metadata: { workspace_id: 'w1' } });
  assert.doesNotMatch(html, /session-draft/);
});

test('every right-click menu is the shared ContextMenu, with no hand-rolled backdrop left', () => {
  const root = fileURLToPath(new URL('../', import.meta.url));
  const files = dir => readdirSync(dir, { withFileTypes: true }).flatMap(entry => entry.isDirectory() ? files(join(dir, entry.name)) : entry.name.endsWith('.vue') ? [join(dir, entry.name)] : []);
  const offenders = files(root).filter(file => !file.endsWith('ContextMenu.vue') && /class="[^"]*menu-backdrop/.test(readFileSync(file, 'utf8')));
  assert.deepEqual(offenders, []);
});
