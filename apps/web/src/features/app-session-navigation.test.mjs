import { after, afterEach, before, beforeEach, test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import vue from "@vitejs/plugin-vue";
import { createSSRApp } from "vue";
import { renderToString } from "vue/server-renderer";

let server, App, capabilities, connections, i18n;
const originalGlobals = new Map();
const workspace = id => ({
  id,
  name: `Workspace ${id}`,
  root_path: `/fixtures/${id}`,
  created_at: "2026-09-01T00:00:00Z",
});
const alpha = workspace("alpha"),
  beta = workspace("beta");
const session = (id, changes = {}) => ({
  id,
  workspace_id: alpha.id,
  title: `Session ${id}`,
  provider: "codex",
  interaction_mode: "structured",
  status: "stopped",
  created_at: "2026-09-01T00:00:00Z",
  updated_at: "2026-09-01T00:00:00Z",
  archived_at: null,
  endpoint_profile_id: "fixture-profile",
  provider_session_id: `native-${id}`,
  configuration_revision: 3,
  environment: {},
  error: null,
  ...changes,
});
const first = session("first"),
  second = session("second", { workspace_id: beta.id, provider: "claude_code" });
const filePane = {
  type: "pane",
  id: "files-beta",
  kind: "editor",
  title: "Files",
  metadata: { workspace_id: beta.id },
};
const boundPane = fixture => ({
  type: "pane",
  id: "legacy-view-id",
  kind: "agent_chat",
  title: fixture.title,
  metadata: { workspace_id: fixture.workspace_id, session_id: fixture.id, provider: fixture.provider },
});
const layoutFor = fixture => ({
  version: 1,
  root: {
    type: "stack",
    kind: "stack",
    id: "fixture-stack",
    panes: [filePane, boundPane(fixture)],
    activePaneId: filePane.id,
  },
});
const clone = value => JSON.parse(JSON.stringify(value));

before(async () => {
  // Compile and render the real root SFC, including its imports, without opening
  // HTTP/HMR listeners or mounting any native session components.
  server = await createServer({
    configFile: false,
    root: fileURLToPath(new URL("../../", import.meta.url)),
    plugins: [vue()],
    server: { middlewareMode: true, hmr: false, ws: false },
    appType: "custom",
    optimizeDeps: { noDiscovery: true },
  });
  App = (await server.ssrLoadModule("/src/App.vue")).default;
  capabilities = await server.ssrLoadModule("/src/features/backend-capabilities.ts");
  connections = (await server.ssrLoadModule("/src/features/session-connection.ts")).sessionConnections;
  i18n = (await server.ssrLoadModule("/src/i18n/index.ts")).useI18n();
});

beforeEach(() => {
  const values = new Map();
  const storage = {
    getItem: key => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
    removeItem: key => values.delete(key),
  };
  const fixtures = {
    window: {
      innerWidth: 1440,
      location: { origin: "http://tendly.fixture.invalid" },
      localStorage: storage,
      addEventListener() {
        throw Error("SSR must not mount the application");
      },
      removeEventListener() {},
    },
    document: { documentElement: { lang: "en" }, hidden: false },
    localStorage: storage,
  };
  for (const [name, value] of Object.entries(fixtures)) {
    originalGlobals.set(name, Object.getOwnPropertyDescriptor(globalThis, name));
    Object.defineProperty(globalThis, name, { value, configurable: true, writable: true });
  }
  i18n.setLocale("en");
  capabilities.setBackendCapabilities({
    ok: true,
    api_version: 2,
    capabilities: ["session_archive", "structured_chat"],
  });
});

afterEach(() => {
  capabilities.setBackendCapabilities({ ok: false });
  for (const [name, descriptor] of originalGlobals) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor);
    else delete globalThis[name];
  }
  originalGlobals.clear();
});
after(async () => {
  await server?.close();
});

function deferred() {
  let resolve;
  const promise = new Promise(done => {
    resolve = done;
  });
  return { promise, resolve };
}

