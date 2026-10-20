<script setup lang="ts">
import WorkspaceBranchMenu from "./WorkspaceBranchMenu.vue";
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { ComponentPublicInstance } from "vue";
import type { ProviderKind, Session, Workspace } from "@tendly/protocol";
import { providerLabel } from "./api";
import { emptyWorkspaceGroupPreferences, groupWorkspaces, loadWorkspaceGroupPreferences, saveWorkspaceGroupPreferences, workspaceGroupStorageKey } from "./workspace-groups";
import type { WorkspaceGroup } from "./workspace-groups";
import { recentDays } from "./recent-sessions";
import { isEphemeralSession, isSessionArchived } from "./session-list";
import { useI18n } from "../i18n";
import Icon from "./Icon.vue";
import ProviderIcon from "./ProviderIcon.vue";
import { lastQuickProvider } from "./quick-session";
import { AGENT_CLIENTS } from "./clients";
import SidebarSessionRow from "./SidebarSessionRow.vue";
import MenuFlyout from "./MenuFlyout.vue";

const props = withDefaults(defineProps<{
  workspaces: Workspace[];
  sessions: Session[];
  selectedWorkspaceId?: string;
  selectedSessionId?: string;
  historySupported?: boolean;
  archiveSupported?: boolean;
  ephemeralSupported?: boolean;
  archiveBusyIds?: string[];
  keepBusyIds?: string[];
  deleteBusyIds?: string[];
  sessionsLoading?: boolean;
  storageKey?: string;
  /** Branch of each workspace directory, where known. */
  workspaceBranches?: Record<string, string>;
  /** The page shown over the canvas, if any, for the navigation's current item. */
  currentPage?: string;
}>(), { historySupported: false, archiveSupported: false, ephemeralSupported: false, archiveBusyIds: () => [], keepBusyIds: () => [], deleteBusyIds: () => [], sessionsLoading: false });
const emit = defineEmits<{
  branchSwitched: [id: string];
  selectWorkspace: [id: string];
  openSession: [session: Session];
  sessionEnvironment: [id: string];
  renameSession: [session: Session, title: string];
  openFiles: [id: string];
  openChanges: [id: string];
  newSession: [id: string, provider?: ProviderKind];
  quickSession: [id: string, provider: ProviderKind, ephemeral: boolean];
  loadHistory: [id: string];
  addWorkspace: [];
  allSessions: [];
  system: [];
  usage: [];
  /** A session the workspace groups do not list (archived, temporary): find it in the library page. */
  showInSessions: [id: string, archived: boolean];
  canvas: [];
  archiveSession: [session: Session, archived: boolean];
  archiveSessions: [sessions: Session[], archived: boolean];
  deleteSessions: [sessions: Session[]];
  keepSession: [session: Session];
  refreshSessions: [];
  revealSession: [id: string];
}>();
const { t } = useI18n();
const query = ref("");
const preferences = ref(emptyWorkspaceGroupPreferences());
const searchExpanded = ref<Record<string, boolean>>({});
const openMenuId = ref<string>();
const menuButtons = new Map<string, HTMLElement>();
const sidebarElement = ref<HTMLElement>();
const archiveBusySet = computed(() => new Set(props.archiveBusyIds));
const keepBusySet = computed(() => new Set(props.keepBusyIds));
const deleteBusySet = computed(() => new Set(props.deleteBusyIds));
const sessionCount = computed(() => new Set(props.sessions.map(session => session.id)).size);
const persistenceKey = computed(() => workspaceGroupStorageKey(props.storageKey ?? (typeof window === "undefined" ? "default" : window.location.origin)));
const groups = computed(() => groupWorkspaces(props.workspaces, props.sessions, { query: query.value, selectedWorkspaceId: props.selectedWorkspaceId, preferences: preferences.value, searchExpanded: searchExpanded.value, selectedSessionId: props.selectedSessionId, recentDays: recentDays.value }));
const canvasWorkspaceId = computed(() => props.workspaces.some(workspace => workspace.id === props.selectedWorkspaceId) ? props.selectedWorkspaceId : props.workspaces[0]?.id);

