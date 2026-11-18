import { after, afterEach, before, mock, test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";
import vue from "@vitejs/plugin-vue";
import { createSSRApp, reactive } from "vue";
import { renderToString } from "vue/server-renderer";

let server, CreateDialog, HistoryDialog, capabilities;
const A = "11111111-2222-4333-8444-555555555555",
  B = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee",
  C = "bbbbbbbb-cccc-4ddd-8eee-ffffffffffff";
const workspace = () =>
  reactive({ id: "fixture", name: "Fixture", root_path: "/fixture", created_at: "2026-01-01T00:00:00Z" });
const profile = (id, provider = "claude_code", environment = {}) => ({
  id,
  name: id,
  provider,
  endpoint_url: null,
  model: null,
  permission_mode: "native",
  secret_ref: null,
  native_config: { source_id: id, config_dir: "/fixture/" + id, config_env: null },
  environment,
  created_at: "now",
});
const row = (id, name, value = "", kind = "literal") => ({ id, name, value, kind });
const plain = value => JSON.parse(JSON.stringify(value));

before(async () => {
  server = await createServer({
    configFile: false,
    root: fileURLToPath(new URL("../../", import.meta.url)),
    plugins: [vue()],
    server: { middlewareMode: true, hmr: false, ws: false },
    appType: "custom",
    optimizeDeps: { noDiscovery: true },
  });
  CreateDialog = (await server.ssrLoadModule("/src/features/CreateSessionDialog.vue")).default;
  HistoryDialog = (await server.ssrLoadModule("/src/features/LoadHistoryDialog.vue")).default;
  capabilities = (await server.ssrLoadModule("/src/features/backend-capabilities.ts")).backendCapabilities;
});
after(async () => {
  await server?.close();
});
afterEach(() => {
  mock.restoreAll();
  capabilities.environment = false;
});

async function scenario(Component, props, run) {
  const instrumented = {
    ...Component,
    async setup(componentProps, context) {
      // Exercise the real SFC setup state and request functions before rendering its actual template.
      // onMounted is not run by SSR: history discovery, browsers and native clients are never invoked.
      const bindings = Component.setup(componentProps, context);
      await run(bindings, componentProps);
      return bindings;
    },
  };
  const context = {};
  const html = await renderToString(createSSRApp(instrumented, props), context);
  return html + Object.values(context.teleports ?? {}).join("");
}
function captureRequests() {
  const calls = [];
  mock.method(globalThis, "fetch", async (url, init) => {
    calls.push({ url, body: JSON.parse(init.body) });
    return new Response(
      JSON.stringify({
        id: "fixture-session",
        workspace_id: "fixture",
        provider: "claude_code",
        title: "Fixture",
        status: "stopped",
        created_at: "now",
        updated_at: "now",
        error: null,
      }),
      { status: 200 },
    );
  });
  return calls;
}

test("new-session raw environment drafts survive account switches without crossing provider or profile", async () => {
  capabilities.environment = true;
  await scenario(
    CreateDialog,
    { workspace: workspace(), profiles: [profile(A), profile(B), profile(C, "codex")] },
    async state => {
      state.profileId.value = A;
      const raw = [row("first", " DUPLICATE ", " left "), row("unfinished", "DUPLICATE", ""), row("blank", "", "")];
      state.environmentDraft.value = raw;
      state.profileId.value = B;
      assert.deepEqual(plain(state.environmentDraft.value), []);
      state.environmentDraft.value = [row("other", "OTHER_ACCOUNT", "right")];
      state.profileId.value = A;
      assert.deepEqual(plain(state.environmentDraft.value), raw);
      state.provider.value = "codex";
      assert.deepEqual(plain(state.environmentDraft.value), []);
      state.environmentDraft.value = [row("codex", "CODEX_ONLY", "yes")];
      state.provider.value = "claude_code";
      state.profileId.value = B;
      assert.equal(state.environmentDraft.value[0].name, "OTHER_ACCOUNT");
      state.profileId.value = A;
      assert.deepEqual(plain(state.environmentDraft.value), raw);
    },
  );
});

test("new-session validates the merged default/override count and renders inherited defaults", async () => {
  capabilities.environment = true;
  const defaults = Object.fromEntries(
    Array.from({ length: 64 }, (_, index) => ["SETTING_" + index, { kind: "literal", value: "default" }]),
  );
  const html = await scenario(
    CreateDialog,
    { workspace: workspace(), profiles: [profile(A, "claude_code", defaults)] },
    async state => {
      state.profileId.value = A;
      state.environmentDraft.value = [row("extra", "EXTRA_SETTING", "x")];
      assert.equal(state.validEnvironment.value, false);
      assert.ok(state.environmentErrors.value.includes("At most 64 environment overrides are allowed."));
      state.environmentDraft.value = [row("replace", "SETTING_0", "replacement")];
      assert.equal(state.validEnvironment.value, true);
    },
  );
  assert.ok(html.includes("environment-defaults"));
  assert.ok(html.includes("SETTING_63"));
});

test("new-session validates total merged byte size rather than only the added rows", async () => {
  capabilities.environment = true;
  const defaults = Object.fromEntries(
    Array.from({ length: 8 }, (_, index) => ["SETTING_" + index, { kind: "literal", value: "x".repeat(7800) }]),
  );
  await scenario(
    CreateDialog,
    { workspace: workspace(), profiles: [profile(A, "claude_code", defaults)] },
    async state => {
      state.profileId.value = A;
      state.environmentDraft.value = [row("extra", "EXTRA_SETTING", "y".repeat(4000))];
      assert.equal(state.parsedEnvironment.value.errors.length, 0);
      assert.ok(state.environmentErrors.value.includes("Environment overrides exceed the 64 KiB limit."));
      assert.equal(state.validEnvironment.value, false);
    },
  );
});

test("new-session sends only explicit process overrides; an empty draft omits the environment field", async () => {
  capabilities.environment = true;
  const calls = captureRequests();
  const defaults = { DEFAULT_ONLY: { kind: "literal", value: "profile-default" } };
  await scenario(
    CreateDialog,
    { workspace: workspace(), profiles: [profile(A, "claude_code", defaults)] },
    async state => {
      state.profileId.value = A;
      state.environmentDraft.value = [row("path", "PATH", "$PATH:/fixture/bin")];
      await state.create();
      assert.deepEqual(calls[0].body.environment, { PATH: { kind: "literal", value: "$PATH:/fixture/bin" } });
      assert.equal(calls[0].body.environment.DEFAULT_ONLY, undefined);
      state.environmentDraft.value = [];
      await state.create();
      assert.equal(Object.hasOwn(calls[1].body, "environment"), false);
    },
  );
  assert.ok(calls.every(call => call.url === "/api/workspaces/fixture/sessions"));
});

test("unsupported backends cannot silently discard a nonempty new-session environment draft", async () => {
  capabilities.environment = false;
  const calls = captureRequests();
  const html = await scenario(CreateDialog, { workspace: workspace(), profiles: [profile(A)] }, async state => {
    state.profileId.value = A;
    state.environmentDraft.value = [row("entry", "EXTRA_SETTING", "value")];
    await state.create();
    assert.equal(calls.length, 0);
    assert.equal(state.environmentDraft.value[0].value, "value");
    assert.equal(state.validEnvironment.value, false);
  });
  assert.match(html, /<button\b[^>]*class="primary-button"[^>]*disabled/);
  assert.match(html, /<input\b[^>]*value="EXTRA_SETTING"[^>]*disabled/);
});

test("history environment drafts are scoped by source and workspace and retain unfinished raw rows", async () => {
  capabilities.environment = true;
  await scenario(HistoryDialog, { workspace: workspace() }, async (state, props) => {
    const raw = [row("first", "EXAMPLE", "before"), row("duplicate", "EXAMPLE", "after"), row("blank", "", "")];
    state.sourceId.value = "source-one";
    state.environmentDraft.value = raw;
    state.sourceId.value = "source-two";
    assert.deepEqual(plain(state.environmentDraft.value), []);
    state.environmentDraft.value = [row("second", "OTHER", "two")];
    state.sourceId.value = "source-one";
    assert.deepEqual(plain(state.environmentDraft.value), raw);
    props.workspace.id = "different-workspace";
    assert.deepEqual(plain(state.environmentDraft.value), []);
    props.workspace.id = "fixture";
    assert.deepEqual(plain(state.environmentDraft.value), raw);
  });
});

test("history import keeps consent and omits empty environment on repeated imports", async () => {
  capabilities.environment = true;
  const calls = captureRequests();
  await scenario(HistoryDialog, { workspace: workspace() }, async state => {
    state.sourceId.value = "source-one";
    state.selection.select("fixture", "source-one", {
      id: "history-one",
      title: "Old history",
      provider: "claude_code",
      cwd: "/fixture",
      updated_at: "now",
      imported_session_id: "existing",
    });
    state.confirm.value = true;
    await state.importSelected();
    assert.equal(Object.hasOwn(calls[0].body, "environment"), false);
    assert.equal(calls[0].body.confirmed_original_config, true);
    state.environmentDraft.value = [row("entry", "EXTRA_SETTING", "new-value")];
    await state.importSelected();
    assert.deepEqual(calls[1].body.environment, { EXTRA_SETTING: { kind: "literal", value: "new-value" } });
  });
  assert.ok(calls.every(call => call.url === "/api/workspaces/fixture/native-history/import"));
});

test("unsupported history environment drafts block import without clearing raw values", async () => {
  capabilities.environment = false;
  const calls = captureRequests();
  await scenario(HistoryDialog, { workspace: workspace() }, async state => {
    state.sourceId.value = "source-one";
    state.selection.select("fixture", "source-one", {
      id: "history-one",
      title: "Old history",
      provider: "claude_code",
      cwd: "/fixture",
      updated_at: "now",
      imported_session_id: null,
    });
    state.confirm.value = true;
    state.environmentDraft.value = [row("entry", "EXTRA_SETTING", "kept")];
    await state.importSelected();
    assert.equal(calls.length, 0);
    assert.equal(state.environmentDraft.value[0].value, "kept");
    state.environmentDraft.value = [];
    await state.importSelected();
    assert.equal(calls.length, 1);
    assert.equal(Object.hasOwn(calls[0].body, "environment"), false);
  });
});
