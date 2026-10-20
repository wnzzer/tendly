<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { CHORDS, isRemove, isRename, matches } from './shortcuts';
import { moveInList } from './list-keys';
import type { ProviderKind, Session, Workspace } from '@tendly/protocol';
import { providerLabel } from './api';
import { PROVIDER_KINDS } from './clients';
import { filterSessionList, isEphemeralSession, isSessionArchived } from './session-list';
import type { SessionArchiveFilter, SessionEphemeralFilter } from './session-list';
import { invertSelection, selectRange } from './session-selection';
import PageShell from './PageShell.vue';
import ContextMenu from './ContextMenu.vue';
import Icon from './Icon.vue';
import ProviderIcon from './ProviderIcon.vue';
import { useI18n } from '../i18n';

/**
 * Every session, as a page: find one, filter by workspace, client, status or
 * archive state, and archive, restore or delete several at once. It used to be
 * squeezed into the sidebar; a table has room for what tells sessions apart.
 */
const props = withDefaults(defineProps<{
  workspaces: Workspace[];
  sessions: Session[];
  selectedSessionId?: string;
  archiveSupported?: boolean;
  archiveBusyIds?: string[];
  keepBusyIds?: string[];
  deleteBusyIds?: string[];
  sessionsLoading?: boolean;
  /** Filters to start from, for "locate" landing on an archived or temporary session. */
  initialArchive?: SessionArchiveFilter;
  highlightId?: string;
}>(), { archiveSupported: false, archiveBusyIds: () => [], keepBusyIds: () => [], deleteBusyIds: () => [], sessionsLoading: false, initialArchive: 'current' });
const emit = defineEmits<{
  back: [];
  openSession: [session: Session];
  renameRequest: [id: string];
  sessionEnvironment: [id: string];
  archiveSession: [session: Session, archived: boolean];
  archiveSessions: [sessions: Session[], archived: boolean];
  deleteSessions: [sessions: Session[]];
  keepSession: [session: Session];
  refreshSessions: [];
}>();
const { t } = useI18n();

const query = ref(''), workspaceId = ref(''), provider = ref<ProviderKind | ''>(''), status = ref<Session['status'] | ''>('');
const archive = ref<SessionArchiveFilter>(props.initialArchive), ephemeral = ref<SessionEphemeralFilter>('all');
watch(() => props.initialArchive, value => { archive.value = value; });
const statuses: Session['status'][] = ['starting', 'running', 'waiting', 'stopped', 'failed'];
const filtered = computed(() => filterSessionList(props.sessions, props.workspaces, {
  query: query.value, workspaceId: workspaceId.value, provider: provider.value, status: status.value, archive: archive.value, ephemeral: ephemeral.value,
  statusLabels: Object.fromEntries(statuses.map(entry => [entry, t(entry)])),
}));
const hasFilters = computed(() => Boolean(query.value || workspaceId.value || provider.value || status.value || archive.value !== 'current' || ephemeral.value !== 'all'));
function clearFilters() { query.value = ''; workspaceId.value = ''; provider.value = ''; status.value = ''; archive.value = 'current'; ephemeral.value = 'all'; }

/** Only rows the filters show can be ticked, so a bulk action never reaches a session you cannot see. */
const checkedIds = ref<string[]>([]), confirmBulkDelete = ref(false);
let lastChecked: string | undefined;
const visibleIds = computed(() => filtered.value.map(entry => entry.session.id));
const checkedSet = computed(() => new Set(checkedIds.value));
const checked = computed(() => filtered.value.map(entry => entry.session).filter(session => checkedSet.value.has(session.id)));
const toArchive = computed(() => checked.value.filter(session => !isSessionArchived(session)));
const toRestore = computed(() => checked.value.filter(isSessionArchived));
const toDelete = computed(() => checked.value.filter(session => isSessionArchived(session) || isEphemeralSession(session)));
const allChecked = computed(() => visibleIds.value.length > 0 && visibleIds.value.every(id => checkedSet.value.has(id)));
watch(visibleIds, ids => { const visible = new Set(ids); checkedIds.value = checkedIds.value.filter(id => visible.has(id)); });
watch(checkedIds, () => { confirmBulkDelete.value = false; });
function check(session: Session, event: { shiftKey: boolean }) {
  checkedIds.value = selectRange(visibleIds.value, checkedIds.value, session.id, event.shiftKey ? lastChecked : undefined);
  lastChecked = session.id;
}
function checkAll() { checkedIds.value = allChecked.value ? [] : [...visibleIds.value]; }
function invert() { checkedIds.value = invertSelection(visibleIds.value, checkedIds.value); }
function bulkArchive(archived: boolean) {
  const list = archived ? toArchive.value : toRestore.value;
  if (props.archiveSupported && list.length) { emit('archiveSessions', list, archived); checkedIds.value = []; }
}
function bulkDelete() { if (toDelete.value.length) { emit('deleteSessions', toDelete.value); checkedIds.value = []; confirmBulkDelete.value = false; } }

