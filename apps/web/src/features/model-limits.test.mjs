import { test } from "node:test";
import assert from "node:assert/strict";
import { nextLimits, validWindow, windowValue } from "./model-limits.ts";

test("a typed window reads with separators, and must be a sane size", () => {
  assert.equal(windowValue("200,000"), 200000);
  assert.equal(windowValue(" "), undefined);
  assert.ok(validWindow("") && validWindow("32_768"));
  assert.ok(!validWindow("999") && !validWindow("1.5e3x") && !validWindow("200000000"));
});

test("drafts change only the models they name, and nothing changed saves nothing", () => {
  const saved = { a: 32000, b: 64000 };
  assert.equal(nextLimits(saved, {}), undefined);
  assert.equal(nextLimits(saved, { a: "32,000" }), undefined, "the same value is no change");
  assert.deepEqual(
    nextLimits(saved, { a: "", c: "128000" }),
    { b: 64000, c: 128000 },
    "empty clears one; a new one is added",
  );
});
