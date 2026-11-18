import { test } from "node:test";
import assert from "node:assert/strict";
import { approvalBody, approvalChoices, DECISION_LABEL } from "./chat-approvals.ts";
import { pastedFileName } from "./attachment-state.ts";
import { statusLabel } from "./chat-format.ts";

test("a request offers only the choices the pane can give, in the client order", () => {
  assert.deepEqual(approvalChoices({ choices: ["decline", "accept", "amend", "accept_session"] }), [
    "decline",
    "accept",
    "accept_session",
  ]);
  assert.deepEqual(approvalChoices({ choices: [] }), []);
  assert.equal(DECISION_LABEL.cancel, "Cancel turn");
});

test("a request's JSON splits into the reason and the rest, and other text stays as it is", () => {
  assert.deepEqual(approvalBody('{"reason":"Run the tests","command":"pnpm test","cwd":null}'), {
    reason: "Run the tests",
    detail: '{\n  "command": "pnpm test"\n}',
  });
  assert.deepEqual(approvalBody('{"reason":"  ","command":"ls"}'), {
    reason: undefined,
    detail: '{\n  "command": "ls"\n}',
  });
  assert.deepEqual(approvalBody('{"reason":"Only this"}'), { reason: "Only this", detail: undefined });
  for (const text of ["Run ls?", "[1,2]", "null", ""]) assert.equal(approvalBody(text), undefined, text);
});

test("a pasted screenshot is named by when it was pasted; a named file keeps its name", () => {
  const now = new Date("2026-10-15T08:09:10.123Z");
  assert.equal(pastedFileName("image.png", "image/png", now), "pasted-2026-10-15T08-09-10.png");
  assert.equal(pastedFileName("", "image/svg+xml", now), "pasted-2026-10-15T08-09-10.svgxml");
  assert.equal(pastedFileName("", "", now), "pasted-2026-10-15T08-09-10.bin");
  assert.equal(pastedFileName("notes.txt", "text/plain", now), "notes.txt");
});

test("the status line shows the most pressing state", () => {
  const idle = {
    preview: false,
    booting: false,
    loading: false,
    awaitingApproval: false,
    working: false,
    disconnected: false,
    structured: true,
  };
  assert.equal(statusLabel(idle), "Ready for your next idea");
  assert.equal(statusLabel({ ...idle, structured: false }), "Native client session");
  assert.equal(statusLabel({ ...idle, disconnected: true, structured: false }), "View disconnected");
  assert.equal(statusLabel({ ...idle, working: true, disconnected: true }), "Agent is working");
  assert.equal(statusLabel({ ...idle, awaitingApproval: true, working: true }), "Waiting for your approval");
  assert.equal(statusLabel({ ...idle, loading: true, awaitingApproval: true }), "Loading conversation…");
  assert.equal(statusLabel({ ...idle, booting: true, loading: true }), "Starting {provider}…");
  assert.equal(statusLabel({ ...idle, preview: true, booting: true }), "Read-only UI preview");
});
