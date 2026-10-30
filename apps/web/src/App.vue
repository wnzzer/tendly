<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, provide, reactive, ref, watch } from "vue";
import { useKeyboardFit } from "./features/viewport-height";
import type { EndpointProfile, FileEntry, LayoutDocument, PaneNode, ProviderKind, Session, Workspace } from "@tendly/protocol";
import Canvas from "./components/Canvas.vue";
import { createDefaultLayoutDocument, flattenPanes } from "./layout/layout-engine";
import WorkspaceSidebar from "./features/WorkspaceSidebar.vue";
import WorkspacePane from "./features/WorkspacePane.vue";
import FileExplorer from "./features/FileExplorer.vue";
import LoadHistoryDialog from "./features/LoadHistoryDialog.vue";
import WorkspaceDialog from "./features/WorkspaceDialog.vue";
import CreateSessionDialog from "./features/CreateSessionDialog.vue";
import SettingsPage from "./features/SettingsPage.vue";
import SessionsPage from "./features/SessionsPage.vue";
import SystemPage from "./features/SystemPage.vue";
import UsagePage from "./features/UsagePage.vue";
import { useRoute, useRouter } from "vue-router";
import type { SettingsSection } from "./router";
import HostFilePreview from "./features/HostFilePreview.vue";
import HostUsage from "./features/HostUsage.vue";
import { requestJump } from "./features/file-jumps";
import { announceFilesChanged, createFileWatch } from "./features/file-watch";
import { onPageReturn } from "./features/page-return";
import WorkspaceBranchMenu from "./features/WorkspaceBranchMenu.vue";
import ImageLightbox from "./features/ImageLightbox.vue";
import SessionEnvironmentDialog from "./features/SessionEnvironmentDialog.vue";
import SessionRunInfo from "./features/SessionRunInfo.vue";
import { SESSION_ACTIONS } from "./features/session-actions-context";
import SessionRenameDialog from "./features/SessionRenameDialog.vue";
import AuthDialog from "./features/AuthDialog.vue";
import { linkSignIn } from "./features/link-sign-in";
import Icon from "./features/Icon.vue";
import TabIcon from "./features/TabIcon.vue";
import BottomSheet from "./features/BottomSheet.vue";
import AgentActivity, { type ShowTarget } from "./features/AgentActivity.vue";
import ToastHost from "./features/ToastHost.vue";
import CommandPalette from "./features/CommandPalette.vue";
import ShortcutsDialog from "./features/ShortcutsDialog.vue";
import { CHORDS, PREFIX_COMMANDS, PREFIX_GROUPS, isRename, matches, prefixCommand, prefixKey, shortcutFor } from "./features/shortcuts";
import type { PaletteItem } from "./features/CommandPalette.vue";
import { setThemePreference } from "./features/theme";
import { isSessionArchived } from "./features/session-list";
import { attentionTitle, trackSessions, waitingIds } from "./features/attention";
import { useMobile } from "./features/mobile";
import { useI18n } from "./i18n";
import { ApiConnectionError, ApiError, errorMessage, json, providerLabel, rememberRemote, request, sessionPath, workspacePath } from "./features/api";
import { AGENT_CLIENTS, DEFAULT_CLIENT, PROVIDER_KINDS } from "./features/clients";
import { loadPreferences, preferenceFor } from "./features/preferences";
import { placeKey, placeOf, placeUrl } from "./features/checkout-place";
import { remotePane } from "./features/remote-session";
import { openPeerWindow } from "./features/peers";
import { paneDisplayTitle, paneString, paneWorkspace, paneSession, paneCheckout, filePane, changesPane, sessionPane, renameSessionPanes, acceptsScopedPane, ephemeralSessionIds, withoutDiscardedPanes } from "./features/pane-context";
import { lastQuickProvider, planQuickSession, rememberQuickProvider } from "./features/quick-session";
import { backendCapabilities, setBackendCapabilities, type BackendHealth } from "./features/backend-capabilities";
import { formatLatency } from "./features/host-latency";
import { loadAccountQuotas, loadBalances } from "./features/balance-store";
import { browserStorage, canvasStorageKey } from "./features/canvas-cache";
import { readPanelVisibility, sidebarIsRail, useSidebarWidth, writePanelVisibility } from "./features/panel-state";
import { hasDirtyDrafts, moveFileDrafts } from "./features/file-drafts";
import { hasGitDrafts } from "./features/git-view-state";
import { sessionConnections } from "./features/session-connection";
import { conversationCache } from "./features/conversation-cache";
import { backTargetFor, hasUnsavedWork, rememberPane, resolveReference } from "./features/app-shell";
import { layoutInTrouble, useCanvasPersistence } from "./features/canvas-persistence";
import { opensOnArrival, useSessionChanges } from "./features/session-changes";
import { paletteItems as buildPaletteItems, shortcutHint } from "./features/palette-items";
import { WORKSPACE_GIT, useWorkspaceGit } from "./features/workspace-git";
import { headerBranches } from "./features/session-header";
import "@xterm/xterm/css/xterm.css";
import "./styles.css";

const { t, locale, setLocale } = useI18n();
const workspaces = ref<Workspace[]>([]), sessions = ref<Session[]>([]), profiles = ref<EndpointProfile[]>([]);
const selectedWorkspaceId = ref(""), selectedPaneId = ref<string>();
const selectedWorkspace = computed(() => workspaces.value.find(workspace => workspace.id === selectedWorkspaceId.value));
const layout = ref<LayoutDocument>(createDefaultLayoutDocument());
const canvas = ref<InstanceType<typeof Canvas>>(), explorer = ref<InstanceType<typeof FileExplorer>>();
const workspaceSidebar = ref<InstanceType<typeof WorkspaceSidebar>>();
const focusedPane = computed(() => flattenPanes(layout.value.root).find(pane => pane.id === selectedPaneId.value));
/** The branch the focused session works on, named in a phone's top bar (session-header.ts). */
const focusedBranch = computed(() => focusedPane.value ? headerBranches.get(focusedPane.value.id) : undefined);
const focusedWorkspace = computed(() => focusedPane.value ? paneWorkspace(focusedPane.value, workspaces.value, sessions.value) : undefined);
const contextWorkspace = computed(() => focusedWorkspace.value ?? selectedWorkspace.value);
/** The worktree the focused pane looks at, when it is one: the status bar, the drawer and the file panel follow it there. */
const contextCheckout = computed(() => focusedPane.value && focusedWorkspace.value ? paneCheckout(focusedPane.value, sessions.value) : null);
const contextPlace = computed(() => contextWorkspace.value ? placeKey(contextWorkspace.value.id, contextCheckout.value) : undefined);
/** Where a pane looks (`placeKey`), for one bound to a workspace. */
function panePlace(pane: PaneNode) { const workspace = paneWorkspace(pane, workspaces.value, sessions.value); return workspace ? placeKey(workspace.id, paneCheckout(pane, sessions.value)) : undefined; }
const selectedSessionId = computed(() => focusedPane.value ? paneString(focusedPane.value, "session_id") : undefined);
const activeSessions = computed(() => sessions.value.filter(session => ["running", "waiting", "starting"].includes(session.status)));
const sessionProviders = computed(() => Object.fromEntries(sessions.value.map(session => [session.id, session.provider])));
const panels = readPanelVisibility(browserStorage(), { sidebar: true, explorer: window.innerWidth > 1100 });
const explorerWorkspaceId = ref(""), explorerCheckout = ref<string | null>(null), explorerPinned = ref(false), explorerOpen = ref(panels.explorer);
const sidebarExpanded = ref(panels.sidebar), sidebarRail = ref(sidebarIsRail(window.innerWidth));
/** A drawer-width viewport ignores the stored collapse without overwriting it. */
const sidebarCollapsed = computed(() => sidebarRail.value && !sidebarExpanded.value);
const explorerWorkspace = computed(() => workspaces.value.find(workspace => workspace.id === explorerWorkspaceId.value));
const explorerPlace = computed(() => placeKey(explorerWorkspaceId.value, explorerCheckout.value));
/** Points the file panel at a place, unless it is pinned to one. */
function followWithExplorer(id: string, checkout: string | null) { if (!explorerPinned.value) { explorerWorkspaceId.value = id; explorerCheckout.value = checkout; } }
const activeFilePath = computed(() => focusedPane.value && panePlace(focusedPane.value) === explorerPlace.value ? paneString(focusedPane.value, "path") : undefined);
const sidebarOpen = ref(false), loading = ref(true), apiOnline = ref(false), error = ref("");
const connectionError = ref(""), refreshingResources = ref(false);
const visibleError = computed(() => error.value || connectionError.value);
const platform = ref<string>(), instanceLabel = ref<string>(), serverVersion = ref<string>();
const latencyMs = ref<number>();
const hostStatus = computed(() => formatLatency(latencyMs.value, apiOnline.value));
// The host's OS, cased the way each platform writes its own name.
const PLATFORM_LABELS: Record<string, string> = { windows: "Windows", macos: "macOS", darwin: "macOS", linux: "Linux" };
const platformLabel = computed(() => (platform.value ? PLATFORM_LABELS[platform.value] ?? platform.value : "Local"));
const showWorkspace = ref(false), showAuth = ref(false), workspaceInitialPath = ref<string>();
/** The host has let this browser in: until it says so, nothing that polls is mounted. */
const signedIn = ref(false);
/** A file an agent named outside every workspace, shown read-only. */
const hostPreview = ref<{ path: string; line?: number }>();
/**
 * App also renders without a router (the read-only UI preview, and tests that
 * mount it alone); it is then always on the canvas and navigation is a no-op.
 */