async function harness(
  t,
  { fetch: respond, sessions = [first, second], workspaces = [alpha, beta], layout = layoutFor(first) } = {},
) {
  const requests = [],
    openIntents = [],
    opened = [],
    focused = [],
    revealed = [];
  t.mock.method(globalThis, "fetch", async (url, init = {}) => {
    const request = {
      url: String(url),
      method: init.method ?? "GET",
      body: init.body === undefined ? undefined : JSON.parse(init.body),
    };
    requests.push(request);
    if (!respond) throw Error(`Unexpected network request: ${request.method} ${request.url}`);
    return respond(request);
  });
  t.mock.method(connections, "requestOpen", id => openIntents.push(id));
  let state;
  const instrumented = {
    ...App,
    setup(props, context) {
      state = App.setup(props, context);
      state.workspaces.value = clone(workspaces);
      state.sessions.value = clone(sessions);
      state.layout.value = clone(layout);
      state.selectedWorkspaceId.value = beta.id;
      state.selectedPaneId.value = filePane.id;
      state.loading.value = false;
      // The root handlers are real; its canvas/sidebar instance methods are
      // navigation boundaries, not real browser/native clients in this harness.
      state.canvasReady.value = false;
      state.explorerOpen.value = false;
      return state;
    },
  };
  const html = await renderToString(createSSRApp(instrumented));
  state.canvas.value = { openPane: pane => opened.push(clone(pane)), focusPane: async id => focused.push(id) };
  state.workspaceSidebar.value = { revealSession: async id => revealed.push(id) };
  assert.equal(requests.length, 0, "SSR setup must not bootstrap or connect a native session");
  return { state, html, requests, openIntents, opened, focused, revealed };
}

test("reveal uses the sidebar and mobile drawer, allowing only a read-only Git status refresh", async t => {
  const { state, requests, openIntents, opened, focused, revealed } = await harness(t, {
    fetch: request => {
      assert.deepEqual(request, { url: "/api/workspaces/alpha/git/status", method: "GET", body: undefined });
      return Response.json({ branch: "main", files: [] });
    },
  });
  globalThis.window.innerWidth = 390;
  const before = clone(state.layout.value);
  await state.revealSession(first.id);
  assert.deepEqual(revealed, [first.id]);
  assert.equal(state.sidebarOpen.value, true);
  assert.equal(state.selectedWorkspaceId.value, alpha.id);
  assert.deepEqual(opened, []);
  assert.deepEqual(focused, []);
  assert.deepEqual(openIntents, []);
  assert.equal(requests.length, 1);
  assert.deepEqual(state.layout.value, before);
  await state.revealSession("missing");
  assert.deepEqual(revealed, [first.id]);
  assert.equal(requests.length, 1);
});

test("archive and restore PATCH only list metadata while retaining native identity, running state and every open pane", async t => {
  const running = session("fixture/ with space", { status: "running" });
  const { state, requests, openIntents, opened, focused } = await harness(t, {
    sessions: [running, second],
    layout: layoutFor(running),
    fetch: request =>
      Response.json({
        ...running,
        archived_at: request.body.archived ? "2026-09-14T00:00:00Z" : null,
        updated_at: "2026-09-14T00:00:00Z",
      }),
  });
  const beforeLayout = clone(state.layout.value);
  await state.archiveSession(running, true);
  assert.equal(state.sessions.value[0].archived_at, "2026-09-14T00:00:00Z");
  assert.equal(state.sessions.value[0].provider_session_id, running.provider_session_id);
  assert.equal(state.sessions.value[0].configuration_revision, running.configuration_revision);
  assert.equal(state.sessions.value[0].status, "running");
  assert.equal(state.activeSessions.value.length, 1);
  assert.equal(state.sessionNotice.value, "Session archived. History and running agents are unchanged.");
  await state.archiveSession(state.sessions.value[0], false);
  assert.equal(state.sessions.value[0].archived_at, null);
  assert.equal(state.sessionNotice.value, "Session restored to its workspace list.");
  assert.deepEqual(requests, [
    { url: "/api/sessions/fixture%2F%20with%20space/archive", method: "PATCH", body: { archived: true } },
    { url: "/api/sessions/fixture%2F%20with%20space/archive", method: "PATCH", body: { archived: false } },
  ]);
  assert.deepEqual(state.layout.value, beforeLayout);
  assert.deepEqual(state.archiveBusyIds.value, []);
  assert.deepEqual(openIntents, []);
  assert.deepEqual(opened, []);
  assert.deepEqual(focused, []);
  assert.equal(state.sessions.value.length, 2);
});

