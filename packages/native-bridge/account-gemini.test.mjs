import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parseQuota, signedInEmail } from "./account-gemini.mjs";
import { geminiSession } from "./history-gemini.mjs";

test("agy's quota fills the account's windows from its Gemini models", () => {
  const limits = parseQuota(
    [
      "Gemini Models\tWeekly Limit Remaining\t100%\t2026-12-02T11:14:10Z",
      "Gemini Models\tFive Hour Limit Remaining\t75%\t2026-11-25T16:14:10Z",
      "Claude and GPT models\tWeekly Limit Remaining\t40%\t2026-11-26T12:55:41Z",
      "Claude and GPT models\tFive Hour Limit Remaining\t100%\t2026-11-25T16:19:58Z",
      "not a quota line",
    ].join("\n"),
  );
  assert.deepEqual(limits.primary, {
    used_percent: 25,
    window_minutes: 300,
    window_kind: "five_hour",
    resets_at: Date.parse("2026-11-25T16:14:10Z") / 1000,
  });
  assert.equal(limits.secondary.used_percent, 0);
  assert.equal(limits.secondary.window_kind, "weekly");
  assert.deepEqual(parseQuota(""), {});
});

test("a sign-in is read by its email only, and a missing or broken one is signed out", async () => {
  const home = await mkdtemp(join(tmpdir(), "tendly-gemini-account-"));
  try {
    assert.deepEqual(await signedInEmail(home), { signedIn: false });
    await mkdir(join(home, "antigravity-cli"));
    const token = join(home, "antigravity-cli", "antigravity-oauth-token");
    await writeFile(token, JSON.stringify({ token: { access_token: "secret" }, auth_method: "oauth-personal", id_token_email: "a@example.com" }));
    assert.deepEqual(await signedInEmail(home), { signedIn: true, email: "a@example.com" });
    await writeFile(token, "{");
    assert.deepEqual(await signedInEmail(home), { signedIn: false });
  } finally {
    await rm(home, { recursive: true, force: true });
  }
});

test("a listed conversation has a workspace and steps, and is neither internal nor in the cloud", () => {
  const row = { ID: "c-1", WorkspaceURIs: ["file:///work"], NumSteps: 2, UpdatedAt: "2026-11-25T00:00:00Z" };
  assert.deepEqual(geminiSession(row), { id: "c-1", cwd: "/work", time: Date.parse("2026-11-25T00:00:00Z") });
  assert.equal(geminiSession({ ...row, NumSteps: 0 }), undefined);
  assert.equal(geminiSession({ ...row, WorkspaceURIs: [] }), undefined);
  assert.equal(geminiSession({ ...row, Cloud: true }), undefined);
});
