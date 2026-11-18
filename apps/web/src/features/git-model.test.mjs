import { test } from "node:test";
import assert from "node:assert/strict";
import {
  commitGraph,
  commitRefs,
  hunkPatch,
  isChangedFile,
  isConflictedFile,
  isStagedFile,
  parseUnifiedDiff,
} from "./git-model.ts";

const diff = `diff --git a/a.txt b/a.txt
index 1111111..2222222 100644
--- a/a.txt
+++ b/a.txt
@@ -1,2 +1,3 @@
+zero
 one
 two
@@ -10,2 +11,2 @@ context
 ten
-eleven
+ELEVEN
\\ No newline at end of file
`;

test("a hunk becomes a patch of its own, header included", () => {
  assert.equal(
    hunkPatch(diff, 0),
    `diff --git a/a.txt b/a.txt
index 1111111..2222222 100644
--- a/a.txt
+++ b/a.txt
@@ -1,2 +1,3 @@
+zero
 one
 two
`,
  );
  const second = hunkPatch(diff, 1);
  assert.ok(second.startsWith("diff --git a/a.txt b/a.txt\n"));
  assert.ok(second.includes("@@ -10,2 +11,2 @@ context\n ten\n-eleven\n+ELEVEN\n\\ No newline at end of file\n"));
  assert.ok(!second.includes("+zero"));
  assert.equal(hunkPatch(diff, 2), undefined);
  assert.deepEqual(
    parseUnifiedDiff(diff)
      .filter(row => row.kind === "hunk")
      .map(row => row.hunk),
    [0, 1],
  );
});

test("conflicted files sit in neither the staged nor the changed group", () => {
  const both = { index: "U", worktree: "U", path: "x" },
    added = { index: "A", worktree: "A", path: "y" },
    staged = { index: "M", worktree: " ", path: "z" };
  assert.ok(isConflictedFile(both) && isConflictedFile(added) && !isConflictedFile(staged));
  assert.ok(!isStagedFile(both) && !isChangedFile(both) && isStagedFile(staged));
});

test("decorations read as head, branches, remotes and tags", () => {
  assert.deepEqual(commitRefs(["HEAD -> main", "origin/main", "origin/HEAD", "tag: v1.0", "feature/x"], ["origin"]), [
    { kind: "head", name: "main" },
    { kind: "remote", name: "origin/main" },
    { kind: "tag", name: "v1.0" },
    { kind: "branch", name: "feature/x" },
  ]);
});

test("the graph keeps a lane per line of history and joins merges", () => {
  // m merges b into a; a and b share root r.
  const rows = commitGraph([
    { hash: "m", parents: ["a", "b"] },
    { hash: "b", parents: ["r"] },
    { hash: "a", parents: ["r"] },
    { hash: "r", parents: [] },
  ]);
  assert.deepEqual(
    rows.map(row => row.column),
    [0, 1, 0, 1],
  );
  assert.deepEqual(rows[0], { column: 0, lanes: [], next: [0, 1], into: [], out: [0, 1], width: 2 });
  assert.deepEqual(rows[1].out, [1]);
  assert.deepEqual(rows[2].out, [1], "a joins the lane already waiting for r");
  assert.deepEqual(rows[3].into, [1]);
  assert.deepEqual(rows[3].next, []);
  // A second tip beside a line already running takes the next free lane.
  const tips = commitGraph([
    { hash: "x", parents: ["p"] },
    { hash: "y", parents: ["p"] },
    { hash: "p", parents: [] },
  ]);
  assert.deepEqual(
    tips.map(row => row.column),
    [0, 1, 0],
  );
  assert.deepEqual(tips[1].lanes, [0]);
  assert.deepEqual(tips[1].into, []);
});
