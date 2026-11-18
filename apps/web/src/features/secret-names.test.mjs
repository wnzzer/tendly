import test from "node:test";
import assert from "node:assert/strict";
import { ownStoredSecret, referencedSecret, secretNameFor } from "./secret-names.ts";

test("profile names become secret names, never reusing a taken one", () => {
  assert.equal(secretNameFor("Work proxy", []), "TENDLY_SECRET_WORK_PROXY");
  assert.equal(secretNameFor("  deepseek-v3 (fast)", []), "TENDLY_SECRET_DEEPSEEK_V3_FAST");
  assert.equal(secretNameFor("中文端点", []), "TENDLY_SECRET_KEY");
  assert.equal(secretNameFor("work", ["TENDLY_SECRET_WORK", "TENDLY_SECRET_WORK_2"]), "TENDLY_SECRET_WORK_3");
});

test("references parse only in their exact form", () => {
  assert.equal(referencedSecret("env:TENDLY_SECRET_A1"), "TENDLY_SECRET_A1");
  assert.equal(referencedSecret(" env:TENDLY_SECRET_A1 "), "TENDLY_SECRET_A1");
  for (const bad of ["TENDLY_SECRET_A", "env:HOME", "env:TENDLY_SECRET_a", "", null, undefined])
    assert.equal(referencedSecret(bad), undefined);
});

test("a profile reuses only a stored key that is its own alone", () => {
  const stored = [
    { name: "TENDLY_SECRET_MINE", source: "tendly" },
    { name: "TENDLY_SECRET_ENV", source: "environment" },
  ];
  assert.equal(ownStoredSecret("env:TENDLY_SECRET_MINE", stored, []), "TENDLY_SECRET_MINE");
  assert.equal(ownStoredSecret("env:TENDLY_SECRET_MINE", stored, ["env:TENDLY_SECRET_MINE"]), undefined);
  assert.equal(ownStoredSecret("env:TENDLY_SECRET_ENV", stored, []), undefined);
  assert.equal(ownStoredSecret(null, stored, []), undefined);
});
