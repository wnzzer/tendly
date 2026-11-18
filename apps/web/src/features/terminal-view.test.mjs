import { test } from "node:test";
import assert from "node:assert/strict";
import { concatBytes, rowAt, terminalTheme, themeVariables } from "./terminal-view.ts";

test("a theme maps onto wterm's variables, normal colours first and bright ones eight on", () => {
  const light = themeVariables(terminalTheme(false));
  assert.equal(light["--term-bg"], "#ffffff");
  assert.equal(light["--term-fg"], "#243343");
  assert.equal(light["--term-cursor"], "#0a8278");
  assert.equal(light["--term-color-1"], "#b4374c", "red");
  assert.equal(light["--term-color-9"], "#c73f55", "bright red");
  assert.equal(light["--term-color-15"], "#384658", "bright white");
  assert.equal(Object.keys(light).length, 3 + 16);
});

test("switching to a theme without a palette clears the one set before it", () => {
  const dark = themeVariables(terminalTheme(true));
  // Present but undefined, so the view removes the variable rather than keeping the light colour.
  assert.ok("--term-color-1" in dark);
  assert.equal(dark["--term-color-1"], undefined);
  assert.equal(dark["--term-bg"], "#17232d");
});

test("a tap finds its row in an evenly divided grid", () => {
  assert.equal(rowAt(100, 100, 240, 24), 0);
  assert.equal(rowAt(109.9, 100, 240, 24), 0);
  assert.equal(rowAt(110, 100, 240, 24), 1);
  assert.equal(rowAt(339, 100, 240, 24), 23);
  // Outside the grid, or a grid that has not been laid out.
  assert.equal(rowAt(99, 100, 240, 24), undefined);
  assert.equal(rowAt(340, 100, 240, 24), undefined);
  assert.equal(rowAt(150, 100, 0, 24), undefined);
  assert.equal(rowAt(150, 100, 240, 0), undefined);
});

test("parts are joined in order into one buffer", () => {
  const joined = concatBytes([Uint8Array.of(1, 2), Uint8Array.of(), Uint8Array.of(3)]);
  assert.deepEqual([...joined], [1, 2, 3]);
  const only = Uint8Array.of(7);
  assert.equal(concatBytes([only]), only, "a single part needs no copy");
  assert.equal(concatBytes([]).byteLength, 0);
});