const menu = ref<{ session: Session; x: number; y: number; confirmDelete?: boolean }>();
function openMenu(session: Session, event: MouseEvent) {
  const target = (event.currentTarget as HTMLElement).getBoundingClientRect();
  menu.value = event.type === 'contextmenu' ? { session, x: event.clientX, y: event.clientY } : { session, x: target.right - 200, y: target.bottom + 4 };
}
/** Close the menu, then act on the session it was opened for. */
function act(run: (session: Session) => void) { const current = menu.value; menu.value = undefined; if (current) run(current.session); }
/**
 * The list's keys, on a session's title: ↑ ↓ Home End move, Shift+↑ ↓ select
 * a range, Space ticks it, Enter opens it, F2 renames, Delete (⌘⌫ on a Mac)
 * archives it -- or, for one already archived, asks to delete it.
 */
function rowKeydown(event: KeyboardEvent, session: Session) {
  const own = event.currentTarget as HTMLElement, table = own.closest('.sessions-table') ?? document;
  if (moveInList(event, table, '.sessions-title')) return;
  if (event.shiftKey && (event.key === 'ArrowDown' || event.key === 'ArrowUp')) {
    event.preventDefault();
    const ids = visibleIds.value, next = ids[ids.indexOf(session.id) + (event.key === 'ArrowDown' ? 1 : -1)];
    if (!next) return;
    if (!checkedSet.value.has(session.id)) check(session, { shiftKey: false });
    checkedIds.value = selectRange(ids, checkedIds.value, next, lastChecked ?? session.id);
    table.querySelectorAll<HTMLElement>('.sessions-title')[ids.indexOf(next)]?.focus();
    return;
  }
  if (event.key === ' ') { event.preventDefault(); check(session, event); return; }
  if (isRename(event)) { event.preventDefault(); emit('renameRequest', session.id); return; }
  if (!isRemove(event)) return;
  event.preventDefault();
  if (isSessionArchived(session) || isEphemeralSession(session)) { const box = own.getBoundingClientRect(); menu.value = { session, x: box.left + 24, y: box.bottom + 4, confirmDelete: true }; return; }
  if (props.archiveSupported && !busy(session.id)) emit('archiveSession', session, true);
}
/** Find, here, is this page's search. */
const searchInput = ref<HTMLInputElement>();
function pageKeydown(event: KeyboardEvent) {
  if (!matches(event, CHORDS.find) || document.querySelector('[aria-modal="true"]')) return;
  event.preventDefault(); searchInput.value?.focus(); searchInput.value?.select();
}
onMounted(() => document.addEventListener('keydown', pageKeydown));
onBeforeUnmount(() => document.removeEventListener('keydown', pageKeydown));
const busy = (id: string) => props.archiveBusyIds.includes(id) || props.deleteBusyIds.includes(id) || props.keepBusyIds.includes(id);

function relative(at?: string | null) {
  const time = Date.parse(at ?? '');
  if (!Number.isFinite(time)) return t('Not run yet');
  const minutes = Math.round((Date.now() - time) / 60000);
  if (minutes < 1) return t('just now');
  if (minutes < 60) return t('{n} min ago', { n: minutes });
  const hours = Math.round(minutes / 60);
  if (hours < 24) return t('{n} h ago', { n: hours });
  return t('{n} d ago', { n: Math.round(hours / 24) });
}
</script>

