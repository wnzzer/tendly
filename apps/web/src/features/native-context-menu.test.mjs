import test from "node:test";
import assert from "node:assert/strict";
import { keepsNativeMenu } from "./native-context-menu.ts";

const element = matches => ({ closest: selector => (matches && selector.split(", ").includes(matches) ? {} : null) });

test("the browser menu gives way in the app chrome", () => {
  assert.equal(keepsNativeMenu(element(null)), false);
  assert.equal(keepsNativeMenu({ parentElement: element(null) }), false, "a text node asks its parent");
});

test("the browser menu stays for typing, selection, links, images and the terminal", () => {
  for (const kind of ["input", "textarea", "a[href]", "img", ".xterm", ".wterm", ".terminal-host", ".chat-markdown"])
    assert.equal(keepsNativeMenu(element(kind)), true, kind);
  assert.equal(keepsNativeMenu(element(null), "selected words"), true);
  assert.equal(keepsNativeMenu(null), true, "nothing to judge by");
});
