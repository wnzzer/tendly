<script setup lang="ts">
import { computed, inject, nextTick, onMounted, onBeforeUnmount, ref, watch } from "vue";
import type { LayoutNode, LeafPaneKind, PaneNode, SplitDirection } from "@tendly/protocol/layout";
import type { ProviderKind } from "@tendly/protocol";
import { isPaneNode, normalizeRatio, snapRatio, SPLIT_GAP, type DockPosition } from "../layout/layout-engine";
import { dockPosition, keyedRatio, keyedTab, splitGrid, tabsAround } from "../features/layout-node";
import { paneDisplayTitle } from "../features/pane-context";
import { useI18n } from "../i18n";
import TabIcon from "../features/TabIcon.vue";
import { shortcutHint } from "../features/palette-items";
import { isRemove } from "../features/shortcuts";
import Icon from "../features/Icon.vue";
import ContextMenu from "../features/ContextMenu.vue";
import MenuFlyout from "../features/MenuFlyout.vue";
import { lastQuickProvider } from "../features/quick-session";
import SessionCommonActions from "../features/SessionCommonActions.vue";
import { SESSION_ACTIONS } from "../features/session-actions-context";
import { sessionStatuses } from "../features/attention";
import { DEFAULT_CLIENT, PROVIDER_KINDS, providerLabel } from "../features/clients";
const { t } = useI18n();

const props = defineProps<{ node: LayoutNode; selected?: string | null; maximized?: string | null; locatedPaneId?: string | null; workspaceLabels?: Record<string, string>; mixedWorkspaces?: boolean; workspaceBranches?: Record<string, string>; sessionBranches?: Record<string, string>; sessionProviders?: Record<string, ProviderKind>; ephemeralSessionIds?: string[]; ephemeralSupported?: boolean }>();
defineSlots<{ pane(props: { pane: PaneNode }): unknown }>();
const emit = defineEmits<{
  select: [pane: PaneNode, groupId?: string];
  split: [id: string, direction: SplitDirection];
  resize: [id: string, ratio: number, final: boolean];
  resizing: [active: boolean];
  close: [id: string];
  maximize: [id: string];
  "drop-pane": [targetId: string, pane: PaneNode, position: DockPosition];
  "add-pane": [targetId: string, kind: LeafPaneKind];
  "create-session": [targetId: string, provider: ProviderKind, ephemeral: boolean];
}>();

const splitElement = ref<HTMLElement | null>(null);
const paneElement = ref<HTMLElement | null>(null);
const dropPosition = ref<DockPosition | null>(null);
const addMenu = ref<HTMLDetailsElement | null>(null);
/**
 * Tab context menu.
 *
 * Closing tabs one cross at a time is the tedious part of a busy layout, so
 * the bulk actions live here. Each one resolves to a list of panes first and
 * then closes them, because the tab list changes as they go.
 */
const tabMenu = ref<{ pane: PaneNode; x: number; y: number } | null>(null);
function openTabMenu(event: MouseEvent, pane: PaneNode) {
  event.preventDefault();
  tabMenu.value = { pane, x: event.clientX, y: event.clientY };
}
function closeTabMenu() { tabMenu.value = null; }
async function openSessionActions(pane: PaneNode) {
  closeTabMenu();
  select(pane);
  await nextTick();
  const menu = Array.from(document.querySelectorAll<HTMLDetailsElement>('details[data-session-menu-pane]'))
    .find(element => element.dataset.sessionMenuPane === pane.id);
  if (menu) {
    menu.open = true;
    await nextTick();
    menu.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus();
  }
}

const sessionActions = inject(SESSION_ACTIONS, null);
const menuSession = computed(() => { const id = tabMenu.value?.pane.metadata?.session_id; return typeof id === "string" ? sessionActions?.session(id) : undefined; });
const menuTabs = computed(() => tabMenu.value ? tabsAround(tabs.value, tabMenu.value.pane.id) : { left: [], right: [], others: [] });
const menuOthers = computed(() => menuTabs.value.others);
const menuLeft = computed(() => menuTabs.value.left);
const menuRight = computed(() => menuTabs.value.right);
/** Right-click on a pane's empty space: the tab strip beside the tabs, or an
 * empty pane. Offers what the "+" menu and the header buttons do. */
