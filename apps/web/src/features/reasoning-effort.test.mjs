import assert from "node:assert/strict";
import { test } from "node:test";
import { effortFill, effortStops, effortVivid, modelEfforts, restingStop } from "./reasoning-effort.ts";

test("Claude exposes its native effort vocabulary without requiring a guessed model catalog", () => {
  assert.deepEqual(modelEfforts("claude_code", "claude-sonnet", undefined), ["low", "medium", "high", "xhigh", "max"]);
});

test("Codex only exposes levels advertised by the selected model", () => {
  const catalog = {
    models: [{ id: "o4-mini", name: "o4-mini", efforts: ["low", "high", "bogus"] }],
    source_url: "codex://model/list",
    has_more: false,
  };
  assert.deepEqual(modelEfforts("codex", "o4-mini", catalog), ["low", "high"]);
  assert.deepEqual(modelEfforts("codex", "unknown", catalog), []);
});

test("an existing effort remains visible while a refreshed catalog is temporarily missing it", () => {
  const catalog = {
    models: [{ id: "model", name: "model", efforts: ["low"] }],
    source_url: "fixture://models",
    has_more: false,
  };
  assert.deepEqual(modelEfforts("codex", "model", catalog, "high"), ["low", "high"]);
});

test("the depth slider starts with a stop that leaves the depth to the client", () => {
  assert.deepEqual(effortStops(["low", "high"]), ["", "low", "high"]);
  assert.deepEqual(effortStops([]), [""]);
});

test("the slider rests on the level in use, or at the start for one it does not offer", () => {
  const stops = effortStops(["low", "medium", "high"]);
  assert.equal(restingStop(stops, "medium"), 2);
  assert.equal(restingStop(stops, ""), 0);
  assert.equal(restingStop(stops, "max"), 0);
});

test("the track fills up to the thumb", () => {
  const stops = effortStops(["low", "medium", "high"]);
  assert.equal(effortFill(stops, 0), 0);
  assert.equal(effortFill(stops, 3), 1);
  assert.equal(effortFill(stops, 1), 1 / 3);
  assert.equal(effortFill([""], 0), 0);
});

test("only the expensive end of what is offered is marked", () => {
  const stops = effortStops(["low", "medium", "high", "max"]);
  assert.equal(effortVivid(stops, 4, "max"), true);
  assert.equal(effortVivid(stops, 3, "high"), true);
  assert.equal(effortVivid(stops, 2, "medium"), false);
  assert.equal(
    effortVivid(effortStops(["high"]), 1, "high"),
    false,
    "a single level is not the expensive end of anything",
  );
  assert.equal(effortVivid(stops, 0, ""), false);
});

test("custom endpoints permit manual effort without model metadata, but honor advertised levels", () => {
  assert.deepEqual(modelEfforts("codex", "custom", undefined, undefined, true), [
    "low",
    "medium",
    "high",
    "xhigh",
    "max",
  ]);
  assert.deepEqual(modelEfforts("pi", "custom", undefined, undefined, true), ["low", "medium", "high"]);
  const catalog = {
    models: [{ id: "custom", name: "custom", efforts: ["low", "high"] }],
    source_url: "https://example.invalid/models",
    has_more: false,
  };
  assert.deepEqual(modelEfforts("codex", "custom", catalog, undefined, true), ["low", "high"]);
  assert.deepEqual(modelEfforts("codex", "custom", undefined), []);
});
