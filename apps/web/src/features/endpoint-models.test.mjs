import test from "node:test";
import assert from "node:assert/strict";
import { parseModelAliases, formatModelAliases } from "./endpoint-models.ts";

test("model aliases retain exact provider IDs and round trip", () => {
  const aliases = parseModelAliases("fast = remote-model-x\nreview = model-y");
  assert.equal(aliases.fast, "remote-model-x");
  assert.deepEqual(parseModelAliases(formatModelAliases(aliases)), aliases);
});
test("model aliases reject ambiguity instead of silently overwriting", () => {
  assert.throws(() => parseModelAliases("fast=x\nfast=y"));
  assert.throws(() => parseModelAliases("no separator"));
  assert.throws(() => parseModelAliases("key="));
});