const route = useRoute() ?? reactive({ name: "canvas", params: {}, query: {}, fullPath: "/" });
const router = useRouter() ?? ({ push: async () => undefined, replace: async () => undefined, back: () => undefined } as unknown as ReturnType<typeof useRouter>);
/** Which page, if any, is drawn over the canvas. The canvas itself never unmounts. */
const page = computed(() => route.name === "settings" || route.name === "sessions" || route.name === "system" || route.name === "usage" ? route.name : undefined);
const settingsRouteSection = computed(() => String(route.params.section ?? "preferences") as SettingsSection);
// Session menus are teleported out of their tabs to the body, so a page over
// the canvas cannot cover them; the body says a page is open and they step aside.
watch(page, value => { const body = typeof document === "undefined" ? undefined : document.body; if (!body) return; if (value) body.dataset.page = value; else delete body.dataset.page; }, { immediate: true });
function openSettings(section: SettingsSection = "preferences") { sidebarOpen.value = false; void router.push({ name: "settings", params: { section } }); }
function openPage(name: "sessions" | "system" | "usage", query?: Record<string, string>) { sidebarOpen.value = false; void router.push({ name, query }); }
/** Back from a page: to wherever you came from inside the app, else the canvas. */
function leavePage() {
  if (window.history.state?.back) router.back();
  else void router.push({ name: "canvas" });
}
const quickBusy = ref(false);
const ephemeralIds = computed(() => ephemeralSessionIds(sessions.value));
const historyWorkspaceId = ref<string>();
const environmentSessionId = ref<string>();
const renameSessionId = ref<string>();
const environmentSession = computed(() => sessions.value.find(session => session.id === environmentSessionId.value));
const renameSessionTarget = computed(() => sessions.value.find(session => session.id === renameSessionId.value));
const historyWorkspace = computed(() => workspaces.value.find(workspace => workspace.id === historyWorkspaceId.value));
const { gitStatuses, fileRefresh, gitRefresh, currentGit, currentGitAvailable, currentGitKnown, workspaceBranches, tick, gitChanged, refreshGit, filesSaved } = useWorkspaceGit({ contextPlace, activeSessions, paused: () => showAuth.value, report });
const storage = browserStorage();
const storageKey = computed(() => canvasStorageKey(window.location.origin, workspaces.value));
const { canvasReady, layoutStatus, canvasRevision, canvasConflict, localRecovery, cacheLayout, queueLayout, flushLayout, initializeCanvas, resolveCanvasConflict, restoreLocalRecovery, agentCanvas, hasPendingLayout, showWithoutSaving, dispose: disposeCanvas } = useCanvasPersistence({
  layout, selectedPaneId, sessions, storage, storageKey, report,
  onReady: preferred => { explorerWorkspaceId.value = contextWorkspace.value?.id ?? preferred.id; explorerCheckout.value = contextWorkspace.value ? contextCheckout.value : null; },
});
const layoutTrouble = computed(() => layoutInTrouble(layoutStatus.value));
let disposed = false, bootstrapRevision = 0;
let pollTimer: ReturnType<typeof setInterval> | undefined;
const sessionsLoading = ref(false);
/** Set once the session list has loaded, so an empty list before then is not read as "every session is gone". */
const sessionsKnown = ref(false);
let sessionsRevision = 0;
const { archiveBusyIds, keepBusyIds, deleteBusyIds, discardPrompt, sessionNotice, archiveSession, archiveSessions, deleteSessions, confirmClosePane, confirmDiscard, keepSession, keepSessionById, renameSession, enableChat } = useSessionChanges({ sessions, layout, canvas, report, t, touch: () => { sessionsRevision++; }, returnTo: returnToConversation });
let workspaceRegistryRevision = 0;
let wasDesktop = window.innerWidth > 1100;
/**
 * A phone gets one pane at a time. What the tab strip and layout tools do on
 * a desktop -- switching, opening, closing -- moves to a sheet opened from the
 * title, and the status bar's contents move into the drawer.
 */
const mobile = useMobile();
useKeyboardFit();
const switcherOpen = ref(false);
const openPanes = computed(() => flattenPanes(layout.value.root));
const paneTitle = (pane: PaneNode) => paneDisplayTitle(pane, t);
function paneDetail(pane: PaneNode) {
  const session = paneSession(pane, sessions.value), workspace = paneWorkspace(pane, workspaces.value, sessions.value);
  const place = panePlace(pane);
  const branch = session?.checkout_branch ?? (place ? gitStatuses.value[place]?.branch : undefined);
  return [session && session.provider !== "terminal" ? providerLabel(session.provider) : undefined, workspace?.name, branch].filter(Boolean).join(" · ");
}
const focusedSession = computed(() => focusedPane.value ? paneSession(focusedPane.value, sessions.value) : undefined);
function switchPane(pane: PaneNode) { switcherOpen.value = false; void canvas.value?.focusPane(pane.id); }
async function closeOpenPane(pane: PaneNode) { await canvas.value?.closePane(pane.id); }
/** An agent changed something: show it without waiting for the next poll. */
function agentChanged() { void refreshProfiles(); void refreshResources(); void loadPreferences(true); void loadBalances(); }
/** An agent pointed at something (tendly_show), or a notice's Open. */
async function agentShow(target: ShowTarget) {
  if (target.kind === "file") void openReference(target.workspace_id, { path: target.path, line: target.line ?? undefined, checkout: target.checkout });
  else if (target.kind === "changes") openChanges(target.workspace_id, target.checkout ?? null);
  else {
    // A session an agent just started may not be in the list yet.
    let session = sessions.value.find(item => item.id === target.session_id);
    if (!session) { await refreshSessions(); session = sessions.value.find(item => item.id === target.session_id); }
    if (session) openSession(session);
  }
}
function sheetAction(action: () => void) { switcherOpen.value = false; action(); }
/**
 * Files and Git open like a pushed page: the top bar trades its menu for a
 * back arrow that returns to the view they were opened from, usually the
 * conversation. The panes themselves stay open, as on a desktop.
 */
const paneHistory = ref<string[]>([]);
let goingBack = false;
watch(selectedPaneId, (id, previous) => {
  if (goingBack) { goingBack = false; return; }
  if (!mobile.value || !previous || id === previous) return;
  paneHistory.value = rememberPane(paneHistory.value, previous, id);
});
const backTarget = computed(() => backTargetFor(focusedPane.value, paneHistory.value, new Set(openPanes.value.map(pane => pane.id))));
function goBack() {
  const target = backTarget.value;
  if (!target) return;
  paneHistory.value = paneHistory.value.filter(id => id !== target);
  goingBack = true;
  void canvas.value?.focusPane(target);
}

function report(cause: unknown) {
  if (cause instanceof ApiConnectionError) {
    apiOnline.value = false;
    if (cause.outcomeUnknown) error.value = cause.message;
    else connectionError.value = cause.message;
    return;
  }
  if (cause instanceof ApiError && cause.status === 401) { showAuth.value = true; signedIn.value = false; apiOnline.value = false; }
  else error.value = errorMessage(cause);
}
function rememberWorkspace(id: string) { try { storage?.setItem("tendly:last-workspace", id); } catch { /* Optional preference. */ } }
function rememberedWorkspace() { try { return storage?.getItem("tendly:last-workspace"); } catch { return null; } }
function selectWorkspace(id: string) {
  if (!workspaces.value.some(workspace => workspace.id === id)) return;
  selectedWorkspaceId.value = id; rememberWorkspace(id);
  if (!focusedWorkspace.value) followWithExplorer(id, null);
  void refreshGit(id);
}
function selectPane(pane: PaneNode) {
  selectedPaneId.value = pane.id;
  const workspace = paneWorkspace(pane, workspaces.value, sessions.value);
  if (workspace) {
    const checkout = paneCheckout(pane, sessions.value);
    followWithExplorer(workspace.id, checkout);
    void refreshGit(placeKey(workspace.id, checkout));
  }
}
// Custom balances show in every message box, so read them once the backend says it has them.
watch(() => backendCapabilities.customBalances, enabled => { if (enabled) void loadBalances(); }, { immediate: true });
watch(() => backendCapabilities.accounts, enabled => { if (enabled) void loadAccountQuotas(); }, { immediate: true });
watch(contextPlace, place => { if (place) { const { workspaceId, checkout } = placeOf(place); followWithExplorer(workspaceId, checkout); void refreshGit(place); } });
// A canvas just put in place gives the keys to its selected pane, as moving to
// it would; on a phone that would raise the keyboard unasked, so not there.
watch(canvasReady, async ready => {
  if (!ready || mobile.value) return;
  await nextTick();
  if (!document.activeElement || document.activeElement === document.body) canvas.value?.focusSelected();
});
// A tab keeps the name its session had when it opened. The server renames a
// session too -- after its first message -- so every change in the list
// reaches the tabs, not only a rename made here. Every open page does this,
// so none saves it (showWithoutSaving).
watch(sessions, list => {
  let root = layout.value.root;
  for (const session of list) root = renameSessionPanes(root, session.id, session.title);
  if (root !== layout.value.root) void showWithoutSaving({ ...layout.value, root });
});
// A temporary window restored from a saved layout whose session has since been
// discarded (a server restart, another page closing it) has nothing to show.
watch([sessions, sessionsKnown, canvasRevision, canvasReady], () => {
  if (!sessionsKnown.value || !canvasReady.value) return;
  const root = withoutDiscardedPanes(layout.value.root, sessions.value);
  if (root !== layout.value.root) layout.value = { ...layout.value, root };
});
function openSession(session: Session) {
  if (!workspaces.value.some(workspace => workspace.id === session.workspace_id)) {
    // A session can arrive before its workspace does -- one opened in a
    // worktree made a moment ago, from inside a pane. Fetch it rather than
    // refusing a session that exists.
    void request<Workspace>(workspacePath(session.workspace_id)).then(workspace => { worktreeWorkspace(workspace); openSession(session); }).catch(() => { error.value = t("Pane source unavailable"); });
    return;
  }
  sessions.value = [session, ...sessions.value.filter(item => item.id !== session.id)];
  // Opening a session from a page lands on the canvas, where it opens.
  if (route.name !== "canvas") void router.push({ name: "canvas" });
  // Opening a session selects its bound view. A legacy PTY shown in the
  // conversation shell must not start just because a pane was mounted;
  // structured sessions, terminals and an explicit native-terminal view
  // retain the existing start intent.
  if (opensOnArrival(session)) sessionConnections.requestOpen(session.id);
  sidebarOpen.value = false;
  canvas.value?.openPane(sessionPane(session));
}
/** A paired machine's session, in a tab here. Starting it is decided as for one of ours, on what that machine says of it. */
async function openRemoteSession(machine: { id: string; name: string }, session: { id: string; title: string; provider: string }) {
  // No workspace here means no canvas to hold a tab: its own window it is.
  if (!canvasReady.value) { openPeerWindow(machine.id, session.id).catch(report); return; }
  rememberRemote(machine.id, [session.id]);
  if (route.name !== "canvas") void router.push({ name: "canvas" });
  sidebarOpen.value = false;
  canvas.value?.openPane(remotePane(machine, session));
  try { const found = await request<Session>(sessionPath(session.id)); if (opensOnArrival(found)) sessionConnections.requestOpen(found.id); }
  catch { /* The tab says what is wrong. */ }
}
/** Back from an escape-hatch terminal to the conversation it reopened: its tab, opened again if it was closed meanwhile.
 * The view it held from before is dropped, so the conversation is read afresh rather than resumed from it; the open
 * intent `openSession` records is what makes a view already on screen read it again. */
