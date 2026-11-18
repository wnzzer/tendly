import { test } from "node:test";
import assert from "node:assert/strict";
import { createFrameBatch } from "./frame-batch.ts";

test("pieces that arrive within a frame are applied together, once", () => {
  const applied = [],
    frames = [];
  const batch = createFrameBatch(
    items => applied.push(items),
    run => frames.push(run),
  );
  for (const piece of ["a", "b", "c"]) batch.push(piece);
  assert.equal(frames.length, 1, "one frame asked for");
  assert.deepEqual(applied, []);
  frames.shift()();
  assert.deepEqual(applied, [["a", "b", "c"]]);
  batch.push("d");
  assert.equal(frames.length, 1);
  frames.shift()();
  assert.deepEqual(applied, [["a", "b", "c"], ["d"]]);
});

test("a flush applies what is waiting now, and the frame that follows finds nothing", () => {
  const applied = [],
    frames = [];
  const batch = createFrameBatch(
    items => applied.push(items),
    run => frames.push(run),
  );
  batch.push("a");
  batch.flush();
  assert.deepEqual(applied, [["a"]]);
  frames.shift()();
  assert.deepEqual(applied, [["a"]]);
  batch.push("b");
  batch.clear();
  frames.shift()?.();
  assert.equal(batch.size, 0);
  assert.deepEqual(applied, [["a"]]);
});

test("leading: the first item after a quiet spell goes at once, the rest of the burst waits for a frame", () => {
  const applied = [],
    frames = [];
  const batch = createFrameBatch(
    items => applied.push(items),
    run => frames.push(run),
    { leading: true },
  );
  batch.push("echo");
  assert.deepEqual(applied, [["echo"]], "no frame of delay for the first");
  batch.push("a");
  batch.push("b");
  assert.deepEqual(applied, [["echo"]]);
  frames.shift()();
  assert.deepEqual(applied, [["echo"], ["a", "b"]]);
  // Still busy: the window stays open, so the next arrival waits for a frame too.
  batch.push("c");
  assert.deepEqual(applied, [["echo"], ["a", "b"]]);
  frames.shift()();
  assert.deepEqual(applied, [["echo"], ["a", "b"], ["c"]]);
  // A frame that finds nothing closes the window; the next item goes at once again.
  frames.shift()();
  assert.equal(frames.length, 0);
  batch.push("later");
  assert.deepEqual(applied.at(-1), ["later"]);
});

test("leading: clearing drops what waits without disturbing the window", () => {
  const applied = [],
    frames = [];
  const batch = createFrameBatch(
    items => applied.push(items),
    run => frames.push(run),
    { leading: true },
  );
  batch.push("a");
  batch.push("b");
  batch.clear();
  frames.shift()();
  assert.deepEqual(applied, [["a"]]);
});
