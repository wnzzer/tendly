import { test } from "node:test";
import assert from "node:assert/strict";
import { dockHeading, duration, statsLine, turnUsageLine, turnUsageTitle } from "./chat-format.ts";
import { toolIndicator } from "./tool-runs.ts";
import { sessionPath } from "./api.ts";

const t = (text, values = {}) => text.replace(/\{(\w+)\}/g, (_, key) => String(values[key]));
const tool = (name, status = "running", extra = {}) => ({
  type: "tool",
  id: name + status,
  name,
  status,
  text: "",
  ...extra,
});

test("a duration reads in seconds, then minutes and seconds", () => {
  assert.equal(duration(41_600, t), "42s");
  assert.equal(duration(59_400, t), "59s");
  assert.equal(duration(185_000, t), "3m 5s");
});

test("a tool card shows whichever of its uses, tokens and time were reported", () => {
  assert.equal(statsLine({ tools: 12, tokens: 4500, ms: 62_000 }, t), "12 tool uses · 4.5k tokens · 1m 2s");
  assert.equal(statsLine({ tokens: 800 }, t), "800 tokens");
  assert.equal(statsLine({}, t), "");
});

test("a turn shows what it read and wrote, and the cached share only when there is one", () => {
  assert.equal(turnUsageLine({ input: 9400, output: 1200, cached: 9380 }, t), "↑ 9.4k · ↓ 1.2k · 99.7% cached");
  assert.equal(turnUsageLine({ output: 5 }, t), "↑ 0 · ↓ 5");
  assert.equal(turnUsageLine({ input: 100, output: 5, cached: 0 }, t), "↑ 100 · ↓ 5");
  assert.match(
    turnUsageTitle({ input: 2000, cached: 1500, output: 10 }, t),
    /^This turn read 2.000 tokens \(1.500 from the prompt cache\) and wrote 10$/,
  );
});

test("the dock heading counts subagents and background commands apart", () => {
  assert.equal(dockHeading([tool("Agent")], t), "1 subagent running");
  assert.equal(dockHeading([tool("Agent"), tool("Task")], t), "2 subagents running");
  assert.equal(dockHeading([tool("Bash")], t), "1 background command running");
  assert.equal(dockHeading([tool("Bash"), tool("Bash")], t), "2 background commands running");
  assert.equal(dockHeading([tool("Agent"), tool("Bash"), tool("Bash")], t), "1 subagents and 2 commands running");
  assert.equal(
    dockHeading([tool("Agent", "completed"), tool("Agent", "running", { ended: true })], t),
    "Subagents finished",
    "one cut off by its session is not running",
  );
  assert.equal(dockHeading([tool("Agent", "completed"), tool("Bash", "failed")], t), "Background work finished");
});

test("a call is marked done, failed, cut off or working", () => {
  assert.equal(toolIndicator({ status: "completed" }), "✓");
  assert.equal(toolIndicator({ status: "failed", ended: true }), "!");
  assert.equal(toolIndicator({ status: "running", ended: true }), "–");
  assert.equal(toolIndicator({ status: "running" }), "↻");
});

test("a session id is escaped in its API path", () => {
  assert.equal(sessionPath("a/b c"), "/sessions/a%2Fb%20c");
});