function returnToConversation(id: string) {
  const source = sessions.value.find(item => item.id === id);
  if (!source) return;
  conversationCache.forget(id);
  openSession(source);
}
function sessionPaneDropped(pane: PaneNode) {
  const session = paneSession(pane, sessions.value);
  if (session && paneWorkspace(pane, workspaces.value, sessions.value) && opensOnArrival(session)) sessionConnections.requestOpen(session.id);
}
function acceptDroppedPane(pane: PaneNode) { return acceptsScopedPane(pane, workspaces.value, sessions.value); }
async function revealSession(id: string) {
  const session = sessions.value.find(item => item.id === id);
  if (!session) return;
  selectWorkspace(session.workspace_id);
  sidebarOpen.value = window.innerWidth <= 1100;
  await nextTick();
  await workspaceSidebar.value?.revealSession(id);
}
// The phone's drawer opens on the session in view, not wherever its list was
// last left: among many alike-named rows, the current one is the anchor.
watch(sidebarOpen, async open => {
  const id = selectedSessionId.value;
  if (!open || !mobile.value || !id) return;
  // Archived sessions live in the library, not the drawer's groups; revealing
  // one would leave the drawer for that page.
  const session = sessions.value.find(item => item.id === id);
  if (!session || isSessionArchived(session)) return;
  await nextTick();
  const row = () => document.querySelector<HTMLElement>(`.workspace-groups [data-sidebar-session-id="${CSS.escape(id)}"]`);
  // Its workspace may be folded: unfold it first, then centre the row.
  if (!row()) await workspaceSidebar.value?.revealSession(id);
  row()?.scrollIntoView({ block: "center" });
});
function requestRenameSession(id: string) { if (sessions.value.some(session => session.id === id)) renameSessionId.value = id; }
const runInfoSessionId = ref<string>();
const runInfoSession = computed(() => sessions.value.find(session => session.id === runInfoSessionId.value));
provide(SESSION_ACTIONS, {
  session: id => sessions.value.find(session => session.id === id),
  all: () => sessions.value,
  open: session => openSession(session),
  keepBusy: id => keepBusyIds.value.includes(id),
  rename: requestRenameSession,
  environment: id => { environmentSessionId.value = id; },
  keep: keepSessionById,
  info: id => { runInfoSessionId.value = id; },
});
provide(WORKSPACE_GIT, { branch: id => workspaceBranches.value[id], openGit: id => openChanges(id, null) });
async function saveRenameSession(title: string) {
  const session = renameSessionTarget.value;
  if (!session) return;
  // Saving the name it already has is done, not refused.
  if (title.trim() === session.title || await renameSession(session, title)) renameSessionId.value = undefined;
}
function sessionEnvironmentSaved(session: Session) {
  sessions.value = sessions.value.map(item => item.id === session.id ? session : item);
  environmentSessionId.value = undefined;
  // Updating metadata is not an open gesture: never requestOpen or start here.
}
/** `checkout`: the worktree the file is in; without it, the workspace directory. */
function openFile(id: string, file: FileEntry | string, checkout: string | null = null) {
  if (!workspaces.value.some(workspace => workspace.id === id)) return;
  canvas.value?.openPane(filePane(id, typeof file === "string" ? file : file.path, checkout));
  if (window.innerWidth <= 1100) explorerOpen.value = false;
}
/** A file or folder renamed from the explorer: open panes on it, or inside it, show it under its new name, drafts and all. */
function fileMoved(id: string, checkout: string | null, from: string, to: string) {
  const place = placeKey(id, checkout);
  moveFileDrafts(place, from, to);
  for (const pane of flattenPanes(layout.value.root)) {
    const path = paneString(pane, "path");
    if (!path || (pane.kind !== "editor" && pane.kind !== "file_preview") || panePlace(pane) !== place) continue;
    if (path === from) canvas.value?.retarget(pane.id, filePane(id, to, checkout));
    else if (path.startsWith(`${from}/`)) canvas.value?.retarget(pane.id, filePane(id, to + path.slice(from.length), checkout));
  }
}
/**
 * Open a file a chat message pointed at: an absolute path inside the
 * workspace (or the session's worktree) becomes relative, a path with a
 * directory opens as it is, and a bare name is looked up in the workspace
 * index -- an exact name match, the shallowest if several. A line is kept for
 * the file pane to show.
 */
async function openReference(id: string, reference: { path: string; line?: number; checkout?: string | null }) {
  const workspace = workspaces.value.find(item => item.id === id); if (!workspace) return;
  const target = resolveReference(reference, workspace, workspaces.value);
  if (target.kind === "host") { hostPreview.value = { path: target.path, line: reference.line }; return; }
  let path: string;
  if (target.kind === "search") {
    try {
      const found = await request<{ files: { path: string; name: string; kind: string }[] }>(placeUrl(placeKey(id, target.checkout), `/files/search?${new URLSearchParams({ q: target.name, limit: '20' })}`));
      const exact = found.files.filter(hit => hit.kind === 'file' && hit.name === target.name).sort((a, b) => a.path.split('/').length - b.path.split('/').length || a.path.length - b.path.length);
      if (!exact.length) { error.value = t('Could not find {name} in this workspace.', { name: target.name }); return; }
      path = exact[0].path;
    } catch (cause) { report(cause); return; }
  } else path = target.path;
  if (reference.line) requestJump(placeKey(target.workspaceId, target.checkout), path, reference.line);
  openFile(target.workspaceId, path, target.checkout);
}
/** A workspace's Git changes; with no workspace named, those of the place in context, a session's worktree included. */
function openChanges(id?: string, checkout: string | null = null) {
  if (!id) { id = contextWorkspace.value?.id; checkout = contextCheckout.value; }
  if (!id || !workspaces.value.some(workspace => workspace.id === id)) return;
  canvas.value?.openPane(changesPane(id, checkout)); sidebarOpen.value = false;
}
function openFiles(id = selectedWorkspaceId.value, checkout: string | null = null) {
  if (!workspaces.value.some(workspace => workspace.id === id)) return;
  explorerWorkspaceId.value = id; explorerCheckout.value = checkout; explorerPinned.value = false; explorerOpen.value = true; sidebarOpen.value = false;
}
/**
 * The configured way to start a session: a dialog over the canvas, not a tab
 * among the sessions. Nothing exists on the server until Create. The dialog
 * only hides on the settings page, so an account added there can be picked
 * on the way back.
 */
const newSessionTarget = ref<{ workspaceId: string; provider: ProviderKind }>();
const newSessionWorkspace = computed(() => workspaces.value.find(workspace => workspace.id === newSessionTarget.value?.workspaceId));
function newSession(id = selectedWorkspaceId.value, provider: ProviderKind = DEFAULT_CLIENT) {
  if (!workspaces.value.some(workspace => workspace.id === id)) return;
  if (route.name !== "canvas") void router.push({ name: "canvas" });
  sidebarOpen.value = false;
  // A draft tab a layout saved before the dialog still holds is that form, already started.
  const existing = flattenPanes(layout.value.root).find(pane => pane.metadata?.draft === true && paneString(pane, "workspace_id") === id);
  if (existing) { void canvas.value?.focusPane(existing.id); return; }
  // Asking again for the same workspace returns to the form already started.
  if (newSessionTarget.value?.workspaceId !== id) newSessionTarget.value = { workspaceId: id, provider };
}
async function newSessionCreated(session: Session) {
  newSessionTarget.value = undefined;
  sessionsRevision++;
  sessions.value = [session, ...sessions.value.filter(item => item.id !== session.id)];
  if (opensOnArrival(session)) sessionConnections.requestOpen(session.id);
  canvas.value?.openPane(sessionPane(session));
  await nextTick(); canvas.value?.focusSelected();
}
/** A draft tab becomes the session it created, in the same place. */
function draftCreated(paneId: string, session: Session) {
  sessionsRevision++;
  sessions.value = [session, ...sessions.value.filter(item => item.id !== session.id)];
  if (opensOnArrival(session)) sessionConnections.requestOpen(session.id);
  canvas.value?.replaceWith(paneId, sessionPane(session));
}
/**
 * One click from the sidebar to a usable session. The configuration dialog is
 * for the rare configured case; making every session pay for it is what made
 * opening a window feel heavy. Falls back to the dialog only when an account
 * genuinely cannot be chosen for the user.
 */
async function quickSession(id = selectedWorkspaceId.value, provider: ProviderKind = lastQuickProvider(), ephemeral = false, targetPaneId?: string) {
  if (!workspaces.value.some(workspace => workspace.id === id) || quickBusy.value) return;
  await loadPreferences();
  const plan = planQuickSession(provider, profiles.value, {
    preferredProfileId: preferenceFor(provider)?.endpoint_profile_id,
    ephemeral,
    structuredChat: backendCapabilities.structuredChat,
    ephemeralSupported: backendCapabilities.ephemeralSessions,
    existingTitles: sessions.value.filter(session => session.workspace_id === id).map(session => session.title),
  });
  if (plan.needsDialog || !plan.body) { newSession(id, provider); return; }
  quickBusy.value = true;
  try {
    const session = await request<Session>(`${workspacePath(id)}/sessions`, json("POST", plan.body));
    rememberQuickProvider(provider);
    sessionsRevision++;
    sessions.value = [session, ...sessions.value.filter(item => item.id !== session.id)];
    if (opensOnArrival(session)) sessionConnections.requestOpen(session.id);
    sidebarOpen.value = false;
    // A "+" pressed on a tab strip opens there; the sidebar has no target and
    // falls back to the canvas's own placement.
    canvas.value?.openPaneAt(targetPaneId, sessionPane(session));
    // Made to be used: its input takes the keys once drawn (a terminal mounts once attached).
    await nextTick(); canvas.value?.focusSelected();
  } catch (cause) { report(cause); }
  finally { quickBusy.value = false; }
}
/** Created from a tab strip's "+", so it lands in that stack. */
function createSessionInPane(targetId: string, provider: ProviderKind, ephemeral: boolean) {
  const pane = flattenPanes(layout.value.root).find(item => item.id === targetId);
  const workspaceId = (pane && paneString(pane, "workspace_id")) ?? contextWorkspace.value?.id ?? selectedWorkspaceId.value;
  void quickSession(workspaceId, provider, ephemeral, targetId);
}
function loadHistory(id: string) { if (backendCapabilities.nativeHistory) historyWorkspaceId.value = id; }
async function historyLoaded(session: Session) { historyWorkspaceId.value = undefined; await nextTick(); openSession(session); }
/** A worktree made while creating a session: registered without leaving the dialog. */
function worktreeWorkspace(workspace: Workspace) { workspaceRegistryRevision++; workspaces.value = [workspace, ...workspaces.value.filter(item => item.id !== workspace.id)]; }
async function workspaceCreated(workspace: Workspace) {
  workspaceRegistryRevision++;
  showWorkspace.value = false;
  workspaces.value = [workspace, ...workspaces.value.filter(item => item.id !== workspace.id)];
  selectWorkspace(workspace.id);
  if (!canvasReady.value) await initializeCanvas(workspace);
}
/** A branch switched from the status bar or the sidebar: its files and sessions changed under it. */
function branchSwitched(id: string) { filesSaved(id); void refreshSessions(); }
async function refreshSessions() {
  if (showAuth.value || sessionsLoading.value) return;
  sessionsLoading.value = true;
  const own = ++sessionsRevision;
  try { const list = await request<Session[]>("/sessions"); if (own === sessionsRevision) { sessions.value = list; sessionsKnown.value = true; } apiOnline.value = true; connectionError.value = ""; }
  catch (cause) { apiOnline.value = false; report(cause); }
  finally { sessionsLoading.value = false; }
}
let profileRefreshRevision = 0;
async function refreshProfiles(imported?: EndpointProfile) {
  const own = ++profileRefreshRevision;
  if (imported) profiles.value = [imported, ...profiles.value.filter(profile => profile.id !== imported.id)];
  try { const list = await request<EndpointProfile[]>("/endpoint-profiles"); if (own === profileRefreshRevision) profiles.value = list; }
  catch (cause) { if (own === profileRefreshRevision) report(cause); }
}
/** Every place on screen (`placeKey`): its Git status is polled and its files watched. */
function trackedPlaces() {
  return new Set([contextPlace.value, explorerWorkspace.value ? explorerPlace.value : undefined, ...flattenPanes(layout.value.root).map(panePlace)].filter((id): id is string => !!id));
}
/** Files changed on the host, pushed for every workspace on screen. The Git poll stays as the fallback. */
const fileWatch = createFileWatch({
  createSocket: url => new WebSocket(url),
  url: id => { const url = new URL(`/api${placeUrl(id, "/files/ws")}`, window.location.href); url.protocol = url.protocol === "https:" ? "wss:" : "ws:"; return url.href; },
  onChange(id, paths, git) {
    announceFilesChanged(id, paths);
    // A file edited again stays `M` in status, but its diff moved.
    if (paths?.length !== 0) tick(gitRefresh, id);
    if (paths?.length !== 0 || git) void refreshGit(id);
  },
});
watch(() => showAuth.value ? "" : [...trackedPlaces()].sort().join("\n"), ids => fileWatch.sync(ids ? ids.split("\n") : []), { immediate: true });
let stopPageReturn: (() => void) | undefined;
async function refreshResources() {
  if (refreshingResources.value || loading.value) return;
  refreshingResources.value = true; error.value = ""; connectionError.value = "";
  const registryRevision = workspaceRegistryRevision;
  try {
    const health = await request<BackendHealth>("/health");
    if (!health.ok) throw new ApiError("Tendly health check failed", 503);
    const previousSharedStorage = backendCapabilities.sharedCanvas;
    setBackendCapabilities(health); apiOnline.value = true;
    platform.value = health.platform; instanceLabel.value = health.instance_label; serverVersion.value = health.version;
    const [workspaceList] = await Promise.all([request<Workspace[]>("/workspaces"), refreshSessions(), refreshProfiles(), loadPreferences(true)]);
    if (registryRevision === workspaceRegistryRevision) {
      workspaces.value = workspaceList;
      if (!workspaceList.some(workspace => workspace.id === selectedWorkspaceId.value)) selectedWorkspaceId.value = workspaceList[0]?.id ?? "";
    }
    if (selectedWorkspace.value && (!canvasReady.value || previousSharedStorage !== backendCapabilities.sharedCanvas)) {
      if (canvasReady.value) cacheLayout(layout.value, true);
      await initializeCanvas(selectedWorkspace.value);
    }
    await Promise.all([...trackedPlaces()].map(async id => { await refreshGit(id); tick(fileRefresh, id); tick(gitRefresh, id); }));
    if (layoutStatus.value === "Save failed" && !canvasConflict.value) queueLayout(layout.value);
  } catch (cause) { report(cause); }
  finally { refreshingResources.value = false; }
}
/** The main New session button starts one at once, with the client used last, as the sidebar's + does. */
const quickProviderLabel = computed(() => { const provider = lastQuickProvider(); return provider === "terminal" ? t("Terminal") : providerLabel(provider); });
function closeNewSessionMenu(event: Event) { (event.currentTarget as HTMLElement | null)?.closest("details")?.removeAttribute("open"); }
/** The command palette and the shortcut sheet, and the keys that open everything else. */
const paletteOpen = ref(false), shortcutsOpen = ref(false);
/** What the palette lists: everything, the open views (Ctrl+B w) or the sessions (Ctrl+B s). */
type PaletteMode = "all" | "views" | "sessions";
const paletteMode = ref<PaletteMode>("all");
function openPalette(mode: PaletteMode = "all") { paletteMode.value = mode; paletteOpen.value = true; }
const hint = shortcutHint;
type CanvasApi = NonNullable<typeof canvas.value>;
const onCanvas = (run: (canvas: CanvasApi) => unknown) => () => { if (!page.value && canvas.value) void run(canvas.value); };
/**
 * Every action a key can run, by the id it has in SHORTCUTS or
 * PREFIX_COMMANDS; the palette lists the same ones.
 */
