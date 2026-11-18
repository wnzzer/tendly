import { after, before, test } from "node:test";
import assert from "node:assert/strict";
import { createServer } from "vite";
import vue from "@vitejs/plugin-vue";
import { createSSRApp } from "vue";
import { renderToString } from "vue/server-renderer";
import { fileURLToPath } from "node:url";
import { readFileSync } from "node:fs";
import { styleOf } from "../source-text.mjs";

let server, SessionPane, model, i18n, capabilities;
before(async () => {
  server = await createServer({
    configFile: false,
    root: fileURLToPath(new URL("../../", import.meta.url)),
    plugins: [vue()],
    server: { middlewareMode: true, hmr: false, ws: false },
    appType: "custom",
    optimizeDeps: { noDiscovery: true },
  });
  SessionPane = (await server.ssrLoadModule("/src/features/SessionPane.vue")).default;
  model = await server.ssrLoadModule("/src/features/session-view.ts");
  i18n = await server.ssrLoadModule("/src/i18n/index.ts");
  i18n.useI18n().setLocale("en");
  capabilities = await server.ssrLoadModule("/src/features/backend-capabilities.ts");
  capabilities.setBackendCapabilities({
    ok: true,
    api_version: 2,
    capabilities: ["structured_chat", "session_configuration"],
  });
  model.resetSessionViewsForTests();
});
after(async () => {
  await server?.close();
});

const fixture = (changes = {}) => ({
  id: "session-view-fixture",
  workspace_id: "workspace-fixture",
  provider: "codex",
  title: "Fixture session",
  status: "stopped",
  created_at: "2026-09-09T00:00:00Z",
  updated_at: "2026-09-09T00:00:00Z",
  endpoint_profile_id: null,
  provider_session_id: null,
  error: null,
  interaction_mode: "pty",
  configuration_revision: 3,
  ...changes,
});
async function render(props) {
  const context = {};
  const html = await renderToString(createSSRApp(SessionPane, props), context);
  return html + Object.values(context.teleports ?? {}).join("");
}
async function renderWithState(props, change) {
  const instrumented = {
    ...SessionPane,
    setup(componentProps, context) {
      const bindings = SessionPane.setup(componentProps, context);
      change(bindings);
      return bindings;
    },
  };
  const context = {};
  const html = await renderToString(createSSRApp(instrumented, props), context);
  return { html, dialog: context.teleports?.body ?? "" };
}

test("non-terminal PTY sessions start in the chat view and preferences are isolated by session id", () => {
  const a = fixture({ id: "a" }),
    b = fixture({ id: "b" });
  assert.equal(model.sessionView(a), "chat");
  assert.equal(model.sessionView(b), "chat");
  model.setSessionView(a, "native");
  assert.equal(model.sessionView(a), "native");
  assert.equal(model.sessionView(b), "chat");
  assert.equal(model.sessionView({ ...a, interaction_mode: "structured" }), "chat");
});

test("terminal sessions remain native and cannot be switched to a chat view", () => {
  const terminal = fixture({ provider: "terminal" });
  assert.equal(model.defaultSessionView(terminal), "native");
  assert.equal(model.setSessionView(terminal, "chat"), "native");
  assert.equal(model.sessionView(terminal), "native");
});

test("an escape-hatch session opens as a terminal, not as a second copy of its conversation", () => {
  // It runs the client interactively on a conversation that is already on
  // screen in the structured session it came from. Routing it to the chat view
  // would defeat the entire point of opening it.
  const reopen = fixture({
    id: "reopen",
    provider: "claude_code",
    interaction_mode: "pty",
    resume_source_id: "source-session",
    provider_session_id: "thread_01",
  });
  assert.equal(model.defaultSessionView(reopen), "native");
  assert.equal(model.sessionView(reopen), "native");
  // Not even a stale preference from an earlier session may send it to chat.
  model.setSessionView(reopen, "chat");
  assert.equal(model.sessionView(reopen), "native");
  model.resetSessionViewsForTests();
});

test("an escape-hatch session offers no chat view of its own", async () => {
  const reopen = fixture({
    id: "reopen-shell",
    provider: "claude_code",
    interaction_mode: "pty",
    resume_source_id: "source-session",
    provider_session_id: "thread_01",
  });
  const html = await render({ session: reopen, sessions: [reopen], profiles: [] });
  assert.match(html, /data-view="native"/);
  assert.ok(html.includes('class="agent-session-pane"'));
  assert.doesNotMatch(html, />Chat</, "its conversation already has a pane under the session it reopened");
});

