import { test } from "node:test";
import assert from "node:assert/strict";
import { anchorRow } from "./older-history.ts";

const items = [
  { key: "a", start: 0, end: 100 },
  { key: "b", start: 100, end: 250 },
  { key: 3, start: 250, end: 400 },
];

test("the row kept in place is the first one still in view, with how far below the top it sits", () => {
  assert.deepEqual(anchorRow(items, 0), { key: "a", offset: 0 });
  assert.deepEqual(
    anchorRow(items, 130),
    { key: "b", offset: -30 },
    "partly scrolled past, it is still the one in view",
  );
  assert.deepEqual(anchorRow(items, 100), { key: "b", offset: 0 }, "a row ending exactly at the top is out of view");
});

test("keys are compared as text, as the row keys are", () => {
  assert.equal(anchorRow(items, 300).key, "3");
});

test("with nothing in view there is nothing to keep", () => {
  assert.equal(anchorRow(items, 500), undefined);
  assert.equal(anchorRow([], 0), undefined);
});