const commands: Record<string, () => void> = {
  palette: () => { if (paletteOpen.value && paletteMode.value === "all") closeOverlay(() => { paletteOpen.value = false; }); else openPalette(); },
  help: () => { if (shortcutsOpen.value) closeOverlay(() => { shortcutsOpen.value = false; }); else shortcutsOpen.value = true; },
  settings: () => openSettings(),
  sidebar: toggleSidebar,
  files: () => { explorerOpen.value = !explorerOpen.value; },
  sessions: () => openPage("sessions"),
  system: () => openPage("system"),
  usage: () => openPage("usage"),
  canvas: () => void router.push({ name: "canvas" }),
  "new-session": () => { if (contextWorkspace.value) newSession(contextWorkspace.value.id); },
  "split-horizontal": onCanvas(c => c.splitSelected("horizontal")),
  "split-vertical": onCanvas(c => c.splitSelected("vertical")),
  "next-pane": onCanvas(c => c.cyclePane(1)),
  "last-pane": onCanvas(c => c.focusLast()),
  "maximize-pane": onCanvas(c => { if (selectedPaneId.value) return c.maximizePane(selectedPaneId.value); }),
  "swap-previous": onCanvas(c => c.swapSelected(-1)),
  "swap-next": onCanvas(c => c.swapSelected(1)),
  "next-layout": onCanvas(c => c.nextPreset()),
  "close-pane": onCanvas(c => c.closeSelected()),
  "next-tab": onCanvas(c => c.cycleTab(1)),
  "previous-tab": onCanvas(c => c.cycleTab(-1)),
  "rename-session": () => { if (focusedSession.value) requestRenameSession(focusedSession.value.id); },
  "new-here": onCanvas(c => c.createHere(lastQuickProvider())),
  "new-terminal": onCanvas(c => c.createHere("terminal")),
  "open-changes": () => openChanges(),
  "focus-input": onCanvas(c => c.focusSelected()),
  "choose-view": () => { if (!page.value) openPalette("views"); },
  "choose-session": () => openPalette("sessions"),
  "find-file": () => void findFile(),
  "focus-sidebar": () => void focusSidebar(),
  "focus-files": () => void focusFiles(),
};
type Direction = "left" | "right" | "up" | "down";
function runCommand(id: string) {
  if (/^focus-(left|right|up|down)$/.test(id)) onCanvas(c => c.focusDirection(id.slice(6) as Direction))();
  else if (id.startsWith("resize-")) onCanvas(c => c.nudgeSelected(id.slice(7) as Direction))();
  else if (/^tab-\d$/.test(id)) onCanvas(c => c.selectTab(Number(id.slice(4))))();
  else commands[id]?.();
}
/** Into the sidebar, shown if it was hidden: at the session in view, else the first one, else its search. */
async function focusSidebar() {
  if (sidebarRail.value) sidebarExpanded.value = true; else sidebarOpen.value = true;
  await nextTick();
  const sidebar = document.querySelector<HTMLElement>(".workspace-sidebar");
  const row = sidebar?.querySelector<HTMLElement>(".sidebar-session-row.selected [data-sidebar-row-focus]") ?? sidebar?.querySelector<HTMLElement>("[data-sidebar-row-focus]");
  (row ?? sidebar?.querySelector<HTMLElement>("input"))?.focus();
}
/** Into the file tree, shown if it was hidden: at the open file, else the first row, else the filter. */
async function focusFiles() {
  if (!workspaces.value.length) return;
  explorerOpen.value = true;
  await nextTick();
  const panel = document.querySelector<HTMLElement>(".explorer-panel");
  (panel?.querySelector<HTMLElement>(".tree-row.is-selected") ?? panel?.querySelector<HTMLElement>(".tree-row") ?? panel?.querySelector<HTMLElement>("[data-explorer-filter]"))?.focus();
}
/** The file panel's filter, opened if it was hidden; ↓ from there walks the tree. */
async function findFile() {
  if (!workspaces.value.length) return;
  explorerOpen.value = true;
  await nextTick();
  document.querySelector<HTMLInputElement>(".explorer-panel [data-explorer-filter]")?.focus();
}
const VIEW_ICONS: Record<string, string> = { agent_chat: "spark", terminal: "terminal", git_diff: "git", editor: "file", file_preview: "image" };
const viewItems = computed<PaletteItem[]>(() => openPanes.value.map(pane => {
  const session = paneSession(pane, sessions.value);
  return { id: `view:${pane.id}`, title: paneTitle(pane), subtitle: paneDetail(pane) || undefined, group: t("Open views"), provider: session && session.provider !== "terminal" ? session.provider : undefined, icon: VIEW_ICONS[pane.kind], suggested: true,
    run: () => void canvas.value?.focusPane(pane.id).then(found => { if (found) canvas.value?.focusSelected(); }) };
}));
/** Listed by the palette as well; the rest of the prefix keys are already there under their own names, or only make sense as keys. */
const PALETTE_COMMANDS = PREFIX_COMMANDS.filter(command => !command.action && !command.pages && !["files", "choose-view"].includes(command.id));
const allPaletteItems = computed(() => buildPaletteItems({ sessions: sessions.value, workspaces: workspaces.value, here: contextWorkspace.value, t }, {
  openSession, quickSession: (id, provider) => void quickSession(id, provider), newSession: id => newSession(id), openPage, openSettings, toggleSidebar, setTheme: setThemePreference,
  openCanvas: () => void router.push({ name: "canvas" }),
  openWorkspace: id => { void router.push({ name: "canvas" }); selectWorkspace(id); },
  addWorkspace: () => { showWorkspace.value = true; },
  toggleFiles: () => { explorerOpen.value = !explorerOpen.value; },
  showShortcuts: () => { shortcutsOpen.value = true; },
}).concat(page.value ? [] : PALETTE_COMMANDS.map(command => ({ id: command.id, title: t(command.label), group: t("Pane actions"), icon: "layout", hint: hint(command.id), run: () => runCommand(command.id) }))));
const paletteItems = computed(() => paletteMode.value === "views" ? viewItems.value
  : paletteMode.value === "sessions" ? allPaletteItems.value.filter(item => item.id.startsWith("session:")).map(item => ({ ...item, suggested: true }))
  : allPaletteItems.value);
const palettePlaceholder = computed(() => paletteMode.value === "views" ? t("Go to an open view…") : paletteMode.value === "sessions" ? t("Find a session…") : undefined);
/** Closing the palette or the shortcut sheet without going anywhere returns to the pane you were in. */
function closeOverlay(close: () => void) {
  close();
  void nextTick(() => { if (!document.activeElement || document.activeElement === document.body) canvas.value?.focusSelected(); });
}
function onGlobalKey(event: KeyboardEvent) {
  if (showAuth.value || event.defaultPrevented || prefixPassthrough.has(event)) return;
  // F2 on a session's tab or in its pane, outside a field, renames it as the sidebar does.
  if (isRename(event) && event.target instanceof Element && !event.target.closest("input, textarea, select, [contenteditable='true'], .xterm, .terminal-host, .workspace-sidebar, .explorer-panel") && event.target.closest(".dock-canvas") && focusedSession.value) {
    event.preventDefault(); requestRenameSession(focusedSession.value.id); return;
  }
  // Esc in the sidebar or the file panel goes back to the pane; menus there close
  // themselves first, and a field keeps it, but for the file filter Ctrl+B f opens.
  if (event.key === "Escape" && event.target instanceof Element && event.target.closest(".workspace-sidebar, .explorer-panel") && (!event.target.closest("input, textarea, select") || event.target.matches("[data-explorer-filter], [data-sidebar-search]"))) {
    event.preventDefault(); canvas.value?.focusSelected(); return;
  }
  // Find, in the sidebar or the file panel, is that panel's own search.
  if (matches(event, CHORDS.find) && event.target instanceof Element) {
    const field = event.target.closest(".workspace-sidebar, .explorer-panel")?.querySelector<HTMLInputElement>("[data-sidebar-search], [data-explorer-filter]");
    if (field) { event.preventDefault(); field.focus(); field.select(); return; }
  }
  const shortcut = shortcutFor(event);
  if (!shortcut) return;
  // Under a dialog only the chord that opened it answers, to close it again.
  if (document.querySelector('[aria-modal="true"]') && !(shortcut.id === "palette" && paletteOpen.value) && !(shortcut.id === "help" && shortcutsOpen.value)) return;
  event.preventDefault();
  runCommand(shortcut.id);
}
/**
 * Ctrl+B, then a key. A small label says the prefix is waiting; the whole map
 * of keys follows only after a pause, so keys pressed in a run never flash it.
 * After a repeatable key the prefix stays a moment for the same key again.
 */
