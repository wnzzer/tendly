<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import type { Workspace } from "@tendly/protocol";
import { useI18n } from "../i18n";
import { errorMessage, json, request } from "./api";
import { backendCapabilities } from "./backend-capabilities";
import { addHostRoot, isOutsideRoots, loadHostRoots } from "./host-roots";
import Icon from "./Icon.vue";
import ModalDialog from "./ModalDialog.vue";

interface DirectoryRoot { id: string; label: string; path: string }
interface DirectoryEntry { name: string; path: string }
interface DirectoryListing {
  roots: DirectoryRoot[];
  root_id: string;
  path: string;
  relative_path: string;
  parent: string | null;
  entries: DirectoryEntry[];
  truncated: boolean;
}

const { t } = useI18n();
const props = defineProps<{ initialPath?: string }>();
const emit = defineEmits<{ close: []; created: [workspace: Workspace] }>();
const name = ref(""), path = ref(""), error = ref(""), busy = ref(false);
const listing = ref<DirectoryListing | null>(null);
const availableRoots = ref<DirectoryRoot[]>([]);
const rootId = ref("");
const filter = ref("");
const browsing = ref(false);
const browseError = ref("");
const recent = ref<Workspace[]>([]);
const advanced = ref(false);
/** On the machine Tendly runs on, a folder outside the allowed ones can be allowed from here. */
const mayAllow = ref(false);
let lastSuggestedName = "";
let browseController: AbortController | undefined;
let alive = true;

const entries = computed(() => {
  const query = filter.value.toLocaleLowerCase().trim();
  return listing.value?.entries.filter(entry => entry.name.toLocaleLowerCase().includes(query)) ?? [];
});
const breadcrumbs = computed(() => {
  const parts = listing.value?.relative_path.split("/").filter(Boolean) ?? [];
  return parts.map((label, index) => ({ label, path: parts.slice(0, index + 1).join("/") }));
});
const currentRoot = computed(() => availableRoots.value.find(root => root.id === listing.value?.root_id));
const existingWorkspace = computed(() => recent.value.find(workspace => workspace.root_path === path.value.trim()));

async function browse(relativePath = "", nextRoot = rootId.value) {
  if (!backendCapabilities.directories) { advanced.value = true; return; }
  browseController?.abort();
  const controller = new AbortController();
  browseController = controller;
  browsing.value = true;
  browseError.value = "";
  filter.value = "";
  const query = new URLSearchParams({ path: relativePath });
  if (nextRoot !== "") query.set("root", nextRoot);
  try {
    const result = await request<DirectoryListing>(`/host/directories?${query}`, { signal: controller.signal });
    if (controller.signal.aborted || !alive) return;
    listing.value = result;
    availableRoots.value = result.roots;
    rootId.value = result.root_id;
  } catch (cause) {
    if (!controller.signal.aborted && alive) {
      browseError.value = errorMessage(cause);
      // Keep the last successful location visible and the root selector in sync.
      if (listing.value) rootId.value = listing.value.root_id;
    }
  } finally {
    if (!controller.signal.aborted && alive) browsing.value = false;
  }
}

function selectDirectory(directory: string) {
  path.value = directory;
  error.value = "";
  if (!name.value.trim() || name.value === lastSuggestedName) {
    lastSuggestedName = directory.split(/[\\/]/).filter(Boolean).at(-1) ?? t("My project");
    name.value = lastSuggestedName;
  }
}

/** Allow the chosen folder, then add it: what the error asked for, in one press. */
async function allowAndCreate() {
  busy.value = true;
  try { await addHostRoot(path.value.trim()); }
  catch (cause) { if (alive) { error.value = errorMessage(cause); busy.value = false; } return; }
  busy.value = false;
  await create();
}

async function create() {
  if (busy.value || !name.value.trim() || !path.value.trim()) return;
  if (existingWorkspace.value) {
    emit("created", existingWorkspace.value);
    return;
  }
  busy.value = true;
  error.value = "";
  try {
    const workspace = await request<Workspace>("/workspaces", json("POST", { name: name.value.trim(), root_path: path.value.trim() }));
    if (alive) emit("created", workspace);
  } catch (cause) {
    if (alive) error.value = errorMessage(cause);
  } finally {
    if (alive) busy.value = false;
  }
}

