import test from "node:test";
import assert from "node:assert/strict";
import { readTreeViewState, saveTreeViewState } from "./tree-state.ts";

test("tree expansion, selected file and filter survive explorer remounts without retaining file entries", () => {
  const state = {
    expanded: new Set(["src", "src/components"]),
    loadedDirectories: new Set(["", "src", "src/components", "media"]),
    selectedPath: "src/components/App.vue",
    focusedPath: "src/components/App.vue",
    query: "App",
  };
  saveTreeViewState("tree-remount-test", state);
  assert.deepEqual(readTreeViewState("tree-remount-test"), state);
  assert.equal("entries" in readTreeViewState("tree-remount-test"), false);
});

test("workspaces and saved snapshots cannot mutate each other through shared Sets", () => {
  const state = readTreeViewState("tree-snapshot-test");
  state.expanded.add("src");
  saveTreeViewState("tree-snapshot-test", state);
  state.expanded.add("outside-snapshot");
  assert.deepEqual([...readTreeViewState("tree-snapshot-test").expanded], ["src"]);
  readTreeViewState("tree-snapshot-test").expanded.clear();
  assert.deepEqual([...readTreeViewState("tree-snapshot-test").expanded], ["src"]);
  assert.equal(readTreeViewState("tree-other-workspace-test").expanded.size, 0);
  assert.equal(readTreeViewState("tree-other-workspace-test").query, "");
});