const paneMenu = ref<{ x: number; y: number } | null>(null);
function openPaneMenu(event: MouseEvent) {
  // A tab's own right-click has already opened the tab menu.
  if (event.defaultPrevented) return;
  event.preventDefault();
  refreshLastKind();
  paneMenu.value = { x: event.clientX, y: event.clientY };
}
function paneAction(run: () => void) { paneMenu.value = null; run(); }
function closePanes(panes: PaneNode[]) {
  closeTabMenu();
  for (const pane of [...panes]) emit("close", pane.id);
}
const activePane = computed(() => {
  const node = props.node;
  return node.type === "pane" ? node : node.type === "stack" ? node.panes.find((pane) => pane.id === node.activePaneId) ?? node.panes[0] : undefined;
});
const tabs = computed(() => props.node.type === "pane" ? [props.node] : props.node.type === "stack" ? props.node.panes : []);
async function revealActiveTab() {
  await nextTick();
  const active = paneElement.value?.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]');
  const strip = active?.parentElement;
  if (!active || !strip) return;
  const bounds = active.getBoundingClientRect(), viewport = strip.getBoundingClientRect();
  if (bounds.right > viewport.right) strip.scrollLeft += bounds.right - viewport.right;
  if (bounds.left < viewport.left) strip.scrollLeft -= viewport.left - bounds.left;
}
watch(() => [activePane.value?.id, tabs.value.map(pane => pane.id).join(',')], revealActiveTab);
onMounted(() => { void revealActiveTab(); window.addEventListener('resize', revealActiveTab); });
const targetId = computed(() => props.node.type === "stack" ? props.node.collapsedFrom ?? props.node.id : props.node.id);
const splitStyle = computed(() => props.node.type === "split" ? splitGrid(props.node.direction, props.node.ratio) : {});
/**
 * Views that are meaningful while still empty, offered by "+". A session pane
 * is not one of them: an agent tab bound to no session is a dead placeholder,
 * and "+" consistently means "new session" — created through the entries above
 * these. An empty file editor or preview is not one either: opening a file
 * gives it a pane of its own, so an empty one only ever waited for a drop.
 */
const paneTypes: { kind: LeafPaneKind; title: string }[] = [
  { kind: "git_diff", title: "Git Changes" },
];
const newSessionKinds: { provider: ProviderKind; title: string; values?: Record<string, string> }[] = PROVIDER_KINDS.map(provider =>
  provider === "terminal" ? { provider, title: "New terminal" } : { provider, title: "New {provider} session", values: { provider: providerLabel(provider) } });
/**
 * New sessions are one row, the kind made last time, with every kind beside
 * it: one click for the usual, and a list that does not grow down the menu
 * with each client added. Read again whenever a menu opens.
 */
const lastProvider = ref<ProviderKind>(lastQuickProvider());
const lastKind = computed(() => newSessionKinds.find(entry => entry.provider === lastProvider.value) ?? newSessionKinds[0]);
function refreshLastKind() { lastProvider.value = lastQuickProvider(); }
/** Running or waiting on you, for the dot on a session's tab; nothing for the rest. */
function tabStatus(pane: PaneNode) {
  const id = pane.metadata?.session_id;
  const status = typeof id === "string" ? sessionStatuses[id] : undefined;
  return status === "waiting" || status === "running" || status === "starting" ? status : undefined;
}
const titleFor = (pane: PaneNode) => paneDisplayTitle(pane, t);
/** A branch only when the session has one of its own -- a worktree. One in
 * the workspace directory shares the branch the status bar already names. */
function branchFor(pane: PaneNode) {
  if (pane.kind !== "agent_chat" && pane.kind !== "terminal") return undefined;
  const session = pane.metadata?.session_id;
  return typeof session === "string" ? props.sessionBranches?.[session] : undefined;
}
function workspaceFor(pane: PaneNode) { const id = pane.metadata?.workspace_id; return typeof id === "string" ? props.workspaceLabels?.[id] : undefined; }
const ephemeralIds = computed(() => new Set(props.ephemeralSessionIds ?? []));
function isEphemeral(pane: PaneNode) { const id = pane.metadata?.session_id; return typeof id === "string" && ephemeralIds.value.has(id); }
function qualifiedTitle(pane: PaneNode) { const workspace = workspaceFor(pane); return titleFor(pane) + (workspace ? " · " + workspace : ""); }
function tabLabel(pane: PaneNode) { return isEphemeral(pane) ? t('{title} · temporary window', { title: qualifiedTitle(pane) }) : qualifiedTitle(pane); }
function select(pane: PaneNode) { emit("select", pane, props.node.type === "stack" ? props.node.collapsedFrom : undefined); }
function tabKey(event: KeyboardEvent, pane: PaneNode) {
  // Delete closes the tab, as it removes a row anywhere else; the session lives on.
  if (isRemove(event)) { event.preventDefault(); emit("close", pane.id); return; }
  if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
  event.preventDefault();
  const nextIndex = keyedTab(event.key, tabs.value.findIndex((item) => item.id === pane.id), tabs.value.length);
  if (nextIndex === undefined) return;
  const next = tabs.value[nextIndex];
  if (next) { select(next); paneElement.value?.querySelectorAll<HTMLElement>("[role=tab]")[nextIndex]?.focus(); }
}
function add(kind: LeafPaneKind) { if (addMenu.value) addMenu.value.open = false; emit("add-pane", targetId.value, kind); }
function createSession(provider: ProviderKind, ephemeral = false) { if (addMenu.value) addMenu.value.open = false; emit("create-session", targetId.value, provider, ephemeral); }
function forwardCreate(id: string, provider: ProviderKind, ephemeral: boolean) { emit("create-session", id, provider, ephemeral); }

