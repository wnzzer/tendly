import { test } from "node:test";
import assert from "node:assert/strict";
import { invertSelection, selectRange } from "./session-selection.ts";

const order = ["a", "b", "c", "d", "e"];

test("a plain click toggles one id and keeps list order", () => {
  assert.deepEqual(selectRange(order, [], "c"), ["c"]);
  assert.deepEqual(selectRange(order, ["d", "a"], "c"), ["a", "c", "d"]);
  assert.deepEqual(selectRange(order, ["a", "c"], "c"), ["a"]);
});

test("a Shift click sets the whole span, in either direction, to the clicked row's new state", () => {
  assert.deepEqual(selectRange(order, ["b"], "d", "b"), ["b", "c", "d"]);
  assert.deepEqual(selectRange(order, ["d"], "b", "d"), ["b", "c", "d"]);
  assert.deepEqual(selectRange(order, ["a", "b", "c", "d", "e"], "d", "b"), ["a", "e"]);
  // An anchor no longer shown falls back to a single toggle.
  assert.deepEqual(selectRange(order, [], "c", "gone"), ["c"]);
});

test("invert ticks exactly the shown ids that were not ticked", () => {
  assert.deepEqual(invertSelection(order, ["b", "d"]), ["a", "c", "e"]);
  assert.deepEqual(invertSelection(order, []), order);
  assert.deepEqual(invertSelection(order, order), []);
});
