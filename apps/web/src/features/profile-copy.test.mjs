import { test } from "node:test";
import assert from "node:assert/strict";
import { copiedEnvironment, copiedProfilePayload, copyName } from "./profile-copy.ts";

const source = {
  id: "relay-cc",
  name: "Relay CC",
  provider: "claude_code",
  endpoint_url: "https://relay.example",
  model: "opus-fixture",
  effort: "high",
  permission_mode: "plan",
  secret_ref: "env:TENDLY_SECRET_RELAY",
  proxy_url: "http://127.0.0.1:7890",
  models: ["opus-fixture", "sonnet-fixture"],
  model_aliases: { fast: "sonnet-fixture" },
  created_at: "2026-01-01T00:00:00Z",
  environment: {
    ANTHROPIC_DEFAULT_OPUS_MODEL: { kind: "main_model" },
    ANTHROPIC_DEFAULT_HAIKU_MODEL: { kind: "literal", value: "haiku-fixture" },
    CLAUDE_CODE_MAX_CONTEXT_TOKENS: { kind: "main_model_context" },
    MY_FLAG: { kind: "literal", value: "1" },
  },
};
const draft = overrides => ({
  name: "Relay CC copy",
  provider: "claude_code",
  endpoint_url: "https://relay.example",
  model: "opus-fixture",
  secret_ref: source.secret_ref,
  ...overrides,
});

test("a copy on the same client and endpoint keeps everything but its name", () => {
  const payload = copiedProfilePayload(source, draft({}));
  assert.equal(payload.name, "Relay CC copy");
  assert.equal(payload.secret_ref, "env:TENDLY_SECRET_RELAY");
  assert.equal(payload.permission_mode, "plan");
  assert.equal(payload.effort, "high");
  assert.equal(payload.proxy_url, "http://127.0.0.1:7890");
  assert.deepEqual(payload.models, source.models);
  assert.deepEqual(payload.model_aliases, source.model_aliases);
  assert.deepEqual(payload.environment, source.environment);
});

test("another model starts on the default depth; another endpoint drops its model list and aliases", () => {
  const payload = copiedProfilePayload(
    source,
    draft({ model: "sonnet-fixture", endpoint_url: "https://backup.example" }),
  );
  assert.equal(payload.effort, undefined);
  assert.equal(payload.models, undefined);
  assert.equal(payload.model_aliases, undefined);
  assert.equal(payload.proxy_url, "http://127.0.0.1:7890");
});

test("a copy for another client drops what only the old client reads and keeps what the person added", () => {
  const payload = copiedProfilePayload(source, draft({ provider: "codex", model: "" }));
  assert.equal(payload.model, null);
  assert.equal(payload.permission_mode, "native");
  assert.deepEqual(payload.environment, { MY_FLAG: { kind: "literal", value: "1" } });
  assert.equal(payload.secret_ref, "env:TENDLY_SECRET_RELAY");
});

test("a copy onto Claude Code gets slots that follow its main model, as a new profile does", () => {
  const codex = { ...source, provider: "codex", environment: { MY_FLAG: { kind: "literal", value: "1" } } };
  const environment = copiedEnvironment(codex, "claude_code");
  assert.deepEqual(Object.keys(environment).sort(), [
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "MY_FLAG",
  ]);
  assert.equal(environment.ANTHROPIC_DEFAULT_SONNET_MODEL.kind, "main_model");
});

test("environment can be left out, for model discovery", () => {
  assert.equal("environment" in copiedProfilePayload(source, draft({}), false), false);
});

test("a copy name skips names already taken", () => {
  const label = n => (n === 1 ? "Relay copy" : `Relay copy ${n}`);
  assert.equal(copyName(["Relay"], label), "Relay copy");
  assert.equal(copyName(["Relay", "Relay copy", "Relay copy 2"], label), "Relay copy 3");
});
