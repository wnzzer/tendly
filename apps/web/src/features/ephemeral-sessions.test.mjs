import { after, afterEach, before, beforeEach, test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';
import vue from '@vitejs/plugin-vue';
import { createSSRApp, reactive } from 'vue';
import { renderToString } from 'vue/server-renderer';
import { filterSessionList } from './session-list.ts';
import { groupWorkspaces } from './workspace-groups.ts';
import { markEphemeralPanes, sessionPane, withoutDiscardedPanes } from './pane-context.ts';
import { flattenPanes } from '../layout/layout-engine.ts';

let server, App, Canvas, CreateSessionDialog, SidebarSessionRow, capabilities, connections, i18n;
const originalGlobals = new Map();
const workspace = id => ({ id, name: `Workspace ${id}`, root_path: `/fixtures/${id}`, created_at: '2026-09-01T00:00:00Z' });
const alpha = workspace('alpha'), beta = workspace('beta');
const session = (id, changes = {}) => ({
  id, workspace_id: alpha.id, title: `Session ${id}`, provider: 'codex', interaction_mode: 'structured',
  status: 'stopped', created_at: '2026-09-01T00:00:00Z', updated_at: '2026-09-01T00:00:00Z',
  archived_at: null, ephemeral: false, endpoint_profile_id: null, provider_session_id: null,
  configuration_revision: 0, environment: {}, error: null, ...changes,
});
const keeper = session('keeper');
const scratch = session('scratch', { ephemeral: true, title: 'Scratch window' });
const filePaneNode = { type: 'pane', id: 'files-alpha', kind: 'editor', title: 'Files', metadata: { workspace_id: alpha.id } };
const clone = value => JSON.parse(JSON.stringify(value));
const stackOf = (...panes) => ({ version: 1, root: { type: 'stack', kind: 'stack', id: 'fixture-stack', panes, activePaneId: panes[0]?.id } });

before(async () => {
  server = await createServer({ configFile: false, root: fileURLToPath(new URL('../../', import.meta.url)), plugins: [vue()], server: { middlewareMode: true, hmr: false, ws: false }, appType: 'custom', optimizeDeps: { noDiscovery: true } });
  App = (await server.ssrLoadModule('/src/App.vue')).default;
  Canvas = (await server.ssrLoadModule('/src/components/Canvas.vue')).default;
  CreateSessionDialog = (await server.ssrLoadModule('/src/features/CreateSessionDialog.vue')).default;
  SidebarSessionRow = (await server.ssrLoadModule('/src/features/SidebarSessionRow.vue')).default;
  capabilities = await server.ssrLoadModule('/src/features/backend-capabilities.ts');
  connections = (await server.ssrLoadModule('/src/features/session-connection.ts')).sessionConnections;
  i18n = (await server.ssrLoadModule('/src/i18n/index.ts')).useI18n();
});
after(async () => { await server?.close(); });

let storageValues;
beforeEach(() => {
  storageValues = new Map();
  const storage = { getItem: key => storageValues.get(key) ?? null, setItem: (key, value) => storageValues.set(key, value), removeItem: key => storageValues.delete(key) };
  const fixtures = {
    window: { innerWidth: 1440, location: { origin: 'http://ephemeral.fixture.invalid' }, localStorage: storage, addEventListener() { throw Error('SSR must not mount the application'); }, removeEventListener() {} },
    document: { documentElement: { lang: 'en' }, hidden: false },
    localStorage: storage,
  };
  for (const [name, value] of Object.entries(fixtures)) {
    originalGlobals.set(name, Object.getOwnPropertyDescriptor(globalThis, name));
    Object.defineProperty(globalThis, name, { value, configurable: true, writable: true });
  }
  i18n.setLocale('en');
  capabilities.setBackendCapabilities({ ok: true, api_version: 2, capabilities: ['ephemeral_sessions', 'shared_canvas'] });
});
afterEach(() => {
  capabilities.setBackendCapabilities({ ok: false });
  for (const [name, descriptor] of originalGlobals) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor);
    else delete globalThis[name];
  }
  originalGlobals.clear();
});

