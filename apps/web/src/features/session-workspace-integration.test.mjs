import { after, afterEach, before, test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';
import vue from '@vitejs/plugin-vue';
import { createSSRApp, reactive } from 'vue';
import { renderToString } from 'vue/server-renderer';
import { paneSession, paneWorkspace, sessionPane } from './pane-context.ts';
import { workspaceSessionPane, groupWorkspaces } from './workspace-groups.ts';
import {
  applyPreset, closePane, cloneNode, dockPane, flattenPanes,
  projectLayout, restoreCollapsed, splitPane, validateLayout,
} from '../layout/layout-engine.ts';

let server;
let CreateSessionDialog;
let ChatSessionPane;
let WorkspacePane;
let setBackendCapabilities;

const workspace = (id, name = id) => ({ id, name, root_path: `/projects/${id}`, created_at: 'now' });
const session = (id, workspace_id, provider = 'codex', interaction_mode = 'pty') => ({
  id, workspace_id, provider, interaction_mode, title: `Session ${id}`, status: 'stopped',
  created_at: 'now', updated_at: 'now', endpoint_profile_id: null, provider_session_id: null,
  configuration_revision: 0, native_source_id: null, endpoint_snapshot: null, error: null,
});

before(async () => {
  server = await createServer({
    configFile: false,
    root: fileURLToPath(new URL('../../', import.meta.url)),
    plugins: [vue()],
    server: { middlewareMode: true, hmr: false, ws: false },
    appType: 'custom',
    optimizeDeps: { noDiscovery: true },
  });
  ({ setBackendCapabilities } = await server.ssrLoadModule('/src/features/backend-capabilities.ts'));
  CreateSessionDialog = (await server.ssrLoadModule('/src/features/CreateSessionDialog.vue')).default;
  ChatSessionPane = (await server.ssrLoadModule('/src/features/ChatSessionPane.vue')).default;
  WorkspacePane = (await server.ssrLoadModule('/src/features/WorkspacePane.vue')).default;
});

after(async () => { await server?.close(); });
afterEach(() => { setBackendCapabilities({ ok: false }); });

test('session panes keep one global session identity while the selected workspace changes', () => {
  const alpha = workspace('alpha', 'Frontend');
  const beta = workspace('beta', 'Backend');
  const shared = session('shared-session', alpha.id, 'claude_code');
  const sessions = [shared, session('beta-session', beta.id, 'codex')];

  const fromAlpha = sessionPane(shared);
  const fromSidebarSelection = workspaceSessionPane(shared);
  assert.equal(fromAlpha.id, 'session-shared-session');
  assert.equal(fromAlpha.id, fromSidebarSelection.id);
  assert.deepEqual(fromAlpha.metadata, {
    workspace_id: 'alpha', session_id: 'shared-session', provider: 'claude_code',
  });

  // Selecting another sidebar group changes only the group projection; it
  // cannot rebind an already-open session pane to that workspace.
  assert.deepEqual(groupWorkspaces([alpha, beta], sessions, { selectedWorkspaceId: 'beta' }).map(g => g.workspace.id), ['alpha', 'beta']);
  assert.equal(paneWorkspace(fromAlpha, [beta, alpha], sessions)?.id, 'alpha');
  assert.equal(paneSession(fromAlpha, sessions)?.id, 'shared-session');
  assert.equal(paneWorkspace({ ...fromAlpha, metadata: { ...fromAlpha.metadata, workspace_id: 'beta' } }, [alpha, beta], sessions), undefined);
});

test('opening or docking the same session repeatedly keeps one layout slot', () => {
  const first = sessionPane(session('dock-once', 'alpha', 'codex'));
  const placeholder = { type: 'pane', id: 'placeholder', kind: 'agent_chat', title: 'Empty', metadata: { workspace_id: 'alpha' } };
  let root = dockPane(placeholder, first, placeholder.id, 'center');
  root = dockPane(root, sessionPane(session('dock-once', 'alpha', 'codex')), root.id, 'center');
  root = dockPane(root, { ...first, title: 'stale drag title' }, root.id, 'center');

  const opened = flattenPanes(root).filter(pane => pane.metadata?.session_id === 'dock-once');
  assert.equal(opened.length, 1);
  assert.equal(opened[0].id, first.id);
  assert.equal(opened[0].title, first.title, 'the canonical session pane wins over stale drag metadata');
  assert.equal(validateLayout({ version: 1, root }), true);
});

test('splits, responsive collapse, preset remounts and restore preserve session/workspace/provider metadata', () => {
  const claude = session('layout-claude', 'alpha', 'claude_code');
  const terminal = session('layout-terminal', 'beta', 'terminal');
  let canonical = dockPane(sessionPane(claude), sessionPane(terminal), 'session-layout-claude', 'right');
  canonical = splitPane(canonical, 'session-layout-terminal', 'vertical');
  canonical = applyPreset(canonical, '1:2:1', 'session-layout-terminal');
  assert.equal(validateLayout({ version: 1, root: canonical }), true);

  const expected = new Map([
    ['layout-claude', { workspace_id: 'alpha', provider: 'claude_code' }],
    ['layout-terminal', { workspace_id: 'beta', provider: 'terminal' }],
  ]);
  const assertMetadata = root => {
    for (const pane of flattenPanes(root)) {
      const id = pane.metadata?.session_id;
      if (!id || !expected.has(id)) continue;
      assert.deepEqual({ workspace_id: pane.metadata.workspace_id, provider: pane.metadata.provider }, expected.get(id));
      assert.equal(paneSession(pane, [claude, terminal])?.id, id);
    }
  };

  const before = JSON.stringify(canonical);
  const narrow = projectLayout(canonical, 390, 640, { selectedPaneId: 'session-layout-terminal' });
  assert.equal(narrow.type, 'stack');
  assertMetadata(narrow);
  assertMetadata(projectLayout(canonical, 1600, 900));
  assert.equal(JSON.stringify(canonical), before, 'responsive projection is a disposable view');

  // What an older version stored when the canvas was too narrow: one group of
  // tabs, and the split to put back. It still has to read back intact.
  assert.equal(canonical.type, 'split');
  const tabs = flattenPanes(canonical);
  const collapsed = { version: 1, root: { type: 'stack', kind: 'stack', id: 'legacy-tabs', panes: tabs, activePaneId: tabs[0].id }, collapsed: [{ splitId: canonical.id, replacementId: 'legacy-tabs', node: cloneNode(canonical) }] };
  assertMetadata(collapsed.root);
  const restored = restoreCollapsed(collapsed);
  assertMetadata(restored.root);
  assertMetadata(cloneNode(restored.root));
  assertMetadata(applyPreset(restored.root, '2x2'));
  assert.equal(validateLayout(restored), true);
  // Closing a pane removes only its view; the other session remains bound.
  const withoutTerminal = closePane(restored.root, 'session-layout-terminal');
  assert.equal(paneSession(flattenPanes(withoutTerminal).find(p => p.metadata?.session_id === 'layout-claude'), [claude, terminal])?.id, 'layout-claude');
});

async function renderChatWithDraft(chatSession, setDraft) {
  const instrumented = {
    ...ChatSessionPane,
    setup(props, context) {
      const bindings = ChatSessionPane.setup(props, context);
      if (setDraft) setDraft(bindings.draft.value);
      return bindings;
    },
  };
  const context = {};
  return renderToString(createSSRApp(instrumented, { session: chatSession, profiles: [] }), context);
}

test('the actual chat SFC keeps drafts scoped to the bound session across remounts', async () => {
  setBackendCapabilities({ ok: true, api_version: 2, capabilities: ['structured_chat'] });
  const first = session('draft-bound-a', 'alpha', 'codex', 'structured');
  const second = session('draft-bound-b', 'alpha', 'codex', 'structured');

  await renderChatWithDraft(first, draft => { draft.text = 'draft for first session'; });
  const firstRemount = await renderChatWithDraft(first);
  const secondInitial = await renderChatWithDraft(second);
  assert.match(firstRemount, /draft for first session/);
  assert.doesNotMatch(secondInitial, /draft for first session/);

  await renderChatWithDraft(second, draft => { draft.text = 'draft for second session'; });
  assert.match(await renderChatWithDraft(first), /draft for first session/);
  assert.match(await renderChatWithDraft(second), /draft for second session/);
});

async function createDialogRequests(capabilities, run) {
  setBackendCapabilities(capabilities);
  const calls = [];
  const previousFetch = globalThis.fetch;
  globalThis.fetch = async (url, init) => {
    calls.push({ url: String(url), body: JSON.parse(init.body) });
    return new Response(JSON.stringify({
      id: `created-${calls.length}`, workspace_id: 'alpha', provider: calls.at(-1).body.provider,
      title: calls.at(-1).body.title, status: 'stopped', created_at: 'now', updated_at: 'now',
      endpoint_profile_id: null, provider_session_id: null, error: null,
    }), { status: 200, headers: { 'content-type': 'application/json' } });
  };
  try {
    const component = {
      ...CreateSessionDialog,
      async setup(props, context) {
        const bindings = CreateSessionDialog.setup(props, context);
        await run(bindings);
        return bindings;
      },
    };
    const props = { workspace: reactive(workspace('alpha', 'Frontend')), profiles: [{ id: '11111111-2222-4333-8444-555555555555', name: 'Host Claude', provider: 'claude_code', endpoint_url: null, model: null, permission_mode: 'native', secret_ref: null, native_config: { source_id: 'claude-default', config_dir: '/fixture/claude', config_env: null }, created_at: '2026-01-01T00:00:00Z' }], initialProvider: 'claude_code' };
    await renderToString(createSSRApp(component, props), {});
    return calls;
  } finally {
    globalThis.fetch = previousFetch;
  }
}

test('the real create-session payload selects structured chat for agents, never for terminals, and omits it on old backends', async () => {
  const modern = await createDialogRequests({ ok: true, api_version: 2, capabilities: ['structured_chat'] }, async state => {
    state.title.value = 'Agent';
    await state.create();
    state.provider.value = 'terminal';
    state.title.value = 'Shell';
    await state.create();
  });
  assert.equal(modern.length, 2);
  assert.equal(modern[0].body.provider, 'claude_code');
  assert.equal(modern[0].body.interaction_mode, 'structured');
  assert.equal(modern[1].body.provider, 'terminal');
  assert.equal(Object.hasOwn(modern[1].body, 'interaction_mode'), false);

  const legacy = await createDialogRequests({ ok: true }, async state => {
    state.title.value = 'Legacy agent';
    await state.create();
  });
  assert.equal(legacy.length, 1);
  assert.equal(legacy[0].body.provider, 'claude_code');
  assert.equal(Object.hasOwn(legacy[0].body, 'interaction_mode'), false);
});

test('the rendered workspace pane resolves the session workspace from pane metadata, not current sidebar selection', async () => {
  setBackendCapabilities({ ok: true });
  const pane = sessionPane(session('render-bound', 'alpha', 'claude_code'));
  const html = await renderToString(createSSRApp(WorkspacePane, {
    pane,
    workspaces: [workspace('beta', 'Backend'), workspace('alpha', 'Frontend')],
    sessions: [session('render-bound', 'alpha', 'claude_code')],
    profiles: [],
    gitRefresh: {},
  }), {});
  assert.match(html, /data-workspace-id="alpha"/);
  assert.match(html, /Frontend/);
  assert.doesNotMatch(html, /Backend/);
});
