<script setup lang="ts">
import { computed, inject, nextTick, onBeforeUnmount, ref } from 'vue';
import type { GitBranches, GitCleanup, GitCleanupCandidate, GitCleanupOutcome, GitOutcome, GitRef, GitWorktree } from '@tendly/protocol';
import { ApiError, errorMessage, json, request, workspacePath } from './api';
import { claimPrune } from './branch-prune';
import { relativeTime } from './git-model';
import { SESSION_ACTIONS } from './session-actions-context';
import { useI18n } from '../i18n';

/** When each workspace was last pruned, shared across every menu instance so
 * the status bar, sidebar and Git pane throttle against one clock. */
const prunedAt = new Map<string, number>();

/**
 * The workspace directory's branch, and managing the repository's branches
 * and worktrees.
 *
 * Shown wherever a workspace is named -- the status bar, the sidebar, the Git
 * pane -- as one control, so the three never disagree. Branches are grouped
 * by what can be done with them: the current one; recent ones a click
 * switches to; those a session's worktree holds, which open that session;
 * and those whose work is already on the base branch (merged, squashed or
 * rebased) or whose upstream is gone, which can be cleaned up. Right-click or
 * the row's "⋯" opens what else can be done to a branch (merge, rebase,
 * rename, delete) or a worktree (remove). A remote branch checks out as a
 * local branch that tracks it. A session on a branch of its own moves from
 * its own chip instead.
 */
type Repo = GitBranches;
const props = defineProps<{ workspaceId: string; branch?: string | null; compact?: boolean }>();
const emit = defineEmits<{ switched: []; changed: [] }>();
const { t } = useI18n();
const open = ref(false), busy = ref(false), error = ref(''), created = ref(''), note = ref(''), filter = ref('');
const repo = ref<Repo>();
const anchor = ref<HTMLElement>(), panel = ref<HTMLElement>();
const place = ref<Record<string, string>>({});
/** The row whose actions are showing, and what is being done to it. */
const actions = ref<{ kind: 'branch' | 'worktree' | 'remote'; key: string }>();
const renaming = ref(''), renameTo = ref('');
const confirming = ref<{ kind: 'delete' | 'remove' | 'remote'; key: string; force: boolean; reason?: string }>();

const sessionActions = inject(SESSION_ACTIONS, null);
const cleanup = ref<GitCleanup>();
const searching = ref(false), creating = ref(false), showAll = ref(false), sweeping = ref(false);
const sweepSkipped = ref<Array<{ name: string; reason?: string }>>([]);
const worktrees = computed(() => (repo.value?.worktrees ?? []).filter(tree => !tree.main));
const matches = (name: string) => !filter.value.trim() || name.toLowerCase().includes(filter.value.trim().toLowerCase());
const details = computed(() => new Map((repo.value?.local ?? []).map(entry => [entry.name, entry])));
const candidates = computed(() => new Map((cleanup.value?.branches ?? []).map(entry => [entry.name, entry])));
const treeOf = (name: string) => worktrees.value.find(tree => tree.branch === name);
/** Most recently committed first, as Git lists them; a branch Git gave no details for goes last. */
const ordered = computed(() => {
  const known = (repo.value?.local ?? []).map(entry => entry.name);
  return [...known, ...(repo.value?.branches ?? []).filter(name => !known.includes(name))];
});
const recent = computed(() => ordered.value.filter(name => name !== repo.value?.current && !treeOf(name) && !candidates.value.has(name) && matches(name)));
const RECENT_LIMIT = 8;
const recentShown = computed(() => showAll.value || filter.value.trim() ? recent.value : recent.value.slice(0, RECENT_LIMIT));
const held = computed(() => worktrees.value.filter(tree => !(tree.branch && candidates.value.has(tree.branch)) && matches(tree.branch ?? tree.path)));
const cleanable = computed(() => (cleanup.value?.branches ?? []).filter(entry => matches(entry.name)));
/** What one press cleans up: only work found on the base, never a branch whose upstream is merely gone. */
const merged = computed(() => (cleanup.value?.branches ?? []).filter(entry => entry.reason !== 'gone'));
/** The menu top to bottom; a group with nothing in it is left out. */
const groups = computed(() => {
  const current = repo.value?.current;
  return [
    { key: 'current', names: current && matches(current) ? [current] : [] },
    { key: 'recent', names: recentShown.value },
    { key: 'held', names: held.value.map(tree => tree.path) },
    { key: 'clean', names: cleanable.value.map(entry => entry.name) },
  ].filter(group => group.names.length);
});
const remotes = computed(() => (repo.value?.remote ?? []).filter(entry => matches(entry.name)));
const showRemotes = ref(false);
const age = (name: string) => { const date = details.value.get(name)?.date; return date ? relativeTime(date, t) : ''; };
/** A one-commit squash is the same patch as its commit, so it is found the
 * way a rebase is; both read as merged, with how in the tooltip. */