onMounted(() => {
  if (backendCapabilities.directories) void browse(); else advanced.value = true;
  // Opened for a folder already known -- the one holding a file an agent named.
  if (props.initialPath) selectDirectory(props.initialPath);
  if (backendCapabilities.hostRoots) void loadHostRoots().then(roots => { if (alive) mayAllow.value = roots.editable; }).catch(() => { /* Then the error alone explains it. */ });
  void request<Workspace[]>("/workspaces").then(workspaces => {
    if (alive) recent.value = workspaces;
  }).catch(() => { /* Existing workspaces are a convenience; browsing remains available. */ });
});
onUnmounted(() => { alive = false; browseController?.abort(); });
</script>

<template>
  <ModalDialog :title="t('Add workspace')" wide :closable="!busy" @close="emit('close')">
    <form class="form-stack workspace-form" @submit.prevent="create">
      <p class="form-description">{{ t('Choose an existing folder on the host running Tendly. Your files stay where they are.') }}</p>

      <section v-if="recent.length" class="recent-workspaces" :aria-label="t('Recent workspaces')">
        <h3>{{ t('Recent workspaces') }}</h3>
        <div class="recent-workspace-list">
          <button v-for="workspace in recent.slice(0, 5)" :key="workspace.id" type="button" :disabled="busy" :title="workspace.root_path" @click="emit('created', workspace)">
            <Icon name="clock" :size="15" />
            <span><strong>{{ workspace.name }}</strong><small>{{ workspace.root_path }}</small></span>
            <Icon name="arrow" :size="14" />
          </button>
        </div>
      </section>

      <p v-if="!backendCapabilities.directories" class="inline-notice">{{ t('This backend needs an upgrade for folder browsing. You can still register an existing absolute path.') }}</p>
      <section v-else class="directory-browser" :aria-label="t('Browse host folders')" :aria-busy="browsing">
        <header class="browser-header">
          <div class="browser-title"><Icon name="folder" :size="17" /><h3>{{ t('Browse host folders') }}</h3></div>
          <label v-if="availableRoots.length" class="root-selector">
            <span>{{ t('Location') }}</span>
            <select v-model="rootId" :disabled="busy || browsing" @change="browse('', rootId)">
              <option v-for="root in availableRoots" :key="root.id" :value="root.id">{{ root.label }} · {{ root.path }}</option>
            </select>
          </label>
        </header>

        <div v-if="listing" class="browser-location">
          <button type="button" class="icon-button" :disabled="busy || browsing || listing.parent === null" :title="t('Parent folder')" :aria-label="t('Parent folder')" @click="browse(listing.parent ?? '')"><Icon name="back" :size="16" /></button>
          <nav class="folder-breadcrumbs" :aria-label="t('Current folder')">
            <button type="button" :disabled="busy || browsing" :title="currentRoot?.path" @click="browse('')">{{ currentRoot?.label ?? t('Root') }}</button>
            <template v-for="crumb in breadcrumbs" :key="crumb.path"><span aria-hidden="true">/</span><button type="button" :disabled="busy || browsing" @click="browse(crumb.path)">{{ crumb.label }}</button></template>
          </nav>
          <button type="button" class="icon-button" :disabled="busy || browsing" :title="t('Refresh folders')" :aria-label="t('Refresh folders')" @click="browse(listing.relative_path)"><Icon name="refresh" :size="15" /></button>
        </div>

        <div v-if="browseError" class="browser-error" role="alert">
          <span>{{ browseError }}</span>
          <button type="button" class="text-button" :disabled="browsing" @click="browse(listing?.relative_path ?? '')">{{ t('Retry') }}</button>
        </div>

        <div v-if="listing" class="directory-filter">
          <Icon name="search" :size="14" />
          <input v-model="filter" :placeholder="t('Filter folders in this directory…')" :aria-label="t('Filter folders')" autocomplete="off" :disabled="busy || browsing" />
        </div>

        <div class="directory-list" role="list" :aria-label="t('Folders')">
          <p v-if="browsing" class="directory-empty" role="status">{{ t('Loading folders…') }}</p>
          <template v-else-if="listing">
            <button v-for="entry in entries" :key="entry.path" type="button" class="directory-row" :disabled="busy" @click="browse(entry.path)">
              <Icon name="folder" :size="17" /><span>{{ entry.name }}</span><Icon name="chevron" :size="14" />
            </button>
            <p v-if="!entries.length" class="directory-empty">{{ filter.trim() ? t('No matching folders.') : t('No subfolders. You can use this directory.') }}</p>
          </template>
          <p v-else-if="!browseError" class="directory-empty">{{ t('Select a location to browse.') }}</p>
        </div>

        <footer v-if="listing" class="browser-footer">
          <span :title="listing.path">{{ listing.path }}</span>
          <button type="button" class="secondary-button" :disabled="busy || browsing" @click="selectDirectory(listing.path)"><Icon :name="path === listing.path ? 'check' : 'folder'" :size="14" />{{ path === listing.path ? t('Selected') : t('Use this directory') }}</button>
        </footer>
      </section>
      <p v-if="listing?.truncated" class="form-help">{{ t('Showing the first 2,000 folders. Use the advanced path field for other folders.') }}</p>
      <p class="form-help browser-help">{{ t('Browsing locations are configured on the server. Credential folders and symbolic links are hidden.') }}</p>

      <div v-if="path" class="selected-directory" aria-live="polite">
        <span class="selected-directory-icon"><Icon name="check" :size="17" /></span>
        <span><small>{{ t('Selected directory') }}</small><strong>{{ path }}</strong></span>
      </div>

      <label>{{ t('Workspace name') }}<input v-model="name" required maxlength="120" :placeholder="t('My project')" :disabled="busy" /></label>

      <details :open="advanced" class="advanced-directory" @toggle="advanced = ($event.target as HTMLDetailsElement).open">
        <summary>{{ t('Advanced: enter a host path') }}</summary>
        <label>{{ t('Absolute host directory') }}<input :value="path" :placeholder="t('/path/to/your/project')" autocomplete="off" spellcheck="false" :disabled="busy" @input="selectDirectory(($event.target as HTMLInputElement).value)" /></label>
        <p class="form-help">{{ t('This path is on the server, not your browser’s computer. No folder, container or repository will be created.') }}</p>
        <p class="form-help">{{ t(backendCapabilities.hostRoots ? 'More locations can be added in Settings → Preferences → Available folders.' : 'Deployment tip: set TENDLY_BROWSE_ROOTS to choose the folders available in the browser.') }}</p>
      </details>

      <p v-if="existingWorkspace" class="form-help">{{ t('This directory is already registered. We will open the existing workspace.') }}</p>
      <div v-if="error" class="inline-error" role="alert">
        <span>{{ t(error) }}</span>
        <button v-if="mayAllow && isOutsideRoots(error)" type="button" class="secondary-button allow-folder" :disabled="busy" @click="allowAndCreate">{{ t('Allow this folder and add it') }}</button>
      </div>
      <div class="dialog-actions">
        <button type="button" class="secondary-button" :disabled="busy" @click="emit('close')">{{ t('Cancel') }}</button>
        <button class="primary-button" :disabled="busy || !name.trim() || !path.trim()">{{ busy ? t('Opening…') : existingWorkspace ? t('Open workspace') : t('Add workspace') }}</button>
      </div>
    </form>
  </ModalDialog>
