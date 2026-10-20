<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import type { ComponentPublicInstance } from "vue";
import type { FileEntry } from "@tendly/protocol";
import { errorMessage, formatBytes, request, workspacePath } from "./api";
import { flattenTree, isTreeChild, sortTreeEntries, treeAncestors, treeKeyAction, treeParent } from "./tree-model";
import type { TreeDirectory, TreeRow } from "./tree-model";
import { readTreeViewState, saveTreeViewState } from "./tree-state";
import { createFileSearch, highlight } from "./file-search";
import { nameTaken, newFilePath, renamedPath } from "./file-actions";
import { directoriesToReload, onFilesChanged } from "./file-watch";
import type { SearchResults } from "./file-search";
import Icon from "./Icon.vue";
import ContextMenu from "./ContextMenu.vue";
import { filePane } from "./pane-context";
import { copyText as writeClipboard } from "./clipboard";
import { useI18n } from "../i18n";

const { t } = useI18n();
const props = defineProps<{ workspaceId: string; refreshToken?: number; selectedPath?: string }>();
const emit = defineEmits<{ open: [file: FileEntry]; close: [] }>();
const directories = ref(new Map<string, TreeDirectory>());
const expanded = ref(new Set<string>());
const query = ref("");
const selected = ref(props.selectedPath ?? "");
const focusedPath = ref("");
const revealFailure = ref("");
const error = ref("");
/**
 * Tree context menu.
 *
 * Copying a path is the thing people came to the tree for most often and had
 * to do by hand; deleting was not possible here at all.
 *
 * The empty space below the rows opens it too, with no entry to act on: asking
 * a folder's blank area for a new file is how file managers have always worked,
 * and a tree with nothing in it yet has no row to ask instead.
 */
const rowMenu = ref<{ entry: FileEntry | null; x: number; y: number } | null>(null);
const copied = ref("");
const deleting = ref(false);
const confirmDelete = ref(false);
function openRowMenu(event: MouseEvent, entry: FileEntry | null = null) {
  event.preventDefault(); event.stopPropagation();
  confirmDelete.value = false; copied.value = "";
  rowMenu.value = { entry, x: event.clientX, y: event.clientY };
}
/**
 * The same menu, from a finger.
 *
 * A phone has no right button, and a long press is what it has instead. Chrome
 * on Android raises `contextmenu` for one; Safari does not, so the press is
 * timed here rather than waited for.
 */
let pressTimer: ReturnType<typeof setTimeout> | undefined;
let pressOrigin: { x: number; y: number } | undefined;
function pressStart(event: PointerEvent, entry: FileEntry | null = null) {
  if (event.pointerType !== "touch") return;
  pressOrigin = { x: event.clientX, y: event.clientY };
  clearTimeout(pressTimer);
  pressTimer = setTimeout(() => {
    pressOrigin = undefined;
    confirmDelete.value = false; copied.value = "";
    rowMenu.value = { entry, x: event.clientX, y: event.clientY };
  }, 500);
}
/** A press that travels is a scroll, and a scroll is not a menu. */
function pressMove(event: PointerEvent) {
  if (!pressOrigin) return;
  if (Math.abs(event.clientX - pressOrigin.x) > 10 || Math.abs(event.clientY - pressOrigin.y) > 10) pressEnd();
}
function pressEnd() { clearTimeout(pressTimer); pressTimer = undefined; pressOrigin = undefined; }
function closeRowMenu() { rowMenu.value = null; confirmDelete.value = false; }
/** The shared copy falls back to the browser's own copy command where the
 * Clipboard API is refused (a plain-http page, a denied permission); if that
 * fails too, the failure says so rather than silently doing nothing. */