const REASONS: Record<GitCleanupCandidate['reason'], [label: string, how: string]> = {
  merged: ['Merged', 'Merged into {base}'],
  squashed: ['Merged (squash)', 'Squashed into {base}'],
  rebased: ['Merged', 'The same changes are on {base}'],
  gone: ['Remote deleted · not merged', 'Its upstream branch was deleted, and its changes were not found on {base}'],
};
/** The sessions that work in a worktree, most recently active first. */
function sessionsIn(tree: GitWorktree) {
  return (sessionActions?.all() ?? []).filter(session => session.workspace_id === props.workspaceId && session.checkout_path === tree.path && !session.archived_at)
    .sort((a, b) => (b.updated_at ?? '').localeCompare(a.updated_at ?? ''));
}
function openTree(tree: GitWorktree) {
  const session = sessionsIn(tree)[0]; if (!session || !sessionActions) return;
  close(); sessionActions.open(session);
}
/** `origin/feature/x` checks out as `feature/x`. */
const localFor = (remote: string) => remote.slice(remote.indexOf('/') + 1);
const remoteOf = (remote: string) => remote.slice(0, remote.indexOf('/'));
const track = (entry?: GitRef) => entry ? [entry.ahead ? `↑${entry.ahead}` : '', entry.behind ? `↓${entry.behind}` : ''].filter(Boolean).join(' ') : '';
/** Git checks a branch out in one place at a time; one a worktree holds cannot come here. */
const heldElsewhere = (name: string) => worktrees.value.some(tree => tree.branch === name);
// Worktree paths are the host's own: `\`-separated on a Windows server.
const shortPath = (path: string) => path.split(/[\\/]/).slice(-2).join('/');

async function load() {
  void loadCleanup();
  try { repo.value = await request<Repo>(`${workspacePath(props.workspaceId)}/git/branches`); await nextTick(); position(); }
  catch (cause) { error.value = errorMessage(cause); }
}
/** Read apart from the list, since it runs a few Git commands per branch; the list shows without it. */
async function loadCleanup() {
  try { cleanup.value = await request<GitCleanup>(`${workspacePath(props.workspaceId)}/git/branches/cleanup`); }
  catch { cleanup.value = undefined; }
}
/** Deletes every branch found on the base, and a clean worktree holding one;
 * the server checks each again and says why it left any. */
async function sweep() {
  if (busy.value) return;
  busy.value = true; error.value = ''; note.value = ''; sweepSkipped.value = [];
  try {
    const outcome = await request<GitCleanupOutcome>(`${workspacePath(props.workspaceId)}/git/branches/cleanup`, json('POST', { branches: merged.value.map(entry => entry.name) }));
    repo.value = outcome.branches; sweeping.value = false;
    const removed = outcome.results.filter(result => result.removed).length;
    sweepSkipped.value = outcome.results.filter(result => !result.removed);
    note.value = t(removed === 1 ? 'Cleaned up {count} branch' : 'Cleaned up {count} branches', { count: removed });
    emit('changed'); await loadCleanup();
  } catch (cause) { error.value = errorMessage(cause); }
  finally { busy.value = false; }
}
/** A background `fetch --prune` on open so branches deleted on the remote stop
 * lingering, then a reload to show it -- throttled per workspace, and silent:
 * offline, no remote or a declined credential just leaves the cached list. */
