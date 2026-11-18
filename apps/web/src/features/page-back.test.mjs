import { test } from "node:test";
import assert from "node:assert/strict";
import { backLabel } from "./page-back.ts";

test("the back button names the page it returns to, and the workspace when there is none", () => {
  assert.equal(backLabel(undefined), "Back to workspace");
  assert.equal(backLabel("canvas"), "Back to workspace");
  assert.equal(backLabel("session"), "Back to workspace");
  assert.equal(backLabel("sessions"), "Back to all sessions");
  assert.equal(backLabel("settings"), "Back to settings");
  assert.equal(backLabel("somewhere-new"), "Back");
});