async function harness(t, { fetch: respond, sessions = [keeper, scratch], workspaces = [alpha, beta], layout = stackOf(filePaneNode, sessionPane(scratch)) } = {}) {
  const requests = [], openIntents = [], closed = [];
  t.mock.method(globalThis, 'fetch', async (url, init = {}) => {
    const request = { url: String(url), method: init.method ?? 'GET', body: init.body === undefined ? undefined : JSON.parse(init.body) };
    requests.push(request);
    if (!respond) throw Error(`Unexpected network request: ${request.method} ${request.url}`);
    return respond(request);
  });
  t.mock.method(connections, 'requestOpen', id => openIntents.push(id));
  let state;
  const instrumented = {
    ...App,
    setup(props, context) {
      state = App.setup(props, context);
      state.workspaces.value = clone(workspaces);
      state.sessions.value = clone(sessions);
      state.layout.value = clone(layout);
      state.selectedWorkspaceId.value = alpha.id;
      state.loading.value = false;
      state.canvasReady.value = false;
      state.explorerOpen.value = false;
      return state;
    },
  };
  await renderToString(createSSRApp(instrumented));
  // The canvas is a navigation boundary in this harness: closing records the id
  // the root asked for instead of running the real layout component.
  state.canvas.value = { openPane() {}, focusPane: async () => {}, closePane: async id => { closed.push(id); } };
  assert.equal(requests.length, 0, 'SSR setup must not talk to the backend');
  return { state, requests, openIntents, closed };
}

// ---------------------------------------------------------------- creation

async function createDialogRequests(health, run) {
  capabilities.setBackendCapabilities(health);
  const calls = [];
  const previousFetch = globalThis.fetch;
  globalThis.fetch = async (url, init) => {
    const body = JSON.parse(init.body);
    calls.push({ url: String(url), body });
    return Response.json({ ...session('created'), title: body.title, provider: body.provider, ephemeral: body.ephemeral === true });
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
    // ModalDialog teleports its body, so the markup arrives through the SSR context.
    const context = {};
    await renderToString(createSSRApp(component, { workspace: reactive(alpha), profiles: [], initialProvider: 'claude_code' }), context);
    return { calls, html: Object.values(context.teleports ?? {}).join('') };
  } finally { globalThis.fetch = previousFetch; }
}

test('the create payload carries ephemeral only when the user opted in on a backend that supports it', async () => {
  const supported = await createDialogRequests({ ok: true, api_version: 2, capabilities: ['ephemeral_sessions'] }, async state => {
    state.title.value = 'Permanent';
    await state.create();
    state.title.value = 'Scratch';
    state.ephemeral.value = true;
    await state.create();
  });
  assert.equal(supported.calls.length, 2);
  assert.equal(supported.calls[0].url, '/api/workspaces/alpha/sessions');
  assert.equal(Object.hasOwn(supported.calls[0].body, 'ephemeral'), false, 'an unchecked box must not send the field');
  assert.equal(supported.calls[1].body.ephemeral, true);
  assert.match(supported.html, /session-ephemeral-choice/);
  assert.match(supported.html, /Temporary window/);
});

test('an old backend hides the choice and never sends the field, even if the box was somehow checked', async () => {
  const legacy = await createDialogRequests({ ok: true }, async state => {
    state.title.value = 'Legacy';
    state.ephemeral.value = true;
    await state.create();
  });
  assert.equal(legacy.calls.length, 1);
  assert.equal(Object.hasOwn(legacy.calls[0].body, 'ephemeral'), false);
  assert.doesNotMatch(legacy.html, /session-ephemeral-choice/);

  // An api_version-2 backend that does not advertise the capability is equally old here.
  const unadvertised = await createDialogRequests({ ok: true, api_version: 2, capabilities: [] }, async state => {
    state.title.value = 'Unadvertised';
    state.ephemeral.value = true;
    await state.create();
  });
  assert.equal(Object.hasOwn(unadvertised.calls[0].body, 'ephemeral'), false);
  assert.doesNotMatch(unadvertised.html, /session-ephemeral-choice/);
});

