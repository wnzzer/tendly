import test from "node:test";
import assert from "node:assert/strict";
import { MENU_SELECTOR } from "./dismiss-menus.ts";

test("outside presses close menus but never content disclosures", () => {
  for (const menu of ["dock-add-menu", "dock-layout-menu", "chat-menu", "session-menu", "chip-menu"])
    assert.ok(MENU_SELECTOR.includes(`details.${menu}[open]`), menu);
  for (const content of ["environment-editor", "chat-tool-run", "account-more", "advanced-directory"])
    assert.ok(!MENU_SELECTOR.includes(content), content);
});
