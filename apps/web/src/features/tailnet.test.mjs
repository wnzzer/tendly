import test from "node:test";
import assert from "node:assert/strict";
import { tailnetSteps, tokenFromHash } from "./tailnet.ts";

const base = {
  enabled: true,
  forced: false,
  phase: "off",
  url: null,
  machine: null,
  verified: null,
  checked_at: null,
  problem: null,
  target: "http://127.0.0.1:51000",
  via_tailnet: false,
};
const machine = {
  name: "mac.tail1234.ts.net",
  ip: "100.101.102.103",
  login: "me@example.com",
  tailnet: "me@example.com",
  https: true,
};
const states = status =>
  tailnetSteps(status)
    .map(step => step.state)
    .join(" ");
const problem = code => ({ code, detail: "", command: null, link: null });

test("nothing is marked done while switched off", () => {
  assert.equal(states({ ...base, enabled: false }), "pending pending pending pending pending");
});

test("an attempt under way shows how far it got", () => {
  assert.equal(states({ ...base, phase: "starting" }), "active pending pending pending pending");
  assert.equal(states({ ...base, phase: "starting", machine }), "done done done active pending");
});

test("a failure stops at the step it belongs to", () => {
  assert.equal(
    states({ ...base, phase: "error", problem: problem("not_installed") }),
    "failed pending pending pending pending",
  );
  assert.equal(
    states({ ...base, phase: "error", problem: problem("not_running") }),
    "done failed pending pending pending",
  );
  assert.equal(
    states({ ...base, phase: "error", problem: problem("no_magic_dns") }),
    "done done failed pending pending",
  );
  assert.equal(
    states({ ...base, phase: "error", machine, problem: problem("serve_consent") }),
    "done done done failed pending",
  );
  // A code this page does not know yet still lands on publishing.
  assert.equal(
    states({ ...base, phase: "error", machine, problem: problem("something_new") }),
    "done done done failed pending",
  );
});

test("a live address is only fully done once this machine reached it", () => {
  assert.equal(states({ ...base, phase: "live", machine, verified: null }), "done done done done active");
  assert.equal(states({ ...base, phase: "live", machine, verified: true }), "done done done done done");
  assert.equal(
    states({ ...base, phase: "live", machine, verified: false, problem: problem("unreachable") }),
    "done done done done warning",
  );
});

test("a sign-in link carries its token in the fragment only", () => {
  assert.equal(tokenFromHash("#tendly-token=abc123"), "abc123");
  assert.equal(tokenFromHash("tendly-token=abc123&x=1"), "abc123");
  assert.equal(tokenFromHash("#other=1"), null);
  assert.equal(tokenFromHash("#tendly-token="), null);
  assert.equal(tokenFromHash(""), null);
});