const prefixActive = ref(false), prefixRepeating = ref(false), prefixMap = ref(false);
const prefixContext = ref<ReturnType<CanvasApi["prefixContext"]>>();
const prefixPassthrough = new WeakSet<KeyboardEvent>();
let prefixMapTimer: ReturnType<typeof setTimeout> | undefined, prefixRepeatTimer: ReturnType<typeof setTimeout> | undefined;
function cancelPrefix() {
  prefixActive.value = prefixRepeating.value = prefixMap.value = false;
  clearTimeout(prefixMapTimer); clearTimeout(prefixRepeatTimer);
}
function startPrefix() {
  cancelPrefix();
  prefixActive.value = true;
  prefixContext.value = page.value ? undefined : canvas.value?.prefixContext();
  prefixMapTimer = setTimeout(() => { prefixMap.value = true; }, 600);
}
/** Whether a key in the map would do anything here; the rest are shown dimmed. */
function prefixUsable(id: string): boolean {
  const context = prefixContext.value;
  if (PREFIX_COMMANDS.find(command => command.id === id)?.pages) return true;
  if (page.value) return false;
  if (id === "files" || id === "focus-files") return workspaces.value.length > 0;
  if (id === "find-file") return workspaces.value.length > 0;
  if (id === "open-changes") return !!contextWorkspace.value;
  if (!context) return false;
  if (["focus", "swap-previous", "swap-next"].includes(id)) return context.groups > 1;
  if (["next-pane", "last-pane"].includes(id)) return context.panes > 1;
  if (["next-tab", "previous-tab", "tab"].includes(id)) return context.tabs > 1;
  if (id === "resize") return context.splits;
  if (["maximize-pane", "close-pane", "focus-input", "choose-view", "next-layout"].includes(id)) return !!context.selected;
  return true;
}
function onPrefixKey(event: KeyboardEvent) {
  if (showAuth.value || document.querySelector('[aria-modal="true"]')) { cancelPrefix(); return; }
  const action = prefixKey(event, prefixActive.value, prefixRepeating.value);
  if (!action) return;
  if (action === "pass" || action === "cancel-pass") {
    cancelPrefix();
    prefixPassthrough.add(event);
    return;
  }
  event.preventDefault();
  event.stopPropagation();
  if (action === "ignore") return;
  if (action === "prefix") { startPrefix(); return; }
  const hit = prefixCommand(event.key), repeat = hit?.command.repeat && action !== "cancel";
  cancelPrefix();
  if (action === "cancel") return;
  // On a page only the keys that go somewhere answer; the canvas keys wait for the canvas.
  if (page.value && !hit?.command.pages) return;
  runCommand(action);
  if (repeat) { prefixActive.value = prefixRepeating.value = true; prefixRepeatTimer = setTimeout(cancelPrefix, 900); }
}
onMounted(() => {
  window.addEventListener("keydown", onPrefixKey, true);
  window.addEventListener("blur", cancelPrefix);
  window.addEventListener("pointerdown", cancelPrefix, true);
});
onUnmounted(() => {
  window.removeEventListener("keydown", onPrefixKey, true);
  window.removeEventListener("blur", cancelPrefix);
  window.removeEventListener("pointerdown", cancelPrefix, true);
  cancelPrefix();
});
onMounted(() => window.addEventListener("keydown", onGlobalKey));
onUnmounted(() => window.removeEventListener("keydown", onGlobalKey));

/**
 * Every fresh session list goes through the attention tracker, which knows
 * which sessions are waiting on you and says so: a toast, a desktop
 * notification in the background, and a count in the page title.
 */
watch(sessions, list => trackSessions(list, { focusedSessionId: selectedSessionId.value, open: openSession, t }));
const baseTitle = typeof document === "undefined" ? "Tendly" : document.title;
watch(() => waitingIds.value.length, count => { if (typeof document !== "undefined") document.title = attentionTitle(baseTitle, count); }, { immediate: true });
/**
 * A link to one session (/session/:id) opens it on the canvas once sessions
 * have loaded, and the address goes back to the canvas's.
 */
watch([() => route.name, () => route.params.id, () => sessions.value.length, canvasReady], () => {
  if (route.name !== "session" || !canvasReady.value) return;
  const session = sessions.value.find(item => item.id === route.params.id);
  if (!session) return;
  void router.replace({ name: "canvas" });
  openSession(session);
}, { immediate: true });
/** Reached from a phone, the sign-in dialog says where the token is (link-sign-in.ts). */
const authViaTailnet = ref(false), linkSignInFailed = ref(false);
async function bootstrap() {
  const own = ++bootstrapRevision;
  loading.value = true; error.value = ""; connectionError.value = "";
  try {
    linkSignInFailed.value = (await linkSignIn) === "failed";
    const auth = await request<{ required: boolean; authenticated: boolean; via_tailnet?: boolean }>("/auth");
    authViaTailnet.value = !!auth.via_tailnet;
    if (auth.required && !auth.authenticated) { showAuth.value = true; signedIn.value = false; return; }
    showAuth.value = false; signedIn.value = true;
    // The appearance is a preference too; it applies itself when it arrives (theme.ts).
    void loadPreferences(true);
    const [workspaceList, sessionList, profileList, health] = await Promise.all([
      request<Workspace[]>("/workspaces"), request<Session[]>("/sessions"),
      request<EndpointProfile[]>("/endpoint-profiles"), request<BackendHealth>("/health"),
    ]);
    if (disposed || own !== bootstrapRevision) return;
    apiOnline.value = health.ok; platform.value = health.platform; instanceLabel.value = health.instance_label; serverVersion.value = health.version;
    setBackendCapabilities(health);
    workspaces.value = workspaceList; sessions.value = sessionList; profiles.value = profileList;
    const preferred = workspaceList.find(workspace => workspace.id === selectedWorkspaceId.value || workspace.id === rememberedWorkspace()) ?? workspaceList[0];
    if (preferred) {
      selectedWorkspaceId.value = preferred.id;
      if (!canvasReady.value) await initializeCanvas(preferred);
      for (const id of trackedPlaces()) void refreshGit(id);
    }
  } catch (cause) { apiOnline.value = false; report(cause); }
  finally { if (own === bootstrapRevision) loading.value = false; }
}
function adaptDrawers() { const desktop = window.innerWidth > 1100; if (wasDesktop && !desktop) { explorerOpen.value = false; sidebarOpen.value = false; } wasDesktop = desktop; sidebarRail.value = sidebarIsRail(window.innerWidth); }
const { bodyStyle, resizeSidebar, resetSidebarWidth } = useSidebarWidth(storage, sidebarRail, sidebarCollapsed);
/** Below the rail width the same control drives the drawer, so the remembered
 * desktop preference is left untouched. */
function toggleSidebar() { if (sidebarRail.value) sidebarExpanded.value = !sidebarExpanded.value; else sidebarOpen.value = !sidebarOpen.value; }
// Persist wherever the panels change, including opening the explorer by
// revealing a file, so the shell reopens the way the user left it.
watch([sidebarExpanded, explorerOpen], ([sidebar, explorer]) => writePanelVisibility(storage, { sidebar, explorer }));
function beforeUnload(event: BeforeUnloadEvent) {
  if (hasUnsavedWork({ fileDrafts: hasDirtyDrafts(), gitDrafts: hasGitDrafts(), layoutPending: hasPendingLayout(), layoutStatus: layoutStatus.value })) { event.preventDefault(); event.returnValue = ""; }
}
/** Time a /health round-trip so the host card can show how responsive the
 * backend is. Silent on failure: the connection error is already reported by
 * the session poll; here we only drop the reading and mark the host offline. */
async function pingHost() {
  const started = performance.now();
  try { const health = await request<BackendHealth>("/health"); latencyMs.value = Math.round(performance.now() - started); apiOnline.value = health.ok; }
  catch { latencyMs.value = undefined; apiOnline.value = false; }
}
onMounted(() => {
  void bootstrap();
  void pingHost();
  pollTimer = setInterval(() => { if (!document.hidden && !showAuth.value) { void refreshSessions(); void pingHost(); for (const id of trackedPlaces()) void refreshGit(id); } }, 5000);
  window.addEventListener("beforeunload", beforeUnload); window.addEventListener("resize", adaptDrawers);
  stopPageReturn = onPageReturn(stale => fileWatch.wake(stale));
});
onUnmounted(() => { disposeCanvas(); disposed = true; clearInterval(pollTimer); fileWatch.dispose(); stopPageReturn?.(); window.removeEventListener("beforeunload", beforeUnload); window.removeEventListener("resize", adaptDrawers); });
</script>