async function copyText(value: string, label: string) {
  try {
    await writeClipboard(value);
    copied.value = label;
    setTimeout(() => { if (copied.value === label) copied.value = ""; }, 1400);
  } catch { error.value = t("Could not copy to the clipboard in this browser."); closeRowMenu(); }
}
async function deleteEntry() {
  const entry = rowMenu.value?.entry; if (!entry) return;
  deleting.value = true;
  try {
    await request(workspacePath(props.workspaceId) + "/file" + "?path=" + encodeURIComponent(entry.path), { method: "DELETE", headers: { "X-Tendly-Client": "web" } });
    closeRowMenu();
    await loadDirectory(treeParent(entry.path) ?? "", true);
  } catch (cause) { error.value = errorMessage(cause); }
  finally { deleting.value = false; }
}
/**
 * Adding a file, and renaming one.
 *
 * Both are edits to the tree itself rather than to a file's contents, so they
 * happen in the tree: a new name is typed where the row will be, and a rename
 * is typed over the row it renames. Neither can overwrite anything -- the host
 * refuses a name that is taken -- so the only way to lose work here is still
 * the delete below, which asks first.
 */
const creating = ref<{ base: string; name: string; busy: boolean; directory: boolean } | null>(null);
const renaming = ref<{ entry: FileEntry; name: string; busy: boolean } | null>(null);
const createInput = ref<HTMLInputElement>();
let renameInput: HTMLInputElement | undefined;
/** Where a new file lands: the folder in hand, or the one holding the file in
 * hand, so "new file" means new here rather than new at the root. */
function baseDirectory() {
  const path = selected.value || focusedPath.value;
  if (!path) return "";
  const entry = directories.value.get(treeParent(path) ?? "")?.entries.find(file => file.path === path);
  return entry?.kind === "directory" ? entry.path : treeParent(path) ?? "";
}
async function startCreate(base?: string, directory = false) {
  error.value = ""; renaming.value = null; closeRowMenu();
  creating.value = { base: base ?? baseDirectory(), name: "", busy: false, directory };
  await nextTick();
  createInput.value?.focus();
}
async function createFile() {
  const draft = creating.value;
  if (!draft || draft.busy) return;
  // A path is as welcome as a name: typing `docs/notes.md` makes the folder too.
  const path = newFilePath(draft.base, draft.name);
  if (!path) return;
  if (nameTaken(directories.value, path)) { error.value = t("{name} already exists in this folder.", { name: draft.name.trim() }); return; }
  draft.busy = true;
  try {
    const entry = await request<FileEntry>(workspacePath(props.workspaceId) + "/file/create", { method: "POST", body: JSON.stringify(draft.directory ? { path, directory: true } : { path }) });
    creating.value = null;
    await loadDirectory(treeParent(entry.path) ?? "", true);
    await reveal(entry.path);
    // A file made to be written in opens to be written in; a folder is only shown.
    if (!draft.directory) emit("open", entry);
  } catch (cause) { error.value = errorMessage(cause); draft.busy = false; }
}
async function startRename(entry: FileEntry) {
  error.value = ""; creating.value = null; closeRowMenu();
  renaming.value = { entry, name: entry.name, busy: false };
  await nextTick();
  renameInput?.focus();
  // The extension is rarely what is being changed, so it starts unselected.
  const stem = entry.name.lastIndexOf(".");
  renameInput?.setSelectionRange(0, stem > 0 ? stem : entry.name.length);
}
async function renameEntry() {
  const draft = renaming.value;
  if (!draft || draft.busy) return;
  const to = renamedPath(draft.entry.path, draft.name);
  if (!to) return;
  if (to === draft.entry.path) { renaming.value = null; return; }
  const parent = treeParent(draft.entry.path) ?? "";
  if (nameTaken(directories.value, to)) { error.value = t("{name} already exists in this folder.", { name: draft.name.trim() }); return; }
  draft.busy = true;
  try {
    const entry = await request<FileEntry>(workspacePath(props.workspaceId) + "/file/rename", { method: "POST", body: JSON.stringify({ from: draft.entry.path, to }) });
    renaming.value = null;
    // The row is gone under its old name; the tree follows it to the new one.
    if (selected.value === draft.entry.path) selected.value = entry.path;
    // An open folder stays open under its new name, which means loading it
    // there: its children were listed under a path that no longer exists.
    if (expanded.value.delete(draft.entry.path)) { expanded.value.add(entry.path); await loadDirectory(entry.path, true); }
    await loadDirectory(parent, true);
    await reveal(entry.path);
  } catch (cause) { error.value = errorMessage(cause); draft.busy = false; }
}
const refreshing = ref(false);
const treeElement = ref<HTMLElement>();
const rowElements = new Map<string, HTMLElement>();
let generation = 0, revealRevision = 0;
let pending = new Map<string, Promise<boolean>>();
let restorePaths: string[] = [];

