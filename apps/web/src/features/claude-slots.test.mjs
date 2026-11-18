import { test } from "node:test";
import assert from "node:assert/strict";
import { followingRows, setSlot, slotState, slotSummary } from "./claude-slots.ts";
import { parseEnvironmentRows } from "./environment-model.ts";

const HAIKU = "ANTHROPIC_DEFAULT_HAIKU_MODEL";
const row = (name, kind, value = "") => ({ id: name, name, kind, value });

test("a slot is read from its environment variable, and none means the client's own model", () => {
  assert.deepEqual(slotState([], HAIKU), { mode: "client" });
  assert.deepEqual(slotState([row(HAIKU, "main_model")], HAIKU), { mode: "main" });
  assert.deepEqual(slotState([row(HAIKU, "literal", "claude-haiku-4-5")], HAIKU), {
    mode: "model",
    model: "claude-haiku-4-5",
  });
  assert.deepEqual(slotState([row(HAIKU, "unset")], HAIKU), { mode: "client" });
});

test("setting a slot edits only its own variable, in place", () => {
  const rows = [row("HTTPS_PROXY", "literal", "http://proxy"), row(HAIKU, "main_model"), row("OTHER", "literal", "x")];
  const pinned = setSlot(rows, HAIKU, { mode: "model", model: "claude-haiku-4-5" });
  assert.deepEqual(
    pinned.map(entry => [entry.name, entry.kind, entry.value]),
    [
      ["HTTPS_PROXY", "literal", "http://proxy"],
      [HAIKU, "literal", "claude-haiku-4-5"],
      ["OTHER", "literal", "x"],
    ],
  );
  assert.deepEqual(
    setSlot(pinned, HAIKU, { mode: "client" }).map(entry => entry.name),
    ["HTTPS_PROXY", "OTHER"],
    "the client default is no variable at all",
  );
  assert.equal(setSlot([], HAIKU, { mode: "main" })[0].kind, "main_model");
});

test("a new Claude Code endpoint follows its main model in every slot, shown as variables", () => {
  const rows = followingRows([row("HTTPS_PROXY", "literal", "http://proxy")]);
  assert.equal(slotSummary(rows), "main");
  const { environment, errors } = parseEnvironmentRows(rows);
  assert.deepEqual(errors, []);
  assert.deepEqual(environment.ANTHROPIC_DEFAULT_OPUS_MODEL, { kind: "main_model" });
  assert.deepEqual(environment.HTTPS_PROXY, { kind: "literal", value: "http://proxy" });
  assert.equal(slotSummary([]), "client");
  assert.equal(slotSummary(setSlot(rows, HAIKU, { mode: "client" })), "mixed");
});