// ------------------------------------------------------------ list treatment

test('a temporary window stays in its workspace list, because it is open and in use', () => {
  // Hiding it made it unmanageable: there was no way to reach rename, keep or
  // locate. It disappears on its own when closed, so it never accumulates.
  const groups = groupWorkspaces([alpha], [keeper, scratch], { selectedWorkspaceId: alpha.id });
  assert.deepEqual(groups[0].sessions.map(item => item.id), [keeper.id, scratch.id]);
  assert.equal(groups[0].totalSessions, 2);
  // Archiving is an explicit request to hide, and still does.
  const archived = { ...keeper, archived_at: '2026-09-14T00:00:00Z' };
  assert.deepEqual(groupWorkspaces([alpha], [archived, scratch], { selectedWorkspaceId: alpha.id })[0].sessions.map(item => item.id), [scratch.id]);

  const library = filterSessionList([keeper, scratch], [alpha]);
  assert.deepEqual(library.map(entry => entry.session.id), [keeper.id, scratch.id], 'the library shows them by default');
  assert.deepEqual(filterSessionList([keeper, scratch], [alpha], { ephemeral: 'only' }).map(entry => entry.session.id), [scratch.id]);
  assert.deepEqual(filterSessionList([keeper, scratch], [alpha], { ephemeral: 'hidden' }).map(entry => entry.session.id), [keeper.id]);
  // The archive axis still works independently of the temporary one.
  const archivedScratch = { ...scratch, archived_at: '2026-09-14T00:00:00Z' };
  assert.deepEqual(filterSessionList([keeper, archivedScratch], [alpha], { archive: 'archived' }).map(entry => entry.session.id), [archivedScratch.id]);
});

test('a running temporary session counts as active, since it is a real running client', () => {
  const busy = session('busy', { ephemeral: true, status: 'running' });
  const [group] = groupWorkspaces([alpha], [busy], { selectedWorkspaceId: alpha.id });
  assert.deepEqual(group.sessions.map(item => item.id), [busy.id]);
  assert.equal(group.activeCount, 1);
});

test('the sidebar row marks a temporary session and only then offers to keep it', async () => {
  const kept = [];
  async function render(fixture) {
    const component = {
      ...SidebarSessionRow,
      setup(props, context) { const state = SidebarSessionRow.setup(props, context); state.menuOpen.value = true; return state; },
    };
    return renderToString(createSSRApp(component, { session: fixture, archiveSupported: true, onKeep: item => kept.push(item.id) }));
  }
  const temporary = await render(scratch);
  assert.match(temporary, /session-temporary-badge/);
  assert.match(temporary, /session-keep-action/);
  assert.match(temporary, /Keep this session/);
  const permanent = await render(keeper);
  assert.doesNotMatch(permanent, /session-temporary-badge/);
  assert.doesNotMatch(permanent, /session-keep-action/);
  assert.deepEqual(kept, [], 'rendering alone never promotes anything');
});

// -------------------------------------------------------------- close/discard

test('closing a stopped temporary pane discards its record silently, with no dialog', async t => {
  const { state, requests } = await harness(t, { fetch: () => Response.json({ id: scratch.id }) });
  const pane = flattenPanes(state.layout.value.root).find(item => item.metadata?.session_id === scratch.id);
  assert.equal(await state.confirmClosePane(pane), true, 'the view may close once the record is gone');
  assert.deepEqual(requests, [{ url: `/api/sessions/${scratch.id}`, method: 'DELETE', body: undefined }]);
  assert.deepEqual(state.sessions.value.map(item => item.id), [keeper.id]);
  assert.equal(state.discardPrompt.value, undefined);
  assert.equal(state.error.value, '');
});

