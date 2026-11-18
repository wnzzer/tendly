import { test } from "node:test";
import assert from "node:assert/strict";
import { createStableRows, same } from "./stable-items.ts";

const message = (id, text) => ({ type: "message", id, role: "assistant", text });
const run = (id, status, children = []) => ({
  type: "tool_run",
  id,
  tools: [{ type: "tool", id, name: "Bash", status, text: "ls", children }],
});

test("an unchanged row is the same object as before; a changed one is new", () => {
  const stabilize = createStableRows();
  const first = stabilize([message("a", "hi"), run("t", "running"), message("b", "Hel")]);
  const second = stabilize([message("a", "hi"), run("t", "running"), message("b", "Hello")]);
  assert.equal(second.rows[0], first.rows[0]);
  assert.equal(second.rows[1], first.rows[1]);
  assert.notEqual(second.rows[2], first.rows[2]);
  assert.equal(second.rows[2].text, "Hello");
  // A nested call that changes makes its run a changed row.
  const third = stabilize([
    message("a", "hi"),
    run("t", "running", [{ type: "tool", id: "c", status: "completed" }]),
    message("b", "Hello"),
  ]);
  assert.notEqual(third.rows[1], second.rows[1]);
  assert.equal(third.rows[2], second.rows[2]);
});

test("keys name a row by what it is, never by where it sits, and stay unique", () => {
  const stabilize = createStableRows();
  const before = stabilize([message("a", "x"), message("b", "y")]);
  // History dropped from the front does not rename the rows after it.
  const after = stabilize([message("b", "y"), message("c", "z")]);
  assert.deepEqual(before.keys, ["message:a", "message:b"]);
  assert.deepEqual(after.keys, ["message:b", "message:c"]);
  assert.equal(after.rows[0], before.rows[1]);
  const repeated = stabilize([
    { type: "error", id: "e", text: "1" },
    { type: "error", id: "e", text: "2" },
  ]);
  assert.deepEqual(repeated.keys, ["error:e", "error:e#1"]);
});

test("structural equality covers what rows are made of", () => {
  assert.ok(same({ a: [1, { b: "c" }] }, { a: [1, { b: "c" }] }));
  assert.ok(!same({ a: 1 }, { a: 1, b: undefined }));
  assert.ok(!same([1], { 0: 1 }));
  assert.ok(!same({ a: "x" }, { a: "y" }));
});
