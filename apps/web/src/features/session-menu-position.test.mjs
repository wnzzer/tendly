import { test } from "node:test";
import assert from "node:assert/strict";
import { sessionMenuPosition } from "./session-menu-position.ts";

test("tab menus stay within narrow viewport edges", () => {
  const box = { left: 16, top: 60, right: 44, bottom: 88, width: 28, height: 28 };
  const placed = sessionMenuPosition(box, { width: 320, height: 568 }, { width: 205, height: 220 });
  assert.equal(placed.left, 8);
  assert.equal(placed.top, 93);
  assert.equal(placed.maxWidth, 304);
  assert.ok(placed.left + 205 <= 312);
});

test("bottom split menus open upwards and tall panels remain scrollable", () => {
  const box = { left: 600, top: 510, right: 628, bottom: 538, width: 28, height: 28 };
  const placed = sessionMenuPosition(box, { width: 700, height: 600 }, { width: 205, height: 220 });
  assert.equal(placed.top, 285);
  assert.equal(placed.left, 423);
  const tall = sessionMenuPosition(
    { ...box, top: 60, bottom: 88 },
    { width: 320, height: 400 },
    { width: 400, height: 600 },
  );
  assert.equal(tall.maxWidth, 304);
  assert.equal(tall.maxHeight, 299);
  assert.equal(tall.left, 8);
});

test("a style is rewritten only when it says something new", async () => {
  const { sameStyle } = await import("./session-menu-position.ts");
  assert.ok(sameStyle({ left: "1px", top: "2px" }, { top: "2px", left: "1px" }));
  assert.ok(!sameStyle({ left: "1px" }, { left: "2px" }));
  assert.ok(!sameStyle({ left: "1px" }, { left: "1px", top: "0px" }));
  assert.ok(sameStyle({}, {}));
});
