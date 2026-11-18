import { test } from "node:test";
import assert from "node:assert/strict";
import {
  ownsNativeConfig,
  sharesHostConfig,
  nativeProfileRenamePayload,
  nativeProfileUpdatePayload,
  profileEnvironmentPayload,
  profileEnvironmentDraftChanged,
  sessionEffortOverride,
  sessionModelOverride,
} from "./native-profiles.ts";
import { environmentRows } from "./environment-model.ts";

const source = {
  id: "native-codex",
  provider: "codex",
  label: "Host Codex",
  path: "/host/native/config",
  available: true,
};
const profile = {
  id: "shared-profile",
  provider: "codex",
  native_config: { source_id: source.id, config_dir: source.path },
};

test("renaming a native profile sends no mutable client settings", () => {
  assert.deepEqual(nativeProfileRenamePayload("  Host work account "), { name: "Host work account" });
});

test("native environment edits send only name and explicit overrides, including an empty replacement", () => {
  const environment = {
    MODE: { kind: "literal", value: "new" },
    OPENAI_API_KEY: { kind: "secret_ref", reference: "env:TENDLY_SECRET_FIXTURE" },
  };
  const initial = environmentRows({ MODE: { kind: "literal", value: "old" } });
  assert.deepEqual(nativeProfileUpdatePayload(" Shared ", environmentRows(environment), true, initial), {
    name: "Shared",
    environment,
  });
  assert.deepEqual(nativeProfileUpdatePayload(" Shared ", [], true, initial), { name: "Shared", environment: {} });
  assert.deepEqual(profileEnvironmentPayload([], true), { environment: {} });
});

test("unsupported backends never silently discard nonempty, invalid or cleared environment edits", () => {
  const initial = environmentRows({ MODE: { kind: "literal", value: "old" } });
  assert.deepEqual(nativeProfileUpdatePayload("Rename only", initial, false, initial), { name: "Rename only" });
  assert.deepEqual(profileEnvironmentPayload([], false), {});
  assert.throws(
    () => profileEnvironmentPayload(environmentRows({ MODE: { kind: "literal", value: "new" } }), false, initial),
    /Upgrade/,
  );
  assert.throws(() => profileEnvironmentPayload([], false, initial), /Upgrade/);
  assert.throws(
    () => profileEnvironmentPayload([{ id: "draft", name: "", kind: "literal", value: "keep draft" }], false),
    /Upgrade/,
  );
  assert.throws(
    () => profileEnvironmentPayload(environmentRows({ HOME: { kind: "literal", value: "/forbidden" } }), true),
    /cannot be overridden/,
  );
  assert.equal(
    profileEnvironmentDraftChanged(
      initial.map(row => ({ ...row, id: "different-ui-row" })),
      initial,
    ),
    false,
  );
});

test("shared native sessions ignore stale model overrides rather than changing the original configuration", () => {
  assert.deepEqual(sessionModelOverride("codex", profile, "stale-custom-model"), {});
  assert.deepEqual(sessionModelOverride("claude_code", { ...profile, provider: "claude_code" }, "sonnet"), {});
});

test("custom and isolated sessions retain model selection; terminals never get a model override", () => {
  assert.deepEqual(sessionModelOverride("codex", undefined, "  custom-model  "), { model: "custom-model" });
  assert.deepEqual(sessionModelOverride("claude_code", { native_config: null }, "fast"), { model: "fast" });
  assert.deepEqual(sessionModelOverride("codex", undefined, "  "), {});
  assert.deepEqual(sessionModelOverride("terminal", undefined, "custom-model"), {});
});

test("session effort overrides stay with the selected non-native client configuration", () => {
  assert.deepEqual(sessionEffortOverride("codex", undefined, " high "), { effort: "high" });
  assert.deepEqual(sessionEffortOverride("claude_code", { native_config: null }, "low"), { effort: "low" });
  assert.deepEqual(sessionEffortOverride("codex", profile, "high"), {});
  assert.deepEqual(sessionEffortOverride("terminal", undefined, "high"), {});
  assert.deepEqual(sessionEffortOverride("codex", undefined, "  "), {});
});

test("a directory Tendly made for one account is not a shared host sign-in", () => {
  // The distinction the UI got wrong: having a native_config says the profile
  // runs against a client's configuration directory, not whose directory it is.
  const owned = {
    source_id: "account:4b624e9e-e721-4207-be56-9a9b17c0663d",
    config_dir: "/state/.tendly/accounts/4b624e9e",
  };
  assert.equal(ownsNativeConfig(owned), true);
  assert.equal(sharesHostConfig(owned), false);
  // An imported one points at a directory the host already had, where a profile
  // switcher or a native re-login changes this account too.
  for (const source_id of ["claude-default", "codex-default", "claude-cc-switch"]) {
    assert.equal(sharesHostConfig({ source_id, config_dir: "/home/kl/.claude" }), true, source_id);
    assert.equal(ownsNativeConfig({ source_id, config_dir: "/home/kl/.claude" }), false, source_id);
  }
  // A profile with no native configuration is neither; it has no directory to
  // describe, so both answers stay false rather than defaulting to shared.
  for (const empty of [undefined, null]) {
    assert.equal(ownsNativeConfig(empty), false);
    assert.equal(sharesHostConfig(empty), false);
  }
  // The prefix has to open the id: a directory merely named after the word is
  // still the host's.
  assert.equal(ownsNativeConfig({ source_id: "host-account:1" }), false);
});