/**
 * Workspace-wide results, when the host can provide them.
 *
 * The tree filter only ever saw directories already loaded, so a file two
 * unopened folders down simply did not exist to it. The index covers the whole
 * workspace and honours .gitignore, so build output and dependencies stay out.
 *
 * `undefined` means there is nothing workspace-wide to show — no query, the
 * host cannot search, or the query failed — and the tree filter stands in.
 */
const workspaceResults = ref<SearchResults | undefined>();
const fileSearch = createFileSearch();
watch([query, () => props.workspaceId], ([term, workspace]) => {
  fileSearch.search(workspace, term, results => { workspaceResults.value = results; });
});
onBeforeUnmount(() => fileSearch.cancel());
const searchingWorkspace = computed(() => !!workspaceResults.value?.available && !!query.value.trim());
const searchHits = computed(() => workspaceResults.value?.files ?? []);
function hitParts(hit: { path: string; name: string; indices: number[] }) { return highlight(hit.path, hit.name, hit.indices); }
const hitDirectory = treeParent;
function openHit(hit: { path: string; name: string; kind: "file" | "directory" }) {
  if (hit.kind === "directory") { void reveal(hit.path); query.value = ""; return; }
  selected.value = hit.path;
  emit("open", { path: hit.path, name: hit.name, kind: "file" } as FileEntry);
}

const rows = computed(() => flattenTree(directories.value, expanded.value, query.value));
const root = computed(() => directories.value.get(""));
const loading = computed(() => refreshing.value || [...directories.value.values()].some(directory => directory.loading));
const loadedCount = computed(() => flattenTree(directories.value, new Set(directories.value.keys())).length);
const tabPath = computed(() => rows.value.find(row => row.entry.path === focusedPath.value)?.entry.path ?? rows.value.find(row => row.entry.path === selected.value)?.entry.path ?? rows.value[0]?.entry.path);
const crumbs = computed(() => {
  if (!selected.value) return [];
  const parts = selected.value.split("/");
  return parts.map((name, index) => ({ name, path: parts.slice(0, index + 1).join("/") }));
});

function directory(path: string): TreeDirectory {
  if (!directories.value.has(path)) directories.value.set(path, { entries: [], loaded: false, loading: false, error: "" });
  return directories.value.get(path)!;
}

function loadDirectory(path: string, force = false): Promise<boolean> {
  if (pending.has(path)) return pending.get(path)!;
  const state = directory(path);
  if (state.loaded && !force) return Promise.resolve(true);
  const requestGeneration = generation;
  state.loading = true;
  state.error = "";
  const operation = request<FileEntry[]>(`${workspacePath(props.workspaceId)}/files?path=${encodeURIComponent(path)}`)
    .then(entries => {
      if (generation !== requestGeneration) return false;
      state.entries = sortTreeEntries(entries.filter(entry => isTreeChild(path, entry.path)));
      state.loaded = true;
      return true;
    })
    .catch(cause => {
      if (generation === requestGeneration) state.error = errorMessage(cause);
      return false;
    })
    .finally(() => {
      if (generation === requestGeneration) { state.loading = false; pending.delete(path); }
    });
  pending.set(path, operation);
  return operation;
}

function setRowRef(path: string, element: Element | ComponentPublicInstance | null) {
  if (element instanceof HTMLElement) rowElements.set(path, element);
  else rowElements.delete(path);
}

async function focusRow(path: string) {
  focusedPath.value = path;
  await nextTick();
  const element = rowElements.get(path);
  element?.scrollIntoView({ block: "nearest", inline: "nearest" });
  element?.focus({ preventScroll: true });
}

async function toggle(file: FileEntry) {
  if (file.kind !== "directory") return;
  const wasExpanded = rows.value.find(row => row.entry.path === file.path)?.expanded ?? expanded.value.has(file.path);
  clearFilterKeepingPath(file.path);
  if (wasExpanded) expanded.value.delete(file.path);
  else { expanded.value.add(file.path); await loadDirectory(file.path); }
}