<template>
  <div :class="['app-shell', { 'explorer-hidden': !explorerOpen || !explorerWorkspace, 'sidebar-collapsed': sidebarCollapsed, 'is-mobile': mobile }]">
    <header class="topbar">
      <div class="brand"><button v-if="mobile && backTarget" class="icon-button mobile-menu mobile-back" :aria-label="t('Back')" @click="goBack"><Icon name="chevron" :size="22" /></button><button v-else class="icon-button mobile-menu" :aria-label="t('Toggle workspace navigation')" @click="sidebarOpen = !sidebarOpen"><Icon name="menu" /></button><button class="icon-button sidebar-toggle" :class="{selected:!sidebarCollapsed}" :aria-pressed="!sidebarCollapsed" :aria-label="t(sidebarCollapsed?'Expand workspace panel':'Collapse workspace panel')" :title="t(sidebarCollapsed?'Expand workspace panel':'Collapse workspace panel') + ' · ' + hint('sidebar')" @click="toggleSidebar"><Icon name="panelLeft"/></button><span class="brand-mark"><span /></span><strong :title="serverVersion?t('Tendly {version}',{version:serverVersion}):undefined">Tendly<span v-if="instanceLabel" class="brand-version">{{ instanceLabel }}</span></strong></div>
      <span v-if="mobile && page" class="mobile-title mobile-page-title"><strong>{{ t(page === "settings" ? "Settings" : page === "sessions" ? "All sessions" : page === "usage" ? "Usage" : "System") }}</strong></span>
      <button v-else-if="mobile && canvasReady" class="mobile-title" type="button" :aria-label="t('Switch view')" @click="switcherOpen = true">
        <TabIcon v-if="focusedPane" :kind="focusedPane.kind" :metadata="focusedPane.metadata" :session-providers="sessionProviders" :size="16" />
        <span class="mobile-title-copy"><strong>{{ focusedPane ? paneTitle(focusedPane) : (contextWorkspace?.name || 'Tendly') }}</strong><small v-if="focusedPane && paneDetail(focusedPane)">{{ paneDetail(focusedPane) }}</small></span>
        <span v-if="openPanes.length > 1" class="mobile-title-count">{{ openPanes.length }}</span><Icon name="chevron" :size="14" class="mobile-title-chevron" />
      </button>
      <!-- Its own pointerdown stays here: the open card would otherwise take it as a tap outside, close, and be reopened by the click. -->
      <button v-if="mobile && focusedBranch" v-show="!page" type="button" :class="['mobile-branch', { worktree: focusedBranch.worktree }]" :disabled="focusedBranch.disabled" :aria-label="t('Branch for this session')" :title="focusedBranch.label" @pointerdown.stop @click="focusedBranch.toggle()"><svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true"><path d="M5 3v7M5 10a2 2 0 1 0 0 4 2 2 0 0 0 0-4Zm0-7a2 2 0 1 0 0-.01M11 5a2 2 0 1 0 0-.01M11 7c0 2-2 3-6 3" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg><span>{{ focusedBranch.label }}</span></button>
      <span v-if="mobile && focusedPane?.metadata?.session_id" v-show="!page" class="mobile-session-actions" :id="`session-actions-${focusedPane.id}`" />
      <div class="top-crumb"><span>{{ t('Shared workspace canvas') }}</span><Icon name="chevron" :size="13"/><strong>{{ contextWorkspace?.name || t('Your next workspace') }}</strong></div>
      <div class="top-actions">
        <span v-if="loading||!apiOnline" class="connection-badge"><i :class="['state-dot', apiOnline ? 'running' : 'stopped']"/>{{ t(loading ? 'Connecting' : apiOnline ? 'Host connected' : 'Offline') }}</span>
        <button type="button" class="topbar-search" :title="t('Search and commands') + ' · ' + hint('palette')" :aria-label="t('Search and commands')" @click="paletteOpen=true"><Icon name="search" :size="14"/><span>{{ t('Search…') }}</span><kbd>{{ hint('palette') }}</kbd></button>
        <div v-if="workspaces.length" class="new-session-split">
          <button class="primary-button new-session-top" :disabled="!contextWorkspace || quickBusy" :aria-label="t('New session')" :title="contextWorkspace ? t('New {provider} session in {workspace}', { provider: quickProviderLabel, workspace: contextWorkspace.name }) : t('New session')" @click="quickSession(contextWorkspace?.id)"><Icon :name="mobile ? 'edit' : 'plus'" :size="mobile ? 19 : 14"/><span>{{ t('New session') }}</span></button>
          <details v-if="!mobile" class="new-session-more"><summary class="primary-button" :aria-label="t('More ways to start a session')" :title="t('More ways to start a session')"><Icon name="chevron" :size="12" class="new-session-caret"/></summary>
            <nav class="new-session-menu" :aria-label="t('New session')">
              <button v-for="provider in PROVIDER_KINDS" :key="provider" type="button" :disabled="!contextWorkspace || quickBusy" @click="closeNewSessionMenu($event); quickSession(contextWorkspace?.id, provider)"><TabIcon :kind="provider === 'terminal' ? 'terminal' : 'agent_chat'" :metadata="provider === 'terminal' ? undefined : { provider }" :size="16" /><span class="new-session-label">{{ provider === 'terminal' ? t('New terminal') : t('New {provider} session', { provider: providerLabel(provider) }) }}</span></button>
              <hr/>
              <button type="button" :disabled="!contextWorkspace" @click="closeNewSessionMenu($event); newSession(contextWorkspace?.id)"><Icon name="settings" :size="15"/><span class="new-session-label">{{ t('New session with options…') }}</span><kbd>{{ hint('new-session') }}</kbd></button>
            </nav>
          </details>
        </div>
        <button class="icon-button" :aria-label="t('Settings')" :title="t('Settings') + ' · ' + hint('settings')" @click="openSettings()"><Icon name="settings"/></button>
        <button v-if="workspaces.length" :class="['icon-button',{selected:explorerOpen}]" :aria-pressed="explorerOpen" :aria-label="t(explorerOpen?'Collapse file panel':'Expand file panel')" :title="t(explorerOpen?'Collapse file panel':'Expand file panel') + ' · ' + hint('files')" @click="explorerOpen = !explorerOpen"><Icon name="panelRight"/></button>
      </div>
    </header>
    <div class="app-body" :style="bodyStyle">
      <button v-if="sidebarOpen" class="drawer-overlay sidebar-overlay" :aria-label="t('Close workspace navigation')" @click="sidebarOpen = false"/>
      <aside :class="['sidebar',{'drawer-open':sidebarOpen}]">
        <div v-if="sidebarRail&&!sidebarCollapsed" class="sidebar-resizer" role="separator" aria-orientation="vertical" :aria-label="t('Resize workspace panel')" :title="t('Drag to resize · double-click to reset')" @pointerdown.prevent="resizeSidebar" @dblclick="resetSidebarWidth"/>
        <WorkspaceSidebar ref="workspaceSidebar" @reveal-session="revealSession" :workspace-branches="workspaceBranches" @branch-switched="branchSwitched" :workspaces="workspaces" :sessions="sessions" :selected-workspace-id="selectedWorkspaceId" :selected-session-id="selectedSessionId" :history-supported="backendCapabilities.nativeHistory" :storage-key="storageKey" :archive-supported="backendCapabilities.sessionArchive" :ephemeral-supported="backendCapabilities.ephemeralSessions" :archive-busy-ids="archiveBusyIds" :keep-busy-ids="keepBusyIds" :delete-busy-ids="deleteBusyIds" :sessions-loading="sessionsLoading" @select-workspace="selectWorkspace" @open-session="openSession" @open-remote-session="openRemoteSession" @rename-session="renameSession" @archive-session="archiveSession" @archive-sessions="archiveSessions" @delete-sessions="deleteSessions" @keep-session="keepSession" @refresh-sessions="refreshSessions" @session-environment="environmentSessionId=$event;sidebarOpen=false" @open-files="openFiles" @open-changes="openChanges" @new-session="newSession" @quick-session="quickSession" @load-history="loadHistory" @add-workspace="showWorkspace=true" @canvas="sidebarOpen=false;router.push({ name: 'canvas' })" @all-sessions="openPage('sessions')" @system="openPage('system')" @usage="openPage('usage')" @machines="openSettings('machines')" @show-in-sessions="(id, archived) => openPage('sessions', { highlight: id, archive: archived ? 'archived' : 'current' })" :current-page="page ?? 'canvas'"/>
        <div v-if="!mobile" class="host-card"><span class="host-symbol"><Icon name="monitor" :size="17"/></span><div class="host-card-text"><strong>{{ t('Runs on this machine') }}</strong><small>{{ platformLabel }}</small></div><span class="host-latency" :class="hostStatus.level" :title="apiOnline ? t('Round-trip to host: {ms}', { ms: hostStatus.text }) : t('Offline')" :aria-label="apiOnline ? t('Round-trip to host: {ms}', { ms: hostStatus.text }) : t('Offline')"><i class="host-dot"/><span>{{ hostStatus.text }}</span></span></div>
      
        <div v-else class="drawer-footer">
          <div v-if="contextWorkspace" class="drawer-git"><span :title="contextCheckout ?? contextWorkspace.root_path"><Icon name="git" :size="15"/><span v-if="contextCheckout" class="status-worktree" :title="t('Works in worktree {path}', { path: contextCheckout })"><svg viewBox="0 0 16 16" width="11" height="11" aria-hidden="true"><path d="M5 3v7M5 10a2 2 0 1 0 0 4 2 2 0 0 0 0-4Zm0-7a2 2 0 1 0 0-.01M11 5a2 2 0 1 0 0-.01M11 7c0 2-2 3-6 3" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg>{{ currentGit.branch || (currentGitKnown ? t('Detached HEAD') : '') }}</span><WorkspaceBranchMenu v-else-if="currentGitAvailable" :workspace-id="contextWorkspace.id" :branch="currentGit.branch" @switched="branchSwitched(contextWorkspace.id)" @changed="branchSwitched(contextWorkspace.id)" /><template v-else-if="currentGitKnown">{{ t('Git unavailable') }}</template></span><button v-if="currentGitAvailable" type="button" @click="openChanges()">{{ t(currentGit.files.length===1?'{count} change':'{count} changes',{count:currentGit.files.length}) }}</button></div>
          <div class="drawer-host"><HostUsage v-if="signedIn&&!showAuth" :sessions="sessions" @open="openPage('system')" /><span v-if="layoutTrouble" class="danger-text">{{ t('Layout {status}',{status:t(layoutStatus)}) }}</span><button class="icon-button" type="button" :aria-label="t('Settings')" :title="t('Settings') + ' · ' + hint('settings')" @click="openSettings()"><Icon name="settings" :size="19"/></button></div>
        </div>
      </aside>
      <main class="main-workspace">
        <div v-if="visibleError" class="app-error" role="alert"><span>{{ visibleError }}</span><button class="text-button" :disabled="refreshingResources||loading" @click="canvasReady?refreshResources():bootstrap()">{{ t('Retry') }}</button><button class="icon-button" :aria-label="t('Dismiss error')" @click="error='';connectionError=''"><Icon name="close" :size="14"/></button></div>
        <div v-if="canvasReady && !backendCapabilities.sharedCanvas" class="canvas-compat-notice" role="status"><Icon name="info" :size="15"/><span>{{ t('This backend is older. The shared canvas is saved in this browser; running sessions and existing workspace layouts are unchanged.') }}<small>{{ t('Native configuration import and history loading require an updated backend.') }}</small></span></div>
        <div v-if="discardPrompt" class="confirmation-bar" role="alert">{{ t('“{session}” is a temporary window and is still working. Closing it discards the session and stops it.',{session:discardPrompt.session.title}) }}<button class="small-button" @click="discardPrompt=undefined">{{ t('Keep it open') }}</button><button class="small-button danger" @click="confirmDiscard">{{ t('Close and discard') }}</button></div>
        <div v-if="canvasConflict" class="confirmation-bar" role="alert">{{ t('Another page updated the shared canvas. Your current layout is kept locally.') }}<button class="small-button" @click="resolveCanvasConflict(false)">{{ t('Load server layout') }}</button><button class="small-button danger" @click="resolveCanvasConflict(true)">{{ t('Save my current layout instead') }}</button></div>
        <div v-if="localRecovery" class="canvas-compat-notice"><span>{{ t('A local recovery layout is available.') }}</span><button class="text-button" @click="restoreLocalRecovery">{{ t('Restore local layout') }}</button><button class="icon-button" :aria-label="t('Dismiss error')" @click="localRecovery=undefined"><Icon name="close" :size="13"/></button></div>
        <template v-if="workspaces.length">
          <Canvas v-if="canvasReady" ref="canvas" v-model="layout" :compact="mobile" :selected-pane-id="selectedPaneId" :default-workspace-id="contextWorkspace?.id" :accept-pane="acceptDroppedPane" :confirm-close-pane="confirmClosePane" :ephemeral-session-ids="ephemeralIds" :ephemeral-supported="backendCapabilities.ephemeralSessions" @create-session="createSessionInPane" :workspace-labels="Object.fromEntries(workspaces.map(workspace=>[workspace.id,workspace.name]))" :mixed-workspaces="new Set(flattenPanes(layout.root).map(pane=>paneString(pane,'workspace_id')).filter(Boolean)).size>1" :session-branches="Object.fromEntries(sessions.flatMap(session=>session.checkout_path&&session.checkout_branch?[[session.id,session.checkout_branch]]:[]))" :checkout-branches="Object.fromEntries(sessions.flatMap(session=>session.checkout_path&&session.checkout_branch?[[session.checkout_path,session.checkout_branch]]:[]))" :workspace-branches="workspaceBranches" :session-providers="sessionProviders" @select-pane="selectPane" @open-pane="sessionPaneDropped">
            <template #pane="{ pane }"><WorkspacePane :key="pane.id" :pane="pane" :workspaces="workspaces" :sessions="sessions" :profiles="profiles" :git-refresh="gitRefresh" @session-changed="refreshSessions" @new-session="(id,provider)=>newSession(id??selectedWorkspaceId,provider)" @open-session="openSession" @open-remote-session="openRemoteSession" @reveal-session="revealSession" @keep-session="keepSessionById" @rename-request="requestRenameSession" @session-environment="environmentSessionId=$event" @structured-session="enableChat" @profiles="openSettings('endpoints')" @git-changed="gitChanged" @open-file="openFile" @open-reference="openReference" @files-saved="filesSaved" @browse="id=>openFiles(id??selectedWorkspaceId)" @draft-created="draftCreated" @close-draft="id=>canvas?.closePane(id)" @leave-terminal="id=>canvas?.closePane(id)"/></template>
          </Canvas>
          <div v-else class="pane-empty"><p>{{ t('Opening workspace…') }}</p></div>
        </template>
        <section v-else class="welcome-screen"><div class="welcome-composition" aria-hidden="true"><span class="welcome-tile"><Icon name="spark" :size="35"/></span><span class="welcome-tile"><Icon name="git" :size="26"/><Icon name="file" :size="26"/></span></div><div class="eyebrow">{{ t('A LITTLE SPACE. ENDLESS POSSIBILITIES.') }}</div><h1>{{ t('Your agents.') }}<br/>{{ t('Your workspace.') }}</h1><p>{{ t('Claude Code, Codex, files and Git — together on your own machine. Arrange everything the way you think.') }}</p><button class="primary-button" :disabled="loading" @click="showWorkspace=true"><Icon name="plus" :size="17"/>{{ t(loading?'Connecting to host…':'Add a workspace') }}</button><small>{{ t('Rust + Vue · host native · browser first') }}</small></section>
        <SettingsPage v-if="page==='settings'" :profiles="profiles" :section="settingsRouteSection" @back="leavePage" @navigate="section => router.replace({ name: 'settings', params: { section } })" @changed="refreshProfiles"/>
        <SessionsPage v-else-if="page==='sessions'" :workspaces="workspaces" :sessions="sessions" :selected-session-id="selectedSessionId" :archive-supported="backendCapabilities.sessionArchive" :archive-busy-ids="archiveBusyIds" :keep-busy-ids="keepBusyIds" :delete-busy-ids="deleteBusyIds" :sessions-loading="sessionsLoading" :initial-archive="route.query.archive === 'archived' ? 'archived' : 'current'" :highlight-id="typeof route.query.highlight === 'string' ? route.query.highlight : undefined" @back="leavePage" @open-session="openSession" @rename-request="requestRenameSession" @session-environment="environmentSessionId=$event" @archive-session="archiveSession" @archive-sessions="archiveSessions" @delete-sessions="deleteSessions" @keep-session="keepSession" @refresh-sessions="refreshSessions"/>
        <SystemPage v-else-if="page==='system'" :sessions="sessions" @back="leavePage" @open-session="openSession"/>
        <UsagePage v-else-if="page==='usage'" :sessions="sessions" :workspaces="workspaces" @back="leavePage" @open-session="openSession"/>
      </main>
      <button v-if="explorerOpen && explorerWorkspace" class="drawer-overlay explorer-overlay" :aria-label="t('Close file explorer')" @click="explorerOpen=false"/>
      <aside v-if="explorerOpen && explorerWorkspace" class="explorer-panel">
        <div class="explorer-workspace-control"><label><span>{{ t('Explorer workspace') }}</span><select :value="explorerWorkspaceId" :aria-label="t('Explorer workspace')" @change="explorerWorkspaceId=($event.target as HTMLSelectElement).value;explorerCheckout=null;explorerPinned=true"><option v-for="workspace in workspaces" :key="workspace.id" :value="workspace.id">{{ workspace.name }}</option></select></label><button class="small-button" :class="{primary:explorerPinned}" :aria-pressed="explorerPinned" :title="t(explorerPinned?'Unpin explorer and follow the focused pane':'Pin explorer to this workspace')" @click="explorerPinned=!explorerPinned;!explorerPinned&&contextWorkspace&&followWithExplorer(contextWorkspace.id,contextCheckout)">{{ t(explorerPinned?'Pinned':'Follow focus') }}</button></div>
        <p v-if="explorerCheckout" class="explorer-checkout" :title="t('Works in worktree {path}', { path: explorerCheckout })"><svg viewBox="0 0 16 16" width="11" height="11" aria-hidden="true"><path d="M5 3v7M5 10a2 2 0 1 0 0 4 2 2 0 0 0 0-4Zm0-7a2 2 0 1 0 0-.01M11 5a2 2 0 1 0 0-.01M11 7c0 2-2 3-6 3" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg><span>{{ gitStatuses[explorerPlace]?.branch || explorerCheckout.split(/[\\/]/).pop() }}</span><small>{{ t('Own directory') }}</small></p>
        <FileExplorer ref="explorer" :workspace-id="explorerWorkspace.id" :checkout="explorerCheckout" :refresh-token="fileRefresh[explorerPlace]??0" :selected-path="activeFilePath" @open="openFile(explorerWorkspace.id,$event,explorerCheckout)" @moved="(from,to)=>fileMoved(explorerWorkspace!.id,explorerCheckout,from,to)" @close="explorerOpen=false"/>
      </aside>
    </div>
    <footer v-if="!mobile" class="statusbar"><span :title="contextCheckout ?? contextWorkspace?.root_path"><Icon name="git" :size="13"/>{{ contextWorkspace?.name || t('No workspace') }} · <span v-if="contextCheckout" class="status-worktree" :title="t('This session works in its own directory, {path}. The workspace directory\'s branch is in the sidebar.', { path: contextCheckout })"><svg viewBox="0 0 16 16" width="11" height="11" aria-hidden="true"><path d="M5 3v7M5 10a2 2 0 1 0 0 4 2 2 0 0 0 0-4Zm0-7a2 2 0 1 0 0-.01M11 5a2 2 0 1 0 0-.01M11 7c0 2-2 3-6 3" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg>{{ currentGit.branch || (currentGitKnown ? t('Detached HEAD') : '') }}</span><WorkspaceBranchMenu v-else-if="contextWorkspace&&currentGitAvailable" :workspace-id="contextWorkspace.id" :branch="currentGit.branch" @switched="branchSwitched(contextWorkspace.id)" @changed="branchSwitched(contextWorkspace.id)" /><template v-else-if="currentGitKnown">{{ t('Git unavailable') }}</template></span><button v-if="contextWorkspace" @click="openChanges()">{{ currentGitAvailable?t(currentGit.files.length===1?'{count} change':'{count} changes',{count:currentGit.files.length}):t('Changes') }}</button><button v-if="contextWorkspace&&currentGit.ahead!==undefined" type="button" :class="['git-count',{due:currentGit.ahead}]" :title="t('{count} commits to push · open Changes',{count:currentGit.ahead})" @click="openChanges()">↑ {{ currentGit.ahead }}</button><button v-if="contextWorkspace&&currentGit.behind!==undefined" type="button" :class="['git-count',{due:currentGit.behind}]" :title="t('{count} commits to pull · open Changes',{count:currentGit.behind})" @click="openChanges()">↓ {{ currentGit.behind }}</button><span class="flex-spacer"/><span v-if="canvasReady" :class="{'danger-text':layoutTrouble}">{{ t('Layout {status}',{status:t(layoutStatus)}) }}</span><HostUsage v-if="signedIn&&!showAuth" :sessions="sessions" @open="openPage('system')" /><span class="status-agent-count"><i :class="['state-dot',activeSessions.length?'running':'stopped']"/>{{ t('{count} active sessions',{count:activeSessions.length}) }}</span></footer>
  </div>
  <WorkspaceDialog v-if="showWorkspace" :initial-path="workspaceInitialPath" @close="showWorkspace=false;workspaceInitialPath=undefined" @created="workspaceCreated"/>
  <HostFilePreview v-if="hostPreview" :key="hostPreview.path" :path="hostPreview.path" :line="hostPreview.line" @close="hostPreview=undefined" @add-workspace="folder=>{hostPreview=undefined;workspaceInitialPath=folder;showWorkspace=true}"/>
  <ImageLightbox />
  <ToastHost />
  <div v-if="prefixActive" class="prefix-hint">
    <div v-if="prefixMap && !prefixRepeating" class="prefix-map" aria-hidden="true">
      <section v-for="group in PREFIX_GROUPS" :key="group">
        <h4>{{ t(group) }}</h4>
        <div v-for="command in PREFIX_COMMANDS.filter(item => item.group === group)" :key="command.id" :class="['prefix-row', { off: !prefixUsable(command.id) }]"><kbd>{{ command.display ?? command.keys.join(' ') }}</kbd><span>{{ t(command.label) }}</span></div>
      </section>
    </div>
    <div class="prefix-pill" role="status"><kbd>Ctrl+B</kbd><span>{{ prefixRepeating ? t('Press H J K L again to keep moving, any other key to stop') : t('Waiting for a key…') }}</span><small v-if="!prefixRepeating">{{ t('Esc to cancel · Ctrl+B again to pass through') }}</small></div>
  </div>
  <CommandPalette v-if="paletteOpen" :key="paletteMode" :items="paletteItems" :placeholder="palettePlaceholder" @close="closeOverlay(() => { paletteOpen = false; })" />
  <ShortcutsDialog v-if="shortcutsOpen" @close="closeOverlay(() => { shortcutsOpen = false; })" />
  <AgentActivity :enabled="!showAuth && apiOnline" @changed="agentChanged" @show="agentShow" @canvas="agentCanvas" />
  <BottomSheet v-if="switcherOpen" :title="t('Open views')" @close="switcherOpen=false">
    <div class="switcher-list">
      <div v-for="pane in openPanes" :key="pane.id" :class="['switcher-item', { current: pane.id === focusedPane?.id }]">
        <button type="button" class="sheet-row" :class="{ current: pane.id === focusedPane?.id }" @click="switchPane(pane)">
          <TabIcon :kind="pane.kind" :metadata="pane.metadata" :session-providers="sessionProviders" :size="18" />
          <span class="sheet-row-copy"><strong>{{ paneTitle(pane) }}</strong><small v-if="paneDetail(pane)">{{ paneDetail(pane) }}</small></span>
          <i v-if="paneSession(pane, sessions)" :class="['state-dot', paneSession(pane, sessions)!.status]" />
        </button>
        <button type="button" class="icon-button switcher-close" :aria-label="t('Close {title} pane', { title: paneTitle(pane) })" @click="closeOpenPane(pane)"><Icon name="close" :size="17" /></button>
      </div>
    </div>
    <div class="sheet-section">{{ t('New') }}</div>
    <button v-for="client in AGENT_CLIENTS" :key="client" type="button" class="sheet-row" :disabled="quickBusy" @click="sheetAction(() => quickSession(contextWorkspace?.id, client))"><TabIcon kind="agent_chat" :metadata="{ provider: client }" :size="18" /><span class="sheet-row-copy"><strong>{{ t('New {provider} session', { provider: providerLabel(client) }) }}</strong></span></button>
    <button type="button" class="sheet-row" :disabled="quickBusy" @click="sheetAction(() => quickSession(contextWorkspace?.id, 'terminal'))"><TabIcon kind="terminal" :size="18" /><span class="sheet-row-copy"><strong>{{ t('New terminal') }}</strong></span></button>
    <button type="button" class="sheet-row" @click="sheetAction(() => newSession(contextWorkspace?.id))"><Icon name="settings" :size="20" /><span class="sheet-row-copy"><strong>{{ t('New session with options…') }}</strong></span></button>
    <template v-if="contextWorkspace">
      <hr class="sheet-divider" />
      <button type="button" class="sheet-row" @click="sheetAction(() => openFiles(contextWorkspace!.id, contextCheckout))"><Icon name="folder" :size="20" /><span class="sheet-row-copy"><strong>{{ t('Files') }}</strong><small>{{ contextWorkspace.name }}</small></span></button>
      <button type="button" class="sheet-row" @click="sheetAction(() => openChanges())"><Icon name="git" :size="20" /><span class="sheet-row-copy"><strong>{{ t('Git Changes') }}</strong><small v-if="currentGitAvailable">{{ t(currentGit.files.length===1?'{count} change':'{count} changes',{count:currentGit.files.length}) }}</small></span></button>
    </template>
  </BottomSheet>
  <SessionRunInfo v-if="runInfoSession" :session="runInfoSession" @close="runInfoSessionId=undefined"/>
  <SessionEnvironmentDialog v-if="environmentSession" :key="environmentSession.id" :session="environmentSession" @close="environmentSessionId=undefined" @saved="sessionEnvironmentSaved"/>
  <SessionRenameDialog v-if="renameSessionTarget" :key="renameSessionTarget.id" :session="renameSessionTarget" @close="renameSessionId=undefined" @save="saveRenameSession"/>
  <CreateSessionDialog v-if="newSessionWorkspace" :key="newSessionWorkspace.id" :workspace="newSessionWorkspace" :profiles="profiles" :initial-provider="newSessionTarget?.provider" :hidden="route.name === 'settings'" @close="newSessionTarget=undefined" @created="newSessionCreated" @profiles="openSettings('endpoints')" @workspace="worktreeWorkspace"/>
  <LoadHistoryDialog v-if="historyWorkspace && backendCapabilities.nativeHistory" :workspace="historyWorkspace" @close="historyWorkspaceId=undefined" @loaded="historyLoaded"/>
  <AuthDialog v-if="showAuth" :via-tailnet="authViaTailnet" :link-failed="linkSignInFailed" @authenticated="bootstrap"/>
