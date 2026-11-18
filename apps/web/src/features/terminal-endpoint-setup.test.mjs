import { after, before, test } from "node:test";
import assert from "node:assert/strict";
import { createServer } from "vite";
import vue from "@vitejs/plugin-vue";
import { createSSRApp } from "vue";
import { renderToString } from "vue/server-renderer";
import { fileURLToPath } from "node:url";
let server, Setup, capabilities;
before(async () => {
  server = await createServer({
    configFile: false,
    root: fileURLToPath(new URL("../../", import.meta.url)),
    plugins: [vue()],
    server: { middlewareMode: true, hmr: false, ws: false },
    appType: "custom",
    optimizeDeps: { noDiscovery: true },
  });
  Setup = (await server.ssrLoadModule("/src/features/TerminalEndpointSetup.vue")).default;
  capabilities = (await server.ssrLoadModule("/src/features/backend-capabilities.ts")).backendCapabilities;
  capabilities.sessionConfiguration = true;
  (await server.ssrLoadModule("/src/i18n/index.ts")).useI18n().setLocale("en");
});
after(async () => {
  await server?.close();
});
const session = (provider, changes = {}) => ({
  id: "terminal-setup",
  provider,
  status: "stopped",
  endpoint_profile_id: null,
  provider_session_id: null,
  ...changes,
});
async function render(props) {
  let state;
  const wrapper = {
    ...Setup,
    setup(props, context) {
      state = Setup.setup(props, context);
      return state;
    },
  };
  const html = await renderToString(createSSRApp(wrapper, props));
  return { html, state };
}
test("the preconnection picker offers only endpoints for its client, for all three agents", async () => {
  const profiles = ["claude_code", "codex", "pi"].map(provider => ({
    id: provider,
    provider,
    name: `${provider} endpoint`,
  }));
  for (const provider of ["claude_code", "codex", "pi"]) {
    const { html } = await render({ session: session(provider), profiles });
    assert.ok(html.includes(`${provider} endpoint`));
    for (const other of profiles.filter(profile => profile.provider !== provider))
      assert.ok(!html.includes(other.name));
    assert.ok(html.includes("Manage endpoint profiles"));
  }
});
test("unchanged endpoints perform no request; changes are saved before returning the session to connect", async t => {
  const calls = [];
  const updated = session("pi", { endpoint_profile_id: "pi-endpoint" });
  t.mock.method(globalThis, "fetch", async (url, init) => {
    calls.push({ url, init });
    return new Response(JSON.stringify(updated), { status: 200 });
  });
  const { state } = await render({ session: session("pi"), profiles: [] });
  assert.equal((await state.prepare()).endpoint_profile_id, null);
  assert.equal(calls.length, 0);
  state.selected.value = "pi-endpoint";
  assert.deepEqual(await state.prepare(), updated);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].url, "/api/sessions/terminal-setup/configuration");
  assert.equal(calls[0].init.method, "PATCH");
  assert.deepEqual(JSON.parse(calls[0].init.body), { endpoint_profile_id: "pi-endpoint", confirmed: true });
});
test("existing native conversations require confirmation, and failed configuration prevents connection", async t => {
  let calls = 0;
  t.mock.method(globalThis, "fetch", async () => {
    calls++;
    return new Response(JSON.stringify({ error: "Cannot change endpoint" }), { status: 409 });
  });
  const { state } = await render({
    session: session("codex", { provider_session_id: "existing-thread" }),
    profiles: [],
  });
  state.selected.value = "new-endpoint";
  assert.equal(await state.prepare(), undefined);
  assert.equal(calls, 0);
  state.confirmed.value = true;
  assert.equal(await state.prepare(), undefined);
  assert.equal(calls, 1);
  assert.equal(state.error.value, "Cannot change endpoint");
});
test("shells, native-history imports and conversation reopens keep their original configuration", async () => {
  for (const fixture of [
    session("terminal"),
    session("pi", { native_source_id: "history" }),
    session("claude_code", { resume_source_id: "source" }),
  ]) {
    const { html } = await render({ session: fixture, profiles: [] });
    assert.ok(!html.includes("<select"));
  }
});