function storage(): Storage | undefined {
  try { return typeof localStorage === "undefined" ? undefined : localStorage; } catch { return undefined; }
}
function persist() { saveWorkspaceGroupPreferences(persistenceKey.value, preferences.value, storage()); }
watch(persistenceKey, key => { preferences.value = loadWorkspaceGroupPreferences(key, storage()); query.value = ""; searchExpanded.value = {}; openMenuId.value = undefined; }, { immediate: true });
watch(query, () => { searchExpanded.value = {}; openMenuId.value = undefined; });
watch(() => props.workspaces, workspaces => { if (openMenuId.value && !workspaces.some(workspace => workspace.id === openMenuId.value)) openMenuId.value = undefined; });

function setExpanded(id: string, value: boolean) {
  if (query.value.trim()) searchExpanded.value = { ...searchExpanded.value, [id]: value };
  else { preferences.value.expanded = { ...preferences.value.expanded, [id]: value }; persist(); }
}
function toggleOlder(id: string) { preferences.value.olderExpanded = { ...preferences.value.olderExpanded, [id]: !preferences.value.olderExpanded[id] }; persist(); }
function selectWorkspace(group: WorkspaceGroup) { setExpanded(group.workspace.id, true); openMenuId.value = undefined; emit("selectWorkspace", group.workspace.id); }
function togglePin(group: WorkspaceGroup) {
  const id = group.workspace.id;
  preferences.value.pinned = group.pinned ? preferences.value.pinned.filter(value => value !== id) : [id, ...preferences.value.pinned.filter(value => value !== id)];
  persist(); void closeMenu(true);
}
function recordMenuButton(id: string, element: Element | ComponentPublicInstance | null) {
  if (element instanceof HTMLElement) menuButtons.set(id, element); else menuButtons.delete(id);
}
async function closeMenu(restoreFocus = false) {
  const id = openMenuId.value; openMenuId.value = undefined;
  if (restoreFocus && id) { await nextTick(); menuButtons.get(id)?.focus(); }
}
function terminal(id: string) { openMenuId.value = undefined; emit("newSession", id, "terminal"); }
/** Quick create never opens the dialog here; the owner decides if a choice is unavoidable. */
function quick(id: string, provider: ProviderKind, ephemeral = false) { openMenuId.value = undefined; emit("quickSession", id, provider, ephemeral); if (!ephemeral) quickProvider.value = provider; }
/** The kind "+" makes: the one made last, read again whenever a menu opens (another view may have made one since). */
const quickProvider = ref<ProviderKind>(lastQuickProvider());
watch(openMenuId, id => { if (id) quickProvider.value = lastQuickProvider(); });
const quickLabel = computed(() => quickProvider.value === "terminal" ? t("Terminal") : providerLabel(quickProvider.value));
function history(id: string) { if (props.historySupported) { openMenuId.value = undefined; emit("loadHistory", id); } }
function escapeMenu(event: KeyboardEvent) { if (event.key === "Escape" && openMenuId.value) { event.preventDefault(); event.stopPropagation(); void closeMenu(true); } }
/** A right-click opens this menu without moving focus, so focus alone cannot
 * dismiss it; a pointer landing anywhere outside does. */
function closeMenuOutside(event: PointerEvent) {
  const id = openMenuId.value; if (!id) return;
  const target = event.target as Node | null;
  if (!target) return;
  const panel = document.getElementById(`workspace-menu-${id}`);
  if (panel?.contains(target) || menuButtons.get(id)?.contains(target)) return;
  openMenuId.value = undefined;
}
onMounted(() => document.addEventListener("pointerdown", closeMenuOutside));
onBeforeUnmount(() => document.removeEventListener("pointerdown", closeMenuOutside));
function navigateWorkspace(event: KeyboardEvent, group: WorkspaceGroup) {
  if (event.altKey || event.ctrlKey || event.metaKey) return;
  if (event.key === "ArrowLeft" || event.key === "ArrowRight") { event.preventDefault(); event.stopPropagation(); setExpanded(group.workspace.id, event.key === "ArrowRight"); }
}
function archiveSession(session: Session, archived: boolean) {
  if (props.archiveSupported && !archiveBusySet.value.has(session.id)) emit("archiveSession", session, archived);
}
function renameSession(session: Session, title: string) { emit("renameSession", session, title); }
function deleteSession(session: Session) { if (!deleteBusySet.value.has(session.id)) emit("deleteSessions", [session]); }
function keepSession(session: Session) { if (!keepBusySet.value.has(session.id)) emit("keepSession", session); }

