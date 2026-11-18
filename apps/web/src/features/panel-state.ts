import { computed, ref, type Ref } from "vue";
import { draggedSidebarWidth, storedSidebarWidth } from "./app-shell";

type PanelStorage = Pick<Storage, "getItem" | "setItem">;

/**
 * Whether the shell's side panels are expanded. This is a per-browser display
 * preference, so it stays in local storage and never reaches the server: a
 * collapsed panel says nothing about workspaces, sessions or layout.
 */
export interface PanelVisibility {
  sidebar: boolean;
  explorer: boolean;
}
export const PANEL_STORAGE_KEY = "tendly.panels.v1";

/**
 * Below this width the sidebar is a drawer rather than a layout column, so a
 * remembered collapse must not apply — it would hide the drawer itself. The
 * explorer needs no such guard: it stays reachable as an overlay at every width.
 */
export const SIDEBAR_RAIL_MIN_WIDTH = 761;

export function readPanelVisibility(storage: PanelStorage | undefined, fallback: PanelVisibility): PanelVisibility {
  if (!storage) return fallback;
  try {
    const stored: unknown = JSON.parse(storage.getItem(PANEL_STORAGE_KEY) ?? "null");
    if (!stored || typeof stored !== "object" || Array.isArray(stored)) return fallback;
    const entry = stored as Partial<Record<keyof PanelVisibility, unknown>>;
    // Each panel falls back on its own, so one unreadable field never discards
    // a preference the user did set for the other panel.
    return {
      sidebar: typeof entry.sidebar === "boolean" ? entry.sidebar : fallback.sidebar,
      explorer: typeof entry.explorer === "boolean" ? entry.explorer : fallback.explorer,
    };
  } catch {
    return fallback;
  }
}

export function writePanelVisibility(storage: PanelStorage | undefined, value: PanelVisibility): boolean {
  if (!storage) return false;
  try {
    storage.setItem(PANEL_STORAGE_KEY, JSON.stringify({ sidebar: value.sidebar, explorer: value.explorer }));
    return true;
  } catch {
    return false;
  }
}

/**
 * Whether the sidebar occupies a layout column at this width. Below the rail
 * width it is a drawer, so the stored collapse is ignored for display without
 * being overwritten — widening the window restores the user's choice.
 */
export function sidebarIsRail(width: number): boolean {
  return width >= SIDEBAR_RAIL_MIN_WIDTH;
}

export const SIDEBAR_WIDTH_KEY = "tendly.sidebar-width.v1";

/**
 * The workspace column can be dragged narrower or wider; how much room it
 * needs depends on how many workspaces and sessions there are. Remembered in
 * this browser, and a double-click on the edge puts it back. A width applies
 * only while the column is a rail and expanded.
 */
export function useSidebarWidth(
  storage: PanelStorage | undefined,
  sidebarRail: Readonly<Ref<boolean>>,
  sidebarCollapsed: Readonly<Ref<boolean>>,
) {
  const sidebarWidth = ref<number | undefined>(
    (() => {
      try {
        return storedSidebarWidth(storage?.getItem(SIDEBAR_WIDTH_KEY));
      } catch {
        return undefined;
      }
    })(),
  );
  const bodyStyle = computed(() =>
    sidebarWidth.value && sidebarRail.value && !sidebarCollapsed.value
      ? { "--rail-left": sidebarWidth.value + "px" }
      : undefined,
  );
  function resizeSidebar(event: PointerEvent) {
    const handle = event.currentTarget as HTMLElement,
      left = handle.parentElement?.getBoundingClientRect().left ?? 0;
    handle.setPointerCapture(event.pointerId);
    const move = (next: PointerEvent) => {
      sidebarWidth.value = draggedSidebarWidth(next.clientX, left);
    };
    const end = () => {
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", end);
      handle.removeEventListener("pointercancel", end);
      try {
        if (sidebarWidth.value) storage?.setItem(SIDEBAR_WIDTH_KEY, String(sidebarWidth.value));
      } catch {
        /* The width lasts for this page when storage is unavailable. */
      }
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", end);
    handle.addEventListener("pointercancel", end);
  }
  function resetSidebarWidth() {
    sidebarWidth.value = undefined;
    try {
      storage?.setItem(SIDEBAR_WIDTH_KEY, "");
    } catch {
      /* Nothing stored. */
    }
  }
  return { sidebarWidth, bodyStyle, resizeSidebar, resetSidebarWidth };
}