async function prune() {
  if (!claimPrune(prunedAt, props.workspaceId, Date.now())) return;
  try { await request(`${workspacePath(props.workspaceId)}/git/fetch`, json('POST', {})); if (open.value) await load(); }
  catch { /* keep the list we have */ }
}
async function toggle() {
  if (open.value) { close(); return; }
  open.value = true; error.value = ''; resetRow(); filter.value = ''; searching.value = false; creating.value = false; showAll.value = false;
  await nextTick(); position(); void load(); void prune();
  document.addEventListener('pointerdown', outside, true);
}
/** Esc on the chip itself, where focus stays after opening, closes the menu too. */
function anchorEscape(event: KeyboardEvent) { if (!open.value) return; event.stopPropagation(); event.preventDefault(); escape(); }
function close() { open.value = false; resetRow(); document.removeEventListener('pointerdown', outside, true); }
function resetRow() { actions.value = undefined; renaming.value = ''; confirming.value = undefined; blocking.value = undefined; sweeping.value = false; sweepSkipped.value = []; note.value = ''; }
/** Esc steps back one level: an open question or form first, then the menu. */
function escape() {
  if (actions.value || renaming.value || confirming.value || blocking.value || sweeping.value) resetRow();
  else if (creating.value) creating.value = false;
  else if (searching.value) { searching.value = false; filter.value = ''; }
  else close();
}
async function beginSearch() { searching.value = !searching.value; if (!searching.value) filter.value = ''; await nextTick(); panel.value?.querySelector<HTMLInputElement>('.ws-filter')?.focus(); }
async function beginCreate() { creating.value = true; await nextTick(); panel.value?.querySelector<HTMLInputElement>('.ws-create input')?.focus(); }
function outside(event: Event) {
  const target = event.target as Node;
  if (!anchor.value?.contains(target) && !panel.value?.contains(target)) close();
}
/** Opens toward whichever side of the anchor has room: up from the status bar, down from the sidebar. */
function position() {
  const box = anchor.value?.getBoundingClientRect(); if (!box) return;
  const width = Math.min(340, window.innerWidth - 16), left = Math.max(8, Math.min(box.left, window.innerWidth - width - 8));
  const below = window.innerHeight - box.bottom, above = box.top;
  place.value = below >= 280 || below >= above
    ? { left: left + 'px', top: box.bottom + 6 + 'px', width: width + 'px', maxHeight: below - 16 + 'px' }
    : { left: left + 'px', bottom: window.innerHeight - box.top + 6 + 'px', width: width + 'px', maxHeight: above - 16 + 'px' };
}
async function run(work: () => Promise<Repo | unknown>, after?: () => void) {
  if (busy.value) return false;
  busy.value = true; error.value = '';
  try { const result = await work(); if (result && typeof result === 'object' && 'branches' in result) repo.value = result as Repo; else await load(); resetRow(); after?.(); emit('changed'); return true; }
  catch (cause) { error.value = errorMessage(cause); return false; }
  finally { busy.value = false; }
}
/** Sessions in the workspace directory that a switch would change the files under. */
const blocking = ref<{ branch: string; create: boolean; start?: string; titles: string[] }>();
async function switchTo(branch: string, create = false, stopSessions = false, start?: string) {
  branch = branch.trim();
  if (!branch || branch === repo.value?.current || heldElsewhere(branch) || busy.value) return;
  busy.value = true; error.value = '';
  try {
    await request(`${workspacePath(props.workspaceId)}/git/switch`, json('POST', { branch, create, stop_sessions: stopSessions, ...(start ? { start_point: start } : {}) }));
    blocking.value = undefined; created.value = ''; close(); emit('switched'); emit('changed');
  } catch (cause) {
    const titles = cause instanceof ApiError && cause.status === 409 && !stopSessions ? await runningHere() : [];
    if (titles.length) blocking.value = { branch, create, start, titles };
    else error.value = errorMessage(cause);
  } finally { busy.value = false; }
}
async function runningHere() {
  try {
    const sessions = await request<Array<{ title: string; status: string; checkout_path?: string | null }>>(`/sessions?workspace_id=${encodeURIComponent(props.workspaceId)}`);
    return sessions.filter(session => !session.checkout_path && ['running', 'starting', 'waiting'].includes(session.status)).map(session => session.title);
  } catch { return []; }
}
/** A remote branch: its local branch if there is one, else a new one tracking it. */
function checkoutRemote(name: string) {
  const local = localFor(name);
  if (repo.value?.branches.includes(local)) void switchTo(local);
  else void switchTo(local, true, false, name);
}
/** Merge a branch into the current one, or rebase the current one onto it. */
async function integrate(kind: 'merge' | 'rebase', rev: string) {
  if (busy.value) return;
  busy.value = true; error.value = ''; note.value = ''; actions.value = undefined;
  try {
    const outcome = await request<GitOutcome>(`${workspacePath(props.workspaceId)}/git/${kind}`, json('POST', { rev }));
    note.value = outcome.conflicts ? t('Stopped on conflicts. Resolve them under Changes in the Git pane, then continue.') : t(kind === 'merge' ? 'Merged {branch}' : 'Rebased onto {branch}', { branch: rev });
    emit('changed'); await load();
  } catch (cause) { error.value = errorMessage(cause); }
  finally { busy.value = false; }
}
function showActions(kind: 'branch' | 'worktree' | 'remote', key: string) { renaming.value = ''; confirming.value = undefined; actions.value = actions.value?.key === key && actions.value.kind === kind ? undefined : { kind, key }; }
function beginRename(name: string) { actions.value = undefined; renaming.value = name; renameTo.value = name; }
async function rename() {
  const from = renaming.value, to = renameTo.value.trim();
  if (!to || to === from) { renaming.value = ''; return; }
  await run(() => request(`${workspacePath(props.workspaceId)}/git/branches/rename`, json('POST', { from, to })), () => { if (from === repo.value?.current || from === props.branch) emit('switched'); });
}
/** Deleting and removing ask first; if Git refuses for a reason force would
 * override -- unmerged commits, uncommitted files -- that reason is shown
 * and the second press is the forced one. */