/** Reveal the actual bound session without changing the canvas selection or launching a client. */
async function revealSession(id: string): Promise<boolean> {
  const session = props.sessions.find(item => item.id === id);
  if (!session) return false;
  query.value = ""; searchExpanded.value = {}; openMenuId.value = undefined;
  preferences.value.expanded = { ...preferences.value.expanded, [session.workspace_id]: true }; persist();
  preferences.value.olderExpanded = { ...preferences.value.olderExpanded, [session.workspace_id]: true }; persist();
  // Temporary windows are deliberately absent from the workspace groups, so the
  // library is where revealing one has to land.
  const needsLibrary = isSessionArchived(session) || isEphemeralSession(session) || !props.workspaces.some(workspace => workspace.id === session.workspace_id);
  if (needsLibrary) { emit("showInSessions", id, isSessionArchived(session)); return true; }
  await nextTick();
  const container = sidebarElement.value?.querySelector(".workspace-groups");
  const row = Array.from(container?.querySelectorAll<HTMLElement>("[data-sidebar-session-id]") ?? []).find(element => element.dataset.sidebarSessionId === id);
  row?.scrollIntoView?.({ block: "nearest", inline: "nearest", behavior: "auto" });
  row?.querySelector<HTMLButtonElement>(".workspace-session")?.focus({ preventScroll: true });
  return true;
}
defineExpose({ revealSession });
</script>

