import { nextTick, ref, watch, type Ref } from "vue";
import type { LayoutDocument, LayoutNode, PaneNode, Session, Workspace } from "@tendly/protocol";
import { createDefaultLayoutDocument, flattenPanes, validateLayout } from "../layout/layout-engine";
import { ApiError, json, request, workspacePath } from "./api";
import { backendCapabilities } from "./backend-capabilities";
import { readCanvasCache, sameCanvasLayout, writeCanvasCache, type browserStorage } from "./canvas-cache";
import { markEphemeralPanes, scopeLegacyLayout } from "./pane-context";

/**
 * Where the shared canvas lives and how it gets there: loaded from the server
 * (or this browser, or a workspace's old layout) when the app starts, saved
 * back a moment after every change, and reconciled when another page or an
 * agent saved first.
 */

export interface SharedCanvas { layout: LayoutDocument | null; revision: number }

/** A server answer worth believing: a whole, non-negative revision, and a layout that is either absent or valid. */
export function isSharedCanvas(shared: SharedCanvas): boolean {
  return Number.isSafeInteger(shared.revision) && shared.revision >= 0 && (shared.layout === null || validateLayout(shared.layout));
}

/** The pane to focus in a layout just loaded: the first pane, or the tab a stack had in front. */
export function initialPane(node: LayoutNode): PaneNode | undefined {
  if (node.type === "pane") return node;
  if (node.type === "stack") return node.panes.find(pane => pane.id === node.activePaneId) ?? node.panes[0];
  return initialPane(node.first) ?? initialPane(node.second);
}

/** The save states the status bar shows as a problem: the layout is not on the server and may not be anywhere. */
export const LAYOUT_TROUBLE = ["Save failed", "Save conflict", "Memory only"];
export const layoutInTrouble = (status: string) => LAYOUT_TROUBLE.includes(status);

export function copyLayout(document: LayoutDocument): LayoutDocument { return JSON.parse(JSON.stringify(document)); }

export interface CanvasPersistenceOptions {
  layout: Ref<LayoutDocument>;
  selectedPaneId: Ref<string | undefined>;
  sessions: Ref<Session[]>;
  storage: ReturnType<typeof browserStorage>;
  storageKey: Readonly<Ref<string | undefined>>;
  report: (cause: unknown) => void;
  /** A canvas is in place and focused, before its first save: the shell points its explorer at it. */
  onReady: (preferred: Workspace) => void;
}

