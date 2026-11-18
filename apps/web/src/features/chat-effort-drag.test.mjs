import { test } from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

const source = await readFile(new URL("./ChatSessionPane.vue", import.meta.url), "utf8");

test("the depth slider sends one change when it is let go, not one per step", () => {
  // Sending a step at a time disabled the chip under the finger -- a request is
  // in flight, so the control locks -- and the drag ended at the second stop.
  // The gesture has to be able to run to its end before anything is sent.
  assert.match(source, /type="range"[^>]*@input="dragEffort"[^>]*@change="commitEffort"/);
  const body = name => new RegExp(`function ${name}\\(event: Event\\) \\{([^{}]*)\\}`).exec(source)?.[1] ?? "";
  assert.doesNotMatch(body("dragEffort"), /applyEffort/, "a step in a drag must not send anything");
  assert.match(body("commitEffort"), /applyEffort/, "letting go is what sends the level");
  // And the thumb has to follow the pointer while nothing is being sent, or the
  // drag would look frozen even though it works.
  assert.match(source, /const effortIndex = computed\(\(\) => heldEffort\.value \?\?/);
});
