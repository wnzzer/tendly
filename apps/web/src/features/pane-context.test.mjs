import { test } from "node:test";
import assert from "node:assert/strict";
import {
  acceptsScopedPane,
  changesPane,
  filePane,
  paneSession,
  paneWorkspace,
  renameSessionPanes,
  scopeLegacyLayout,
  sessionPane,
} from "./pane-context.ts";
import { applyPreset, dockPane, flattenPanes, projectLayout, validateLayout } from "../layout/layout-engine.ts";

const workspaces = [
  { id: "a", name: "A" },
  { id: "b", name: "B" },
];
const sessions = [
  { id: "one", workspace_id: "a", provider: "codex", title: "Agent" },
  { id: "two", workspace_id: "b", provider: "claude_code", title: "Agent" },
];
test("same relative file names and Git panes are distinct across workspaces", () => {
  assert.notEqual(filePane("a", "README.md").id, filePane("b", "README.md").id);
  assert.notEqual(filePane("a-b", "c").id, filePane("a", "b-c").id);
  assert.notEqual(changesPane("a").id, changesPane("b").id);
  assert.equal(filePane("b", "assets/a.png").kind, "file_preview");
  assert.deepEqual(filePane("b", "README.md").metadata, { workspace_id: "b", path: "README.md" });
});
test("session source is authoritative and cannot be reassigned by a sidebar selection or payload", () => {
  const pane = sessionPane(sessions[0]);
  assert.equal(paneWorkspace(pane, workspaces, sessions).id, "a");
  assert.equal(paneSession(pane, sessions).id, "one");
  assert.equal(
    paneWorkspace({ ...pane, metadata: { ...pane.metadata, workspace_id: "b" } }, workspaces, sessions),
    undefined,
  );
  assert.equal(paneSession({ ...pane, kind: "terminal" }, sessions), undefined);
  assert.equal(paneWorkspace(filePane("deleted", "README.md"), workspaces, sessions), undefined);
  assert.equal(paneWorkspace({ type: "pane", id: "blank", kind: "editor" }, workspaces, sessions), undefined);
});
test("renaming a session updates every bound tab while preserving pane and layout identities", () => {
  const root = {
    type: "split",
    id: "root",
    direction: "horizontal",
    ratio: 0.5,
    first: sessionPane(sessions[0]),
    second: {
      type: "stack",
      kind: "stack",
      id: "tabs",
      activePaneId: "session-two",
      panes: [sessionPane(sessions[1]), { ...sessionPane(sessions[0]), id: "second-view" }],
    },
  };
  const renamed = renameSessionPanes(root, "one", "Review billing");
  assert.equal(renamed.id, "root");
  assert.equal(renamed.first.id, "session-one");
  assert.equal(renamed.first.title, "Review billing");
  assert.equal(renamed.second.panes[1].id, "second-view");
  assert.equal(renamed.second.panes[1].title, "Review billing");
  assert.equal(renamed.second.panes[0].title, "Agent");
});
test("a session whose name has not changed leaves the layout as it is", () => {
  const root = {
    type: "split",
    id: "root",
    direction: "horizontal",
    ratio: 0.5,
    first: sessionPane(sessions[0]),
    second: sessionPane(sessions[1]),
  };
  assert.equal(renameSessionPanes(root, "one", root.first.title), root);
});
test("foreign drag scopes and traversal do not enter the shared canvas", () => {
  assert.equal(acceptsScopedPane(filePane("a", "src/a #?.txt"), workspaces, sessions), true);
  for (const path of ["../secret", "/absolute", "a/../../secret", "a\0b"])
    assert.equal(acceptsScopedPane(filePane("a", path), workspaces, sessions), false);
  assert.equal(acceptsScopedPane(filePane("unknown", "a.txt"), workspaces, sessions), false);
  assert.equal(acceptsScopedPane(sessionPane({ ...sessions[0], workspace_id: "b" }), workspaces, sessions), false);
});
test("legacy layout migration preserves geometry, active tab and original document", () => {
  const old = {
    version: 1,
    root: {
      type: "split",
      id: "root",
      direction: "horizontal",
      ratio: 0.37,
      first: {
        type: "stack",
        kind: "stack",
        id: "left",
        activePaneId: "old-file",
        panes: [
          {
            type: "pane",
            id: "old-session",
            kind: "agent_chat",
            metadata: { session_id: "two", provider: "claude_code" },
          },
          { type: "pane", id: "old-file", kind: "editor", metadata: { path: "README.md" } },
        ],
      },
      second: { type: "pane", id: "changes", kind: "git_diff" },
    },
  };
  const before = JSON.stringify(old),
    migrated = scopeLegacyLayout(old, "a", sessions);
  assert.equal(JSON.stringify(old), before);
  assert.equal(migrated.root.ratio, 0.37);
  assert.equal(migrated.root.first.activePaneId, filePane("a", "README.md").id);
  const panes = flattenPanes(migrated.root);
  assert.equal(panes[0].metadata.workspace_id, "b");
  assert.equal(panes[1].metadata.workspace_id, "a");
  assert.equal(panes[2].metadata.workspace_id, "a");
  assert.equal(validateLayout(migrated), true);
});
test("explicit saved scopes are never silently rebound during migration", () => {
  const old = {
    version: 1,
    root: { type: "pane", id: "old", kind: "editor", metadata: { workspace_id: "b", path: "README.md" } },
  };
  const migrated = scopeLegacyLayout(old, "a", sessions);
  assert.equal(migrated.root.metadata.workspace_id, "b");
});
test("cross-workspace docking, presets and responsive tabs retain all operation targets", () => {
  let root = { type: "stack", kind: "stack", id: "canvas", panes: [] };
  const panes = [
    sessionPane(sessions[0]),
    sessionPane(sessions[1]),
    filePane("a", "README.md"),
    filePane("b", "README.md"),
    changesPane("a"),
    changesPane("b"),
  ];
  for (const pane of panes) root = dockPane(root, pane, "canvas", "center");
  root = applyPreset(root, "1:2:1");
  const projected = projectLayout(root, 390, 650);
  const lookup = Object.fromEntries(panes.map(pane => [pane.id, pane.metadata.workspace_id]));
  for (const pane of flattenPanes(projected)) assert.equal(pane.metadata.workspace_id, lookup[pane.id]);
  assert.equal(flattenPanes(root).length, 6);
  assert.equal(validateLayout({ version: 1, root }), true);
});