async function confirm() {
  const target = confirming.value; if (!target) return;
  busy.value = true; error.value = '';
  if (target.kind === 'remote') {
    try { await request(`${workspacePath(props.workspaceId)}/git/push`, json('POST', { remote: remoteOf(target.key), branch: localFor(target.key), delete: true })); resetRow(); await load(); emit('changed'); }
    catch (cause) { error.value = errorMessage(cause); }
    finally { busy.value = false; }
    return;
  }
  const url = target.kind === 'delete' ? 'git/branches/delete' : 'git/worktrees/remove';
  const body = target.kind === 'delete' ? { branch: target.key, force: target.force } : { path: target.key, force: target.force };
  try { repo.value = await request<Repo>(`${workspacePath(props.workspaceId)}/${url}`, json('POST', body)); resetRow(); emit('changed'); }
  catch (cause) {
    const reason = errorMessage(cause);
    if (!target.force && /not fully merged|contains modified or untracked|is dirty|untracked files/i.test(reason)) confirming.value = { ...target, force: true, reason };
    else error.value = reason;
  } finally { busy.value = false; }
}
onBeforeUnmount(close);
</script>

<template>
  <button ref="anchor" type="button" :class="['ws-branch', { compact }]" :aria-expanded="open" :title="t('Branch of the workspace directory · click to switch')" @click.stop="toggle" @keydown.esc="anchorEscape" @contextmenu.prevent.stop="open || toggle()">
    <svg viewBox="0 0 16 16" width="11" height="11" aria-hidden="true"><path d="M5 3v7M5 10a2 2 0 1 0 0 4 2 2 0 0 0 0-4Zm0-7a2 2 0 1 0 0-.01M11 5a2 2 0 1 0 0-.01M11 7c0 2-2 3-6 3" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg>
    <span>{{ branch || t('Detached HEAD') }}</span>
  </button>
  <Teleport to="body">
    <div v-if="open" ref="panel" class="ws-branch-panel" :style="place" role="dialog" :aria-label="t('Switch the workspace branch')" @keydown.esc.stop="escape">
      <header class="ws-head">
        <span>{{ t('Workspace directory branch') }}</span>
        <span class="ws-tip" tabindex="0" role="note" :title="t('Switches the directory every session here shares. A session in its own worktree is not affected. Right-click a branch or worktree for more.')" :aria-label="t('Switches the directory every session here shares. A session in its own worktree is not affected. Right-click a branch or worktree for more.')">?</span>
        <button type="button" class="ws-icon" :aria-pressed="searching" :aria-label="t('Filter branches')" :title="t('Filter branches')" @click="beginSearch"><svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true"><circle cx="7" cy="7" r="4.5" fill="none" stroke="currentColor" stroke-width="1.5"/><path d="m10.5 10.5 3 3" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg></button>
      </header>
      <input v-if="searching" v-model="filter" class="ws-filter" :placeholder="t('Filter branches')" :aria-label="t('Filter branches')" spellcheck="false" autocomplete="off" />
      <p v-if="error" class="ws-branch-error" role="alert">{{ error }}</p>
      <div v-else-if="note" class="ws-branch-ok" role="status">{{ note }}<ul v-if="sweepSkipped.length"><li v-for="skip in sweepSkipped" :key="skip.name"><b>{{ skip.name }}</b> · {{ t(skip.reason ?? '') }}</li></ul></div>
      <div v-if="blocking" class="ws-confirm ws-blocking" role="alertdialog">
        <p>{{ t('Switching to {branch} changes the files under these running sessions:', { branch: blocking.branch }) }}</p>
        <ul><li v-for="(title, index) in blocking.titles" :key="index">{{ title }}</li></ul>
        <p class="ws-branch-note">{{ t('End them and switch, or move a session to its own branch from its branch chip.') }}</p>
        <div><button type="button" class="ws-btn danger solid" :disabled="busy" @click="switchTo(blocking.branch, blocking.create, true, blocking.start)">{{ t('End them and switch') }}</button><button type="button" class="ws-btn quiet" @click="blocking = undefined">{{ t('Cancel') }}</button></div>
      </div>
      <p v-if="!repo && !error" class="ws-branch-note">{{ t('Loading…') }}</p>
      <template v-else-if="repo">
        <template v-for="group in groups" :key="group.key">
          <div v-if="group.key === 'clean'" class="ws-group-head">
            <span :title="cleanup?.base ? t('Their work is already on {branch}', { branch: cleanup.base }) : undefined">{{ t('Can be cleaned up') }} · {{ cleanable.length }}</span>
            <button v-if="merged.length" type="button" class="ws-btn quiet ws-sweep" :disabled="busy" @click="resetRow(); sweeping = true">{{ t('Clean up merged') }}</button>
          </div>
          <div v-else-if="group.key === 'held'" class="ws-group-head"><span>{{ t('In their own directory') }} · {{ held.length }}</span></div>
          <div v-else-if="group.key === 'recent'" class="ws-group-head"><span>{{ t('Recent') }}</span></div>
          <div v-if="group.key === 'clean' && sweeping" class="ws-confirm" role="alertdialog">
            <p>{{ t(merged.length === 1 ? 'Delete {count} branch whose work is already on {base}?' : 'Delete {count} branches whose work is already on {base}?', { count: merged.length, base: cleanup?.base ?? '' }) }}<template v-if="merged.some(entry => entry.worktree)"> {{ t('A worktree holding one goes too, unless a session runs there or it has uncommitted changes.') }}</template></p>
            <div><button type="button" class="ws-btn danger solid" :disabled="busy" @click="sweep">{{ t('Clean up') }}</button><button type="button" class="ws-btn quiet" @click="sweeping = false">{{ t('Cancel') }}</button></div>
          </div>
          <template v-if="group.key === 'held'">
            <div v-for="tree in held" :key="tree.path" class="ws-row" @contextmenu.prevent="showActions('worktree', tree.path)">
              <form v-if="tree.branch && renaming === tree.branch" class="ws-rename" @submit.prevent="rename"><input v-model="renameTo" :aria-label="t('New name for {branch}', { branch: tree.branch })" maxlength="200" spellcheck="false" autocomplete="off" @vue:mounted="({ el }: { el: HTMLInputElement }) => { el.focus(); el.select(); }" /><button type="submit" class="ws-btn primary" :disabled="busy">{{ t('Rename') }}</button><button type="button" class="ws-btn quiet" @click="renaming=''">{{ t('Cancel') }}</button></form>
              <template v-else>
                <button type="button" class="ws-row-main" :disabled="!sessionsIn(tree).length" :title="sessionsIn(tree).length ? t('Open {session}', { session: sessionsIn(tree)[0].title }) : tree.path" @click="openTree(tree)">
                  <span><i class="ws-glyph" aria-hidden="true">⎇</i>{{ tree.branch || t('Detached HEAD') }}</span>
                  <small v-if="sessionsIn(tree).length" class="ws-session">{{ sessionsIn(tree)[0].title }}<template v-if="sessionsIn(tree).length > 1"> +{{ sessionsIn(tree).length - 1 }}</template> →</small>
                  <small v-else class="ws-path">{{ shortPath(tree.path) }}</small>
                </button>
                <button type="button" class="ws-more" :aria-label="t('More actions for {name}', { name: tree.branch || tree.path })" :aria-expanded="actions?.kind === 'worktree' && actions.key === tree.path" @click="showActions('worktree', tree.path)">⋯</button>
              </template>
              <div v-if="actions?.kind === 'worktree' && actions.key === tree.path" class="ws-actions" role="menu">
                <button v-if="tree.branch" type="button" role="menuitem" class="ws-btn" @click="beginRename(tree.branch!)">{{ t('Rename branch…') }}</button>
                <button type="button" role="menuitem" class="ws-btn danger" @click="actions = undefined; confirming = { kind: 'remove', key: tree.path, force: false }">{{ t('Remove worktree…') }}</button>
              </div>
              <div v-if="confirming?.kind === 'remove' && confirming.key === tree.path" class="ws-confirm" role="alertdialog">
                <p>{{ confirming.force ? t('This worktree has uncommitted changes. Removing it anyway deletes them.') : t('Remove this worktree? Its directory is deleted; the branch stays. Stopped sessions in it move back to the workspace directory.') }}</p>
                <div><button type="button" class="ws-btn danger solid" :disabled="busy" @click="confirm">{{ t(confirming.force ? 'Remove anyway' : 'Remove') }}</button><button type="button" class="ws-btn quiet" @click="confirming = undefined">{{ t('Cancel') }}</button></div>
              </div>
            </div>
          </template>
          <div v-for="name in group.names" v-else :key="name" class="ws-row" :class="{ current: name === repo.current }" @contextmenu.prevent="showActions('branch', name)">
            <form v-if="renaming === name" class="ws-rename" @submit.prevent="rename"><input v-model="renameTo" :aria-label="t('New name for {branch}', { branch: name })" maxlength="200" spellcheck="false" autocomplete="off" @vue:mounted="({ el }: { el: HTMLInputElement }) => { el.focus(); el.select(); }" /><button type="submit" class="ws-btn primary" :disabled="busy">{{ t('Rename') }}</button><button type="button" class="ws-btn quiet" @click="renaming=''">{{ t('Cancel') }}</button></form>
            <template v-else>
              <button type="button" class="ws-row-main" :disabled="busy || !!treeOf(name)" :title="treeOf(name) ? t('Checked out in another worktree') : name === repo.current ? t('Current') : t('Switch this checkout to {branch}', { branch: name })" @click="switchTo(name)">
                <span><i v-if="treeOf(name)" class="ws-glyph" aria-hidden="true">⎇</i>{{ name }}</span>
                <small v-if="candidates.get(name)" :class="['ws-reason', candidates.get(name)!.reason]" :title="t(REASONS[candidates.get(name)!.reason][1], { base: cleanup?.base ?? '' })">{{ t(REASONS[candidates.get(name)!.reason][0]) }}</small>
                <small v-else class="ws-meta"><b v-if="track(details.get(name))" class="ws-track" :title="details.get(name)?.upstream">{{ track(details.get(name)) }}</b>{{ age(name) }}</small>
              </button>
              <button type="button" class="ws-more" :aria-label="t('More actions for {name}', { name })" :aria-expanded="actions?.kind === 'branch' && actions.key === name" @click="showActions('branch', name)">⋯</button>
            </template>
            <div v-if="actions?.kind === 'branch' && actions.key === name" class="ws-actions" role="menu">
              <button v-if="repo.current && name !== repo.current" type="button" role="menuitem" class="ws-btn" :disabled="busy" @click="integrate('merge', name)">{{ t('Merge into {branch}', { branch: repo.current }) }}</button>
              <button v-if="repo.current && name !== repo.current" type="button" role="menuitem" class="ws-btn" :disabled="busy" @click="integrate('rebase', name)">{{ t('Rebase onto this') }}</button>
              <button type="button" role="menuitem" class="ws-btn" @click="beginRename(name)">{{ t('Rename…') }}</button>
              <button type="button" role="menuitem" class="ws-btn danger" :disabled="name === repo.current || !!treeOf(name)" :title="name === repo.current || treeOf(name) ? t('A branch that is checked out cannot be deleted') : undefined" @click="actions = undefined; confirming = { kind: 'delete', key: name, force: false }">{{ t('Delete branch…') }}</button>
            </div>
            <div v-if="confirming?.kind === 'delete' && confirming.key === name" class="ws-confirm" role="alertdialog">
              <p>{{ confirming.force ? t('{branch} has commits not merged anywhere else. Deleting it anyway loses them.', { branch: name }) : t('Delete branch {branch}?', { branch: name }) }}</p>
              <div><button type="button" class="ws-btn danger solid" :disabled="busy" @click="confirm">{{ t(confirming.force ? 'Delete anyway' : 'Delete') }}</button><button type="button" class="ws-btn quiet" @click="confirming = undefined">{{ t('Cancel') }}</button></div>
            </div>
          </div>
          <button v-if="group.key === 'recent' && recentShown.length < recent.length" type="button" class="ws-section-toggle ws-show-all" @click="showAll = true">{{ t('Show all {count}', { count: recent.length }) }}</button>
        </template>
        <section v-if="repo.remote?.length" class="ws-trees ws-remotes">
          <button type="button" class="ws-section-toggle" :aria-expanded="showRemotes || !!filter.trim()" @click="showRemotes = !showRemotes"><span :class="{ open: showRemotes || !!filter.trim() }">›</span>{{ t('Remote branches') }} <small>{{ repo.remote.length }}</small></button>
          <template v-if="showRemotes || filter.trim()">
            <div v-for="entry in remotes" :key="entry.name" class="ws-row" @contextmenu.prevent="showActions('remote', entry.name)">
              <button type="button" class="ws-row-main" :disabled="busy" :title="t('Check out {branch} as a local branch that tracks it', { branch: entry.name })" @click="checkoutRemote(entry.name)"><span>{{ entry.name }}</span><small v-if="repo.branches.includes(localFor(entry.name))">{{ t('Local') }}</small></button>
              <button type="button" class="ws-more" :aria-label="t('More actions for {name}', { name: entry.name })" :aria-expanded="actions?.kind === 'remote' && actions.key === entry.name" @click="showActions('remote', entry.name)">⋯</button>
              <div v-if="actions?.kind === 'remote' && actions.key === entry.name" class="ws-actions" role="menu">
                <button v-if="repo.current" type="button" role="menuitem" class="ws-btn" :disabled="busy" @click="integrate('merge', entry.name)">{{ t('Merge into {branch}', { branch: repo.current }) }}</button>
                <button v-if="repo.current" type="button" role="menuitem" class="ws-btn" :disabled="busy" @click="integrate('rebase', entry.name)">{{ t('Rebase onto this') }}</button>
                <button type="button" role="menuitem" class="ws-btn danger" @click="actions = undefined; confirming = { kind: 'remote', key: entry.name, force: false }">{{ t('Delete from remote…') }}</button>
              </div>
              <div v-if="confirming?.kind === 'remote' && confirming.key === entry.name" class="ws-confirm" role="alertdialog">
                <p>{{ t('Delete {branch} on {remote}? Everyone sharing the remote loses it.', { branch: localFor(entry.name), remote: remoteOf(entry.name) }) }}</p>
                <div><button type="button" class="ws-btn danger solid" :disabled="busy" @click="confirm">{{ t('Delete') }}</button><button type="button" class="ws-btn quiet" @click="confirming = undefined">{{ t('Cancel') }}</button></div>
              </div>
            </div>
          </template>
        </section>
        <form v-if="creating" class="ws-create" @submit.prevent="switchTo(created, true)"><input v-model="created" :placeholder="t('New branch name')" maxlength="200" spellcheck="false" autocomplete="off" :aria-label="t('New branch name')" /><button type="submit" class="ws-btn primary" :disabled="busy || !created.trim()">{{ t('Create') }}</button></form>
        <button v-else type="button" class="ws-section-toggle ws-new" @click="beginCreate"><span>＋</span>{{ t('New branch…') }}</button>
      </template>
    </div>
  </Teleport>
