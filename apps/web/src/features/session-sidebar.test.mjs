import { after, before, test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import vue from "@vitejs/plugin-vue";
import { createSSRApp } from "vue";
import { renderToString } from "vue/server-renderer";

let server, Sidebar, SessionRow, SessionsPage, i18n;
const workspace = {
  id: "sidebar-workspace",
  name: "Fixture workspace",
  root_path: "/fixture/sidebar",
  created_at: "2026-09-01T00:00:00Z",
};
const session = (id, changes = {}) => ({
  id,
  workspace_id: workspace.id,
  title: `Fixture ${id}`,
  provider: "codex",
  status: "stopped",
  endpoint_profile_id: null,
  provider_session_id: null,
  error: null,
  created_at: "2026-09-01T00:00:00Z",
  updated_at: "2026-09-01T00:00:00Z",
  ...changes,
});
const current = session("current");
const archived = session("archived", { archived_at: "2026-09-12T00:00:00Z", status: "running" });
let renderId = 0;

before(async () => {
  server = await createServer({
    configFile: false,
    root: fileURLToPath(new URL("../../", import.meta.url)),
    plugins: [vue()],
    server: { middlewareMode: true, hmr: false, ws: false },
    appType: "custom",
    optimizeDeps: { noDiscovery: true },
  });
  Sidebar = (await server.ssrLoadModule("/src/features/WorkspaceSidebar.vue")).default;
  SessionRow = (await server.ssrLoadModule("/src/features/SidebarSessionRow.vue")).default;
  SessionsPage = (await server.ssrLoadModule("/src/features/SessionsPage.vue")).default;
  i18n = (await server.ssrLoadModule("/src/i18n/index.ts")).useI18n();
  i18n.setLocale("en");
});
after(async () => {
  await server?.close();
});

async function render(component, props, configure) {
  let state, exposed;
  const wrapper = {
    ...component,
    async setup(props, context) {
      state = component.setup(props, {
        ...context,
        expose(value) {
          exposed = value;
          context.expose(value);
        },
      });
      await configure?.(state);
      return state;
    },
  };
  const html = await renderToString(createSSRApp(wrapper, props));
  return { html, state, exposed };
}
const sidebarProps = changes => ({
  workspaces: [workspace],
  sessions: [current, archived],
  selectedWorkspaceId: workspace.id,
  archiveSupported: true,
  storageKey: `session-sidebar-test-${++renderId}`,
  ...changes,
});

test("All sessions is a page now: the sidebar links to it and lists no library of its own", async t => {
  let requests = 0,
    opened = 0;
  t.mock.method(globalThis, "fetch", async () => {
    requests++;
    throw Error("unexpected request");
  });
  const { html } = await render(Sidebar, sidebarProps({ currentPage: "sessions", onAllSessions: () => opened++ }));
  assert.match(html, /aria-current="page"[^>]*>.*All sessions/s);
  assert.doesNotMatch(
    html,
    /sidebar-session-library|all-sessions-list|role="dialog"|data-sidebar-session-id="archived"/,
  );
  assert.equal(requests, 0);
});

test("the session library page filters by archive state and fuzzy query without duplicating rows", async () => {
  const { html, state } = await render(
    SessionsPage,
    { workspaces: [workspace], sessions: [current, archived], archiveSupported: true },
    state => {
      state.archive.value = "archived";
      state.query.value = "fxtr arcvd";
    },
  );
  assert.deepEqual(
    state.filtered.value.map(entry => entry.session.id),
    [archived.id],
  );
  assert.equal((html.match(/<strong[^>]*>Fixture archived<\/strong>/g) ?? []).length, 1);
  for (const label of ["workspace", "provider", "status", "archive state"])
    assert.ok(html.includes(`Filter sessions by ${label}`));
});

test("archive and restore actions emit only reversible metadata intent and reject unsupported or busy attempts", async t => {
  let requests = 0;
  t.mock.method(globalThis, "fetch", async () => {
    requests++;
    throw Error("unexpected request");
  });
  for (const fixture of [current, archived]) {
    const changes = [];
    const { html, state } = await render(
      SessionRow,
      { session: fixture, archiveSupported: true, onArchive: (...args) => changes.push(args) },
      state => state.toggleMenu(),
    );
    assert.ok(html.includes(fixture.archived_at ? "Restore session" : "Archive session"));
    state.archiveSession();
    assert.deepEqual(changes, [[fixture, !fixture.archived_at]]);
    assert.equal(state.menuOpen.value, false);
  }
  for (const props of [{ archiveSupported: false }, { archiveSupported: true, archiveBusy: true }]) {
    const changes = [];
    const { html, state } = await render(
      SessionRow,
      { session: current, ...props, onArchive: (...args) => changes.push(args) },
      state => state.toggleMenu(),
    );
    assert.match(html, /<button[^>]*class="session-archive-action"[^>]*\bdisabled\b/);
    state.archiveSession();
    assert.deepEqual(changes, []);
  }
  assert.equal(requests, 0);
});

test("sidebar guards duplicate or unsupported archive intent as well as row controls", async () => {
  for (const props of [{ archiveSupported: false }, { archiveBusyIds: [current.id] }]) {
    const events = [];
    const { state } = await render(
      Sidebar,
      sidebarProps({ ...props, onArchiveSession: (...args) => events.push(args) }),
    );
    state.archiveSession(current, true);
    assert.deepEqual(events, []);
  }
  const events = [];
  const { state } = await render(Sidebar, sidebarProps({ onArchiveSession: (...args) => events.push(args) }));
  state.archiveSession(current, true);
  assert.deepEqual(events, [[current, true]]);
});

test("session environment and Escape stay accessible from the row menu without opening a client", async () => {
  const environment = [],
    opened = [];
  const { html, state } = await render(
    SessionRow,
    {
      session: current,
      archiveSupported: true,
      onEnvironment: id => environment.push(id),
      onOpen: value => opened.push(value),
    },
    state => state.toggleMenu(),
  );
  assert.ok(html.includes("Session actions for Fixture current"));
  // Clicking the row already brings its view forward, so the menu no longer
  // repeats it as a separate "locate" item.
  assert.ok(!html.includes("Locate in canvas"));
  assert.ok(html.includes("Session environment"));
  state.environment();
  assert.deepEqual(environment, [current.id]);
  assert.deepEqual(opened, []);
  let focused = 0,
    prevented = 0,
    stopped = 0;
  state.menuButton.value = {
    focus() {
      focused++;
    },
  };
  state.toggleMenu();
  state.escapeMenu({
    key: "Escape",
    preventDefault() {
      prevented++;
    },
    stopPropagation() {
      stopped++;
    },
  });
  await Promise.resolve();
  assert.equal(state.menuOpen.value, false);
  assert.equal(prevented, 1);
  assert.equal(stopped, 1);
  assert.ok(focused >= 1);
});

test("session rows expose a rename action without opening or changing the session", async () => {
  const renamed = [];
  const { html, state } = await render(
    SessionRow,
    { session: current, archiveSupported: true, onRename: (...args) => renamed.push(args) },
    state => state.toggleMenu(),
  );
  assert.ok(html.includes("Rename session"));
  state.beginRename();
  assert.equal(state.renaming.value, true);
  assert.equal(state.titleDraft.value, current.title);
  state.titleDraft.value = "  Concrete Claude task  ";
  state.submitRename();
  assert.equal(state.renaming.value, false);
  assert.deepEqual(renamed, [[current, "Concrete Claude task"]]);
});

test("revealSession expands and scrolls the matching row while preserving canvas selection and CLI state", async () => {
  const events = [],
    scrolls = [],
    focuses = [];
  const { state, exposed } = await render(
    Sidebar,
    sidebarProps({ onSelectWorkspace: id => events.push(id), onOpenSession: value => events.push(value) }),
  );
  state.query.value = "nonmatching search";
  state.preferences.value.expanded[workspace.id] = false;
  state.sidebarElement.value = {
    querySelector(selector) {
      assert.equal(selector, ".workspace-groups");
      return {
        querySelectorAll() {
          return [
            {
              dataset: { sidebarSessionId: current.id },
              scrollIntoView(options) {
                scrolls.push(options);
              },
              querySelector() {
                return {
                  focus(options) {
                    focuses.push(options);
                  },
                };
              },
            },
          ];
        },
      };
    },
  };
  assert.equal(await exposed.revealSession(current.id), true);
  assert.equal(state.query.value, "");
  assert.equal(state.preferences.value.expanded[workspace.id], true);
  assert.equal(scrolls.length, 1);
  assert.deepEqual(focuses, [{ preventScroll: true }]);
  assert.deepEqual(events, []);
  assert.equal(await exposed.revealSession("missing"), false);
});

test("revealSession sends an archived session to the library page, filtered to archived", async () => {
  const shown = [];
  const { exposed } = await render(Sidebar, sidebarProps({ onShowInSessions: (...args) => shown.push(args) }));
  assert.equal(await exposed.revealSession(archived.id), true);
  assert.deepEqual(shown, [[archived.id, true]]);
});

test("bilingual row actions translate application text but preserve session titles", async () => {
  const title = "Keep original 标题";
  for (const [locale, label] of [
    ["zh-CN", "归档会话"],
    ["en", "Archive session"],
  ]) {
    i18n.setLocale(locale);
    const { html } = await render(SessionRow, { session: { ...current, title }, archiveSupported: true }, state =>
      state.toggleMenu(),
    );
    assert.ok(html.includes(label));
    assert.ok(html.includes(title));
  }
  i18n.setLocale("en");
});

test("delete is offered only for archived or temporary sessions and asks before emitting", async () => {
  const { html: currentHtml } = await render(SessionRow, { session: current, archiveSupported: true }, state =>
    state.toggleMenu(),
  );
  assert.doesNotMatch(currentHtml, /session-delete-action/);
  const deleted = [];
  const { html, state } = await render(
    SessionRow,
    { session: archived, archiveSupported: true, onDelete: value => deleted.push(value) },
    state => state.toggleMenu(),
  );
  assert.ok(html.includes("Delete session…"));
  state.confirmDelete.value = true;
  state.deleteSession();
  assert.deepEqual(deleted, [archived]);
});

test("select mode archives, restores and deletes only the ticked rows that allow it", async () => {
  const events = [];
  const { state } = await render(
    SessionsPage,
    {
      workspaces: [workspace],
      sessions: [current, archived],
      archiveSupported: true,
      onArchiveSessions: (...args) => events.push(["archive", ...args]),
      onDeleteSessions: list => events.push(["delete", list]),
    },
    state => {
      state.archive.value = "all";
    },
  );
  state.checkAll();
  assert.deepEqual(state.checkedIds.value.sort(), [archived.id, current.id].sort());
  state.invert();
  assert.deepEqual(state.checkedIds.value, []);
  state.checkAll();
  state.bulkArchive(true);
  assert.deepEqual(events.shift(), ["archive", [current], true]);
  state.checkAll();
  state.bulkDelete();
  assert.deepEqual(events.shift(), ["delete", [archived]]);
});

test("Claude Code, Codex and pi share identity actions in the sidebar, including the exact native ID", async () => {
  for (const provider of ["claude_code", "codex", "pi"]) {
    const native = `native-${provider}`;
    const { html } = await render(
      SessionRow,
      { session: session(provider, { provider, provider_session_id: native }) },
      state => state.toggleMenu(),
    );
    const client = { claude_code: "Claude Code", codex: "Codex", pi: "Pi" }[provider];
    // What to copy (the resume command, the exact native ID) opens from one "Copy" row.
    for (const label of ["Rename session", "Session environment", "Copy", "Run information"])
      assert.ok(html.includes(label), `${provider}: ${label}`);
    assert.ok(html.includes("menu-flyout-trigger"), `${provider}: copying is a row with its own submenu`);
    assert.ok(client);
  }
  const { html } = await render(SessionRow, { session: current }, state => state.toggleMenu());
  assert.ok(!/Copy [^"<]* session ID/.test(html), "a new session has no native ID to copy");
  assert.ok(!html.includes('menu-flyout-label">Copy<'), "nor a conversation to resume, so no copy row");
});

test("a row says what was last asked there, and archives in one step", async () => {
  const { html } = await render(SessionRow, {
    session: session("claude_code", { title: "Claude Code", preview: "why is the build red" }),
    archiveSupported: true,
  });
  assert.ok(html.includes("why is the build red"), "the last thing said tells alike-named rows apart");
  assert.ok(html.includes("session-archive-quick"), "archive sits on the row, not inside its menu");
  assert.ok(html.includes("session-swipe-action"), "and is what a swipe reveals on a phone");
  const { html: bare } = await render(SessionRow, { session: session("claude_code", { title: "Claude Code" }) });
  assert.ok(!bare.includes("workspace-session-preview"), "nothing said yet means no second line");
  assert.ok(!bare.includes("session-archive-quick"), "and no archive where the server cannot");
});