<template>
  <div ref="sidebarElement" class="workspace-sidebar" @keydown="escapeMenu">
    <nav class="workspace-global-nav" :aria-label="t('Workspace navigation')">
      <button :class="['nav-item', { current: currentPage === 'canvas' }]" :aria-current="currentPage === 'canvas' ? 'page' : undefined" @click="emit('canvas')"><Icon name="grid" :size="16" /><span>{{ t('Workspace canvas') }}</span></button>
      <button :class="['nav-item', { current: currentPage === 'sessions' }]" :aria-current="currentPage === 'sessions' ? 'page' : undefined" @click="emit('allSessions')"><Icon name="clock" :size="16" /><span>{{ t('All sessions') }}</span><small class="count-badge">{{ sessionCount }}</small></button>
      <button :class="['nav-item', { current: currentPage === 'usage' }]" :aria-current="currentPage === 'usage' ? 'page' : undefined" @click="emit('usage')"><Icon name="chart" :size="16" /><span>{{ t('Usage') }}</span></button>
      <button :class="['nav-item', { current: currentPage === 'system' }]" :aria-current="currentPage === 'system' ? 'page' : undefined" @click="emit('system')"><Icon name="gauge" :size="16" /><span>{{ t('System') }}</span></button>
    </nav>
    <header class="workspace-group-label"><span>{{ t('Workspaces') }}</span><small>{{ workspaces.length }}</small><!-- Finding the open session in the list belongs to the list, like an
         explorer's "select opened file": it acts on whichever view has focus. --><button class="icon-button workspace-locate-button" :disabled="!selectedSessionId" :aria-label="t('Locate the focused session in the list')" :title="t('Locate the focused session in the list')" @click="selectedSessionId && emit('revealSession', selectedSessionId)"><Icon name="locate" :size="14" /></button><button class="icon-button" :aria-label="t('Add workspace')" :title="t('Add workspace')" @click="emit('addWorkspace')"><Icon name="plus" :size="15" /></button></header>
    <div v-if="workspaces.length" class="workspace-group-search"><Icon name="search" :size="13" /><input v-model="query" data-sidebar-search :aria-label="t('Search workspaces and sessions')" @keydown.down.prevent="sidebarElement?.querySelector<HTMLElement>('[data-sidebar-row-focus]')?.focus()" @keydown.esc="query && ($event.stopPropagation(), query = '')" :placeholder="t('Find a workspace or session…')" /><button v-if="query" class="icon-button" :aria-label="t('Clear navigation search')" @click="query = ''"><Icon name="close" :size="12" /></button></div>
    <nav class="workspace-groups" :aria-label="t('Workspaces and sessions')">
      <section v-for="group in groups" :key="group.workspace.id" class="workspace-group" :class="{ selected: selectedWorkspaceId === group.workspace.id }" :aria-label="group.workspace.name">
        <div class="workspace-group-header" @contextmenu.prevent="openMenuId = group.workspace.id">
          <button class="icon-button workspace-disclosure" :class="{ expanded: group.expanded }" :aria-expanded="group.expanded" :aria-label="t(group.expanded ? 'Collapse {workspace} sessions' : 'Expand {workspace} sessions', { workspace: group.workspace.name })" :aria-controls="`workspace-sessions-${group.workspace.id}`" @click="setExpanded(group.workspace.id, !group.expanded)"><Icon name="chevron" :size="11" /></button>
          <!-- One row: a long name takes a second line, and the branch sits under
               it when there is one, rather than every workspace giving a row to
               a lone "+". -->
          <div class="workspace-group-title">
            <button class="workspace-group-name" :title="`${group.workspace.name}\n${group.workspace.root_path}`" :aria-expanded="group.expanded" :aria-controls="`workspace-sessions-${group.workspace.id}`" @click="selectWorkspace(group)" @keydown="navigateWorkspace($event, group)"><strong>{{ group.workspace.name }}</strong><svg v-if="group.pinned" width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" :aria-label="t('Pinned workspace')"><path d="m16 3 5 5-4 1-4 5v4l-3-3-6 6 6-6-3-3h4l5-5z" /></svg></button>
            <WorkspaceBranchMenu v-if="workspaceBranches?.[group.workspace.id]" compact :workspace-id="group.workspace.id" :branch="workspaceBranches[group.workspace.id]" @switched="emit('branchSwitched', group.workspace.id)" @changed="emit('branchSwitched', group.workspace.id)" />
          </div>
          <span v-if="group.waitingCount" class="workspace-active-count waiting" :title="t('{count} waiting for you', { count: group.waitingCount })" :aria-label="t('{count} waiting for you', { count: group.waitingCount })"><i class="state-dot waiting" />{{ group.waitingCount }}</span>
          <span v-else-if="group.activeCount" class="workspace-active-count" :title="t('{count} active sessions', { count: group.activeCount })" :aria-label="t('{count} active sessions', { count: group.activeCount })"><i class="state-dot running" />{{ group.activeCount }}</span>
          <button class="icon-button workspace-new-action" :title="t('New {provider} session in {workspace}', { provider: quickLabel, workspace: group.workspace.name })" :aria-label="t('New {provider} session in {workspace}', { provider: quickLabel, workspace: group.workspace.name })" @click="quick(group.workspace.id, quickProvider)"><Icon name="plus" :size="15" /></button>
          <button :ref="element => recordMenuButton(group.workspace.id, element)" class="icon-button workspace-more-button" :class="{ selected: openMenuId === group.workspace.id }" :aria-label="t('Workspace actions for {workspace}', { workspace: group.workspace.name })" :aria-expanded="openMenuId === group.workspace.id" :aria-controls="`workspace-menu-${group.workspace.id}`" @click="openMenuId = openMenuId === group.workspace.id ? undefined : group.workspace.id"><Icon name="more" :size="15" /></button>
        </div>
        <div v-if="openMenuId === group.workspace.id" :id="`workspace-menu-${group.workspace.id}`" class="workspace-more-panel">
          <div class="workspace-menu-label">{{ t('New') }}</div>
          <!-- One row for the kind made last, every kind beside it (MenuFlyout), as in the canvas's menus; the "New" heading names the action, so the row names only the kind. -->
          <MenuFlyout :label="quickLabel" :title="quickProvider === 'terminal' ? t('New terminal') : t('New {provider} session', { provider: quickLabel })" @activate="quick(group.workspace.id, quickProvider)">
            <template #icon><Icon v-if="quickProvider === 'terminal'" name="terminal" :size="13" /><ProviderIcon v-else :provider="quickProvider" :size="13" /></template>
            <button v-for="client in AGENT_CLIENTS" :key="client" role="menuitem" @click="quick(group.workspace.id, client)"><ProviderIcon :provider="client" :size="14" />{{ t('New {provider} session', { provider: providerLabel(client) }) }}</button>
            <button role="menuitem" @click="quick(group.workspace.id, 'terminal')"><Icon name="terminal" :size="14" />{{ t('New terminal') }}</button>
            <template v-if="ephemeralSupported"><hr /><button role="menuitem" class="workspace-scratch-action" :title="t('Discarded when its window is closed, and not kept in the session list.')" @click="quick(group.workspace.id, quickProvider, true)"><Icon name="clock" :size="14" />{{ t('New temporary window') }}</button></template>
          </MenuFlyout>
          <button @click="openMenuId = undefined; emit('newSession', group.workspace.id)"><Icon name="settings" :size="13" />{{ t('New session… (more options)') }}</button>
          <div class="workspace-menu-label">{{ t('Open') }}</div>
          <button @click="openMenuId = undefined; emit('openFiles', group.workspace.id)"><Icon name="folder" :size="13" />{{ t('Files') }}</button>
          <button @click="openMenuId = undefined; emit('openChanges', group.workspace.id)"><Icon name="git" :size="13" />{{ t('Changes') }}</button>
          <span :title="historySupported ? undefined : t('Upgrade the backend to load native session history.')"><button :disabled="!historySupported" @click="history(group.workspace.id)"><Icon name="clock" :size="13" />{{ t('Load existing session') }}</button></span>
          <p v-if="!historySupported">{{ t('Upgrade the backend to load native session history.') }}</p>
          <div class="workspace-menu-label">{{ t('Workspace') }}</div>
          <button @click="togglePin(group)"><Icon name="check" :size="13" />{{ t(group.pinned ? 'Unpin workspace' : 'Pin workspace') }}</button>
        </div>
        <ul v-show="group.expanded" :id="`workspace-sessions-${group.workspace.id}`" class="workspace-session-list" :aria-label="t('Sessions in {workspace}', { workspace: group.workspace.name })">
        <SidebarSessionRow v-for="session in group.recentSessions" :key="session.id" :session="session" :selected="selectedSessionId === session.id" :archive-supported="archiveSupported" :archive-busy="archiveBusySet.has(session.id)" :keep-busy="keepBusySet.has(session.id)" :delete-busy="deleteBusySet.has(session.id)" @open="emit('openSession', $event)" @delete="deleteSession" @rename="renameSession" @environment="emit('sessionEnvironment', $event)" @archive="archiveSession" @keep="keepSession" />
          <li v-if="group.olderSessions.length" class="workspace-older-toggle"><button :aria-expanded="Boolean(preferences.olderExpanded[group.workspace.id])" @click="toggleOlder(group.workspace.id)"><Icon name="chevron" :size="10" /><span>{{ t('Earlier') }}</span><small>{{ group.olderSessions.length }}</small></button></li>
          <template v-if="preferences.olderExpanded[group.workspace.id]">
        <SidebarSessionRow v-for="session in group.olderSessions" :key="session.id" class="older" :session="session" :selected="selectedSessionId === session.id" :archive-supported="archiveSupported" :archive-busy="archiveBusySet.has(session.id)" :keep-busy="keepBusySet.has(session.id)" :delete-busy="deleteBusySet.has(session.id)" @open="emit('openSession', $event)" @delete="deleteSession" @rename="renameSession" @environment="emit('sessionEnvironment', $event)" @archive="archiveSession" @keep="keepSession" />
          </template>
          <li v-if="!group.sessions.length" class="workspace-group-empty"><p>{{ t('No sessions in this workspace.') }}</p><button class="text-button" @click="quick(group.workspace.id, quickProvider)"><Icon name="plus" :size="12" />{{ t('New {provider} session', { provider: quickLabel }) }}</button></li>
        </ul>
      </section>
      <div v-if="workspaces.length && !groups.length" class="workspace-nav-empty"><Icon name="search" :size="22" /><p>{{ t('No matching workspaces or sessions.') }}</p><button class="text-button" @click="query = ''">{{ t('Clear navigation search') }}</button></div>
      <div v-else-if="!workspaces.length" class="workspace-nav-empty"><Icon name="folder" :size="24" /><p>{{ t('Add your first workspace') }}</p><button class="secondary-button" @click="emit('addWorkspace')"><Icon name="plus" :size="14" />{{ t('Add workspace') }}</button></div>
    </nav>
  </div>
