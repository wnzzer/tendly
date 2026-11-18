import { test } from "node:test";
import assert from "node:assert/strict";
import { environmentRows, mergeEnvironment, newEnvironmentRow, parseEnvironmentRows } from "./environment-model.ts";

const row = (name, value = "", kind = "literal") => ({ id: name, name, kind, value });
const bytes = value => new TextEncoder().encode(value).byteLength;
const parsed = (...rows) => parseEnvironmentRows(rows);
const valid = (...rows) => {
  const result = parsed(...rows);
  assert.deepEqual(result.errors, []);
  return result.environment;
};
const invalid = (rows, expected) => {
  const result = parseEnvironmentRows(rows);
  assert.ok(result.errors.includes(expected), JSON.stringify(result.errors));
};

test("environment drafts have stable distinct IDs and explicit rows round trip without a host snapshot", () => {
  const first = newEnvironmentRow(),
    second = newEnvironmentRow();
  assert.notEqual(first.id, second.id);
  assert.deepEqual({ ...first, id: "" }, { id: "", name: "", kind: "literal", value: "" });
  assert.deepEqual(environmentRows(), []);
  const source = Object.freeze({
    MODE: Object.freeze({ kind: "literal", value: "fixture" }),
    OPENAI_API_KEY: Object.freeze({ kind: "secret_ref", reference: "env:TENDLY_SECRET_FIXTURE" }),
    HTTPS_PROXY: Object.freeze({ kind: "unset" }),
  });
  const rows = environmentRows(source);
  assert.equal(new Set(rows.map(value => value.id)).size, rows.length);
  assert.deepEqual(valid(...rows), source);
  rows[0].value = "edited";
  assert.equal(source.MODE.value, "fixture");
});

test("literal values preserve whitespace, empty strings and shell-looking text without expansion", () => {
  const value = "  $PATH ${FIXTURE} $(not-a-command) `not-a-command` ~\\bin\nsecond line  ";
  assert.deepEqual(valid(row(" PATH ", value), row("EMPTY")), {
    PATH: { kind: "literal", value },
    EMPTY: { kind: "literal", value: "" },
  });
});

test("unset emits a removal marker without persisting a stale editor value", () => {
  assert.deepEqual(valid(row("OPENAI_API_KEY", "stale-draft-value", "unset")), {
    OPENAI_API_KEY: { kind: "unset" },
  });
});

test("secret references remain symbolic and accept only the dedicated server namespace", () => {
  assert.deepEqual(valid(row("OPENAI_API_KEY", "  env:TENDLY_SECRET_FIXTURE_9  ", "secret_ref")), {
    OPENAI_API_KEY: { kind: "secret_ref", reference: "env:TENDLY_SECRET_FIXTURE_9" },
  });
  for (const value of [
    "",
    "env:TENDLY_SECRET_",
    "env:OTHER",
    "env:TENDLY_SECRET_lower",
    "ENV:TENDLY_SECRET_X",
    "env:TENDLY_SECRET_A-B",
    "${TENDLY_SECRET_X}",
    "env:TENDLY_SECRET_A\0B",
    "env:TENDLY_SECRET_A\nB",
  ]) {
    invalid([row("OPENAI_API_KEY", value, "secret_ref")], "Use env:TENDLY_SECRET_NAME for a secret reference.");
  }
});

test("sensitive variable names reject plaintext case-insensitively but permit references or unset", () => {
  for (const name of [
    "ANTHROPIC_AUTH_TOKEN",
    "my_secret",
    "PASSWORD",
    "ssh_private_key",
    "openai_api_key",
    "HTTP_AUTHORIZATION",
  ]) {
    invalid(
      [row(name, "synthetic-private-value")],
      "Sensitive variables require a secret reference, not a plain value.",
    );
    valid(row(name, "env:TENDLY_SECRET_FIXTURE", "secret_ref"));
    valid(row(name, "", "unset"));
  }
});

test("account, workspace and Tendly-reserved names are blocked for every value type and case", () => {
  for (const name of [
    "HOME",
    "userprofile",
    "Pwd",
    "oldpwd",
    "CodeX_HoMe",
    "claude_config_dir",
    "ClaudeCode",
    "TENDLY_",
    "tendly_future_setting",
    "TENDLY_SECRET_FIXTURE",
  ]) {
    for (const kind of ["literal", "secret_ref", "unset"]) {
      invalid(
        [row(name, kind === "secret_ref" ? "env:TENDLY_SECRET_FIXTURE" : "fixture", kind)],
        "Account directories, workspace identity and Tendly internal variables cannot be overridden.",
      );
    }
  }
});

test("environment names use the same ASCII and 128-character boundary as Rust", () => {
  valid(row("_"), row("a".repeat(128)), row("fixture_2"));
  for (const name of ["", "  ", "2INVALID", "A-B", "A=B", "A B", "A\0B", "名字", "a".repeat(129)]) {
    invalid([row(name)], "Use a valid environment variable name.");
  }
  invalid([row("MODE"), row(" MODE ")], "Environment variable names must be unique.");
  valid(row("MODE"), row("mode"));
});