</template>

<style scoped>
.workspace-form { gap: 14px; }
.recent-workspaces h3, .browser-title h3 { font-size: var(--text-xs); font-weight: 600; color: var(--ink-soft); }
.recent-workspaces h3 { margin-bottom: 9px; }
.recent-workspace-list { display: flex; gap: 7px; overflow-x: auto; padding-bottom: 3px; }
.recent-workspace-list>button { display: flex; align-items: center; gap: 8px; flex: 0 0 205px; max-width: 250px; min-width: 0; padding: 10px; border: 1px solid var(--fill-hover); border-radius: var(--radius-md); background: var(--surface); color: var(--muted); text-align: left; }
.recent-workspace-list>button:hover { border-color: var(--ok-line); background: var(--ok-soft); }
.recent-workspace-list>button>span { flex: 1; min-width: 0; }
.recent-workspace-list strong, .recent-workspace-list small { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.recent-workspace-list strong { font-size: var(--text-xs); font-weight: 550; color: var(--ink-soft); }
.recent-workspace-list small { margin-top: 4px; font-size: var(--text-xs); color: var(--faint); }
.directory-browser { border: 1px solid var(--line); border-radius: var(--radius-md); overflow: hidden; }
.browser-header { display: flex; justify-content: space-between; align-items: center; gap: 14px; padding: 12px 13px; background: var(--sunken); }
.browser-title { display: flex; gap: 7px; align-items: center; color: var(--muted); flex-shrink: 0; }
.form-stack .root-selector { display: flex; flex-direction: row; gap: 7px; align-items: center; min-width: 0; }
.root-selector>span { font-size: var(--text-xs); white-space: nowrap; }
.form-stack .root-selector>select { font-size: var(--text-xs); padding: 6px 8px; min-width: 0; max-width: 270px; color: var(--ink-soft); background: var(--surface); }
.browser-location { display: flex; gap: 6px; align-items: center; padding: 6px 8px; border-block: 1px solid var(--fill-hover); }
.browser-location>.icon-button { flex-shrink: 0; }
.folder-breadcrumbs { display: flex; gap: 5px; align-items: center; flex: 1; min-width: 0; overflow-x: auto; }
.folder-breadcrumbs>button { flex-shrink: 0; padding: 4px 3px; border: 0; background: none; font-size: var(--text-xs); color: var(--muted); }
.folder-breadcrumbs>button:last-child { color: var(--accent); font-weight: 550; }
.folder-breadcrumbs>span { font-size: var(--text-xs); color: var(--line-strong); }
.directory-filter { display: flex; align-items: center; gap: 7px; margin: 9px 11px 5px; padding: 0 8px; border: 1px solid var(--fill-hover); border-radius: var(--radius-sm); color: var(--faint); background: var(--surface); }
.form-stack .directory-filter>input { border: 0; padding: 7px 0; font-size: var(--text-xs); background: none; outline: none; }
.directory-list { height: 185px; overflow-y: auto; padding: 3px 6px 6px; }
.directory-row { display: flex; align-items: center; gap: 9px; width: 100%; padding: 9px 8px; background: transparent; border: 0; border-radius: var(--radius-sm); text-align: left; color: var(--faint); }
.directory-row:hover { background: var(--ok-soft); }
.directory-row>span { flex: 1; min-width: 0; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; color: var(--ink-soft); font-size: var(--text-xs); }
.directory-row>svg:last-child { color: var(--faint); flex-shrink: 0; }
.directory-empty { padding: 40px 15px; text-align: center; color: var(--faint); font-size: var(--text-xs); line-height: 19px; }
.browser-footer { display: flex; justify-content: space-between; align-items: center; gap: 12px; padding: 10px 12px; border-top: 1px solid var(--accent-soft); background: var(--surface); }
.browser-footer>span { min-width: 0; font-family: var(--mono); font-size: var(--text-xs); color: var(--muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.browser-footer>.secondary-button { display: flex; gap: 6px; align-items: center; flex-shrink: 0; font-size: var(--text-xs); color: var(--accent); border-color: var(--ok-line); }
.browser-error { display: flex; align-items: flex-start; gap: 12px; padding: 10px 13px; color: #bb6263; font-size: var(--text-xs); line-height: 18px; background: var(--danger-soft); }
.browser-error>span { flex: 1; }
.browser-error>.text-button { flex-shrink: 0; font-size: var(--text-xs); }
.browser-help { margin-top: -7px; font-size: var(--text-xs); }
.selected-directory { display: flex; align-items: center; gap: 10px; padding: 11px 12px; background: var(--ok-soft); border: 1px solid var(--accent-line); border-radius: var(--radius-md); }
.selected-directory-icon { width: 29px; height: 29px; flex-shrink: 0; display: grid; place-items: center; border-radius: var(--radius-md); color: var(--ok); background: var(--accent-line); }
.selected-directory>span:last-child { min-width: 0; }
.selected-directory small { display: block; color: var(--muted); font-size: var(--text-xs); }
.selected-directory strong { display: block; margin-top: 3px; font-size: var(--text-xs); font-weight: 500; color: var(--accent-ink); overflow-wrap: anywhere; }
.inline-error:has(.allow-folder) { display: flex; flex-direction: column; align-items: flex-start; gap: 8px; }
.advanced-directory { border-top: 1px solid var(--fill); padding-top: 12px; }
.advanced-directory>summary { cursor: pointer; font-size: var(--text-xs); color: var(--muted); }
.advanced-directory>label { margin-top: 12px; }
.advanced-directory>.form-help { margin-top: 7px; font-size: var(--text-xs); }
@media (max-width: 540px) {
  .browser-header { flex-direction: column; align-items: stretch; gap: 9px; }
  .form-stack .root-selector>select { flex: 1; max-width: none; }
  .browser-footer { flex-direction: column; align-items: stretch; gap: 8px; }
  .browser-footer>.secondary-button { justify-content: center; }
  .directory-list { height: 170px; }
}
</style>