function clearFilterKeepingPath(path: string) {
  if (!query.value.trim()) return;
  query.value = "";
  for (const ancestor of treeAncestors(path)) if (ancestor) expanded.value.add(ancestor);
}

function open(file: FileEntry) {
  selected.value = file.path;
  focusedPath.value = file.path;
  revealFailure.value = "";
  if (file.kind === "directory") void toggle(file);
  else emit("open", file);
}

async function navigate(event: KeyboardEvent, row: TreeRow) {
  const action = treeKeyAction(event.key, event.altKey || event.ctrlKey || event.metaKey, rows.value, row);
  if (action === "ignore") return;
  event.preventDefault(); event.stopPropagation();
  if (action === "none") return;
  if (action.kind === "open") open(row.entry);
  else if (action.kind === "focus") await focusRow(action.path);
  else if (action.kind === "expand") await toggle(row.entry);
  else {
    // Filter results temporarily reveal ancestors; clear the filter before explicitly collapsing.
    clearFilterKeepingPath(action.path); expanded.value.delete(action.path);
  }
}

/** A change announced mid-refresh may predate the listings it already read. */
let changedDuringRefresh = false;
async function refresh() {
  if (refreshing.value) return;
  changedDuringRefresh = false;
  const remembered = new Set([...directories.value.keys(), ...expanded.value, ...restorePaths]);
  restorePaths = [];
  // Revalidate collapsed ancestors too, so a remembered nested expansion remains meaningful.
  for (const path of [...remembered]) if (path) for (const parent of treeAncestors(path)) remembered.add(parent);
  const reloadPaths = [...remembered].filter(Boolean);
  generation++; revealRevision++;
  const refreshGeneration = generation;
  pending = new Map();
  directories.value = new Map();
  refreshing.value = true;
  revealFailure.value = "";
  try {
    if (!await loadDirectory("")) return;
    for (const path of reloadPaths.sort((a, b) => a.split("/").length - b.split("/").length)) {
      if (generation !== refreshGeneration) return;
      const entry = directories.value.get(treeParent(path))?.entries.find(file => file.path === path);
      if (entry?.kind === "directory") await loadDirectory(path);
      else expanded.value.delete(path);
    }
  } finally {
    if (generation === refreshGeneration) {
      refreshing.value = false;
      if (changedDuringRefresh) { changedDuringRefresh = false; void refresh(); }
    }
  }
}

/** Reveal uses only the target's ancestors and never scans unrelated directories. */
async function reveal(path: string): Promise<void> {
  const requestRevision = ++revealRevision, requestGeneration = generation;
  const current = () => requestRevision === revealRevision && requestGeneration === generation;
  query.value = ""; revealFailure.value = "";
  let ancestors: string[];
  try { ancestors = treeAncestors(path); }
  catch { revealFailure.value = path; return; }
  for (const ancestor of ancestors) {
    if (!current()) return;
    if (ancestor) {
      const parent = treeParent(ancestor);
      if (!directories.value.get(parent)?.entries.some(file => file.path === ancestor)) await loadDirectory(parent, true);
      if (!current()) return;
      if (directories.value.get(parent)?.entries.find(file => file.path === ancestor)?.kind !== "directory") { revealFailure.value = path; return; }
      expanded.value.add(ancestor);
    }
    if (!await loadDirectory(ancestor) || !current()) return;
  }
  const parent = treeParent(path);
  if (!directories.value.get(parent)?.entries.some(file => file.path === path)) await loadDirectory(parent, true);
  if (!current()) return;
  if (!directories.value.get(parent)?.entries.some(file => file.path === path)) { revealFailure.value = path; return; }
  selected.value = path;
  focusedPath.value = path;
  await nextTick();
  if (!current()) return;
  const element = rowElements.get(path);
  element?.scrollIntoView({ block: "nearest", inline: "nearest" });
  element?.focus({ preventScroll: true });
}

async function focusRoot() {
  query.value = "";
  await nextTick();
  if (rows.value[0]) await focusRow(rows.value[0].entry.path);
  else treeElement.value?.focus();
}

function drag(event: DragEvent, file: FileEntry) {
  if (file.kind !== "file") return;
  event.dataTransfer?.setData("application/tendly-pane", JSON.stringify(filePane(props.workspaceId, file.path)));
  if (event.dataTransfer) event.dataTransfer.effectAllowed = "copy";
}

