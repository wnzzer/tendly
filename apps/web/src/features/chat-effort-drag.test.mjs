import { test } from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

const chip = await readFile(new URL("./ChatEffortChip.vue", import.meta.url), "utf8");
const pane = await readFile(new URL("./ChatSessionPane.vue", import.meta.url), "utf8");

test("the depth slider sends one change when it is let go, not one per step", () => {
  // Sending a step at a time disabled the chip under the finger -- a request is
  // in flight, so the control locks -- and the drag ended at the second stop.
  // The gesture has to be able to run to its end before anything is sent.
  assert.match(chip, /type="range"[^>]*@input="dragEffort"[^>]*@change="commitEffort"/);
  const body = (source, name, arg = "event: Event") =>
    new RegExp(`function ${name}\\(${arg}\\) \\{([^{}]*)\\}`).exec(source)?.[1] ?? "";
  assert.doesNotMatch(body(chip, "dragEffort"), /emit|applyEffort/, "a step in a drag must not send anything");
  assert.match(body(chip, "commitEffort"), /emit\("pick"/, "letting go is what picks the level");
  // The chip only picks; the session sends, and only a level other than the client's default.
  assert.match(pane, /<ChatEffortChip[^>]*@pick="pickEffort"/);
  assert.match(body(pane, "pickEffort", "stop: string"), /if \(stop\) void applyEffort\(stop\)/);
  // And the thumb has to follow the pointer while nothing is being sent, or the
  // drag would look frozen even though it works.
  assert.match(chip, /const effortIndex = computed\(\(\) => heldEffort\.value \?\?/);
});
