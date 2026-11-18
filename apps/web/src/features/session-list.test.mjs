import { test } from "node:test";
import assert from "node:assert/strict";
import { filterSessionList, isSessionArchived } from "./session-list.ts";

const workspace = (id, name = id) => ({ id, name, root_path: `/projects/${id}`, created_at: "2026-09-01T00:00:00Z" });
const session = (id, workspace_id, changes = {}) => ({
  id,
  workspace_id,
  title: id,
  provider: "codex",
  status: "stopped",
  created_at: "2026-09-01T00:00:00Z",
  updated_at: "2026-09-01T00:00:00Z",
  endpoint_profile_id: null,
  provider_session_id: null,
  error: null,
  ...changes,
});
const workspaces = [workspace("frontend", "Frontend app"), workspace("backend", "后端服务")];
const sessions = [
  session("one", "frontend", { title: "Fix toolbar", status: "running" }),
  session("two", "backend", { title: "修复实名认证", provider: "claude_code", status: "waiting" }),
  session("old", "frontend", { title: "Archived payment", archived_at: "2026-09-02T00:00:00Z", status: "running" }),
  session("shell", "backend", { title: "Host shell", provider: "terminal" }),
];
const ids = (options = {}, items = sessions) =>
  filterSessionList(items, workspaces, options).map(entry => entry.session.id);

test("current list hides archived sessions regardless of whether their agent is running", () => {
  assert.deepEqual(ids(), ["one", "two", "shell"]);
  assert.deepEqual(ids({ archive: "archived" }), ["old"]);
  assert.deepEqual(ids({ archive: "all" }), ["one", "two", "old", "shell"]);
  assert.equal(isSessionArchived({}), false);
  assert.equal(isSessionArchived({ archived_at: null }), false);
  assert.equal(isSessionArchived(sessions[2]), true);
});

test("global search combines fuzzy title, provider, workspace and localized status metadata", () => {
  assert.deepEqual(ids({ query: "frnt tlbr cdx running" }), ["one"]);
  assert.deepEqual(ids({ query: "实名认证 claude" }), ["two"]);
  assert.deepEqual(ids({ query: "ＣＯＤＥＸ" }), ["one"]);
  assert.deepEqual(ids({ query: "等待", statusLabels: { waiting: "等待确认" } }), ["two"]);
  assert.deepEqual(ids({ query: "absent token 1234" }), []);
});

test("workspace, provider, status and archive filters intersect without changing session ownership", () => {
  assert.deepEqual(ids({ workspaceId: "backend", provider: "claude_code", status: "waiting" }), ["two"]);
  assert.deepEqual(ids({ workspaceId: "frontend", provider: "claude_code" }), []);
  assert.deepEqual(ids({ workspaceId: "frontend", provider: "codex", status: "running", archive: "archived" }), [
    "old",
  ]);
  assert.equal(filterSessionList(sessions, workspaces)[1].workspace.name, "后端服务");
});

test("list keeps one canonical row per session ID and preserves orphan sessions for global management", () => {
  const orphan = session("orphan", "unavailable-workspace");
  const entries = filterSessionList([sessions[0], { ...sessions[0], title: "Stale duplicate" }, orphan], workspaces);
  assert.deepEqual(
    entries.map(entry => entry.session.id),
    ["one", "orphan"],
  );
  assert.equal(entries[0].session.title, "Fix toolbar");
  assert.equal(entries[1].workspace, undefined);
});

test("recency breaks equal matches with deterministic ordering for invalid or identical dates", () => {
  const fixtures = [
    session("invalid", "frontend", { updated_at: "invalid" }),
    session("older", "frontend"),
    session("latest", "frontend", { updated_at: "2026-09-12T00:00:00Z" }),
    session("same", "frontend"),
  ];
  assert.deepEqual(ids({}, fixtures), ["latest", "older", "same", "invalid"]);
});

test("filtering is a read-only projection and restoration makes the same identity visible again", () => {
  const snapshot = JSON.stringify(sessions);
  filterSessionList(sessions, workspaces, { archive: "all", query: "payment" });
  assert.equal(JSON.stringify(sessions), snapshot);
  const restored = sessions.map(item => (item.id === "old" ? { ...item, archived_at: null } : item));
  assert.ok(ids({}, restored).includes("old"));
  assert.equal(restored[2].status, "running");
  assert.equal(restored[2].workspace_id, "frontend");
});

test("a resume command names the Tendly session, once there is a conversation to continue", async () => {
  const { resumeCommand } = await import("./session-list.ts");
  assert.equal(resumeCommand({ id: "a1", provider: "claude_code", provider_session_id: "n1" }), "tendly resume a1");
  assert.equal(resumeCommand({ id: "a1", provider: "codex", provider_session_id: null }), undefined);
  assert.equal(resumeCommand({ id: "t1", provider: "terminal", provider_session_id: null }), "tendly resume t1");
});