test('closing a pane bound to a permanent session keeps the old behaviour: no request, no prompt', async t => {
  const { state, requests, openIntents } = await harness(t, { layout: stackOf(filePaneNode, sessionPane(keeper)) });
  const pane = flattenPanes(state.layout.value.root).find(item => item.metadata?.session_id === keeper.id);
  assert.equal(await state.confirmClosePane(pane), true);
  assert.equal(await state.confirmClosePane(filePaneNode), true);
  assert.deepEqual(requests, []);
  assert.equal(state.discardPrompt.value, undefined);
  assert.deepEqual(state.sessions.value.map(item => item.id), [keeper.id, scratch.id]);
  assert.deepEqual(openIntents, []);
});

test('a temporary terminal closes with its tab, without asking, though its process is running', async t => {
  const shell = session('shell', { ephemeral: true, status: 'running', interaction_mode: 'pty', title: 'Scratch (terminal)' });
  const { state, requests } = await harness(t, {
    sessions: [keeper, shell], layout: stackOf(filePaneNode, sessionPane(shell)),
    fetch: () => Response.json({ id: shell.id }),
  });
  const pane = flattenPanes(state.layout.value.root).find(item => item.metadata?.session_id === shell.id);
  assert.equal(await state.confirmClosePane(pane), true);
  assert.equal(state.discardPrompt.value, undefined);
  assert.deepEqual(requests, [{ url: `/api/sessions/${shell.id}`, method: 'DELETE', body: undefined }]);
  assert.deepEqual(state.sessions.value.map(item => item.id), [keeper.id]);
});

test('closing a terminal reopened from a conversation discards it and goes back to that conversation', async t => {
  const hatch = session('hatch', { ephemeral: true, status: 'running', interaction_mode: 'pty', provider: 'claude_code', resume_source_id: keeper.id, title: 'Keeper (terminal)' });
  const { state, requests } = await harness(t, {
    sessions: [keeper, hatch], layout: stackOf(sessionPane(keeper), sessionPane(hatch)),
    fetch: () => Response.json({ id: hatch.id }),
  });
  const opened = [];
  state.canvas.value = { ...state.canvas.value, openPane: pane => opened.push(pane.id) };
  const pane = flattenPanes(state.layout.value.root).find(item => item.metadata?.session_id === hatch.id);
  assert.equal(await state.confirmClosePane(pane), true, 'a terminal does not ask: its process running says nothing about work under way');
  assert.deepEqual(requests, [{ url: `/api/sessions/${hatch.id}`, method: 'DELETE', body: undefined }]);
  assert.deepEqual(opened, [], 'the conversation is shown only after the canvas has settled the close');
  await new Promise(resolve => setTimeout(resolve));
  assert.deepEqual(opened, [sessionPane(keeper).id]);

  // Leaving by the button stops the process under the DELETE, and its exit
  // asks to close the same tab again: that second close waits for the first.
  requests.length = 0; opened.length = 0;
  const again = session('again', { ...hatch, id: 'again' });
  state.sessions.value = [keeper, again];
  let release, arrived;
  const sent = new Promise(resolve => { arrived = resolve; });
  globalThis.fetch.mock.mockImplementation(async (url, init = {}) => { requests.push({ url: String(url), method: init.method }); const reply = new Promise(resolve => { release = resolve; }); arrived(); await reply; return Response.json({ id: again.id }); });
  const first = state.confirmClosePane(sessionPane(again));
  assert.equal(await state.confirmClosePane(sessionPane(again)), false, 'the tab is already on its way out');
  await sent;
  release();
  assert.equal(await first, true);
  assert.equal(requests.length, 1, 'one DELETE, not two');
  await new Promise(resolve => setTimeout(resolve));
  assert.deepEqual(opened, [sessionPane(keeper).id]);

  // Any other temporary window just closes.
  opened.length = 0;
  globalThis.fetch.mock.mockImplementation(async () => Response.json({ id: 'shell' }));
  const shell = session('shell', { ephemeral: true, status: 'running', interaction_mode: 'pty', provider: 'terminal', resume_source_id: keeper.id });
  state.sessions.value = [keeper, shell];
  assert.equal(await state.confirmClosePane(sessionPane(shell)), true);
  await new Promise(resolve => setTimeout(resolve));
  assert.deepEqual(opened, []);
});

