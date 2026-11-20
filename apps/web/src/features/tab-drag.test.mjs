import { test } from "node:test";
import assert from "node:assert/strict";
import { insertionIsNoop, tabInsertion } from "./tab-drag.ts";

const slots = [
  { id: "a", left: 0, width: 100 },
  { id: "b", left: 100, width: 100 },
  { id: "c", left: 200, width: 100 },
];

test("a drop lands before the first tab whose middle is past the pointer, else last", () => {
  assert.equal(tabInsertion(10, slots), "a");
  assert.equal(tabInsertion(60, slots), "b");
  assert.equal(tabInsertion(249, slots), "c");
  assert.equal(tabInsertion(251, slots), null);
  assert.equal(tabInsertion(900, []), null);
});

test("a drop beside the dragged tab itself moves nothing", () => {
  const ids = ["a", "b", "c"];
  assert.equal(insertionIsNoop(ids, "b", "b"), true);
  assert.equal(insertionIsNoop(ids, "b", "c"), true);
  assert.equal(insertionIsNoop(ids, "c", null), true);
  assert.equal(insertionIsNoop(ids, "b", "a"), false);
  assert.equal(insertionIsNoop(ids, "x", "a"), false);
  assert.equal(insertionIsNoop(ids, undefined, null), false);
});