// Vue emits forward positional arguments, not an $event array. Keep every
// recursive hop typed so nested operations address their original node IDs.
function forwardSelect(pane: PaneNode, groupId?: string) { emit("select", pane, groupId); }
function forwardSplit(id: string, direction: SplitDirection) { emit("split", id, direction); }
function forwardResize(id: string, ratio: number, final: boolean) { emit("resize", id, ratio, final); }
function forwardDrop(id: string, pane: PaneNode, position: DockPosition) { emit("drop-pane", id, pane, position); }
function forwardAdd(id: string, kind: LeafPaneKind) { emit("add-pane", id, kind); }

let cleanupResize: (() => void) | undefined;
function beginResize(event: PointerEvent) {
  if (props.node.type !== "split" || !splitElement.value || event.button !== 0) return;
  event.preventDefault();
  cleanupResize?.();
  const id = props.node.id;
  const horizontal = props.node.direction === "horizontal";
  const bounds = splitElement.value.getBoundingClientRect();
  const available = Math.max(1, (horizontal ? bounds.width : bounds.height) - SPLIT_GAP);
  const origin = horizontal ? bounds.left : bounds.top;
  let ratio = props.node.ratio;
  const previousCursor = document.body.style.cursor;
  const previousSelection = document.body.style.userSelect;
  document.body.style.cursor = horizontal ? "col-resize" : "row-resize";
  document.body.style.userSelect = "none";
  emit("resizing", true);
  const move = (next: PointerEvent) => {
    ratio = normalizeRatio(((horizontal ? next.clientX : next.clientY) - origin - SPLIT_GAP / 2) / available);
    emit("resize", id, ratio, false);
  };
  const cleanup = () => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", finish);
    window.removeEventListener("pointercancel", finish);
    document.body.style.cursor = previousCursor;
    document.body.style.userSelect = previousSelection;
    cleanupResize = undefined;
  };
  const finish = () => { cleanup(); emit("resize", id, snapRatio(ratio), true); emit("resizing", false); };
  cleanupResize = cleanup;
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", finish, { once: true });
  window.addEventListener("pointercancel", finish, { once: true });
}
function keyboardResize(event: KeyboardEvent) {
  if (props.node.type !== "split") return;
  const ratio = keyedRatio(event.key, props.node.direction, props.node.ratio, event.shiftKey);
  if (ratio === undefined) return;
  event.preventDefault();
  emit("resize", props.node.id, ratio, true);
}
function dragStart(event: DragEvent, pane: PaneNode) {
  if (!event.dataTransfer) return;
  event.dataTransfer.effectAllowed = "move";
  event.dataTransfer.setData("application/tendly-pane", JSON.stringify(pane));
}
function isPaneDrag(event: DragEvent) { return event.dataTransfer?.types.includes("application/tendly-pane"); }
function detectPosition(event: DragEvent): DockPosition {
  const bounds = paneElement.value?.getBoundingClientRect();
  return bounds ? dockPosition(event.clientX - bounds.left, event.clientY - bounds.top, bounds.width, bounds.height) : "center";
}
function dragOver(event: DragEvent) {
  if (!isPaneDrag(event)) return;
  event.preventDefault();
  event.stopPropagation();
  if (event.dataTransfer) {
    event.dataTransfer.dropEffect = ["copy", "copyLink"].includes(event.dataTransfer.effectAllowed) ? "copy" : "move";
  }
  dropPosition.value = detectPosition(event);
}
function dragLeave(event: DragEvent) {
  if (event.relatedTarget instanceof Node && paneElement.value?.contains(event.relatedTarget)) return;
  dropPosition.value = null;
}
function drop(event: DragEvent) {
  if (!isPaneDrag(event)) return;
  event.preventDefault();
  event.stopPropagation();
  const position = dropPosition.value ?? detectPosition(event);
  dropPosition.value = null;
  const raw = event.dataTransfer?.getData("application/tendly-pane");
  if (!raw || raw.length > 32768) return;
  try { const pane: unknown = JSON.parse(raw); if (isPaneNode(pane)) emit("drop-pane", targetId.value, pane, position); } catch { /* Foreign or malformed drag payload. */ }
}
onBeforeUnmount(() => { cleanupResize?.(); window.removeEventListener('resize', revealActiveTab); });
</script>

