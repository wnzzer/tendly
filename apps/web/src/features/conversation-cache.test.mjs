import { test } from "node:test";
import assert from "node:assert/strict";
import { createConversationCache } from "./conversation-cache.ts";

const events = seq => [{ type: "ready", seq }];

test("a conversation shown before is there to show again, with its state", () => {
  const cache = createConversationCache();
  cache.remember("a", { events: events(3), mode: "structured", running: true, truncated: false });
  assert.deepEqual(cache.get("a"), { events: events(3), mode: "structured", running: true, truncated: false });
  assert.equal(cache.get("b"), undefined);
});

test("only a structured conversation with events is kept: there is nothing to resume from otherwise", () => {
  const cache = createConversationCache();
  cache.remember("pty", { events: events(1), mode: "pty", running: true, truncated: false });
  cache.remember("unknown", { events: events(1), running: false, truncated: false });
  cache.remember("empty", { events: [], mode: "structured", running: false, truncated: false });
  cache.remember("unsequenced", { events: [{ type: "ready" }], mode: "structured", running: false, truncated: false });
  assert.equal(cache.size, 0);
  // A view being reset for another session does not erase what was kept.
  cache.remember("a", { events: events(2), mode: "structured", running: false, truncated: false });
  cache.remember("a", { events: [], mode: undefined, running: false, truncated: false });
  assert.deepEqual(cache.get("a")?.events, events(2));
});

test("the least recently shown conversation goes first", () => {
  const cache = createConversationCache(2);
  cache.remember("a", { events: events(1), mode: "structured", running: false, truncated: false });
  cache.remember("b", { events: events(1), mode: "structured", running: false, truncated: false });
  cache.get("a");
  cache.remember("c", { events: events(1), mode: "structured", running: false, truncated: false });
  assert.ok(cache.get("a"));
  assert.equal(cache.get("b"), undefined);
  assert.ok(cache.get("c"));
  cache.forget("a");
  assert.equal(cache.get("a"), undefined);
});

test("the cursor to older history is kept with the conversation", () => {
  const cache = createConversationCache();
  cache.remember("a", { events: events(9), mode: "structured", running: false, truncated: false, older: 5 });
  assert.equal(cache.get("a").older, 5);
  cache.remember("b", { events: events(9), mode: "structured", running: false, truncated: false });
  assert.ok(!("older" in cache.get("b")));
});