export function useCanvasPersistence({ layout, selectedPaneId, sessions, storage, storageKey, report, onReady }: CanvasPersistenceOptions) {
  const canvasReady = ref(false), layoutStatus = ref("Saved"), canvasRevision = ref(0), canvasConflict = ref(false);
  const localRecovery = ref<LayoutDocument>();
  // While a layout is being put in place from outside, the watcher below must
  // not read it as an edit and save it straight back.
  let suspendLayoutSave = true, disposed = false;
  let layoutTimer: ReturnType<typeof setTimeout> | undefined, pendingLayout: LayoutDocument | undefined;
  let saveQueue: Promise<void> = Promise.resolve();

  /** Everything that leaves this page — server canvas, browser cache, recovery
   * snapshot — marks temporary windows, so a reload keeps them open (and
   * closable) while their session lives, and drops them once it is discarded. */
  function persistableLayout(document: LayoutDocument) { return markEphemeralPanes(document, sessions.value); }
  function cacheLayout(document: LayoutDocument, pending: boolean) { return writeCanvasCache(storage, storageKey.value, persistableLayout(document), pending); }
  function queueLayout(document: LayoutDocument) {
    const snapshot = persistableLayout(copyLayout(document));
    const cached = cacheLayout(snapshot, backendCapabilities.sharedCanvas);
    if (!backendCapabilities.sharedCanvas) { layoutStatus.value = cached ? "Saved in this browser" : "Memory only"; return; }
    pendingLayout = snapshot; layoutStatus.value = canvasConflict.value ? "Save conflict" : "Unsaved";
    clearTimeout(layoutTimer); layoutTimer = setTimeout(() => void flushLayout(), 450);
  }
  function persist(document: LayoutDocument) {
    saveQueue = saveQueue.then(async () => {
      if (canvasConflict.value || disposed) return;
      layoutStatus.value = "Saving…";
      try {
        const saved = await request<{ revision: number }>("/canvas/layout", json("PUT", { layout: document, expected_revision: canvasRevision.value }));
        canvasRevision.value = saved.revision;
        if (!pendingLayout && sameCanvasLayout(document, persistableLayout(layout.value))) { cacheLayout(document, false); layoutStatus.value = "Saved"; }
      } catch (cause) {
        if (cause instanceof ApiError && cause.status === 409) {
          canvasConflict.value = true; layoutStatus.value = "Save conflict";
        } else { layoutStatus.value = "Save failed"; report(cause); }
      }
    });
    return saveQueue;
  }
  async function flushLayout() {
    clearTimeout(layoutTimer);
    if (pendingLayout) { const document = pendingLayout; pendingLayout = undefined; void persist(document); }
    await saveQueue;
  }
  watch(layout, document => {
    if (suspendLayoutSave || !canvasReady.value) return;
    if (selectedPaneId.value && !flattenPanes(document.root).some(pane => pane.id === selectedPaneId.value)) selectedPaneId.value = initialPane(document.root)?.id;
    queueLayout(document);
  }, { deep: true });
  /** The server's layout replaces this window's without being saved back; the focus stays put if `keepFocus` and its pane is still there. */
  async function adoptServerLayout(document: LayoutDocument, keepFocus: boolean) {
    suspendLayoutSave = true; layout.value = document;
    if (!keepFocus || !flattenPanes(document.root).some(pane => pane.id === selectedPaneId.value)) selectedPaneId.value = initialPane(document.root)?.id;
    await nextTick(); suspendLayoutSave = false; cacheLayout(layout.value, false); layoutStatus.value = "Saved";
  }

  async function initializeCanvas(preferred: Workspace) {
    suspendLayoutSave = true;
    const local = readCanvasCache(storage, storageKey.value);
    let chosen: LayoutDocument | undefined, initialSave = false;
    if (backendCapabilities.sharedCanvas) {
      const shared = await request<SharedCanvas>("/canvas/layout");
      if (!isSharedCanvas(shared)) throw new Error("Invalid shared canvas response");
      canvasRevision.value = shared.revision;
      if (shared.layout) {
        chosen = shared.layout;
        // This browser's unsaved change lost to the server's: kept aside to restore, not dropped.
        if (local?.pending && !sameCanvasLayout(local.layout, chosen)) {
          localRecovery.value = local.layout;
          writeCanvasCache(storage, storageKey.value + ":recovery", local.layout, true);
        } else localRecovery.value = readCanvasCache(storage, storageKey.value + ":recovery")?.layout;
      } else { chosen = local?.layout; initialSave = true; }
    } else chosen = local?.layout;
    if (!chosen) {
      const legacy = await request<LayoutDocument | null>(workspacePath(preferred.id) + "/layout");
      chosen = scopeLegacyLayout(legacy?.root && validateLayout(legacy) ? legacy : createDefaultLayoutDocument(), preferred.id, sessions.value);
      initialSave = true;
    }
    layout.value = copyLayout(chosen);
    selectedPaneId.value = initialPane(chosen.root)?.id;
    canvasReady.value = true; canvasConflict.value = false;
    onReady(preferred);
    await nextTick();
    suspendLayoutSave = false;
    if (!backendCapabilities.sharedCanvas || initialSave) queueLayout(layout.value);
    else layoutStatus.value = "Saved";
  }
  async function resolveCanvasConflict(useCurrent: boolean) {
    await flushLayout();
    try {
      const shared = await request<SharedCanvas>("/canvas/layout");
      if (!isSharedCanvas(shared)) throw new Error("Invalid shared canvas response");
      canvasRevision.value = shared.revision; canvasConflict.value = false;
      if (useCurrent) queueLayout(layout.value);
      else if (shared.layout) {
        const backup = persistableLayout(copyLayout(layout.value));
        writeCanvasCache(storage, storageKey.value + ":recovery", backup, true); localRecovery.value = backup;
        await adoptServerLayout(shared.layout, false);
      }
    } catch (cause) { report(cause); }
  }
  function restoreLocalRecovery() {
    if (!localRecovery.value) return;
    layout.value = copyLayout(localRecovery.value); selectedPaneId.value = initialPane(layout.value.root)?.id;
    localRecovery.value = undefined;
  }
  /**
   * An agent changed the shared canvas on the server (closing tabs). Load it,
   * unless this window has a change of its own not yet saved: then it is the
   * same situation as another window saving first, and the same bar asks.
   */
  async function agentCanvas(revision: number) {
    if (!canvasReady.value || !backendCapabilities.sharedCanvas || revision <= canvasRevision.value) return;
    if (pendingLayout || canvasConflict.value) { canvasConflict.value = true; return; }
    try {
      const shared = await request<SharedCanvas>("/canvas/layout");
      if (!shared.layout || !validateLayout(shared.layout) || shared.revision <= canvasRevision.value || pendingLayout) return;
      canvasRevision.value = shared.revision;
      await adoptServerLayout(shared.layout, true);
    } catch (cause) { report(cause); }
  }
  /** A change not yet sent, which leaving the page now would lose. */
  const hasPendingLayout = () => !!pendingLayout;
  /** The page is going: a change still waiting is kept in this browser to offer back, and nothing more is sent. */
  function dispose() { if (pendingLayout) cacheLayout(pendingLayout, true); disposed = true; clearTimeout(layoutTimer); }

  /**
   * A change every open page makes for itself -- a tab taking its session's
   * new name -- is shown but not saved: each page saving the same change
   * against the same revision is a conflict for all but the first.
   */
  async function showWithoutSaving(document: LayoutDocument) {
    const held = suspendLayoutSave; suspendLayoutSave = true; layout.value = document;
    await nextTick(); suspendLayoutSave = held;
  }
  return { canvasReady, layoutStatus, canvasRevision, canvasConflict, localRecovery, cacheLayout, queueLayout, flushLayout, initializeCanvas, resolveCanvasConflict, restoreLocalRecovery, agentCanvas, hasPendingLayout, showWithoutSaving, dispose };
}
