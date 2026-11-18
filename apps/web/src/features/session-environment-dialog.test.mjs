import { after, before, test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import vue from "@vitejs/plugin-vue";
import { createSSRApp } from "vue";
import { renderToString } from "vue/server-renderer";

let server, Dialog, Editor, Sidebar, capabilities, i18n;
const fixture = {
  id: "fixture-session",
  workspace_id: "fixture-workspace",
  provider: "claude_code",
  title: "Fixture session",
  status: "stopped",
  environment: {
    LANG: { kind: "literal", value: "zh_CN.UTF-8" },
    OPENAI_API_KEY: { kind: "secret_ref", reference: "env:TENDLY_SECRET_FIXTURE" },
  },
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
};
before(async () => {
  // Render actual SFCs without HTTP/HMR listeners, native clients or real credentials.
  server = await createServer({
    configFile: false,
    root: fileURLToPath(new URL("../../", import.meta.url)),
    plugins: [vue()],
    server: { middlewareMode: true, hmr: false, ws: false },
    appType: "custom",
    optimizeDeps: { noDiscovery: true },
  });
  Dialog = (await server.ssrLoadModule("/src/features/SessionEnvironmentDialog.vue")).default;
  Editor = (await server.ssrLoadModule("/src/features/EnvironmentEditor.vue")).default;
  Sidebar = (await server.ssrLoadModule("/src/features/WorkspaceSidebar.vue")).default;
  capabilities = await server.ssrLoadModule("/src/features/backend-capabilities.ts");
  i18n = (await server.ssrLoadModule("/src/i18n/index.ts")).useI18n();
});
after(async () => {
  await server?.close();
});
async function render(component, props) {
  let state;
  const wrapper = {
    ...component,
    setup(props, ctx) {
      state = component.setup(props, ctx);
      return state;
    },
  };
  const context = {};
  const html = await renderToString(createSSRApp(wrapper, props), context);
  return { html: html + Object.values(context.teleports ?? {}).join(""), state };
}
const saveButton = html => html.match(/<button\b[^>]*class="primary-button"[^>]*>/)?.[0];

test("stopped session editor is expanded, bilingual and shows references only", async () => {
  capabilities.setBackendCapabilities({ ok: true, capabilities: ["session_environment"] });
  for (const [locale, label] of [
    ["zh-CN", "会话环境配置"],
    ["en", "Session environment"],
  ]) {
    i18n.setLocale(locale);
    const { html } = await render(Dialog, { session: fixture });
    assert.ok(html.includes(label));
    assert.match(html, /<details\b[^>]*class="environment-editor"[^>]*\bopen\b/);
    assert.ok(html.includes("env:TENDLY_SECRET_FIXTURE"));
    assert.ok(!/\bdisabled\b/.test(saveButton(html)));
  }
});

test("live sessions and old backends keep editing and saving read-only without any request", async t => {
  let requests = 0;
  t.mock.method(globalThis, "fetch", async () => {
    requests++;
    throw new Error("unexpected network");
  });
  for (const status of ["starting", "running", "waiting"]) {
    capabilities.setBackendCapabilities({ ok: true, capabilities: ["session_environment"] });
    const { html, state } = await render(Dialog, { session: { ...fixture, status } });
    assert.match(saveButton(html), /\bdisabled\b/);
    assert.ok((html.match(/<input\b[^>]*>/g) ?? []).every(tag => /\bdisabled\b/.test(tag)));
    await state.save();
  }
  capabilities.setBackendCapabilities({ ok: true, api_version: 2, capabilities: [] });
  const { html, state } = await render(Dialog, { session: fixture });
  assert.match(saveButton(html), /\bdisabled\b/);
  await state.save();
  assert.equal(requests, 0);
});

test("saving stopped-session environment only PATCHes explicit map and emits metadata, never starts or stops", async t => {
  capabilities.setBackendCapabilities({ ok: true, capabilities: ["session_environment"] });
  const requests = [],
    saved = [];
  t.mock.method(globalThis, "fetch", async (url, init) => {
    requests.push({ url, init });
    return Response.json({ ...fixture, environment: {}, updated_at: "2026-01-02T00:00:00Z" });
  });
  const { state } = await render(Dialog, { session: fixture, onSaved: s => saved.push(s) });
  state.rows.value = [];
  await state.save();
  assert.equal(requests.length, 1);
  assert.equal(requests[0].url, "/api/sessions/fixture-session/environment");
  assert.equal(requests[0].init.method, "PATCH");
  assert.deepEqual(JSON.parse(requests[0].init.body), { environment: {} });
  assert.equal(saved.length, 1);
  assert.equal(saved[0].status, "stopped");
  assert.deepEqual(fixture.environment.LANG, { kind: "literal", value: "zh_CN.UTF-8" });
});

test("validation and a lost save response do not auto-retry mutations or discard drafts", async t => {
  capabilities.setBackendCapabilities({ ok: true, capabilities: ["session_environment"] });
  let requests = 0;
  t.mock.method(globalThis, "fetch", async () => {
    requests++;
    throw new TypeError("network lost");
  });
  const { state } = await render(Dialog, { session: fixture });
  state.rows.value = [{ id: "one", name: "HOME", kind: "literal", value: "/wrong" }];
  await state.save();
  assert.equal(requests, 0);
  const rows = [{ id: "one", name: "LANG", kind: "literal", value: "en_US.UTF-8" }];
  state.rows.value = rows;
  await state.save();
  assert.equal(requests, 1);
  assert.deepEqual(state.rows.value, rows);
  assert.ok(state.error.value);
  assert.equal(state.busy.value, false);
});

test("shared editor keeps invalid rows visible, escapes literals, and shows validation", async () => {
  i18n.setLocale("zh-CN");
  const rows = [
    { id: "a", name: "", kind: "literal", value: "<script>fixture</script>" },
    { id: "b", name: "OPENAI_API_KEY", kind: "literal", value: "synthetic-only" },
  ];
  const { html } = await render(Editor, { modelValue: rows, expanded: true });
  assert.equal((html.match(/class="environment-row"/g) ?? []).length, 2);
  assert.ok(html.includes("&lt;script&gt;fixture&lt;/script&gt;"));
  assert.ok(html.includes("敏感变量必须使用密钥引用"));
  assert.ok(html.includes("变量名须以字母或下划线开头"));
});

test("sidebar keeps compact session actions separate from opening a stopped session", async () => {
  i18n.setLocale("en");
  const workspace = {
    id: fixture.workspace_id,
    name: "Fixture",
    root_path: "/fixture",
    created_at: fixture.created_at,
  };
  const { html } = await render(Sidebar, {
    workspaces: [workspace],
    sessions: [fixture],
    selectedWorkspaceId: workspace.id,
    selectedSessionId: fixture.id,
  });
  assert.ok(html.includes('aria-label="Session actions for Fixture session"'));
  assert.match(html, /<button\b[^>]*session-more-button[^>]*aria-expanded="false"/);
  // The row keeps only what it needs: locating in the canvas is a menu item
  // now, so a long session title gets that width instead.
  assert.equal(html.includes("session-locate-button"), false);
  assert.equal(html.includes("Rename session"), false, "closed menus render no items");
});
