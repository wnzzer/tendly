import { test } from "node:test";
import assert from "node:assert/strict";
import { copyText, preserveNativeContextMenu } from "./clipboard.ts";

test("touch context menus stay native; desktop right-click still works", () => {
  assert.equal(preserveNativeContextMenu({ pointerType: "touch" }, false), true);
  assert.equal(preserveNativeContextMenu({}, true), true);
  assert.equal(preserveNativeContextMenu({ pointerType: "mouse" }, false), false);
});

test("clipboard uses the exact text, including Unicode and newlines", async () => {
  let copied;
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, "navigator");
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: {
      clipboard: {
        writeText: async text => {
          copied = text;
        },
      },
    },
  });
  try {
    await copyText('你好\n  echo "test"\n');
    assert.equal(copied, '你好\n  echo "test"\n');
  } finally {
    if (descriptor) Object.defineProperty(globalThis, "navigator", descriptor);
    else delete globalThis.navigator;
  }
});
