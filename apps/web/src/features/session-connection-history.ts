import { reactive } from "vue";
import type { NativeHistoryItem } from "@tendly/protocol";

/**
 * Consent is tied to the displayed history items, source and workspace that
 * were chosen: changing what is chosen asks again.
 *
 * One item or many: a click chooses only that item, a toggle (the checkbox,
 * Ctrl/Cmd-click) adds or removes one, and a range (Shift-click) chooses
 * everything between the last item clicked and this one, in the order shown.
 */
export function createHistorySelection() {
  const state = reactive<{
    workspaceId?: string;
    sourceId?: string;
    items: NativeHistoryItem[];
    /** The item a range extends from. */
    anchor?: string;
    confirmed: boolean;
  }>({ items: [], confirmed: false });
  function clear() { state.items = []; state.anchor = undefined; state.workspaceId = undefined; state.sourceId = undefined; state.confirmed = false; }
  function scope(workspaceId: string, sourceId: string) {
    if (state.workspaceId !== workspaceId || state.sourceId !== sourceId) { state.items = []; state.anchor = undefined; }
    state.workspaceId = workspaceId; state.sourceId = sourceId; state.confirmed = false;
  }
  function select(workspaceId: string, sourceId: string, item: NativeHistoryItem) {
    scope(workspaceId, sourceId);
    state.items = [item]; state.anchor = item.id;
  }
  function toggle(workspaceId: string, sourceId: string, item: NativeHistoryItem) {
    scope(workspaceId, sourceId);
    state.items = isSelected(item.id) ? state.items.filter(chosen => chosen.id !== item.id) : [...state.items, item];
    state.anchor = item.id;
  }
  /** From the anchor to `item` within `shown`, added to what is chosen. */
  function extend(workspaceId: string, sourceId: string, item: NativeHistoryItem, shown: readonly NativeHistoryItem[]) {
    const from = shown.findIndex(entry => entry.id === state.anchor), to = shown.findIndex(entry => entry.id === item.id);
    if (from < 0 || to < 0 || state.workspaceId !== workspaceId || state.sourceId !== sourceId) return select(workspaceId, sourceId, item);
    const range = shown.slice(Math.min(from, to), Math.max(from, to) + 1);
    scope(workspaceId, sourceId);
    state.items = [...state.items, ...range.filter(entry => !isSelected(entry.id))];
  }
  /** Everything shown, or nothing if all of it already is. */
  function toggleAll(workspaceId: string, sourceId: string, shown: readonly NativeHistoryItem[]) {
    const all = shown.length > 0 && shown.every(entry => isSelected(entry.id)) && state.workspaceId === workspaceId && state.sourceId === sourceId;
    scope(workspaceId, sourceId);
    state.items = all ? state.items.filter(chosen => !shown.some(entry => entry.id === chosen.id)) : [...state.items, ...shown.filter(entry => !isSelected(entry.id))];
    state.anchor = undefined;
  }
  function isSelected(id: string) { return state.items.some(item => item.id === id); }
  /** One request body per chosen item, or none without consent for this exact scope. */
  function payloads(workspaceId: string, sourceId: string) {
    if (!state.confirmed || !state.items.length || state.workspaceId !== workspaceId || state.sourceId !== sourceId) return undefined;
    return state.items.map(item => ({ source_id: sourceId, native_id: item.id, confirmed_original_config: true as const }));
  }
  function payload(workspaceId: string, sourceId: string) { return payloads(workspaceId, sourceId)?.[0]; }
  /** Drop items that were loaded, keeping the rest chosen (and their consent) for a retry. */
  function remove(ids: readonly string[]) { state.items = state.items.filter(item => !ids.includes(item.id)); }
  return { state, clear, select, toggle, extend, toggleAll, isSelected, payloads, payload, remove };
}