<template>
  <div v-if="node.type === 'split'" ref="splitElement" class="dock-split" :class="'is-' + node.direction" :style="splitStyle" :data-split-id="node.id">
    <div class="dock-child">
      <LayoutNode :node="node.first" :selected="selected" :maximized="maximized" :located-pane-id="locatedPaneId" :workspace-labels="workspaceLabels" :mixed-workspaces="mixedWorkspaces" :workspace-branches="workspaceBranches" :session-branches="sessionBranches" :session-providers="sessionProviders" :ephemeral-session-ids="ephemeralSessionIds" :ephemeral-supported="ephemeralSupported" @select="forwardSelect" @split="forwardSplit" @resize="forwardResize" @resizing="emit('resizing', $event)" @close="emit('close', $event)" @maximize="emit('maximize', $event)" @drop-pane="forwardDrop" @add-pane="forwardAdd" @create-session="forwardCreate">
        <template #pane="scope"><slot name="pane" :pane="scope.pane" /></template>
      </LayoutNode>
    </div>
    <div class="dock-separator" role="separator" tabindex="0" :aria-label="t(node.direction === 'horizontal' ? 'Resize columns' : 'Resize rows')" :aria-orientation="node.direction === 'horizontal' ? 'vertical' : 'horizontal'" :aria-valuenow="Math.round(node.ratio * 100)" :aria-valuemin="5" :aria-valuemax="95" :title="t('Drag to resize · double-click for 1:1 · arrow keys for precision')" @pointerdown="beginResize" @keydown="keyboardResize" @dblclick="emit('resize', node.id, 0.5, true)" />
    <div class="dock-child">
      <LayoutNode :node="node.second" :selected="selected" :maximized="maximized" :located-pane-id="locatedPaneId" :workspace-labels="workspaceLabels" :mixed-workspaces="mixedWorkspaces" :workspace-branches="workspaceBranches" :session-branches="sessionBranches" :session-providers="sessionProviders" :ephemeral-session-ids="ephemeralSessionIds" :ephemeral-supported="ephemeralSupported" @select="forwardSelect" @split="forwardSplit" @resize="forwardResize" @resizing="emit('resizing', $event)" @close="emit('close', $event)" @maximize="emit('maximize', $event)" @drop-pane="forwardDrop" @add-pane="forwardAdd" @create-session="forwardCreate">
        <template #pane="scope"><slot name="pane" :pane="scope.pane" /></template>
      </LayoutNode>
    </div>
  </div>
  <section v-else ref="paneElement" class="dock-pane" :class="{ 'is-selected': activePane?.id === selected, 'is-collapsed': node.type === 'stack' && node.collapsedFrom, 'is-located': !!locatedPaneId && activePane?.id === locatedPaneId }" :data-pane-id="activePane?.id" :data-node-id="targetId" @pointerdown="activePane && select(activePane)" @focusin="activePane && activePane.id !== selected && select(activePane)" @dragover="dragOver" @dragleave="dragLeave" @drop="drop">
    <header class="dock-header" @contextmenu="openPaneMenu">
      <div class="dock-tabs" role="tablist" :aria-label="t('Pane tabs')">
        <div v-for="pane in tabs" :key="pane.id" :id="`dock-tab-${pane.id}`" :data-pane-tab-id="pane.id" role="tab" class="dock-tab" :class="{ 'is-active': pane.id === activePane?.id, 'is-ephemeral': isEphemeral(pane) }" :aria-selected="pane.id === activePane?.id" :aria-controls="`dock-panel-${pane.id}`" :aria-label="tabLabel(pane)" :tabindex="pane.id === activePane?.id ? 0 : -1" :title="t(isEphemeral(pane) ? '{title} · temporary window · discarded when closed' : '{title} · drag to arrange', { title: qualifiedTitle(pane) })" draggable="true" @contextmenu="openTabMenu($event, pane)" @dragstart="dragStart($event, pane)" @click.stop="select(pane)" @keydown="tabKey($event, pane)" @keydown.enter.prevent="select(pane)" @keydown.space.prevent="select(pane)">
          <TabIcon :kind="pane.kind" :metadata="pane.metadata" :session-providers="sessionProviders" />
          <span v-if="isEphemeral(pane)" class="dock-tab-ephemeral" role="img" :aria-label="t('Temporary window')" :title="t('Temporary window')" />
          <span class="dock-tab-title">{{ titleFor(pane) }}</span><span v-if="tabStatus(pane)" :class="['dock-tab-status', tabStatus(pane)]" role="img" :aria-label="t(tabStatus(pane)!)" :title="t(tabStatus(pane) === 'waiting' ? 'Needs you' : tabStatus(pane)!)" />
          <span v-if="mixedWorkspaces && workspaceFor(pane) && pane.id === activePane?.id" class="dock-tab-workspace">{{ workspaceFor(pane) }}</span><span v-if="branchFor(pane) && pane.id === activePane?.id" class="dock-tab-branch" :title="branchFor(pane)"><svg viewBox="0 0 16 16" width="9" height="9" aria-hidden="true"><path d="M5 3v7M5 10a2 2 0 1 0 0 4 2 2 0 0 0 0-4Zm0-7a2 2 0 1 0 0-.01M11 5a2 2 0 1 0 0-.01M11 7c0 2-2 3-6 3" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg>{{ branchFor(pane) }}</span>
          <!-- The session menu is teleported here by the active session pane.
               Inactive tabs stay compact and do not reserve an empty action slot. -->
          <!-- Keep every session target mounted. A Teleport menu can outlive a
               tab activation during layout updates; removing its target in
               that frame causes Vue to patch a null vnode. -->
          <span v-if="pane.metadata?.session_id" class="dock-tab-session-actions" :id="`session-actions-${pane.id}`" />
          <button class="dock-tab-close" type="button" :aria-label="t('Close {title} pane', { title: tabLabel(pane) })" :title="t(isEphemeral(pane) ? 'Close and discard this temporary session' : 'Close pane (session keeps running)')" @pointerdown.stop @click.stop="emit('close', pane.id)"><Icon name="close" :size="12" /></button>
        </div>
        <span v-if="!tabs.length" class="dock-empty-label">{{ t('Empty pane') }}</span>
        <ContextMenu v-if="tabMenu" :x="tabMenu.x" :y="tabMenu.y" :label="t('Tab actions')" :width="200" @close="closeTabMenu">
          <template v-if="menuSession">
            <div class="context-menu-label tab-menu-title">{{ menuSession.title }}</div>
            <SessionCommonActions :session="menuSession" :keep-busy="sessionActions!.keepBusy(menuSession.id)" :close-menu="closeTabMenu" @rename="sessionActions!.rename(menuSession!.id)" @environment="sessionActions!.environment(menuSession!.id)" @keep="sessionActions!.keep(menuSession!.id)" @info="sessionActions!.info(menuSession!.id)" />
            <button role="menuitem" @click="openSessionActions(tabMenu.pane)"><Icon name="more" :size="14" />{{ t('More session actions') }}</button>
            <hr/>
          </template>
          <button v-else-if="tabMenu.pane.metadata?.session_id" role="menuitem" @click="openSessionActions(tabMenu.pane)"><Icon name="more" :size="14" />{{ t('Session actions') }}</button>
          <MenuFlyout :label="t('Close tab')" icon="close" :width="180" @activate="closePanes([tabMenu!.pane])">
            <button role="menuitem" :disabled="!menuOthers.length" @click="closePanes(menuOthers)"><Icon name="closeOthers" :size="14" />{{ t('Close other tabs') }}</button>
            <button role="menuitem" :disabled="!menuLeft.length" @click="closePanes(menuLeft)"><Icon name="closeLeft" :size="14" />{{ t('Close tabs to the left') }}</button>
            <button role="menuitem" :disabled="!menuRight.length" @click="closePanes(menuRight)"><Icon name="closeRight" :size="14" />{{ t('Close tabs to the right') }}</button>
            <button role="menuitem" @click="closePanes(tabs)"><Icon name="closeAll" :size="14" />{{ t('Close all tabs') }}</button>
          </MenuFlyout>
        </ContextMenu>
      </div>
      <div class="dock-actions" @pointerdown.stop>
        <button v-if="node.type === 'stack' && node.collapsedFrom" type="button" class="dock-action dock-adaptive" :title="t('Reset folded split to 1:1; expands when space allows')" :aria-label="t('Restore folded split ratio')" @click="emit('resize', targetId, 0.5, true)"><Icon name="restore" :size="13" /></button>
        <details ref="addMenu" class="dock-add-menu" @toggle="refreshLastKind">
          <summary class="dock-action" :title="t('New session here')" :aria-label="t('New session here')"><Icon name="plus" :size="15" /></summary>
          <div class="dock-menu-items">
            <MenuFlyout :label="t(lastKind.title, lastKind.values)" @activate="createSession(lastKind.provider)">
              <template #icon><TabIcon :kind="lastKind.provider === 'terminal' ? 'terminal' : 'agent_chat'" :metadata="{ provider: lastKind.provider }" :size="14" /></template>
              <button v-for="entry in newSessionKinds" :key="entry.provider" type="button" role="menuitem" @click="createSession(entry.provider)"><TabIcon :kind="entry.provider === 'terminal' ? 'terminal' : 'agent_chat'" :metadata="{ provider: entry.provider }" :size="14" />{{ t(entry.title, entry.values) }}</button>
              <template v-if="ephemeralSupported"><hr /><button type="button" role="menuitem" @click="createSession(DEFAULT_CLIENT, true)"><Icon name="clock" :size="14" />{{ t('New temporary window') }}</button></template>
            </MenuFlyout>
            <hr />
            <button v-for="type in paneTypes" :key="type.kind" type="button" @click="add(type.kind)"><TabIcon :kind="type.kind" :size="14" />{{ t(type.title) }}</button>
          </div>
        </details>
        <button class="dock-action" type="button" :aria-label="t('Split side by side')" :title="t('Split side by side') + ' · ' + shortcutHint('split-horizontal')" @click="emit('split', targetId, 'horizontal')"><Icon name="splitHorizontal" :size="15" /></button>
        <button class="dock-action" type="button" :aria-label="t('Split top and bottom')" :title="t('Split top and bottom') + ' · ' + shortcutHint('split-vertical')" @click="emit('split', targetId, 'vertical')"><Icon name="splitVertical" :size="15" /></button>
        <button v-if="activePane" class="dock-action" type="button" :aria-label="t(maximized === activePane.id ? 'Restore pane' : 'Maximize pane')" :title="t(maximized === activePane.id ? 'Restore layout' : 'Maximize pane') + ' · ' + shortcutHint('maximize-pane')" @click="emit('maximize', activePane.id)"><Icon :name="maximized === activePane.id ? 'minimize' : 'maximize'" :size="14" /></button>
        <button v-else class="dock-action" type="button" :aria-label="t('Close empty pane')" :title="t('Close empty pane')" @click="emit('close', targetId)"><Icon name="close" :size="14" /></button>
      </div>
    </header>
    <div v-if="activePane" :id="`dock-panel-${activePane.id}`" :aria-labelledby="`dock-tab-${activePane.id}`" class="dock-content" role="tabpanel"><slot name="pane" :pane="activePane" /></div>
    <div v-else class="dock-empty" @contextmenu="openPaneMenu">
      <span class="dock-empty-symbol"><Icon name="layout" :size="28" /></span><strong>{{ t('A little room for your next idea') }}</strong><p>{{ t('Drop a session here, or add a pane.') }}</p>
      <div class="dock-empty-choices"><button v-for="type in paneTypes" :key="type.kind" type="button" @click="add(type.kind)"><TabIcon :kind="type.kind" :size="14" />{{ t(type.title) }}</button></div>
    </div>
    <ContextMenu v-if="paneMenu" :x="paneMenu.x" :y="paneMenu.y" :label="t('Pane actions')" @close="paneMenu = null">
      <div class="context-menu-label">{{ t('New session here') }}</div>
      <MenuFlyout :label="t(lastKind.title, lastKind.values)" @activate="paneAction(() => createSession(lastKind.provider))">
        <template #icon><TabIcon :kind="lastKind.provider === 'terminal' ? 'terminal' : 'agent_chat'" :metadata="{ provider: lastKind.provider }" :size="14" /></template>
        <button v-for="entry in newSessionKinds" :key="entry.provider" role="menuitem" @click="paneAction(() => createSession(entry.provider))"><TabIcon :kind="entry.provider === 'terminal' ? 'terminal' : 'agent_chat'" :metadata="{ provider: entry.provider }" :size="14" />{{ t(entry.title, entry.values) }}</button>
        <template v-if="ephemeralSupported"><hr /><button role="menuitem" @click="paneAction(() => createSession(DEFAULT_CLIENT, true))"><Icon name="clock" :size="14" />{{ t('New temporary window') }}</button></template>
      </MenuFlyout>
      <hr />
      <button v-for="type in paneTypes" :key="type.kind" role="menuitem" @click="paneAction(() => add(type.kind))"><TabIcon :kind="type.kind" :size="14" />{{ t(type.title) }}</button>
      <hr />
      <button role="menuitem" @click="paneAction(() => emit('split', targetId, 'horizontal'))"><Icon name="splitHorizontal" :size="14" />{{ t('Split side by side') }} <kbd>{{ shortcutHint('split-horizontal') }}</kbd></button>
      <button role="menuitem" @click="paneAction(() => emit('split', targetId, 'vertical'))"><Icon name="splitVertical" :size="14" />{{ t('Split top and bottom') }} <kbd>{{ shortcutHint('split-vertical') }}</kbd></button>
      <button v-if="activePane" role="menuitem" @click="paneAction(() => emit('maximize', activePane!.id))"><Icon :name="maximized === activePane.id ? 'minimize' : 'maximize'" :size="14" />{{ t(maximized === activePane.id ? 'Restore layout' : 'Maximize pane') }} <kbd>{{ shortcutHint('maximize-pane') }}</kbd></button>
      <button v-if="tabs.length" role="menuitem" @click="paneAction(() => closePanes(tabs))"><Icon name="closeAll" :size="14" />{{ t('Close all tabs') }}</button>
      <button v-else role="menuitem" @click="paneAction(() => emit('close', targetId))"><Icon name="close" :size="14" />{{ t('Close empty pane') }}</button>
    </ContextMenu>
    <div v-if="dropPosition" class="dock-drop-overlay" :class="'drop-' + dropPosition"><span>{{ t(dropPosition === 'center' ? 'Add as tab' : 'Split ' + dropPosition) }}</span></div>
  </section>