test("prototype-looking names are plain data keys during parsing, rendering and merge", () => {
  const environment = valid(
    row("__proto__", "proto-fixture"),
    row("constructor", "constructor-fixture"),
    row("hasOwnProperty", "method-fixture"),
  );
  assert.equal(Object.getPrototypeOf(environment), Object.prototype);
  assert.equal(Object.hasOwn(environment, "__proto__"), true);
  assert.deepEqual(environment.__proto__, { kind: "literal", value: "proto-fixture" });
  assert.equal(Object.hasOwn(Object.prototype, "kind"), false);
  assert.deepEqual(valid(...environmentRows(environment)), environment);
  const merged = mergeEnvironment(environment, valid(row("__proto__", "", "unset")));
  assert.equal(Object.getPrototypeOf(merged), Object.prototype);
  assert.equal(Object.hasOwn(merged, "__proto__"), true);
  assert.deepEqual(merged.__proto__, { kind: "unset" });
  assert.deepEqual(merged.constructor, { kind: "literal", value: "constructor-fixture" });
});

test("literal values enforce 8192 UTF-8 bytes rather than JavaScript character counts", () => {
  valid(row("VALUE", "x".repeat(8192)));
  valid(row("VALUE", "字".repeat(2730)));
  valid(row("VALUE", "😀".repeat(2048)));
  for (const value of ["x".repeat(8193), "字".repeat(2731), "😀".repeat(2049), "nul\0suffix"]) {
    invalid([row("VALUE", value)], "Environment values must be at most 8192 bytes and cannot contain NUL.");
  }
});

test("64 explicit keys are accepted and the 65th is rejected", () => {
  const rows = Array.from({ length: 64 }, (_, i) => row("KEY_" + i, "", "unset"));
  valid(...rows);
  invalid([...rows, row("KEY_64", "", "unset")], "At most 64 environment overrides are allowed.");
});

test("64 KiB limit includes UTF-8 JSON field names, tags, and escaped literal bytes", () => {
  const rows = Array.from({ length: 8 }, (_, i) => row("KEY_" + i, i < 7 ? "x".repeat(8192) : ""));
  const remaining = 64 * 1024 - bytes(JSON.stringify(valid(...rows)));
  assert.ok(remaining > 0 && remaining <= 8192);
  rows[7].value = "x".repeat(remaining);
  assert.equal(bytes(JSON.stringify(valid(...rows))), 64 * 1024);
  rows[7].value += "x";
  invalid(rows, "Environment overrides exceed the 64 KiB limit.");

  const escaped = Array.from({ length: 8 }, (_, i) => row("ESCAPED_" + i, "\\".repeat(4096)));
  invalid(escaped, "Environment overrides exceed the 64 KiB limit.");
});

test("proxy and provider base URL credentials cannot be persisted as literal values", () => {
  for (const name of ["HTTP_PROXY", "https_proxy", "ALL_PROXY", "OPENAI_BASE_URL", "anthropic_base_url"]) {
    for (const value of [
      "https://user:password@example.invalid/v1",
      "http://user:@example.invalid",
      "//user:password@example.invalid",
      "user:password@example.invalid",
      "http://user%40name@example.invalid",
    ]) {
      invalid([row(name, value)], "URLs containing credentials require a secret reference.");
    }
    valid(row(name, "https://example.invalid/v1"));
    valid(row(name, "127.0.0.1:9000"));
    valid(row(name, "https://example.invalid/path@leaf"));
    valid(row(name, "env:TENDLY_SECRET_PROXY", "secret_ref"));
  }
});

test("session overrides win over profile defaults while preserving unrelated defaults and removals", () => {
  const base = Object.freeze(valid(row("KEEP", "profile"), row("MODE", "profile"), row("REMOVE", "profile")));
  const overrides = Object.freeze(
    valid(
      row("MODE", "session"),
      row("REMOVE", "ignored", "unset"),
      row("OPENAI_API_KEY", "env:TENDLY_SECRET_FIXTURE", "secret_ref"),
    ),
  );
  assert.deepEqual(mergeEnvironment(base, overrides), {
    KEEP: { kind: "literal", value: "profile" },
    MODE: { kind: "literal", value: "session" },
    REMOVE: { kind: "unset" },
    OPENAI_API_KEY: { kind: "secret_ref", reference: "env:TENDLY_SECRET_FIXTURE" },
  });
  assert.equal(base.MODE.value, "profile");
  assert.equal(overrides.MODE.value, "session");
  assert.deepEqual(mergeEnvironment(), {});
  assert.notEqual(mergeEnvironment(base), base);
});

test("the effective merged environment must also pass limits, even when both inputs are valid", () => {
  const base = valid(...Array.from({ length: 64 }, (_, i) => row("KEY_" + i, "", "unset")));
  const overrides = valid(row("EXTRA", "session"));
  invalid(environmentRows(mergeEnvironment(base, overrides)), "At most 64 environment overrides are allowed.");
});

test("unsupported editor value types fail validation instead of silently creating a value", () => {
  const result = parsed(row("MODE", "fixture", "unknown"));
  assert.deepEqual(result.environment, {});
  assert.ok(result.errors.includes("Choose a supported environment value type."));
});
