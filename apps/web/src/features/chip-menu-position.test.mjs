import { test } from "node:test";
import assert from "node:assert/strict";
import { chipPanelOffset } from "./chip-menu-position.ts";

test("a panel hangs from its chip until that would hang it off the screen", () => {
  // Room to the right: the panel stays where the chip is, which is where the
  // eye already is.
  assert.equal(chipPanelOffset(100, 292, 1200), 0);
  assert.equal(chipPanelOffset(0, 292, 300), 0);
  // The rightmost chip on a phone: shifted left by exactly its overhang, so its
  // right edge lands on the margin and not a pixel further.
  assert.equal(chipPanelOffset(300, 292, 500), -100);
  assert.equal(chipPanelOffset(300, 292, 500) + 300 + 292, 500 - 8);
  // A panel wider than the screen stops at the left margin rather than running
  // off the other side: neither edge is worth sacrificing for the other.
  assert.equal(chipPanelOffset(40, 400, 300), -32);
  assert.equal(chipPanelOffset(0, 400, 300), 0);
  // A chip already at the margin has nowhere to go.
  assert.equal(chipPanelOffset(8, 292, 200), 0);
});