test("SSR shell binds both surfaces to the same session id without duplicate headings or a permanent information row", async () => {
  model.resetSessionViewsForTests();
  const html = await render({ session: fixture(), sessions: [fixture()], profiles: [] });
  assert.ok(
    (html.match(/data-session-id="session-view-fixture"/g) ?? []).length >= 2,
    "shell and child surface stay bound to one session",
  );
  assert.match(html, /Run information/);
  assert.doesNotMatch(
    html,
    /session-shell-bar|session-shell-badge|session-runtime-details|session-runtime-info|Provider session ID|Configuration revision/,
  );
  assert.equal(
    (html.match(/class="chat-header"/g) ?? []).length,
    0,
    "the conversation header is represented by the tab and floating actions",
  );
  assert.ok(html.includes("chat-composer"));
  assert.ok(!html.includes('class="agent-session-pane"'));
});

test("run information opens as a read-only modal for the bound session without fetching or changing views", async () => {
  const calls = [];
  const previousFetch = globalThis.fetch;
  globalThis.fetch = async (...args) => {
    calls.push(args);
    throw new Error("No network is expected for run information");
  };
  try {
    for (const chosenView of ["chat", "native"]) {
      const session = fixture({ id: `run-info-${chosenView}`, provider_session_id: `native-id-${chosenView}` });
      model.setSessionView(session, chosenView);
      let closed = 0;
      const { html, dialog } = await renderWithState({ session, sessions: [session], profiles: [] }, state => {
        assert.equal(state.showInfo.value, false);
        state.showRunInfo(() => {
          closed++;
        });
        assert.equal(state.showInfo.value, true);
      });
      assert.equal(closed, 1, "opening information closes the source actions menu");
      assert.match(html, new RegExp(`data-view="${chosenView}"`));
      assert.match(dialog, /role="dialog"/);
      assert.match(dialog, /aria-label="Run information"/);
      assert.match(dialog, new RegExp(`data-session-id="run-info-${chosenView}"`));
      assert.match(dialog, new RegExp(`native-id-${chosenView}`));
      assert.match(dialog, /Tendly session ID/);
      assert.match(dialog, /(Claude Code|Codex|Pi) session ID/);
      assert.match(dialog, new RegExp(`tendly resume run-info-${chosenView}`));
      assert.match(dialog, /workspace-fixture/);
      assert.match(dialog, /Configuration revision/);
      assert.match(dialog, /aria-label="Close Run information"/, "the information panel has a close action");
    }
    assert.deepEqual(calls, [], "displaying local session metadata never starts, stops, or reloads a process");
  } finally {
    globalThis.fetch = previousFetch;
  }
});

test("structured sessions have no misleading terminal label or native-view control", async () => {
  const session = fixture({ id: "structured-view-fixture", interaction_mode: "structured" });
  const html = await render({ session, sessions: [session], profiles: [] });
  assert.match(html, /data-view="chat"/);
  assert.doesNotMatch(html, /session-shell-mode|Native terminal|Open native client view/);
});

test("SSR shell keeps a terminal session native", async () => {
  const terminal = fixture({ id: "terminal-view-fixture", provider: "terminal" });
  const html = await render({ session: terminal, sessions: [terminal], profiles: [] });
  assert.match(html, /data-session-id="terminal-view-fixture"/);
  assert.ok(html.includes('class="agent-session-pane"'));
  assert.ok(!html.includes("chat-composer"));
  assert.match(html, /Run information/);
  assert.doesNotMatch(html, />Chat<|session-shell-bar|session-runtime-info/);
});

test("switching the native view back to Chat retains the session id without process API calls", async () => {
  const session = fixture({ id: "switch-view-fixture" });
  model.setSessionView(session, "native");
  const nativeHtml = await render({ session, sessions: [session], profiles: [] });
  assert.match(nativeHtml, />Chat<|>Chat<!--/);
  const previousFetch = globalThis.fetch;
  const calls = [];
  globalThis.fetch = async (...args) => {
    calls.push(args);
    throw new Error("View switching does not make network requests");
  };
  try {
    const { html } = await renderWithState({ session, sessions: [session], profiles: [] }, state => state.openChat());
    assert.match(html, /data-view="chat"/);
    assert.equal((html.match(/data-session-id="switch-view-fixture"/g) ?? []).length, 2);
    assert.deepEqual(calls, []);
  } finally {
    globalThis.fetch = previousFetch;
  }
});

test("session shell gives its Chat/Native child a real flex width instead of collapsing chat to a vertical strip", () => {
  const source = styleOf(new URL("./SessionPane.vue", import.meta.url));
  assert.match(source, /\.session-shell-content>\*\{flex:1 1 auto;width:auto;min-width:0;min-height:0\}/);
});
