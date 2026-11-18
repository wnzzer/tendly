import test from "node:test";
import assert from "node:assert/strict";
import { onPageReturn, STALE_AFTER_MS } from "./page-return.ts";

function page() {
  const listeners = new Map();
  const state = { hidden: false, time: 0 };
  const events = {
    hidden: () => state.hidden,
    now: () => state.time,
    on(target, type, listener) {
      const key = `${target}:${type}`;
      listeners.set(key, listener);
      return () => listeners.delete(key);
    },
  };
  const fire = (key, event = {}) => listeners.get(key)?.(event);
  return { state, events, listeners, fire };
}

test("a brief tab switch keeps sockets, a long absence distrusts them", () => {
  const { state, events, fire } = page(),
    calls = [];
  onPageReturn(stale => calls.push(stale), events);
  state.hidden = true;
  fire("document:visibilitychange");
  state.time += 2_000;
  state.hidden = false;
  fire("document:visibilitychange");
  state.hidden = true;
  fire("document:visibilitychange");
  state.time += STALE_AFTER_MS;
  state.hidden = false;
  fire("document:visibilitychange");
  assert.deepEqual(calls, [false, true]);
});

test("a page loaded in the background measures its absence from the start", () => {
  const { state, events, fire } = page(),
    calls = [];
  state.hidden = true;
  onPageReturn(stale => calls.push(stale), events);
  state.time += STALE_AFTER_MS + 1;
  state.hidden = false;
  fire("document:visibilitychange");
  assert.deepEqual(calls, [true]);
});

test("a back-forward cache restore and a returning network are always stale", () => {
  const { state, events, fire } = page(),
    calls = [];
  onPageReturn(stale => calls.push(stale), events);
  fire("window:pageshow", { persisted: false });
  fire("window:pageshow", { persisted: true });
  fire("window:online");
  state.hidden = true;
  fire("window:online");
  assert.deepEqual(calls, [true, true]);
});

test("stopping removes every listener", () => {
  const { events, listeners } = page();
  const stop = onPageReturn(() => {}, events);
  assert.equal(listeners.size, 3);
  stop();
  assert.equal(listeners.size, 0);
});