test('a working temporary session is confirmed inline before anything is stopped', async t => {
  const busy = session('busy', { ephemeral: true, status: 'running', title: 'Busy scratch' });
  const { state, requests, closed } = await harness(t, {
    sessions: [keeper, busy], layout: stackOf(filePaneNode, sessionPane(busy)),
    fetch: () => Response.json({ id: busy.id }),
  });
  const pane = flattenPanes(state.layout.value.root).find(item => item.metadata?.session_id === busy.id);
  assert.equal(await state.confirmClosePane(pane), false, 'the pane must stay until the user answers');
  assert.deepEqual(requests, [], 'asking must not pre-emptively stop the agent');
  assert.equal(state.discardPrompt.value.session.id, busy.id);
  assert.equal(state.discardPrompt.value.paneId, pane.id);

  // Declining leaves everything exactly as it was.
  state.discardPrompt.value = undefined;
  assert.deepEqual(requests, []);
  assert.deepEqual(state.sessions.value.map(item => item.id), [keeper.id, busy.id]);

  await state.confirmClosePane(pane);
  await state.confirmDiscard();
  assert.deepEqual(requests, [{ url: `/api/sessions/${busy.id}`, method: 'DELETE', body: undefined }]);
  assert.deepEqual(state.sessions.value.map(item => item.id), [keeper.id]);
  assert.deepEqual(closed, [pane.id], 'the view closes only after the record is gone');
  assert.equal(state.discardPrompt.value, undefined);
});

test('a failed discard surfaces the error and leaves both the pane and the record in place', async t => {
  const { state, requests, closed } = await harness(t, {
    fetch: () => new Response(JSON.stringify({ error: 'Session is not temporary' }), { status: 409, headers: { 'content-type': 'application/json' } }),
  });
  const pane = flattenPanes(state.layout.value.root).find(item => item.metadata?.session_id === scratch.id);
  assert.equal(await state.confirmClosePane(pane), false, 'the pane must not vanish while the server still has the record');
  assert.equal(requests.length, 1);
  assert.equal(state.error.value, 'Session is not temporary');
  assert.deepEqual(state.sessions.value.map(item => item.id), [keeper.id, scratch.id]);
  assert.deepEqual(closed, []);
});

test('a record the server already discarded lets the view close instead of trapping the pane', async t => {
  const { state, requests } = await harness(t, {
    fetch: () => new Response(JSON.stringify({ error: 'Session not found' }), { status: 404, headers: { 'content-type': 'application/json' } }),
  });
  const pane = flattenPanes(state.layout.value.root).find(item => item.metadata?.session_id === scratch.id);
  assert.equal(await state.confirmClosePane(pane), true);
  assert.equal(requests.length, 1);
  assert.equal(state.error.value, '', 'a already-gone record is not an error to show');
  assert.deepEqual(state.sessions.value.map(item => item.id), [keeper.id]);
});

