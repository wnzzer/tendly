import { test } from "node:test";
import assert from "node:assert/strict";
import { paletteItems, shortcutHint } from "./palette-items.ts";

const t = (key, values = {}) => key.replace(/\{(\w+)\}/g, (_, name) => values[name] ?? "");
const here = { id: "w", name: "Repo", root_path: "/repo" };
const session = (id, extra = {}) => ({
  id,
  title: id,
  provider: "claude_code",
  status: "idle",
  workspace_id: "w",
  updated_at: "2026-01-01T00:00:00Z",
  ...extra,
});

function build(input = {}) {
  const calls = [];
  const record =
    name =>
    (...args) =>
      calls.push([name, ...args]);
  const actions = Object.fromEntries(
    [
      "openSession",
      "quickSession",
      "newSession",
      "openPage",
      "openCanvas",
      "openSettings",
      "openWorkspace",
      "addWorkspace",
      "toggleSidebar",
      "toggleFiles",
      "setTheme",
      "showShortcuts",
    ].map(name => [name, record(name)]),
  );
  const items = paletteItems({ sessions: [], workspaces: [here], here, t, ...input }, actions);
  return { items, calls, byId: id => items.find(item => item.id === id) };
}

test("sessions not archived are listed, those waiting on you in a group of their own and suggested", () => {
  const { items, byId } = build({
    sessions: [
      session("idle"),
      session("waiting", { status: "waiting" }),
      session("old", { archived_at: "2026-01-02T00:00:00Z" }),
    ],
  });
  assert.equal(byId("session:old"), undefined);
  assert.equal(byId("session:waiting").group, "Needs you");
  assert.equal(byId("session:waiting").suggested, true);
  assert.equal(byId("session:waiting").subtitle, "Claude Code · Repo · Needs you");
  assert.equal(byId("session:idle").group, "Sessions");
  assert.equal(items[0].id, "session:idle", "sessions keep the list's order");
});

test("only the six most recently updated sessions are suggested", () => {
  const sessions = Array.from({ length: 8 }, (_, i) => session(`s${i}`, { updated_at: `2026-01-0${i + 1}T00:00:00Z` }));
  const { items } = build({ sessions });
  assert.deepEqual(
    items.filter(item => item.id.startsWith("session:") && item.suggested).map(item => item.id),
    ["s2", "s3", "s4", "s5", "s6", "s7"].map(id => `session:${id}`),
  );
});

test("new sessions are offered in the workspace in view, and not without one", () => {
  const { byId, calls } = build();
  assert.equal(byId("new:terminal").title, "New terminal");
  assert.equal(byId("new:terminal").suggested, false);
  assert.equal(byId("new:codex").subtitle, "Repo");
  byId("new:codex").run();
  byId("new:options").run();
  assert.deepEqual(calls, [
    ["quickSession", "w", "codex"],
    ["newSession", "w"],
  ]);
  assert.equal(byId("new:options").hint, shortcutHint("new-session"));
  assert.equal(
    build({ here: undefined }).items.some(item => item.id.startsWith("new:")),
    false,
  );
});

test("pages, settings, workspaces and commands each run their action", () => {
  const { byId, calls } = build();
  for (const id of [
    "page:usage",
    "page:canvas",
    "settings:accounts",
    "workspace:w",
    "add-workspace",
    "toggle-files",
    "theme:dark",
    "shortcuts",
  ])
    byId(id).run();
  assert.deepEqual(calls, [
    ["openPage", "usage"],
    ["openCanvas"],
    ["openSettings", "accounts"],
    ["openWorkspace", "w"],
    ["addWorkspace"],
    ["toggleFiles"],
    ["setTheme", "dark"],
    ["showShortcuts"],
  ]);
  assert.equal(byId("settings:preferences").hint, shortcutHint("settings"));
  assert.equal(byId("settings:accounts").hint, undefined);
  assert.equal(shortcutHint("no-such-command"), undefined);
});
