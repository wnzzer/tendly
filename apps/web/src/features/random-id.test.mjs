import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { randomId } from "./random-id.ts";

const UUID_V4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

/** Swap the global crypto for the duration of one call. */
function withCrypto(replacement, body) {
  const original = Object.getOwnPropertyDescriptor(globalThis, "crypto");
  Object.defineProperty(globalThis, "crypto", { value: replacement, configurable: true, writable: true });
  try {
    return body();
  } finally {
    if (original) Object.defineProperty(globalThis, "crypto", original);
    else delete globalThis.crypto;
  }
}

test("a secure context uses the browser implementation", () => {
  let asked = 0;
  const id = withCrypto(
    {
      randomUUID: () => {
        asked++;
        return "11111111-2222-4333-8444-555555555555";
      },
    },
    randomId,
  );
  assert.equal(asked, 1);
  assert.equal(id, "11111111-2222-4333-8444-555555555555");
});

test("an insecure origin still produces a usable id", () => {
  // This is the whole point: over plain HTTP to a LAN address, randomUUID is
  // not merely unreliable, it is undefined, and calling it throws. That threw
  // inside the composer, so a message simply would not send.
  const insecure = {
    getRandomValues: bytes => {
      for (let i = 0; i < bytes.length; i++) bytes[i] = (i * 37) % 256;
      return bytes;
    },
  };
  const id = withCrypto(insecure, randomId);
  assert.match(id, UUID_V4, id);
});

test("the fallback is a real version 4 UUID, because the server accepts one format", () => {
  const insecure = {
    getRandomValues: bytes => {
      for (let i = 0; i < bytes.length; i++) bytes[i] = 0xff;
      return bytes;
    },
  };
  const id = withCrypto(insecure, randomId);
  // Version and variant have to survive whatever the random bytes were.
  assert.equal(id[14], "4", id);
  assert.ok("89ab".includes(id[19]), id);
  assert.match(id, UUID_V4, id);
});

test("no crypto at all still returns an id rather than throwing", () => {
  const id = withCrypto(undefined, randomId);
  assert.match(id, UUID_V4, id);
});

test("ids do not repeat", () => {
  const insecure = {
    getRandomValues: bytes => {
      for (let i = 0; i < bytes.length; i++) bytes[i] = Math.floor(Math.random() * 256);
      return bytes;
    },
  };
  const seen = new Set(Array.from({ length: 500 }, () => withCrypto(insecure, randomId)));
  assert.equal(seen.size, 500);
});

test("the composer builds its message id through the helper, not the raw API", () => {
  // The call that broke was in the send path, so the send path is what this
  // pins: a direct crypto.randomUUID() there is the bug returning.
  for (const file of ["ChatSessionPane.vue", "account-ui.ts"]) {
    const source = readFileSync(new URL(`./${file}`, import.meta.url), "utf8");
    assert.doesNotMatch(source, /crypto\.randomUUID/, `${file} must not call the secure-context API directly`);
  }
  const pane = readFileSync(new URL("./ChatSessionPane.vue", import.meta.url), "utf8");
  assert.match(pane, /messageId = randomId\(\)/);
});