</template>

<style scoped>
.dock-tab-status { flex:none; width:7px; height:7px; border-radius:50%; background:var(--ok); }
.dock-tab-status.starting { background:var(--muted); }
.dock-tab-status.waiting { background:var(--warn); box-shadow:0 0 0 3px var(--warn-soft); animation:attention-pulse 1.6s ease-in-out infinite; }
@keyframes attention-pulse { 50% { box-shadow:0 0 0 5px var(--warn-soft); } }
@media (prefers-reduced-motion: reduce) { .dock-tab-status.waiting { animation:none; } }

.dock-split,.dock-child { width:100%;height:100%;min-width:0;min-height:0; }
.dock-split { display:grid; }
.dock-child { overflow:hidden; }
.dock-separator { position:relative;outline:none;touch-action:none;z-index:2; }
.is-horizontal>.dock-separator { cursor:col-resize; }
.is-vertical>.dock-separator { cursor:row-resize; }
.dock-separator::after { content:"";position:absolute;border-radius:var(--radius-xs);inset:1px;background:transparent;transition:background .12s; }
.dock-separator:hover::after,.dock-separator:focus-visible::after { background:var(--focus); }
.dock-pane { position:relative;display:flex;flex-direction:column;min-width:0;min-height:0;width:100%;height:100%;overflow:hidden;border:1px solid var(--border);border-radius:var(--radius-lg);background:var(--surface);box-sizing:border-box;box-shadow:var(--shadow-card),inset 0 1px 0 var(--card-highlight);container:dock-pane / inline-size;animation:rise var(--duration-slow) var(--ease-out) backwards;transition:border-color var(--duration-normal),box-shadow var(--duration-normal); }
.dock-pane.is-selected { border-color:color-mix(in srgb, var(--accent) 45%, var(--border));box-shadow:var(--shadow-lg),0 0 0 3px color-mix(in srgb, var(--accent) 10%, transparent),inset 0 1px 0 var(--card-highlight); }
@media(prefers-reduced-motion:reduce) { .dock-pane { animation:none; } }
.dock-pane.is-located { border-color:var(--ok);box-shadow:inset 0 0 0 1px #46b29d,0 0 0 1px #b6e3dd60;animation:dock-locate 1.4s ease-out; }.dock-pane.is-located .dock-tab.is-active { background:var(--accent-soft); }
@keyframes dock-locate { from { box-shadow:inset 0 0 0 3px #70cabb,0 0 0 1px #b6e3dd60; } to { box-shadow:inset 0 0 0 1px #46b29d,0 0 0 1px #b6e3dd60; } }
@media(prefers-reduced-motion:reduce) { .dock-pane.is-located { animation:none; } }
.dock-header { display:flex;align-items:center;gap:3px;min-width:0;height:38px;min-height:38px;padding:0 5px 0 0;background:linear-gradient(180deg,var(--sunken),color-mix(in srgb, var(--sunken) 55%, var(--surface)));border-bottom:1px solid var(--border);box-shadow:inset 0 1px 0 var(--card-highlight); }
.dock-tabs { display:flex;align-items:stretch;flex:1;min-width:0;height:100%;overflow-x:auto; }
/* Tabs are reached by clicking, not by dragging a bar, so this one only has to
   show that more exist. A full-width scrollbar would take a quarter of the strip. */
.dock-tabs::-webkit-scrollbar { height: 5px; }
.dock-tabs::-webkit-scrollbar-thumb { border-width: 1px; }
/* The transparent top border balances the indicator below, so the tab's content
   sits on the strip's true middle rather than 1px above it. */
.dock-tab { display:flex;align-items:center;gap:6px;max-width:220px;min-width:96px;flex-shrink:0;padding:0 6px 0 11px;color:var(--muted);font-size:var(--text-sm);cursor:grab;border-top:2px solid transparent;border-bottom:2px solid transparent;outline:none;user-select:none; }
.dock-tab.is-active { max-width:300px;color:var(--ink);background:var(--surface);border-bottom-color:var(--accent); }
.dock-tab:focus-visible { box-shadow:inset 0 0 0 2px #53b9b0; }
.dock-tab-title { min-width:48px;flex:0 1 auto;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-weight:600; }
.dock-tab-branch { display:inline-flex;align-items:center;gap:3px;max-width:96px;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;flex:0 1 auto;font-size:var(--text-xs);border-radius:var(--radius-xs);padding:1px 4px;background:var(--fill);color:var(--ink-soft); }
.dock-tab-workspace { max-width:80px;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;flex:0 1 auto;font-size:var(--text-xs);border-radius:var(--radius-xs);padding:1px 4px;background:var(--fill);color:var(--muted); }
.dock-tab.is-ephemeral { border-bottom-style:dashed; }.dock-tab.is-ephemeral.is-active { border-bottom-color:var(--violet); }
.dock-tab-ephemeral { flex-shrink:0;width:5px;height:5px;margin-left:-3px;border-radius:50%;background:var(--violet);box-shadow:0 0 0 2px #efeaf8; }
.tab-menu-title{overflow:hidden;max-width:240px;text-overflow:ellipsis;white-space:nowrap}
.dock-tab-session-actions{position:relative;display:inline-flex;align-items:center;justify-content:center;width:28px;height:28px;flex:none}
.dock-tab:not(.is-active)>.dock-tab-session-actions { display:none; }
.dock-tab-close { display:grid;place-items:center;border:0;padding:0;background:transparent;width:17px;height:19px;color:var(--faint);cursor:pointer;border-radius:var(--radius-xs);font-size:var(--text-base);flex-shrink:0; }
.dock-tab-close:hover { color:var(--danger);background:var(--danger-soft); }
.dock-tab:not(.is-active):not(:hover):not(:focus-within) .dock-tab-close { opacity:0; }
@media(pointer:coarse) { .dock-tab:not(.is-active) .dock-tab-close { opacity:1; } }
.dock-actions { display:flex;align-items:center;gap:1px;flex-shrink:0; }
.dock-action { display:grid;place-items:center;width:26px;height:28px;padding:0;border:0;border-radius:var(--radius-sm);background:transparent;color:var(--ink-soft);cursor:pointer;font-size:var(--text-lg);list-style:none; }
.dock-action:hover,.dock-action:focus-visible { background:var(--fill-hover);color:var(--ink);outline:none; }
.dock-action::-webkit-details-marker { display:none; }
.dock-add-menu { position:static; }.dock-menu-items { position:absolute;right:6px;top:42px;z-index:10;min-width:154px;background:var(--surface);border:1px solid var(--line);border-radius:var(--radius-md);padding:4px;box-shadow:var(--shadow-lg),inset 0 1px 0 var(--card-highlight); }
.dock-menu-items button, .dock-menu-items :deep(.menu-flyout-trigger) { display:flex;align-items:center;gap:9px;width:100%;border:0;border-radius:var(--radius-sm);padding:8px;background:transparent;color:var(--ink-soft);text-align:left;font-size:var(--text-sm);cursor:pointer; }.dock-menu-items button:hover, .dock-menu-items :deep(.menu-flyout-trigger):hover, .dock-menu-items :deep(.menu-flyout-trigger[aria-expanded="true"]) { background:var(--fill);color:var(--ink); }
.dock-adaptive { color:var(--faint);font-size:var(--text-sm);padding:0 3px; }
.dock-content { display:flex;flex-direction:column;flex:1;min-width:0;min-height:0;overflow:hidden; }
.dock-content :deep(> *) { min-width:0;min-height:0; }
.dock-empty-label { padding:12px;color:var(--faint);font-size:var(--text-xs); }
.dock-empty { flex:1;min-height:0;overflow:auto;display:flex;align-items:center;justify-content:center;flex-direction:column;padding:20px;color:var(--muted);text-align:center; }
.dock-empty-symbol { color:var(--faint);font-size:30px;margin-bottom:10px; }.dock-empty strong { font-size:var(--text-md);font-weight:500;color:var(--ink-soft); }.dock-empty p { font-size:var(--text-xs);margin:6px 0 17px; }
.dock-empty-choices { display:flex;flex-wrap:wrap;justify-content:center;gap:6px;max-width:350px; }.dock-empty-choices button { display:flex;align-items:center;gap:6px;border:1px solid var(--line);border-radius:var(--radius-sm);padding:6px 9px;background:var(--surface);font-size:var(--text-xs);color:var(--ink-soft);cursor:pointer; }.dock-empty-choices button:hover { background:var(--fill);border-color:var(--line-strong);color:var(--ink); }
.dock-drop-overlay { position:absolute;inset:5px;z-index:20;display:flex;align-items:center;justify-content:center;pointer-events:none;border:2px solid var(--focus);border-radius:var(--radius-md);background:#b9eae280;color:var(--accent-hover);backdrop-filter:blur(1px);font-size:var(--text-sm);font-weight:600; }.dock-drop-overlay span { border-radius:var(--radius-xl);padding:7px 12px;background:color-mix(in srgb, var(--surface) 94%, transparent);box-shadow:var(--shadow-md); }.drop-left { right:50%; }.drop-right { left:50%; }.drop-top { bottom:50%; }.drop-bottom { top:50%; }
@container dock-pane (max-width:480px) { .dock-tab-workspace,.dock-tab-branch { display:none; }.dock-tab,.dock-tab.is-active { max-width:180px; } }
@media(max-width:520px) { .dock-tab,.dock-tab.is-active { min-width:72px;max-width:180px;padding-left:7px;gap:5px; }.dock-tab-workspace,.dock-tab-branch { display:none; }.dock-action { width:23px; }.dock-adaptive { display:none; } }
</style>
