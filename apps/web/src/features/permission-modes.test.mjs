import { test } from "node:test";
import assert from "node:assert/strict";
import { PERMISSION_LABELS, PERMISSION_NOTES, permissionName } from "./permission-modes.ts";

const t = text => `«${text}»`;

test("every named mode also says what it does", () => {
  assert.deepEqual(Object.keys(PERMISSION_NOTES).sort(), Object.keys(PERMISSION_LABELS).sort());
});

test("the chip names the mode in use, translated", () => {
  assert.equal(permissionName("accept_edits", t), "«Accept edits»");
  assert.equal(permissionName("danger", t), "«Never ask»");
});

test("a mode this does not know keeps the client's name, and none at all reads as the control", () => {
  assert.equal(permissionName("bypass_sandbox", t), "«bypass_sandbox»");
  assert.equal(permissionName("", t), "«Approval»");
});