test('the canvas refuses to remove a pane its owner vetoed, and removes one it allowed', async () => {
  async function run(allow) {
    const asked = [], emitted = [];
    const layout = stackOf(filePaneNode, sessionPane(scratch));
    let state;
    const component = {
      ...Canvas,
      setup(props, context) { state = Canvas.setup(props, context); return state; },
    };
    await renderToString(createSSRApp(component, {
      modelValue: layout, confirmClosePane: pane => { asked.push(pane.id); return allow; },
      ephemeralSessionIds: [scratch.id], 'onUpdate:modelValue': value => emitted.push(value),
    }));
    await state.close(`session-${scratch.id}`);
    return { asked, emitted };
  }
  const vetoed = await run(false);
  assert.deepEqual(vetoed.asked, [`session-${scratch.id}`]);
  assert.deepEqual(vetoed.emitted, [], 'a vetoed close must not mutate the layout');
  const allowed = await run(true);
  assert.deepEqual(allowed.asked, [`session-${scratch.id}`]);
  assert.equal(allowed.emitted.length, 1);
  assert.deepEqual(flattenPanes(allowed.emitted[0].root).map(pane => pane.id), [filePaneNode.id]);
});

test('the canvas tab marks a temporary window and stops promising the session keeps running', async () => {
  const layout = stackOf(filePaneNode, sessionPane(keeper), sessionPane(scratch));
  const html = await renderToString(createSSRApp(Canvas, { modelValue: layout, ephemeralSessionIds: [scratch.id], workspaceLabels: { alpha: alpha.name } }));
  const tab = id => html.slice(html.indexOf(`id="dock-tab-session-${id}"`)).split('</div>')[0];
  assert.match(tab(scratch.id), /dock-tab-ephemeral/);
  assert.match(tab(scratch.id), /is-ephemeral/);
  assert.match(tab(scratch.id), /temporary window/);
  assert.match(html, /Close and discard this temporary session/);
  assert.doesNotMatch(tab(keeper.id), /ephemeral|temporary/);
  assert.match(html, /Close pane \(session keeps running\)/, 'a permanent tab keeps its original promise');
});

// -------------------------------------------------------------------- promote

test('keeping a session posts to its keep route with no body and updates local state only', async t => {
  const { state, requests, openIntents } = await harness(t, {
    fetch: () => Response.json({ ...scratch, ephemeral: false, updated_at: '2026-09-15T00:00:00Z' }),
  });
  await state.keepSession(state.sessions.value[1]);
  assert.deepEqual(requests, [{ url: `/api/sessions/${scratch.id}/keep`, method: 'POST', body: undefined }]);
  assert.equal(state.sessions.value[1].ephemeral, false);
  assert.equal(state.sessions.value[1].title, scratch.title, 'promotion keeps identity and title');
  assert.deepEqual(state.keepBusyIds.value, []);
  assert.deepEqual(state.ephemeralIds.value, []);
  assert.match(state.sessionNotice.value, /stays in the session list/);
  assert.deepEqual(openIntents, [], 'promotion must never start a client');

  // Now permanent: closing its pane is view-only again.
  const pane = flattenPanes(state.layout.value.root).find(item => item.metadata?.session_id === scratch.id);
  assert.equal(await state.confirmClosePane(pane), true);
  assert.equal(requests.length, 1);
});

test('promotion is refused for a session that is already permanent and never issues a duplicate request', async t => {
  const { state, requests } = await harness(t, { fetch: () => Response.json({ ...scratch, ephemeral: false }) });
  await state.keepSession(keeper);
  assert.deepEqual(requests, []);
  const pending = state.keepSession(state.sessions.value[1]);
  assert.deepEqual(state.keepBusyIds.value, [scratch.id]);
  await state.keepSession(state.sessions.value[1]);
  assert.equal(requests.length, 1, 'a second click while in flight is ignored');
  await pending;
});

// -------------------------------------------------------- layout persistence

