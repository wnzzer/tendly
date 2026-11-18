import test from "node:test";
import assert from "node:assert/strict";
import { toolRuns, toolRunNames, toolRunStatus } from "./tool-runs.ts";

const tool = (id, name, status = "completed") => ({ type: "tool", id, name, status, text: "" });
const say = (id, text) => ({ type: "message", id, role: "assistant", text });

test("consecutive tool calls fold into one run, and anything between them ends it", () => {
  const view = toolRuns([
    say("a", "Looking"),
    tool("1", "Bash"),
    tool("2", "Bash"),
    tool("3", "Read"),
    say("b", "Found it"),
    tool("4", "Edit"),
  ]);
  assert.deepEqual(
    view.map(item => (item.type === "tool_run" ? item.tools.map(t => t.id) : item.id)),
    ["a", ["1", "2", "3"], "b", ["4"]],
  );
  assert.equal(toolRunNames(view[1]), "Bash ×2 · Read");
});

test("a failed call is never folded behind a run that reads as fine", () => {
  const view = toolRuns([tool("1", "Bash"), tool("2", "Bash", "failed"), tool("3", "Bash")]);
  assert.deepEqual(
    view.map(run => run.tools.map(t => t.id)),
    [["1"], ["2"], ["3"]],
  );
  assert.equal(toolRunStatus(view[1]), "failed");
});

test("a run is running while any of its calls is", () => {
  const [run] = toolRuns([tool("1", "Bash"), tool("2", "Read", "running")]);
  assert.equal(toolRunStatus(run), "running");
});

test("running subagents are found at any depth, and only a quiet one is stalled", async () => {
  const { runningSubagents, stalledFor, STALL_MS, compactCount } = await import("./tool-runs.ts");
  const inner = { type: "tool", id: "inner", name: "Agent", status: "running", text: "", at: 0 };
  const outer = {
    type: "tool",
    id: "outer",
    name: "Agent",
    status: "running",
    text: "",
    children: [inner, tool("r", "Read", "running")],
  };
  assert.deepEqual(
    runningSubagents([outer, tool("b", "Bash", "running")]).map(t => t.id),
    ["outer", "inner"],
  );
  assert.equal(stalledFor(inner, STALL_MS - 1), undefined);
  assert.equal(stalledFor(inner, STALL_MS), STALL_MS);
  assert.equal(
    stalledFor(outer, STALL_MS * 10),
    undefined,
    "a card that never reported progress is never called stalled",
  );
  assert.deepEqual([12, 3400, 12_000, 1_250_000].map(compactCount), ["12", "3.4k", "12k", "1.3M"]);
});

test("each running subagent not inside another running one gets a dock row", async () => {
  const { dockRoots, toolsById } = await import("./tool-runs.ts");
  const agent = (id, status, children) => ({
    type: "tool",
    id,
    name: "Agent",
    status,
    text: "",
    ...(children ? { children } : {}),
  });
  const deep = agent("deep", "running");
  // A parent that finished while its child runs on leaves the child a row of its own.
  const items = [
    agent("a", "running", [agent("a1", "running")]),
    agent("b", "completed", [deep]),
    tool("c", "Bash", "running"),
  ];
  assert.deepEqual(
    dockRoots(items).map(t => t.id),
    ["a", "deep"],
  );
  // A command the client put in the background is dock work too; a foreground one is not.
  const watch = { ...tool("w", "Bash", "running"), background: true };
  assert.deepEqual(
    dockRoots([watch, tool("f", "Bash", "running")]).map(t => t.id),
    ["w"],
  );
  // And it is never called stalled: a command is quiet until it ends.
  const { stalledFor: quiet } = await import("./tool-runs.ts");
  assert.equal(quiet({ ...watch, at: 0 }, 60 * 60_000), undefined);
  assert.equal(toolsById(items).get("deep"), deep);
  // Ended with its client: neither a row nor a running subagent any more.
  const ended = { ...agent("e", "running"), ended: true };
  assert.deepEqual(dockRoots([ended]), []);
  const { runningSubagents, stalledFor } = await import("./tool-runs.ts");
  assert.deepEqual(runningSubagents([ended]), []);
  assert.equal(stalledFor({ ...ended, at: 0 }, 10 * 60_000), undefined);
});

test("the cached share shows a decimal, and 100.0 only when all of it was cached", async () => {
  const { cachedPercent } = await import("./tool-runs.ts");
  assert.equal(cachedPercent(14_327_235, 14_305_498), "99.8");
  assert.equal(cachedPercent(10_000, 9_996), "99.9", "cut down, never rounded up to 100.0");
  assert.equal(cachedPercent(500, 500), "100.0");
  assert.equal(cachedPercent(1000, 0), "0.0");
  assert.equal(cachedPercent(0, 0), "0.0");
});

test("a running card counts on from its last reported duration, or from its start", async () => {
  const { liveStats } = await import("./tool-runs.ts");
  const reported = {
    type: "tool",
    id: "a",
    name: "Agent",
    status: "running",
    text: "",
    stats: { tools: 2, ms: 10_000 },
    ms_at: 1_000,
    since: 0,
  };
  assert.deepEqual(liveStats(reported, 4_000), { tools: 2, ms: 13_000 });
  assert.deepEqual(liveStats({ ...reported, stats: undefined, ms_at: undefined }, 4_000), { ms: 4_000 });
  assert.deepEqual(
    liveStats({ ...reported, status: "completed" }, 4_000),
    { tools: 2, ms: 10_000 },
    "a finished card keeps the final figure",
  );
  assert.deepEqual(liveStats({ ...reported, ended: true }, 4_000), { tools: 2, ms: 10_000 });
  assert.equal(liveStats({ ...reported, stats: undefined, ms_at: undefined, since: undefined }, 4_000), undefined);
});
