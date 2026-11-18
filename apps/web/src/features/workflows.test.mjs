// Node with tsx executes the actual TypeScript helpers; no copied implementation.
import { afterEach, mock, test } from "node:test";
import assert from "node:assert/strict";
import { toRaw } from "vue";
import { ApiError, assetUrl, errorMessage, json, request } from "./api.ts";
import { fileDraft, hasDirtyDrafts } from "./file-drafts.ts";
import { gitViewState, hasGitDrafts } from "./git-view-state.ts";
import { isChangedFile, isStagedFile, parseUnifiedDiff, pathsForGitFiles } from "./git-model.ts";

let fetchMock;
afterEach(() => {
  mock.restoreAll();
  fetchMock = undefined;
});

test("workspace requests use same origin and the required anti-CSRF header", async () => {
  let call;
  fetchMock = mock.method(globalThis, "fetch", async (path, init) => {
    call = { path, init };
    return new Response(JSON.stringify({ id: "created" }), { status: 201 });
  });
  const result = await request("/workspaces", json("POST", { name: "project", root_path: "/tmp/project" }));
  assert.equal(result.id, "created");
  assert.equal(call.path, "/api/workspaces");
  assert.equal(call.init.headers["X-Tendly-Client"], "web");
  assert.equal(call.init.headers["Content-Type"], "application/json");
  assert.deepEqual(JSON.parse(call.init.body), { name: "project", root_path: "/tmp/project" });
});

test("successful 204 mutations do not attempt to parse an empty JSON response", async () => {
  fetchMock = mock.method(globalThis, "fetch", async () => new Response(null, { status: 204 }));
  assert.equal(await request("/workspaces/id/layout", json("PUT", { version: 1 })), undefined);
});

test("version conflicts preserve the HTTP code and server explanation", async () => {
  fetchMock = mock.method(
    globalThis,
    "fetch",
    async () => new Response(JSON.stringify({ error: "File changed on disk" }), { status: 409 }),
  );
  await assert.rejects(
    request("/workspaces/id/file?path=a.txt", json("PUT", { content: "draft", expected_version: "v1" })),
    error => error instanceof ApiError && error.status === 409 && error.message === "File changed on disk",
  );
});

test("HTML proxy errors are not reflected as raw page content", async () => {
  fetchMock = mock.method(
    globalThis,
    "fetch",
    async () => new Response("<!doctype html><script>bad()</script>", { status: 502 }),
  );
  await assert.rejects(request("/health"), error => error.message === "Request failed (502)");
});

test("asset paths are encoded rather than interpreted as query fragments", () => {
  assert.equal(
    assetUrl("workspace", "media/a #b?.png"),
    "/api/workspaces/workspace/asset?path=media%2Fa%20%23b%3F.png",
  );
  assert.equal(errorMessage(new Error("offline")), "offline");
});

test("file drafts survive remount lookup and are isolated by workspace and path", () => {
  const draft = fileDraft("draft-test-a", "src/main.ts");
  Object.assign(draft, { content: "unsaved", original: "original", version: "one", loaded: true });
  assert.equal(fileDraft("draft-test-a", "src/main.ts"), draft);
  assert.equal(hasDirtyDrafts("draft-test-a"), true);
  assert.equal(fileDraft("draft-test-b", "src/main.ts").content, "");
  assert.equal(fileDraft("draft-test-a", "other.ts").content, "");
  draft.original = draft.content;
  assert.equal(hasDirtyDrafts("draft-test-a"), false);
});

test("a partly staged file appears in both Git groups; untracked is unstaged only", () => {
  const files = [
    { path: "both", index: "M", worktree: "M" },
    { path: "staged", index: "A", worktree: " " },
    { path: "untracked", index: "?", worktree: "?" },
    { path: "removed", index: " ", worktree: "D" },
  ];
  assert.deepEqual(
    files.filter(isStagedFile).map(file => file.path),
    ["both", "staged"],
  );
  assert.deepEqual(
    files.filter(isChangedFile).map(file => file.path),
    ["both", "untracked", "removed"],
  );
});

test("Git commit drafts and diff selection survive responsive view remounts", () => {
  const state = gitViewState("git-draft-test");
  state.message = "Work in progress";
  state.selected = { path: "main.ts", staged: true };
  assert.equal(gitViewState("git-draft-test").message, "Work in progress");
  assert.deepEqual(toRaw(gitViewState("git-draft-test").selected), { path: "main.ts", staged: true });
  assert.equal(gitViewState("different-workspace").message, "");
  assert.equal(hasGitDrafts(), true);
  state.message = "";
  assert.equal(hasGitDrafts(), false);
});

test("rename mutations retain both paths and de-duplicate explicit paths", () => {
  assert.deepEqual(
    pathsForGitFiles([
      { path: "new name.ts", original_path: "old name.ts", index: "R", worktree: " " },
      { path: "new name.ts", index: " ", worktree: "M" },
    ]),
    ["new name.ts", "old name.ts"],
  );
});

test("unified diff line numbers follow hunk starts and reset between files", () => {
  const rows = parseUnifiedDiff(
    "diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -4,3 +4,3 @@\n context\n-old\n+new\n same\ndiff --git a/b b/b\n+++ b/b\n@@ -0,0 +1 @@\n+first",
  );
  assert.equal(rows[2].kind, "meta");
  assert.deepEqual(
    rows.slice(4, 8).map(({ before, after, kind }) => [before, after, kind]),
    [
      [4, 4, "context"],
      [5, "", "removed"],
      ["", 5, "added"],
      [6, 6, "context"],
    ],
  );
  assert.equal(rows[9].kind, "meta");
  assert.deepEqual([rows[11].before, rows[11].after, rows[11].kind], ["", 1, "added"]);
});

test("an empty pane of the same kind in the same workspace is reused; git changes never are", async () => {
  const { reusablePane } = await import("./pane-context.ts");
  const pane = (id, kind, metadata) => ({ type: "pane", id, kind, ...(metadata ? { metadata } : {}) });
  const open = [
    pane("busy", "editor", { path: "a.ts" }),
    pane("draft", "editor", { draft: true }),
    pane("other", "editor", { workspace_id: "w2" }),
    pane("blank", "editor", { workspace_id: "w1" }),
    pane("diff", "git_diff"),
  ];
  assert.equal(reusablePane(open, pane("new", "editor", { workspace_id: "w1", path: "b.ts" }))?.id, "blank");
  assert.equal(
    reusablePane(open, pane("new", "editor", { workspace_id: "w3", path: "b.ts" })),
    undefined,
    "every empty editor belongs to another workspace",
  );
  assert.equal(reusablePane(open, pane("new", "git_diff", { workspace_id: "w1" })), undefined);
  assert.equal(
    reusablePane([pane("loose", "editor")], pane("new", "editor", { workspace_id: "w1" }))?.id,
    "loose",
    "a pane of no workspace can be taken",
  );
});