</template>

<style scoped>
.workspace-sidebar{display:flex;flex-direction:column;min-width:0;min-height:0;flex:1;overflow:hidden}.workspace-global-nav{padding-bottom:10px;border-bottom:1px solid var(--fill-hover)}.workspace-global-nav .nav-item{min-height:36px;padding:8px 10px;font-size:var(--text-md);gap:10px;color:var(--ink)}.workspace-global-nav .count-badge{font-size:var(--text-xs);min-width:20px;padding:1px 6px;background:none;color:var(--faint);font-weight:500}.workspace-group-label{display:flex;align-items:center;gap:7px;padding:16px 8px 6px 10px;font-size:var(--text-xs);letter-spacing:.4px;color:var(--muted);flex-shrink:0}.workspace-group-label>span{font-weight:600}.workspace-group-label>small{font-size:var(--text-xs);color:var(--faint)}.workspace-group-label>.icon-button{width:24px;height:24px}.workspace-group-label>.icon-button:first-of-type{margin-left:auto}.workspace-group-label>.workspace-locate-button>svg{color:var(--muted)}.workspace-group-label>.workspace-locate-button:disabled{opacity:.4}.workspace-group-search{display:flex;align-items:center;gap:7px;padding:6px 10px;margin:2px 0 10px;border:1px solid transparent;border-radius:var(--radius-sm);background:var(--fill);color:var(--faint);flex-shrink:0}.workspace-group-search:focus-within{background:var(--surface);border-color:var(--line-strong);box-shadow:var(--shadow-sm)}.workspace-group-search>input{width:100%;min-width:0;padding:0;border:0;background:none;font-size:var(--text-sm);line-height:18px;color:var(--ink);outline:0}.workspace-group-search>input::placeholder{color:var(--faint)}.workspace-group-search>.icon-button{width:17px;height:17px;padding:2px}.workspace-groups{flex:1;min-height:0;overflow:auto;padding:0 1px 10px}/* Each workspace reads as its own block: a hairline and some air between
   them, and its sessions hang off a guide line under its name, so a session
   row is not mistaken for the next workspace's header (or the reverse). */