test('markEphemeralPanes marks only temporary session panes and leaves other layouts untouched', () => {
  const layout = stackOf(filePaneNode, sessionPane(keeper), sessionPane(scratch));
  const marked = markEphemeralPanes(layout, [keeper, scratch]);
  assert.deepEqual(flattenPanes(marked.root).map(pane => pane.id), [filePaneNode.id, `session-${keeper.id}`, `session-${scratch.id}`]);
  assert.deepEqual(flattenPanes(marked.root).map(pane => pane.metadata?.ephemeral === true), [false, false, true]);
  assert.equal(flattenPanes(layout.root)[2].metadata.ephemeral, undefined, 'the input document is not mutated');
  assert.equal(markEphemeralPanes(layout, [keeper]), layout, 'nothing temporary means the same object');
  assert.equal(markEphemeralPanes(marked, [keeper, scratch]), marked, 'marking twice changes nothing');
});

test('a restored temporary window is dropped once its session is discarded, and only then', () => {
  const restored = markEphemeralPanes(stackOf(filePaneNode, sessionPane(keeper), sessionPane(scratch)), [keeper, scratch]);
  assert.equal(withoutDiscardedPanes(restored.root, [keeper, scratch]), restored.root, 'a live temporary window stays open');
  const pruned = withoutDiscardedPanes(restored.root, [keeper]);
  assert.deepEqual(flattenPanes(pruned).map(pane => pane.id), [filePaneNode.id, `session-${keeper.id}`]);
  // An ordinary pane whose session is missing is not this function's to remove.
  const unmarked = stackOf(sessionPane(keeper));
  assert.equal(withoutDiscardedPanes(unmarked.root, []), unmarked.root);
});

test('the saved shared canvas and its browser cache keep a temporary window, marked', async t => {
  const saved = [];
  const { state, requests } = await harness(t, {
    layout: stackOf(filePaneNode, sessionPane(keeper), sessionPane(scratch)),
    fetch: request => { saved.push(request.body.layout); return Response.json({ revision: 7 }); },
  });
  state.queueLayout(state.layout.value);
  await state.flushLayout();
  assert.equal(requests.length, 1);
  assert.equal(requests[0].url, '/api/canvas/layout');
  assert.deepEqual(flattenPanes(saved[0].root).map(pane => [pane.id, pane.metadata?.ephemeral === true]), [[filePaneNode.id, false], [`session-${keeper.id}`, false], [`session-${scratch.id}`, true]]);
  assert.equal(state.layoutStatus.value, 'Saved');
  const cached = JSON.parse([...storageValues.values()].find(value => value.includes('fixture-stack')));
  assert.deepEqual(flattenPanes(cached.layout.root).map(pane => pane.id), [filePaneNode.id, `session-${keeper.id}`, `session-${scratch.id}`]);
});

test('the localStorage fallback on a backend without a shared canvas is marked the same way', async t => {
  capabilities.setBackendCapabilities({ ok: true, api_version: 2, capabilities: ['ephemeral_sessions'] });
  const { state, requests } = await harness(t, { layout: stackOf(filePaneNode, sessionPane(scratch)) });
  state.queueLayout(state.layout.value);
  await state.flushLayout();
  assert.deepEqual(requests, [], 'an old canvas backend is never contacted');
  assert.equal(state.layoutStatus.value, 'Saved in this browser');
  const cached = JSON.parse([...storageValues.values()].find(value => value.includes('"layout"')));
  assert.deepEqual(flattenPanes(cached.layout.root).map(pane => [pane.id, pane.metadata?.ephemeral === true]), [[filePaneNode.id, false], [`session-${scratch.id}`, true]]);
});

test('a backend without the capability persists every pane exactly as before', async t => {
  capabilities.setBackendCapabilities({ ok: true, api_version: 2, capabilities: ['shared_canvas'] });
  const saved = [];
  // No session can be temporary without the backend, so nothing is ever trimmed.
  const { state } = await harness(t, {
    sessions: [keeper, session('legacy')], layout: stackOf(filePaneNode, sessionPane(keeper)),
    fetch: request => { saved.push(request.body.layout); return Response.json({ revision: 1 }); },
  });
  state.queueLayout(state.layout.value);
  await state.flushLayout();
  assert.deepEqual(saved[0], state.layout.value);
});