<template>
  <PageShell named-above :title="t('All sessions')" :subtitle="t('{count} sessions', { count: sessions.length })" @back="emit('back')">
    <template #actions><button class="secondary-button" :disabled="sessionsLoading" @click="emit('refreshSessions')"><Icon name="refresh" :size="14" />{{ t('Refresh') }}</button></template>
    <div class="sessions-page">
      <div class="sessions-toolbar">
        <label class="sessions-search"><Icon name="search" :size="15" /><input ref="searchInput" v-model="query" type="search" :aria-label="t('Search all sessions')" @keydown.down.prevent="($event.currentTarget as HTMLElement).closest('.sessions-page, body')?.querySelector<HTMLElement>('.sessions-title')?.focus()" :placeholder="t('Find a session…')" /></label>
        <div class="sessions-segments" role="group" :aria-label="t('Filter sessions by archive state')">
          <button v-for="entry in (['current', 'archived', 'all'] as const)" :key="entry" type="button" :class="{ selected: archive === entry }" :aria-pressed="archive === entry" @click="archive = entry">{{ t(entry === 'current' ? 'Current sessions' : entry === 'archived' ? 'Archived sessions' : 'All sessions') }}</button>
        </div>
      </div>
      <div class="sessions-filters">
        <select v-model="workspaceId" :aria-label="t('Filter sessions by workspace')"><option value="">{{ t('All workspaces') }}</option><option v-for="workspace in workspaces" :key="workspace.id" :value="workspace.id">{{ workspace.name }}</option></select>
        <select v-model="provider" :aria-label="t('Filter sessions by provider')"><option value="">{{ t('All providers') }}</option><option v-for="entry in PROVIDER_KINDS" :key="entry" :value="entry">{{ entry === 'terminal' ? t('Terminal') : providerLabel(entry) }}</option></select>
        <select v-model="status" :aria-label="t('Filter sessions by status')"><option value="">{{ t('All statuses') }}</option><option v-for="entry in statuses" :key="entry" :value="entry">{{ t(entry) }}</option></select>
        <select v-model="ephemeral" :aria-label="t('Filter temporary windows')"><option value="all">{{ t('Permanent and temporary') }}</option><option value="only">{{ t('Only temporary windows') }}</option><option value="hidden">{{ t('Hide temporary windows') }}</option></select>
        <button v-if="hasFilters" class="text-button" @click="clearFilters">{{ t('Clear session filters') }}</button>
      </div>

      <div v-if="checked.length" class="sessions-bulk" role="toolbar" :aria-label="t('Selected sessions')">
        <strong>{{ t('{count} selected', { count: checked.length }) }}</strong>
        <button class="text-button" @click="invert">{{ t('Invert selection') }}</button>
        <span class="flex-spacer" />
        <template v-if="confirmBulkDelete">
          <span class="sessions-bulk-warning">{{ t('Delete {count} sessions? Their records and conversations are removed and cannot be restored.', { count: toDelete.length }) }}</span>
          <button class="small-button danger" @click="bulkDelete">{{ t('Delete permanently') }}</button>
          <button class="small-button" @click="confirmBulkDelete = false">{{ t('Cancel') }}</button>
        </template>
        <template v-else>
          <button class="small-button" :disabled="!archiveSupported || !toArchive.length" @click="bulkArchive(true)"><Icon name="archive" :size="13" />{{ t('Archive') }}</button>
          <button class="small-button" :disabled="!archiveSupported || !toRestore.length" @click="bulkArchive(false)"><Icon name="restore" :size="13" />{{ t('Restore') }}</button>
          <button class="small-button danger" :disabled="!toDelete.length" :title="t('Only archived or temporary sessions can be deleted.')" @click="confirmBulkDelete = true">{{ t('Delete') }}</button>
        </template>
      </div>

      <div class="sessions-table" role="table" :aria-label="t('All sessions')" :aria-busy="sessionsLoading">
        <div class="sessions-row head" role="row">
          <span role="columnheader"><input type="checkbox" :checked="allChecked" :indeterminate="checked.length > 0 && !allChecked" :aria-label="t(allChecked ? 'Select none' : 'Select all')" @change="checkAll" /></span>
          <span role="columnheader">{{ t('Session') }}</span><span role="columnheader">{{ t('Workspace') }}</span><span role="columnheader">{{ t('Status') }}</span><span role="columnheader">{{ t('Updated') }}</span><span role="columnheader" />
        </div>
        <div v-for="entry in filtered" :key="entry.session.id" role="row" :class="['sessions-row', { checked: checkedSet.has(entry.session.id), current: entry.session.id === selectedSessionId, highlight: entry.session.id === highlightId, busy: busy(entry.session.id) }]" @contextmenu.prevent="openMenu(entry.session, $event)">
          <span role="cell"><input type="checkbox" :checked="checkedSet.has(entry.session.id)" :aria-label="t('Select {session}', { session: entry.session.title })" @click="check(entry.session, $event)" /></span>
          <button role="cell" type="button" class="sessions-title" @click="emit('openSession', entry.session)" @keydown="rowKeydown($event, entry.session)">
            <ProviderIcon :provider="entry.session.provider" :size="15" />
            <span><strong>{{ entry.session.title }}</strong><small>{{ entry.session.provider === 'terminal' ? t('Terminal') : providerLabel(entry.session.provider) }}<template v-if="isEphemeralSession(entry.session)"> · {{ t('Temporary') }}</template><template v-if="isSessionArchived(entry.session)"> · {{ t('Archived') }}</template></small><small v-if="entry.session.preview" class="session-row-preview">{{ entry.session.preview }}</small></span>
          </button>
          <span role="cell" class="sessions-muted">{{ entry.workspace?.name ?? '—' }}</span>
          <span role="cell"><span :class="['status-pill', entry.session.status]"><i />{{ t(entry.session.status) }}</span></span>
          <span role="cell" class="sessions-muted">{{ relative(entry.session.updated_at) }}</span>
          <span role="cell"><button type="button" class="icon-button" :aria-label="t('Actions for {session}', { session: entry.session.title })" @click="openMenu(entry.session, $event)"><Icon name="more" :size="16" /></button></span>
        </div>
        <p v-if="!filtered.length" class="sessions-empty" role="status">{{ t(sessionsLoading ? 'Loading sessions…' : hasFilters ? 'No session matches these filters.' : 'No sessions yet.') }}</p>
      </div>

      <ContextMenu v-if="menu" :x="menu.x" :y="menu.y" :label="t('Actions for {session}', { session: menu.session.title })" @close="menu = undefined">
        <button role="menuitem" @click="act(session => emit('openSession', session))"><Icon name="layout" :size="14" />{{ t('Open') }}</button>
        <button role="menuitem" @click="act(session => emit('renameRequest', session.id))"><Icon name="edit" :size="14" />{{ t('Rename') }}</button>
        <button role="menuitem" @click="act(session => emit('sessionEnvironment', session.id))"><Icon name="settings" :size="14" />{{ t('Session environment') }}</button>
        <button v-if="isEphemeralSession(menu.session)" role="menuitem" @click="act(session => emit('keepSession', session))"><Icon name="save" :size="14" />{{ t('Keep this session') }}</button>
        <hr />
        <button v-if="archiveSupported" role="menuitem" @click="act(session => emit('archiveSession', session, !isSessionArchived(session)))"><Icon :name="isSessionArchived(menu.session) ? 'restore' : 'archive'" :size="14" />{{ t(isSessionArchived(menu.session) ? 'Restore' : 'Archive') }}</button>
        <template v-if="isSessionArchived(menu.session) || isEphemeralSession(menu.session)">
          <button v-if="!menu.confirmDelete" role="menuitem" class="danger-text" @click="menu.confirmDelete = true"><Icon name="close" :size="14" />{{ t('Delete') }}</button>
          <button v-else role="menuitem" class="danger-text" @click="act(session => emit('deleteSessions', [session]))"><Icon name="close" :size="14" />{{ t('Delete permanently') }}</button>
        </template>
      </ContextMenu>
    </div>
  </PageShell>