</template>

<style scoped>
.ws-branch{display:inline-flex;align-items:center;gap:4px;min-width:0;max-width:160px;padding:1px 6px;border:1px solid transparent;border-radius:var(--radius-sm);background:none;color:inherit;font:inherit;cursor:pointer}
.ws-branch span{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.ws-branch:hover,.ws-branch[aria-expanded=true]{background:var(--fill-hover);color:var(--ink);border-color:var(--line)}
.ws-branch.compact{font-size:var(--text-xs);color:var(--ink-soft);background:var(--fill);max-width:110px}
/* Dressed like the composer's chip menus (ChipMenu.vue): a quiet heading, rows
   that lift on hover, the current one in the selected tone. */
.ws-branch-panel{position:fixed;z-index:1200;overflow-y:auto;padding:5px;border:1px solid var(--border);border-radius:var(--radius-lg);background:var(--surface);box-shadow:var(--shadow-lg),inset 0 1px 0 var(--card-highlight);font-size:var(--text-sm);color:var(--ink);text-align:left}
.ws-head{display:flex;align-items:center;gap:4px;padding:6px 6px 4px 10px;font-size:var(--text-xs);font-weight:600;color:var(--ink-soft)}
.ws-head>span:first-child{flex:1}
.ws-tip{display:inline-grid;place-items:center;width:15px;height:15px;margin-right:4px;border:1px solid var(--line);border-radius:50%;font-size:var(--text-xs);font-weight:600;color:var(--muted);cursor:help}
.ws-icon{display:inline-grid;place-items:center;width:24px;height:24px;border:0;border-radius:var(--radius-sm);background:none;color:var(--muted);cursor:pointer}
.ws-icon:hover,.ws-icon[aria-pressed=true]{background:var(--fill);color:var(--ink)}
.ws-branch-panel>p{margin:0 10px 6px;font-size:var(--text-xs);line-height:1.6;color:var(--muted)}
.ws-branch-panel .ws-branch-ok{margin:0 3px 6px;font-size:var(--text-xs);line-height:1.6;color:var(--ok-ink);background:var(--ok-soft);border-radius:var(--radius-sm);padding:6px 8px}
.ws-branch-ok ul{margin:4px 0 0;padding-left:16px;color:var(--ink-soft)}
.ws-group-head{display:flex;align-items:center;justify-content:space-between;gap:8px;min-height:26px;margin-top:6px;padding:0 4px 0 10px;font-size:var(--text-xs);color:var(--muted)}
.ws-sweep{height:22px;padding:0 8px;color:var(--accent-ink)}
.ws-glyph{margin-right:5px;font-style:normal;color:var(--muted)}
.ws-meta{display:inline-flex;gap:6px}
.ws-meta b{font-weight:400}
.ws-session{max-width:45%;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:var(--accent-ink)!important}
.ws-path{font-family:var(--mono)!important}
.ws-reason.merged,.ws-reason.squashed,.ws-reason.rebased{color:var(--ok-ink)!important}
.ws-reason.gone{color:var(--warn-ink)!important}
.ws-row.current .ws-row-main span::before{content:'';display:inline-block;width:6px;height:6px;margin-right:7px;border-radius:50%;background:currentColor;vertical-align:middle}
.ws-new{margin-top:5px;padding-top:8px;border-top:1px solid var(--line);border-radius:0 0 var(--radius-md) var(--radius-md)}
.ws-new>span{color:var(--muted)}
.ws-show-all{padding-left:20px}
.ws-filter{display:block;width:calc(100% - 6px);height:30px;margin:0 3px 5px;padding:0 8px;border:1px solid var(--line);border-radius:var(--radius-sm);background:var(--surface);color:var(--ink);font:inherit;font-size:var(--text-xs)}
.ws-track{color:var(--accent-ink)!important;font-family:var(--mono)!important}
.ws-section-toggle{display:flex;align-items:center;gap:6px;width:100%;margin:2px 0;padding:6px 10px;border:0;border-radius:var(--radius-md);background:none;font-size:var(--text-xs);color:var(--muted);cursor:pointer;text-align:left}
.ws-section-toggle:hover{background:var(--fill);color:var(--ink)}
.ws-section-toggle>span{display:inline-block;width:10px;transition:transform var(--duration-fast)}
.ws-section-toggle>span.open{transform:rotate(90deg)}
.ws-section-toggle small{color:var(--faint)}
.ws-branch-panel .ws-branch-error{color:var(--danger-ink);background:var(--danger-soft);border-radius:var(--radius-sm);padding:6px 8px}
.ws-row{display:flex;flex-wrap:wrap;align-items:center;border-radius:var(--radius-md)}
.ws-row+.ws-row{margin-top:1px}
.ws-row:hover{background:var(--fill)}
.ws-row.current{background:var(--selected)}
.ws-row-main{flex:1;min-width:0;display:flex;align-items:center;justify-content:space-between;gap:8px;min-height:36px;padding:8px 10px;border:0;border-radius:var(--radius-md);background:none;cursor:pointer;text-align:left;font:inherit;font-size:var(--text-sm);color:var(--ink-soft)}
.ws-row-main span{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.ws-row.current .ws-row-main{color:var(--accent-ink);font-weight:600;cursor:default}
.ws-row-main:disabled{cursor:not-allowed}
.ws-row:not(.current) .ws-row-main:disabled span{color:var(--muted)}
.ws-row-main small{flex-shrink:0;font-size:var(--text-xs);font-weight:400;color:var(--muted)}
.ws-more{flex-shrink:0;width:26px;height:26px;margin-right:2px;border:0;border-radius:var(--radius-sm);background:none;color:var(--muted);font-size:var(--text-base);line-height:1;cursor:pointer;opacity:0}
.ws-row:hover .ws-more,.ws-more:focus-visible,.ws-more[aria-expanded=true]{opacity:1}
.ws-more:hover{background:var(--fill-hover);color:var(--ink)}
.ws-actions{flex-basis:100%;display:flex;flex-wrap:wrap;gap:6px;padding:2px 8px 8px}
/* One set of buttons for the whole menu: neutral by default, filled only to
   commit a form, red only for what destroys something. */
.ws-btn{height:28px;padding:0 10px;border:1px solid var(--line);border-radius:var(--radius-sm);background:var(--surface);font-size:var(--text-xs);color:var(--ink-soft);cursor:pointer;white-space:nowrap}
.ws-btn:hover:not(:disabled){background:var(--fill);color:var(--ink)}
.ws-btn:disabled{opacity:.45;cursor:not-allowed}
.ws-btn.primary{background:var(--teal);border-color:var(--teal);color:var(--on-accent)}
.ws-btn.primary:hover:not(:disabled){background:var(--teal-deep);color:var(--on-accent)}
.ws-btn.danger{color:var(--danger-ink)}
.ws-btn.danger:hover:not(:disabled){background:var(--danger-soft);border-color:var(--danger-line);color:var(--danger-ink)}
.ws-btn.danger.solid{background:var(--danger);border-color:var(--danger);color:var(--on-accent)}
.ws-btn.danger.solid:hover:not(:disabled){background:var(--danger-ink);color:var(--on-accent)}
.ws-btn.quiet{border-color:transparent;background:none}
.ws-btn.quiet:hover:not(:disabled){background:var(--fill)}
.ws-confirm{flex-basis:100%;margin:2px 6px 8px;padding:8px 10px;border:1px solid var(--danger-line);border-radius:var(--radius-md);background:var(--danger-soft)}
.ws-confirm p{margin:0 0 8px;font-size:var(--text-xs);line-height:1.5;color:var(--danger-ink)}
.ws-confirm div{display:flex;gap:6px}
.ws-blocking{margin:0 0 8px}
.ws-blocking ul{margin:0 0 8px;padding-left:18px;font-size:var(--text-sm);color:var(--ink)}
.ws-blocking li{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.ws-blocking .ws-branch-note{color:var(--ink-soft)}
.ws-rename{flex:1;display:flex;gap:6px;padding:4px}
.ws-rename input,.ws-create input{flex:1;min-width:0;height:30px;padding:0 8px;border:1px solid var(--line);border-radius:var(--radius-sm);font:inherit;font-size:var(--text-xs);background:var(--surface);color:var(--ink)}
.ws-create{display:flex;gap:6px;margin:5px 3px 3px;padding-top:6px;border-top:1px solid var(--line)}
.ws-trees{margin-top:5px;padding-top:5px;border-top:1px solid var(--line)}
.ws-trees header{font-size:var(--text-xs);color:var(--muted);margin:4px 10px}
@media(pointer:coarse){.ws-more{opacity:1;width:36px;height:36px}.ws-row-main{min-height:44px}.ws-create input,.ws-rename input,.ws-btn{height:40px}}
</style>
