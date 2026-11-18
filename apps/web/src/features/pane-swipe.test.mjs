import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { swipeAxis, swipeBlockedAt, swipeOffset, swipeOutcome } from "./pane-swipe.ts";
import { markupOf } from "../source-text.mjs";

test("a swipe picks an axis only once the finger has clearly moved", () => {
  assert.equal(swipeAxis(4, 3), undefined);
  assert.equal(swipeAxis(-30, 6), "x");
  // A slightly diagonal scroll stays a scroll.
  assert.equal(swipeAxis(20, 18), "y");
  assert.equal(swipeAxis(2, 40), "y");
});

test("a released swipe commits on distance or a flick, and never past the ends", () => {
  const base = { width: 400, hasPrevious: true, hasNext: true };
  assert.equal(swipeOutcome({ ...base, dx: -150, elapsedMs: 600 }), 1);
  assert.equal(swipeOutcome({ ...base, dx: 150, elapsedMs: 600 }), -1);
  // Short and slow goes back.
  assert.equal(swipeOutcome({ ...base, dx: -60, elapsedMs: 600 }), 0);
  // Short and fast is a flick.
  assert.equal(swipeOutcome({ ...base, dx: -60, elapsedMs: 90 }), 1);
  assert.equal(swipeOutcome({ ...base, dx: -150, elapsedMs: 300, hasNext: false }), 0);
  assert.equal(swipeOutcome({ ...base, dx: 150, elapsedMs: 300, hasPrevious: false }), 0);
});

test("past the first or last view the swipe resists instead of following", () => {
  assert.equal(swipeOffset(-100, true), -100);
  assert.ok(Math.abs(swipeOffset(-100, false)) < 30);
});

test("a touch inside a field or a sideways scroller is left to the page", () => {
  const root = { parentElement: null, tagName: "DIV", scrollWidth: 400, clientWidth: 400 };
  const code = { parentElement: root, tagName: "PRE", scrollWidth: 900, clientWidth: 300 };
  const span = { parentElement: code, tagName: "SPAN", scrollWidth: 50, clientWidth: 50 };
  const overflow = element => (element === code ? "auto" : "visible");
  assert.equal(swipeBlockedAt(span, root, overflow), true);
  // Wide content that is clipped rather than scrollable does not block it.
  assert.equal(
    swipeBlockedAt(span, root, () => "hidden"),
    false,
  );
  const field = { parentElement: root, tagName: "TEXTAREA", scrollWidth: 10, clientWidth: 10 };
  assert.equal(
    swipeBlockedAt(field, root, () => "visible"),
    true,
  );
  const editable = { parentElement: root, tagName: "DIV", isContentEditable: true, scrollWidth: 10, clientWidth: 10 };
  assert.equal(
    swipeBlockedAt(editable, root, () => "visible"),
    true,
  );
});

test("the phone canvas listens for the swipe and leaves the terminal keys alone", () => {
  const source = markupOf(new URL("../components/Canvas.vue", import.meta.url));
  assert.match(source, /class="dock-canvas-stage is-compact" @touchstart\.passive="swipeStart" @touchmove="swipeMove"/);
  assert.match(source, /\.terminal-keys/);
});