</template>

<style scoped>
/* The prefix: a quiet label over the status bar, and after a pause the map of
   keys above it, four columns of key and action. */
.prefix-hint{position:fixed;bottom:36px;left:50%;transform:translateX(-50%);z-index:160;display:flex;flex-direction:column;align-items:center;gap:8px;width:max-content;max-width:calc(100vw - 32px);pointer-events:none}
.prefix-pill{display:flex;align-items:center;gap:8px;padding:5px 12px 5px 6px;border:1px solid var(--line);border-radius:var(--radius-full);background:var(--elevated);box-shadow:var(--shadow-md);font-size:var(--text-sm);color:var(--ink);animation:prefix-in var(--duration-fast) var(--ease-out)}
.prefix-pill>kbd{padding:2px 8px;border-radius:var(--radius-full);background:var(--accent);font:600 var(--text-xs) var(--font-body);color:var(--on-accent)}
.prefix-pill>small{padding-left:8px;border-left:1px solid var(--border);font-size:var(--text-xs);color:var(--muted)}
.prefix-map{display:grid;grid-template-columns:repeat(6,max-content);align-items:start;gap:14px 30px;padding:16px 22px 18px;border:1px solid var(--line);border-radius:var(--radius-xl);background:var(--elevated);box-shadow:var(--shadow-xl);animation:prefix-in var(--duration-normal) var(--ease-out)}
.prefix-map>section{display:grid;grid-template-columns:max-content max-content;align-items:center;gap:7px 10px}
.prefix-map h4{grid-column:1/-1;margin:0;font-size:var(--text-xs);font-weight:600;letter-spacing:.02em;color:var(--muted)}
.prefix-row{display:contents;font-size:var(--text-sm);color:var(--ink);white-space:nowrap}
.prefix-row>span{font-size:var(--text-sm);color:var(--ink)}
.prefix-row>kbd{justify-self:start;padding:1px 6px;border:1px solid var(--line);border-bottom-width:2px;border-radius:var(--radius-xs);background:var(--sunken);font:500 var(--text-xs) var(--font-mono);color:var(--ink-soft)}
.prefix-row.off>*{opacity:.38}
@keyframes prefix-in{from{opacity:0;transform:translateY(4px)}}
@media(max-width:1320px){.prefix-map{grid-template-columns:repeat(3,max-content)}}
@media(max-width:760px){.prefix-map{grid-template-columns:repeat(2,max-content)}}
.new-session-split{position:relative;display:inline-flex}.new-session-split>.new-session-top{border-top-right-radius:0;border-bottom-right-radius:0}
.new-session-more>summary{list-style:none;min-height:30px;padding:0 7px;border-top-left-radius:0;border-bottom-left-radius:0;border-left:1px solid color-mix(in srgb, var(--on-accent) 25%, transparent);cursor:pointer}.new-session-more>summary::-webkit-details-marker{display:none}.new-session-caret{transform:rotate(90deg)}
.new-session-menu{position:absolute;right:0;top:calc(100% + 6px);z-index:60;min-width:240px;padding:5px;border-radius:var(--radius-lg);background:var(--surface);box-shadow:var(--shadow-lg),0 0 0 1px var(--border),inset 0 1px 0 var(--card-highlight)}
.new-session-menu>button{display:flex;align-items:center;gap:10px;width:100%;min-height:36px;padding:0 10px;border:0;border-radius:var(--radius-md);background:none;color:var(--ink);font-size:var(--text-sm);text-align:left;cursor:pointer}.new-session-menu>button:hover:not(:disabled){background:var(--fill)}.new-session-menu .new-session-label{flex:1;white-space:nowrap}
.new-session-menu kbd{font:var(--text-xs) var(--font-body);color:var(--muted)}.new-session-menu hr{border:0;border-top:1px solid var(--border);margin:4px 2px}
.topbar-search{display:inline-flex;align-items:center;gap:8px;min-width:200px;height:32px;padding:0 6px 0 10px;border:1px solid var(--line);border-radius:var(--radius-md);background:color-mix(in srgb, var(--surface) 70%, transparent);color:var(--muted);font-size:var(--text-sm);cursor:pointer;transition:border-color var(--duration-fast),background var(--duration-fast)}.topbar-search:hover{border-color:var(--line-strong);background:var(--surface);color:var(--ink-soft)}.topbar-search>span{flex:1;text-align:left}.topbar-search kbd{padding:0 5px;border:1px solid var(--line);border-radius:var(--radius-xs);background:var(--surface);font:var(--text-xs) var(--font-body);color:var(--muted)}
@media(max-width:1100px){.topbar-search{min-width:0}.topbar-search>span,.topbar-search kbd{display:none}.topbar-search{width:32px;justify-content:center;padding:0}}
.language-select{color:var(--ink-soft);border:1px solid var(--line);border-radius:var(--radius-sm);background:var(--sunken);font-size:var(--text-xs);padding:5px 3px 5px 7px;min-height:28px;cursor:pointer}
.new-session-top{padding:5px 9px;font-size:var(--text-xs);min-height:30px}.top-actions{gap:9px}.main-workspace{padding:10px 12px 0}.canvas-compat-notice>span{flex:1;min-width:0}
.canvas-compat-notice{display:flex;align-items:center;gap:8px;flex:none;padding:8px 11px;margin-bottom:10px;background:var(--ok-soft);border:1px solid var(--accent-line);border-radius:var(--radius-sm);font-size:var(--text-xs);line-height:17px;color:var(--ink-soft)}.canvas-compat-notice>svg{flex:none}.canvas-compat-notice small{display:block;color:var(--muted);font-size:var(--text-xs)}
/* A session's own worktree, where the status bar and the file panel follow it: named, not switchable from here. */
.status-worktree,.explorer-checkout{display:inline-flex;align-items:center;gap:4px;min-width:0;color:var(--accent-ink)}
.status-worktree{padding:1px 6px}
.explorer-checkout{flex:none;margin:0;padding:6px 12px;border-bottom:1px solid var(--glass-edge);font-size:var(--text-xs)}
.explorer-checkout>span{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.explorer-checkout small{flex:none;margin-left:auto;color:var(--muted);white-space:nowrap}
.explorer-workspace-control{display:flex;align-items:end;gap:6px;padding:10px;border-bottom:1px solid var(--glass-edge);background:transparent;flex:none}.explorer-workspace-control label{min-width:0;flex:1;font-size:var(--text-xs);color:var(--faint)}.explorer-workspace-control label>span{display:block;margin-bottom:4px}.explorer-workspace-control select{width:100%;font-size:var(--text-xs);padding:5px;border:1px solid var(--line);border-radius:var(--radius-sm);background:var(--surface);color:var(--ink-soft)}.explorer-workspace-control button{white-space:nowrap;font-size:var(--text-xs)}
@media(max-width:700px){.canvas-compat-notice{font-size:var(--text-xs)}.canvas-compat-notice small{display:none}.workspace-heading h1{font-size:var(--text-xl)}.workspace-heading .branch-badge{display:none}}
@media(max-width:380px){.language-select{font-size:var(--text-xs);width:60px;padding-left:4px}}
@media(max-width:1100px){.top-crumb>span,.top-crumb>svg{display:none}.connection-badge{display:none}}
@media(max-width:700px){.main-workspace{padding:6px 6px 0}.new-session-top>span{display:none}.new-session-top{width:30px;padding:5px}.workspace-refresh-top{display:none}.top-actions{gap:5px}}
</style>