function rememberView(workspaceId: string) {
  saveTreeViewState(workspaceId, {
    expanded: expanded.value,
    loadedDirectories: new Set([...directories.value].filter(([, state]) => state.loaded).map(([path]) => path)),
    selectedPath: selected.value, focusedPath: focusedPath.value, query: query.value,
  });
}

watch(() => props.workspaceId, (workspaceId, previousId) => {
  if (previousId) rememberView(previousId);
  const remembered = readTreeViewState(workspaceId);
  generation++; revealRevision++; pending = new Map();
  directories.value = new Map(); expanded.value = remembered.expanded;
  selected.value = props.selectedPath ?? remembered.selectedPath; focusedPath.value = remembered.focusedPath;
  query.value = remembered.query; revealFailure.value = ""; refreshing.value = false;
  restorePaths = [...remembered.loadedDirectories];
  void refresh();
}, { immediate: true });
watch(() => props.refreshToken, () => void refresh());
watch(() => props.selectedPath, path => { selected.value = path ?? ""; });
/**
 * Changes on the host reload only the folders already loaded that they touch,
 * in place, so the tree neither flickers nor loses its expansion. A listing
 * already in flight may predate the change, so it is repeated after.
 */
const stopWatching = onFilesChanged((workspaceId, paths) => {
  if (workspaceId !== props.workspaceId) return;
  if (refreshing.value) { changedDuringRefresh = true; return; }
  const loaded = [...directories.value].filter(([, state]) => state.loaded).map(([path]) => path);
  for (const path of directoriesToReload(paths, loaded) ?? loaded) {
    const running = pending.get(path);
    if (running) void running.then(() => loadDirectory(path, true)); else void loadDirectory(path, true);
  }
});
onBeforeUnmount(() => { stopWatching(); rememberView(props.workspaceId); generation++; revealRevision++; pressEnd(); });
defineExpose({ reveal });
</script>
<template>
  <section class="file-explorer">
    <header class="section-header"><span><Icon name="folder" /><strong>{{ t('Explorer') }}</strong></span><span><!-- Finding the open file belongs to the tree, as in an editor's
         explorer: it acts on whichever file has focus. --><button class="icon-button" :disabled="!selectedPath" :aria-label="t('Locate the focused file in the tree')" :title="t('Locate the focused file in the tree')" @click="selectedPath && reveal(selectedPath)"><Icon name="locate" :size="15" /></button><button class="icon-button" :aria-label="t('New file')" :title="t('New file')" @click="startCreate()"><Icon name="plus" :size="15" /></button><button class="icon-button" :aria-label="t('New folder')" :title="t('New folder')" @click="startCreate(undefined, true)"><Icon name="folder" :size="15" /></button><button class="icon-button" :aria-label="t('Refresh files')" :title="t('Refresh files')" :disabled="loading" @click="refresh"><Icon name="refresh" :size="15" /></button><button class="icon-button explorer-close" :aria-label="t('Close explorer')" @click="emit('close')"><Icon name="close" :size="16" /></button></span></header>
    <nav class="file-breadcrumb" :aria-label="t('Directory path')"><button @click="focusRoot">{{ t('Root') }}</button><template v-for="crumb in crumbs" :key="crumb.path"><span>/</span><button :title="crumb.path" @click="reveal(crumb.path)">{{ crumb.name }}</button></template></nav>
    <form v-if="creating" class="tree-compose" @submit.prevent="createFile">
      <Icon :name="creating.directory ? 'folder' : 'file'" :size="14" />
      <span v-if="creating.base" class="tree-compose-base" :title="creating.base">{{ creating.base }}/</span>
      <input ref="createInput" v-model="creating.name" :aria-label="t(creating.directory ? 'New folder name' : 'New file name')" :placeholder="t(creating.directory ? 'New folder name' : 'New file name')" :disabled="creating.busy" spellcheck="false" @keydown.esc.prevent.stop="creating = null" />
      <button type="submit" class="text-button" :disabled="!creating.name.trim() || creating.busy">{{ t(creating.busy ? 'Creating…' : 'Create') }}</button>
      <button type="button" class="text-button" @click="creating = null">{{ t('Cancel') }}</button>
    </form>
    <div class="file-search"><Icon name="search" :size="14" /><input v-model="query" data-explorer-filter :aria-label="t('Filter loaded files')" @keydown.down.prevent="($event.currentTarget as HTMLElement).closest('.file-explorer')?.querySelector<HTMLElement>('.tree-row')?.focus()" :placeholder="t('Filter loaded files…')" /><button v-if="query" class="icon-button clear-filter" :aria-label="t('Clear file filter')" @click="query = ''"><Icon name="close" :size="12" /></button></div>
    <p v-if="query" class="tree-filter-note">{{ t('Only loaded folders are searched.') }}</p>
    <p v-if="revealFailure" class="inline-error" role="alert">{{ t('Could not locate {path} in the workspace.', { path: revealFailure }) }}</p>
    <p v-if="error" class="inline-error" role="alert">{{ error }}<button class="text-button" @click="error = ''">{{ t('Dismiss') }}</button></p>
    <div v-if="root?.error" class="inline-error tree-error" role="alert"><span>{{ root.error }}</span><button class="text-button" @click="loadDirectory('', true)">{{ t('Retry') }}</button></div>
    <div v-if="searchingWorkspace" class="file-list search-results" role="listbox" :aria-label="t('Workspace search results')">
      <button v-for="hit in searchHits" :key="hit.path" class="file-row search-hit" role="option" :aria-selected="selected === hit.path" @click="openHit(hit)">
        <Icon :name="hit.kind === 'directory' ? 'folder' : 'file'" :size="15" />
        <span class="search-hit-name"><template v-for="(part, index) in hitParts(hit)" :key="index"><mark v-if="part.match">{{ part.text }}</mark><template v-else>{{ part.text }}</template></template></span>
        <small class="search-hit-dir">{{ hitDirectory(hit.path) }}</small>
      </button>
      <p v-if="!searchHits.length" class="group-empty">{{ t('No files match this search.') }}</p>
      <p v-else-if="workspaceResults?.truncated" class="group-empty">{{ workspaceResults?.timed_out ? t('The workspace is large; showing the matches found so far.') : t('More matches exist; refine the search to narrow them.') }}</p>
    </div>
    <div v-show="!searchingWorkspace" ref="treeElement" class="file-list file-tree" role="tree" :aria-label="t('Workspace files')" :aria-busy="refreshing || root?.loading" :tabindex="rows.length ? -1 : 0" @contextmenu="openRowMenu($event)" @pointerdown="pressStart($event)" @pointermove="pressMove" @pointerup="pressEnd" @pointercancel="pressEnd">
      <template v-for="row in rows" :key="row.entry.path">
        <form v-if="renaming?.entry.path === row.entry.path" class="file-row tree-rename" :style="{ '--tree-depth': row.depth }" @submit.prevent="renameEntry">
          <i class="tree-disclosure" /><Icon :name="row.entry.kind === 'directory' ? 'folder' : 'file'" :size="15" />
          <input :ref="element => renameInput = element as HTMLInputElement" v-model="renaming.name" :aria-label="t('Rename {name}', { name: row.entry.name })" :disabled="renaming.busy" spellcheck="false" @keydown.esc.prevent.stop="renaming = null" @keydown.stop />
          <button type="submit" class="text-button" :disabled="!renaming.name.trim() || renaming.busy">{{ t(renaming.busy ? 'Renaming…' : 'Rename') }}</button>
          <button type="button" class="text-button" @click="renaming = null">{{ t('Cancel') }}</button>
        </form>
        <button v-else :ref="element => setRowRef(row.entry.path, element)" class="file-row tree-row" :class="{ 'is-selected': selected === row.entry.path, 'is-directory': row.entry.kind === 'directory' }" role="treeitem" :aria-level="row.depth + 1" :aria-posinset="row.position" :aria-setsize="row.siblings" :aria-expanded="row.entry.kind === 'directory' ? row.expanded : undefined" :aria-selected="selected === row.entry.path" :aria-label="row.entry.name" :aria-description="row.entry.path" :aria-busy="row.entry.kind === 'directory' ? directories.get(row.entry.path)?.loading : undefined" :tabindex="tabPath === row.entry.path ? 0 : -1" :style="{ '--tree-depth': row.depth }" :title="row.entry.path" :draggable="row.entry.kind === 'file'" @contextmenu="openRowMenu($event, row.entry)" @pointerdown="pressStart($event, row.entry)" @pointermove="pressMove" @pointerup="pressEnd" @pointercancel="pressEnd" @dragstart="drag($event, row.entry)" @click="open(row.entry)" @focus="focusedPath = row.entry.path" @keydown="navigate($event, row)">
          <i class="tree-disclosure" :class="{ expanded: row.expanded }"><Icon v-if="row.entry.kind === 'directory'" name="chevron" :size="11" /></i><Icon :name="row.entry.kind === 'directory' ? 'folder' : 'file'" :size="15" /><span>{{ row.entry.name }}</span><small v-if="row.entry.kind === 'symlink'" :title="t('Symbolic link')">↗</small><small v-else-if="row.entry.kind !== 'directory'">{{ formatBytes(row.entry.size) }}</small>
        </button>
        <div v-if="row.expanded && !query.trim()" role="none" class="tree-folder-state" :style="{ '--tree-depth': row.depth + 1 }">
          <p v-if="directories.get(row.entry.path)?.loading" role="status">{{ t('Loading files…') }}</p>
          <div v-else-if="directories.get(row.entry.path)?.error" class="tree-error" role="alert"><span>{{ directories.get(row.entry.path)?.error }}</span><button class="text-button" @click="loadDirectory(row.entry.path, true)">{{ t('Retry') }}</button></div>
          <p v-else-if="directories.get(row.entry.path)?.loaded && !directories.get(row.entry.path)?.entries.length">{{ t('This directory is empty.') }}</p>
        </div>
      </template>
      <div v-if="root?.loading" class="small-empty" role="status">{{ t('Loading files…') }}</div>
      <div v-else-if="root?.loaded && !rows.length" class="small-empty">{{ t(query.trim() ? 'No matching loaded files.' : 'This directory is empty.') }}</div>
    </div>
    <ContextMenu v-if="rowMenu" :x="rowMenu.x" :y="rowMenu.y" :label="t('File actions')" :width="198" @close="closeRowMenu"><template v-if="!rowMenu.entry"><button role="menuitem" @click="startCreate('')">{{ t('New file') }}</button><button role="menuitem" @click="startCreate('', true)">{{ t('New folder') }}</button><button role="menuitem" @click="closeRowMenu(); refresh()">{{ t('Refresh files') }}</button></template><template v-else><button v-if="rowMenu.entry.kind === 'directory'" role="menuitem" @click="startCreate(rowMenu!.entry!.path)">{{ t('New file here') }}</button><button role="menuitem" @click="startCreate(rowMenu!.entry!.path, true)">{{ t('New folder here') }}</button><button role="menuitem" @click="copyText(rowMenu.entry.path, 'path')">{{ t(copied === 'path' ? 'Copied' : 'Copy path') }}</button><button role="menuitem" @click="copyText(rowMenu.entry.name, 'name')">{{ t(copied === 'name' ? 'Copied' : 'Copy name') }}</button><button v-if="rowMenu.entry.kind === 'file'" role="menuitem" @click="closeRowMenu(); open(rowMenu!.entry)">{{ t('Open') }}</button><button role="menuitem" @click="closeRowMenu(); loadDirectory(rowMenu!.entry.kind === 'directory' ? rowMenu!.entry.path : treeParent(rowMenu!.entry.path) ?? '', true)">{{ t('Refresh files') }}</button><button role="menuitem" @click="startRename(rowMenu!.entry)">{{ t('Rename…') }}</button><hr/><template v-if="confirmDelete"><p class="context-menu-note">{{ t(rowMenu.entry.kind === 'directory' ? 'Delete {name} and everything inside it? This cannot be undone.' : 'Delete {name}? This cannot be undone.', { name: rowMenu.entry.name }) }}</p><button role="menuitem" class="danger" :disabled="deleting" :aria-busy="deleting" @click="deleteEntry">{{ t(deleting ? 'Deleting…' : 'Delete permanently') }}</button><button role="menuitem" @click="confirmDelete = false">{{ t('Cancel') }}</button></template><button v-else role="menuitem" class="danger" @click="confirmDelete = true">{{ t('Delete…') }}</button></template></ContextMenu>
    <footer class="explorer-footer">{{ t('{count} loaded items · host filesystem', { count: loadedCount }) }}<span>{{ t('Click to open · drag into a pane') }}</span></footer>
  </section>
