import test from "node:test";
import assert from "node:assert/strict";
import { lineRange, requestJump, takeJump } from "./file-jumps.ts";

test("a jump waits for its pane and is taken once; line ranges clamp to the text", () => {
  requestJump("w", "a.ts", 3);
  assert.equal(takeJump("w", "b.ts"), undefined);
  assert.equal(takeJump("w", "a.ts"), 3);
  assert.equal(takeJump("w", "a.ts"), undefined);
  assert.deepEqual(lineRange("one\ntwo\nthree", 2), [4, 7]);
  assert.deepEqual(lineRange("one\ntwo", 9), [4, 7]);
  assert.deepEqual(lineRange("only", 1), [0, 4]);
});
