import { test } from "node:test";
import assert from "node:assert/strict";
import { attentionTitle, sessionStatuses, trackSessions, waitingIds } from "./attention.ts";
import { dismissToast, toasts } from "./toasts.ts";

const session = (id, status) => ({ id, workspace_id: "w", title: `Fixture ${id}`, provider: "claude_code", status });
const t = (key, values = {}) => key.replace(/\{(\w+)\}/g, (_, name) => String(values[name]));

test("a session that starts waiting is announced once, unless it is the one in front of you", () => {
  for (const toast of [...toasts]) dismissToast(toast.id);
  const opened = [];
  const context = focused => ({ focusedSessionId: focused, open: item => opened.push(item.id), t });
  // A first sight of a session that is already waiting is not news.
  trackSessions([session("a", "waiting"), session("b", "running"), session("c", "running")], context());
  assert.equal(toasts.length, 0);
  assert.deepEqual(waitingIds.value, ["a"]);

  trackSessions([session("a", "waiting"), session("b", "waiting"), session("c", "waiting")], context("c"));
  assert.equal(toasts.length, 1, "b is announced; c is already focused; a was waiting before");
  assert.match(toasts[0].message, /Fixture b is waiting for you/);
  toasts[0].action.run();
  assert.deepEqual(opened, ["b"]);

  trackSessions([session("a", "waiting"), session("b", "waiting")], context());
  assert.equal(toasts.length, 1, "still waiting is not a new announcement");
  assert.equal(sessionStatuses.c, undefined, "a session that left the list is forgotten");
  for (const toast of [...toasts]) dismissToast(toast.id);
});

test("the page title counts the sessions waiting on you", () => {
  assert.equal(attentionTitle("Tendly", 0), "Tendly");
  assert.equal(attentionTitle("Tendly", 2), "(2) Tendly");
});
