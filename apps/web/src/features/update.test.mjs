import test from "node:test";
import assert from "node:assert/strict";
import { updateStage, waitForVersion } from "./update.ts";

const base = {
  current: "0.1.27",
  latest: "0.1.30",
  available: true,
  installed: null,
  method: "npm",
  writable: true,
  command: "npm install -g @wnzzer/tendly@latest",
  restart: "daemon",
  running_sessions: 0,
  check_error: null,
};

test("a waiting restart is offered before anything else", () => {
  assert.equal(updateStage({ ...base, installed: "0.1.30", available: false }), "restart");
  assert.equal(updateStage({ ...base, installed: "0.1.30", method: "manual" }), "restart");
});

test("an install the server cannot replace shows a command instead of a button", () => {
  assert.equal(updateStage({ ...base, method: "manual", latest: null, available: false }), "manual");
  assert.equal(updateStage({ ...base, writable: false }), "needs-permission");
});

test("a failed check is not reported as up to date", () => {
  assert.equal(updateStage({ ...base, latest: null, available: false, check_error: "offline" }), "check-failed");
  assert.equal(updateStage({ ...base, latest: "0.1.27", available: false }), "current");
  assert.equal(updateStage(base), "available");
});

function clock() {
  let time = 0;
  return {
    now: () => time,
    sleep: async ms => {
      time += ms;
    },
  };
}

test("waiting for a restart ignores the outage and the old version, and ends on the new one", async () => {
  const answers = [
    new Error("down"),
    { ok: true, version: "0.1.27" },
    new Error("down"),
    { ok: true, version: "0.1.30" },
  ];
  const health = async () => {
    const next = answers.shift();
    if (next instanceof Error) throw next;
    return next;
  };
  assert.equal(await waitForVersion("0.1.30", health, { ...clock(), intervalMs: 1000, timeoutMs: 10_000 }), true);
});

test("a restart that never brings the new version back times out", async () => {
  const health = async () => ({ ok: true, version: "0.1.27" });
  assert.equal(await waitForVersion("0.1.30", health, { ...clock(), intervalMs: 1000, timeoutMs: 5_000 }), false);
});