</template>

<style scoped>
.sessions-page{display:flex;flex-direction:column;gap:var(--space-3)}
.sessions-toolbar{display:flex;flex-wrap:wrap;gap:var(--space-3);align-items:center}
.sessions-search{flex:1;min-width:220px;display:flex;align-items:center;gap:8px;height:38px;padding:0 12px;border:1px solid var(--line);border-radius:var(--radius-md);background:var(--surface);color:var(--muted)}
.sessions-search:focus-within{border-color:var(--accent);box-shadow:0 0 0 3px var(--accent-soft)}
.sessions-search input{flex:1;min-width:0;border:0;outline:0;background:none;font-size:var(--text-md);color:var(--ink)}
.sessions-segments{display:inline-flex;padding:3px;border-radius:var(--radius-md);background:var(--fill)}
.sessions-segments button{min-height:30px;padding:0 12px;border:0;border-radius:var(--radius-sm);background:none;color:var(--ink-soft);font-size:var(--text-sm);cursor:pointer}
.sessions-segments button.selected{background:var(--surface);color:var(--ink);box-shadow:var(--shadow-sm);font-weight:550}
.sessions-filters{display:flex;flex-wrap:wrap;gap:var(--space-2);align-items:center}
.sessions-filters select{height:32px;min-width:0;max-width:200px;padding:0 10px;border:1px solid var(--line);border-radius:var(--radius-md);background:var(--surface);color:var(--ink-soft);font-size:var(--text-sm)}
.sessions-bulk{display:flex;flex-wrap:wrap;align-items:center;gap:var(--space-2);padding:8px 12px;border-radius:var(--radius-md);background:var(--accent-soft);font-size:var(--text-sm);color:var(--accent-ink)}
.sessions-bulk-warning{color:var(--danger-ink)}
.sessions-table{border-radius:var(--radius-lg);background:var(--surface);box-shadow:var(--shadow-card);overflow:hidden}
.sessions-row{display:grid;grid-template-columns:40px minmax(0,2.4fr) minmax(0,1fr) 120px 110px 44px;align-items:center;gap:var(--space-2);min-height:52px;padding:0 var(--space-2) 0 var(--space-3);border-bottom:1px solid var(--border);font-size:var(--text-sm)}
.sessions-row:last-of-type{border-bottom:0}
.sessions-row.head{min-height:38px;font-size:var(--text-xs);font-weight:550;color:var(--muted);background:var(--sunken)}
.sessions-row:not(.head):hover{background:var(--sunken)}
.sessions-row.checked{background:var(--accent-soft)}
.sessions-row.highlight{box-shadow:inset 3px 0 0 var(--accent)}
.sessions-row.busy{opacity:.55}
.sessions-row input[type=checkbox]{width:16px;height:16px;cursor:pointer}
.sessions-title{display:flex;align-items:center;gap:10px;min-width:0;padding:6px 0;border:0;background:none;text-align:left;cursor:pointer;color:inherit}
.sessions-title>span{display:flex;flex-direction:column;min-width:0}
.sessions-title strong{font-weight:550;color:var(--ink);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.sessions-title small{font-size:var(--text-xs);color:var(--muted)}
.sessions-title:hover strong{color:var(--accent-ink)}
.sessions-row.current .sessions-title strong{color:var(--accent-ink)}
.sessions-muted{color:var(--ink-soft);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.status-pill{display:inline-flex;align-items:center;gap:6px;padding:2px 8px;border-radius:var(--radius-full);background:var(--fill);color:var(--ink-soft);font-size:var(--text-xs)}
.status-pill i{width:6px;height:6px;border-radius:50%;background:var(--muted)}
.status-pill.running i,.status-pill.starting i{background:var(--ok)}
.status-pill.waiting{background:var(--warn-soft);color:var(--warn-ink)}.status-pill.waiting i{background:var(--warn)}
.status-pill.failed{background:var(--danger-soft);color:var(--danger-ink)}.status-pill.failed i{background:var(--danger)}
.sessions-empty{padding:var(--space-6);text-align:center;color:var(--muted);font-size:var(--text-sm)}
@media (max-width:760px){
  .sessions-row{grid-template-columns:32px minmax(0,1fr) auto 40px;padding-left:var(--space-2)}
  .sessions-row>:nth-child(3),.sessions-row>:nth-child(5){display:none}
  .sessions-row{min-height:60px}
  .sessions-filters select{flex:1 1 140px;max-width:none;height:40px}
}
.session-row-preview{display:block;max-width:100%;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:var(--muted)}
</style>