test("duplicate archive clicks are guarded until confirmation and do not optimistically hide the session", async t => {
  const reply = deferred();
  const { state, requests, openIntents } = await harness(t, { fetch: () => reply.promise });
  const pending = state.archiveSession(first, true);
  assert.deepEqual(state.archiveBusyIds.value, [first.id]);
  assert.equal(state.sessions.value[0].archived_at, null);
  await state.archiveSession(first, false);
  assert.equal(requests.length, 1);
  reply.resolve(Response.json({ ...first, archived_at: "2026-09-14T00:00:00Z" }));
  await pending;
  assert.equal(state.sessions.value[0].archived_at, "2026-09-14T00:00:00Z");
  assert.deepEqual(state.archiveBusyIds.value, []);
  assert.deepEqual(openIntents, []);
});

for (const ordering of ["before", "during"]) {
  test(`a /sessions poll started ${ordering} archiving cannot overwrite the confirmed archived row`, async t => {
    const pollReply = deferred(),
      archiveReply = deferred();
    const updated = { ...first, archived_at: "2026-09-14T00:00:00Z" };
    let pollCount = 0;
    const { state, requests, openIntents } = await harness(t, {
      fetch: request => {
        if (request.url.endsWith("/archive")) return archiveReply.promise;
        if (request.url === "/api/sessions")
          return ++pollCount === 1 ? pollReply.promise : Response.json([updated, second, session("new-session")]);
        throw Error("Unexpected endpoint");
      },
    });
    let poll, archive;
    if (ordering === "before") {
      poll = state.refreshSessions();
      archive = state.archiveSession(first, true);
    } else {
      archive = state.archiveSession(first, true);
      poll = state.refreshSessions();
    }
    assert.equal(state.sessionsLoading.value, true);
    archiveReply.resolve(Response.json(updated));
    await archive;
    assert.equal(state.sessions.value[0].archived_at, updated.archived_at);
    pollReply.resolve(Response.json([first, second]));
    await poll;
    assert.equal(state.sessions.value[0].archived_at, updated.archived_at);
    assert.equal(state.sessionsLoading.value, false);
    await state.refreshSessions();
    assert.equal(state.sessions.value.length, 3, "a later authoritative refresh still applies");
    assert.equal(state.sessions.value[0].archived_at, updated.archived_at);
    assert.equal(requests.length, 3);
    assert.deepEqual(openIntents, []);
  });
}

test("an unadvertised archive capability makes no request and changes no metadata", async t => {
  const { state, requests, openIntents } = await harness(t);
  capabilities.setBackendCapabilities({ ok: true, api_version: 2, capabilities: [] });
  const before = clone(state.sessions.value);
  await state.archiveSession(first, true);
  assert.deepEqual(requests, []);
  assert.deepEqual(openIntents, []);
  assert.deepEqual(state.sessions.value, before);
  assert.deepEqual(state.archiveBusyIds.value, []);
});

test("a lost archive response is reported without retrying, hiding rows, or changing the layout", async t => {
  const { state, requests, openIntents } = await harness(t, {
    fetch: () => {
      throw new TypeError("fixture connection lost");
    },
  });
  const beforeSessions = clone(state.sessions.value),
    beforeLayout = clone(state.layout.value);
  await state.archiveSession(first, true);
  assert.equal(requests.length, 1);
  assert.equal(requests[0].method, "PATCH");
  assert.match(state.error.value, /may have reached the server/);
  assert.equal(state.sessionNotice.value, "");
  assert.deepEqual(state.sessions.value, beforeSessions);
  assert.deepEqual(state.layout.value, beforeLayout);
  assert.deepEqual(state.archiveBusyIds.value, []);
  assert.deepEqual(openIntents, []);
});
