import { test } from "node:test";
import assert from "node:assert/strict";
import { ref } from "vue";
import {
  PANEL_STORAGE_KEY,
  readPanelVisibility,
  sidebarIsRail,
  SIDEBAR_RAIL_MIN_WIDTH,
  SIDEBAR_WIDTH_KEY,
  useSidebarWidth,
  writePanelVisibility,
} from "./panel-state.ts";

const memory = () => {
  const map = new Map();
  return { getItem: key => map.get(key) ?? null, setItem: (key, value) => map.set(key, value), map };
};
const fallback = { sidebar: true, explorer: false };

test("panel visibility round-trips and stores only the two display flags", () => {
  const storage = memory();
  assert.equal(writePanelVisibility(storage, { sidebar: false, explorer: true }), true);
  assert.deepEqual(readPanelVisibility(storage, fallback), { sidebar: false, explorer: true });
  assert.deepEqual(JSON.parse(storage.map.get(PANEL_STORAGE_KEY)), { sidebar: false, explorer: true });
  // Nothing about workspaces, sessions or paths belongs in a display preference.
  assert.equal(storage.map.size, 1);
});

test("unreadable, partial or hostile stored panel state falls back per panel", () => {
  assert.deepEqual(readPanelVisibility(undefined, fallback), fallback);
  assert.equal(writePanelVisibility(undefined, fallback), false);
  const denied = {
    getItem() {
      throw Error("denied");
    },
    setItem() {
      throw Error("quota");
    },
  };
  assert.deepEqual(readPanelVisibility(denied, fallback), fallback);
  assert.equal(writePanelVisibility(denied, fallback), false);
  for (const stored of ["not json", "null", "[]", '"text"', "3"]) {
    const storage = memory();
    storage.setItem(PANEL_STORAGE_KEY, stored);
    assert.deepEqual(readPanelVisibility(storage, fallback), fallback, stored);
  }
  // One unusable field must not discard a preference set for the other panel.
  const partial = memory();
  partial.setItem(PANEL_STORAGE_KEY, JSON.stringify({ explorer: true, sidebar: "yes" }));
  assert.deepEqual(readPanelVisibility(partial, fallback), { sidebar: true, explorer: true });
});

test("a drawer-width viewport never treats the sidebar as a collapsible rail", () => {
  assert.equal(sidebarIsRail(SIDEBAR_RAIL_MIN_WIDTH), true);
  assert.equal(sidebarIsRail(SIDEBAR_RAIL_MIN_WIDTH - 1), false);
  assert.equal(sidebarIsRail(390), false);
  assert.equal(sidebarIsRail(1440), true);
});

test("a remembered column width applies only to an expanded rail, and a reset forgets it", () => {
  const values = new Map([[SIDEBAR_WIDTH_KEY, "260"]]);
  const storage = { getItem: key => values.get(key) ?? null, setItem: (key, value) => values.set(key, value) };
  const rail = ref(true),
    collapsed = ref(false);
  const { sidebarWidth, bodyStyle, resetSidebarWidth } = useSidebarWidth(storage, rail, collapsed);
  assert.equal(sidebarWidth.value, 260);
  assert.deepEqual(bodyStyle.value, { "--rail-left": "260px" });
  collapsed.value = true;
  assert.equal(bodyStyle.value, undefined);
  collapsed.value = false;
  rail.value = false;
  assert.equal(bodyStyle.value, undefined, "a drawer keeps its own width");
  resetSidebarWidth();
  assert.equal(sidebarWidth.value, undefined);
  assert.equal(values.get(SIDEBAR_WIDTH_KEY), "");
  values.set(SIDEBAR_WIDTH_KEY, "999");
  assert.equal(
    useSidebarWidth(storage, rail, collapsed).sidebarWidth.value,
    undefined,
    "a width out of bounds is not used",
  );
  assert.equal(
    useSidebarWidth(
      {
        getItem() {
          throw Error("blocked");
        },
        setItem() {},
      },
      rail,
      collapsed,
    ).sidebarWidth.value,
    undefined,
  );
});