.workspace-group{margin:0 0 4px;padding-top:4px;min-width:0}
.workspace-group+.workspace-group{border-top:1px solid var(--fill-hover);padding-top:8px;margin-top:4px}
.workspace-session-list{margin:2px 0 4px 8px;padding:0 0 0 7px;border-left:1px solid var(--border)}.workspace-group-header{display:flex;align-items:center;gap:4px;min-width:0;min-height:32px;padding-right:2px;border-radius:var(--radius-sm)}.workspace-group-header:hover{background:var(--fill)}.workspace-disclosure{width:17px;height:24px;padding:2px;color:var(--faint)}.workspace-disclosure>svg{transition:transform 120ms ease}.workspace-disclosure.expanded>svg{transform:rotate(90deg)}.workspace-group-name{display:flex;align-items:center;gap:6px;flex:1;min-width:0;padding:6px 0;border:0;background:none;text-align:left;color:var(--ink)}.workspace-group-name>strong{font-size:var(--text-md);font-weight:650;letter-spacing:-.015em;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.workspace-group-name>svg{flex-shrink:0;color:var(--faint)}.workspace-group.selected .workspace-group-name{color:var(--ink)}.workspace-group-name:hover{color:var(--ink)}.workspace-group.selected .workspace-group-name::before{content:"";flex-shrink:0;width:6px;height:6px;margin-right:1px;border-radius:50%;background:var(--accent)}.workspace-active-count{display:flex;align-items:center;gap:4px;font-size:var(--text-xs);padding:1px 6px;border-radius:var(--radius-sm);background:var(--ok-soft);color:var(--accent-ink)}.workspace-active-count>.state-dot{width:6px;height:6px}.workspace-more-button{width:21px;height:23px;padding:2px;font-size:var(--text-xl);line-height:18px}.workspace-more-panel{margin:2px 4px 8px 19px;padding:5px;border:1px solid var(--accent-soft);border-radius:var(--radius-sm);background:var(--surface);box-shadow:var(--shadow-md)}.workspace-more-panel>button,.workspace-more-panel>span>button,.workspace-more-panel :deep(.menu-flyout-trigger){display:flex;align-items:center;gap:6px;width:100%;border:0;background:none;border-radius:var(--radius-xs);padding:6px 5px;text-align:left;font-size:var(--text-xs);line-height:15px;color:var(--ink-soft)}.workspace-more-panel>button:hover:not(:disabled),.workspace-more-panel>span>button:hover:not(:disabled),.workspace-more-panel :deep(.menu-flyout-trigger):hover,.workspace-more-panel :deep(.menu-flyout-trigger[aria-expanded="true"]){background:var(--fill);color:var(--ink)}.workspace-menu-label{padding:5px 5px 2px;font-size:var(--text-xs);font-weight:600;letter-spacing:.3px;color:var(--faint)}.workspace-menu-label:not(:first-child){margin-top:3px;border-top:1px solid var(--fill);padding-top:6px}.workspace-more-panel p{font-size:var(--text-xs);line-height:14px;color:#a18e67;padding:1px 6px 6px}.workspace-session-list{list-style:none;margin:2px 0 0 12px;padding:0}.workspace-older-toggle>button{display:flex;align-items:center;gap:6px;width:100%;margin:2px 0;padding:5px 8px;border:0;border-radius:var(--radius-sm);background:none;font-size:var(--text-xs);color:var(--faint);text-align:left}.workspace-older-toggle>button:hover{background:var(--fill);color:var(--muted)}.workspace-older-toggle>button>svg{transition:transform 120ms ease}.workspace-older-toggle>button[aria-expanded="true"]>svg{transform:rotate(90deg)}.workspace-older-toggle small{font-size:var(--text-xs);color:var(--faint)}.workspace-session-list>.older{opacity:.78}.workspace-group-empty{padding:6px 8px 8px;font-size:var(--text-sm);color:var(--faint);line-height:18px}.workspace-group-empty>.text-button{font-size:var(--text-sm);margin-top:2px}.workspace-nav-empty{padding:25px 8px;text-align:center;color:var(--faint)}.workspace-nav-empty p{font-size:var(--text-xs);line-height:18px;margin:10px 0}.workspace-nav-empty .secondary-button{font-size:var(--text-xs);padding:5px 8px}
.workspace-global-nav{flex-shrink:0}.workspace-global-nav .nav-item>.count-badge{margin-left:auto}.workspace-global-nav .nav-item>svg:first-child{color:var(--muted)}.workspace-global-nav .nav-item.current>svg:first-child{color:var(--accent)}
.workspace-more-panel>button>svg,.workspace-more-panel>span>button>svg,.workspace-more-panel :deep(.menu-flyout-trigger>svg:first-child){color:var(--violet)}.workspace-group-label>.icon-button>svg{color:var(--muted)}.workspace-group-search>svg{color:var(--muted)}
@media(pointer:coarse){.workspace-global-nav .nav-item{min-height:44px}.workspace-group-search>input{font-size:var(--input-text)}.workspace-group-header{min-height:44px}.workspace-disclosure,.workspace-more-button{width:32px;height:44px}.workspace-more-panel button{min-height:44px;font-size:var(--text-sm)}}
@media(prefers-reduced-motion:reduce){.workspace-disclosure>svg{transition:none}}
.workspace-active-count.waiting{color:var(--warn-ink);font-weight:600}

/* The name may take two lines rather than lose its end, and the branch sits
   under it; the controls stay centred on the row beside them either way. */
.workspace-group-title{flex:1;min-width:0;display:flex;flex-direction:column;align-items:flex-start;gap:2px;padding:4px 0}
.workspace-group-title>.workspace-group-name{width:100%;padding:2px 0}
.workspace-group-name>strong{display:-webkit-box;-webkit-box-orient:vertical;-webkit-line-clamp:2;white-space:normal;overflow-wrap:anywhere;line-height:1.35}
/* The selected dot sits beside the name's first line. Inside the clamped
   name it would be a line of its own: a -webkit-box stacks its children. */
.workspace-group.selected .workspace-group-name::before{align-self:flex-start;margin-top:calc(var(--text-md) * 1.35 / 2 - 3px)}
.workspace-group-title :deep(.ws-branch.compact){max-width:100%;margin-left:-2px}
.workspace-disclosure,.workspace-more-button,.workspace-active-count{flex-shrink:0}
.workspace-group-header>.workspace-new-action{flex-shrink:0;width:24px;height:24px;padding:4px;color:var(--teal)}
@media(pointer:coarse){.workspace-group-header>.workspace-new-action{width:36px;height:44px}.workspace-group-title{padding:6px 0}}
</style>