</template>
<style scoped>
.search-results{overflow:auto;flex:1;padding:0 7px 12px;min-height:0}
.search-hit{align-items:center;gap:8px;padding:8px 7px;min-height:34px}
.search-hit-name{flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.search-hit-name mark{background:var(--accent-line);color:var(--accent-hover);border-radius:var(--radius-xs);padding:0 1px}
/* The folder is context, not the answer, so it yields space to the name. */
.search-hit-dir{flex-shrink:1;min-width:0;max-width:45%;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;direction:rtl;text-align:right;font-size:var(--text-xs);color:var(--faint)}

/* Typing a name, in the place the name will be. */
.tree-compose,.tree-rename{display:flex;align-items:center;gap:6px;min-height:34px}
.tree-compose{margin:0 7px 4px;padding:4px 7px;border:1px solid var(--line);border-radius:var(--radius-sm);background:var(--sunken)}
.tree-rename{padding-left:calc(7px + var(--tree-depth) * 13px)}
.tree-compose input,.tree-rename input{flex:1;min-width:0;border:1px solid var(--line-strong);border-radius:var(--radius-sm);padding:4px 6px;font:inherit;font-size:var(--text-xs);background:var(--surface)}
.tree-compose input:focus,.tree-rename input:focus{outline:2px solid var(--faint);outline-offset:-1px}
.tree-compose-base{flex-shrink:1;min-width:0;max-width:40%;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;direction:rtl;font-size:var(--text-xs);color:var(--muted)}
.tree-compose .text-button,.tree-rename .text-button{flex:none;font-size:var(--text-xs)}

/* Teleported so the tree's own scrolling and clipping cannot cut it off. */
.file-breadcrumb { max-height: 66px; overflow: auto; }
.file-breadcrumb button { max-width: 100%; }
.file-search { gap: 5px; }
.file-search input { min-width: 0; }
.clear-filter { width: 17px; height: 17px; padding: 1px; }
.tree-filter-note { margin: -5px 14px 10px; font-size: var(--text-xs); line-height: 15px; color: var(--muted); }
.file-tree { padding-top: 2px; scroll-padding: 6px; }
.tree-row { padding: 7px 7px 7px calc(3px + var(--tree-depth) * 15px); gap: 5px; position: relative; }
.tree-row > svg { flex-shrink: 0; color: var(--faint); }
.tree-row.is-directory > svg { color: var(--faint); }
.tree-row > span { min-width: 38px; }
.tree-row.is-selected { background: var(--accent-soft); box-shadow: inset 2px 0 #45a18c; }
.tree-row.is-selected > span { color: var(--accent-hover); }
.tree-row:focus-visible { outline: 1px solid var(--focus); outline-offset: -2px; background: var(--ok-soft); }
.tree-disclosure { display: flex; width: 11px; height: 15px; align-items: center; justify-content: center; flex: 0 0 11px; color: var(--faint); }
.tree-disclosure > svg { transition: transform 120ms ease; }
.tree-disclosure.expanded > svg { transform: rotate(90deg); }
.tree-folder-state { padding-left: calc(25px + var(--tree-depth) * 15px); }
.tree-folder-state:empty { display: none; }
.tree-folder-state p { color: var(--faint); font-size: var(--text-xs); line-height: 17px; padding: 3px 0 7px; }
.tree-error { display: flex; gap: 7px; align-items: baseline; font-size: var(--text-xs); line-height: 16px; padding: 4px 0 8px; color: var(--danger-ink); }
.tree-error > span { min-width: 0; overflow-wrap: anywhere; }
.tree-error > button { flex-shrink: 0; color: var(--muted); font-size: var(--text-xs); }
.inline-error.tree-error { margin: 0 12px 8px; }
@media (prefers-reduced-motion: reduce) { .tree-disclosure > svg { transition: none; } }
</style>
