import { test } from "node:test";
import assert from "node:assert/strict";
import { atLogEnd, dockedAgents, joinDock } from "./subagent-dock.ts";

test("a subagent joins the dock at the end, once, so rows keep the order they started in", () => {
  assert.deepEqual(joinDock(["a", "b"], ["b", "c", "a", "d"]), ["a", "b", "c", "d"]);
  const ids = ["a"];
  assert.equal(joinDock(ids, ["a"]), ids, "nothing new leaves the list as it was, so nothing re-renders");
});

test("a subagent dismissed or ended has no row in the dock", () => {
  const tools = new Map([
    ["a", { id: "a", status: "running" }],
    ["b", { id: "b", status: "done", ended: true }],
    ["c", { id: "c", status: "done" }],
  ]);
  assert.deepEqual(
    dockedAgents(["c", "a", "b", "gone"], tools).map(tool => tool.id),
    ["c", "a"],
  );
});

test("a log follows new lines only while it is scrolled to its end", () => {
  assert.equal(atLogEnd(1000, 800, 200), true);
  assert.equal(atLogEnd(1000, 780, 200), true, "within a line of the end still counts");
  assert.equal(atLogEnd(1000, 500, 200), false);
});
