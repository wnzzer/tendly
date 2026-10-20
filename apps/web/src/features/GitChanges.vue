<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import { CHORDS, formatChord, isRemove, matches } from "./shortcuts";
import type { GitCommitDetail, GitDiff, GitFile } from "@tendly/protocol";
import { errorMessage } from "./api";
import { gitGet, gitPost, useGit } from "./git-context";
import { hunkPatch, isChangedFile, isConflictedFile, isStagedFile, pathsForGitFiles } from "./git-model";
import { gitViewState } from "./git-view-state";
import GitDiffLines from "./GitDiffLines.vue";
import Icon from "./Icon.vue";
import ContextMenu from "./ContextMenu.vue";
import { useMobile } from "./mobile";
import { useI18n } from "../i18n";

/**
 * The working tree: what changed, what is staged, what conflicts, and the
 * commit that takes the staged part. A file's diff opens beside the list, and
 * each of its hunks can be staged, unstaged or discarded on its own.
 */
const { t } = useI18n();
const git = useGit();
const emit = defineEmits<{ stash: [paths?: string[]] }>();
const status = git.status, busy = git.busy;
const viewState = computed(() => gitViewState(git.workspaceId()));
const selected = computed({ get: () => viewState.value.selected, set: value => { viewState.value.selected = value; } });
const message = computed({ get: () => viewState.value.message, set: value => { viewState.value.message = value; } });
const amend = computed({ get: () => !!viewState.value.amend, set: value => { viewState.value.amend = value; } });
const diff = ref<GitDiff>();
const diffLoading = ref(false), diffError = ref("");
const conflicted = computed(() => status.value.files.filter(isConflictedFile));
const stagedFiles = computed(() => status.value.files.filter(isStagedFile));
const changedFiles = computed(() => status.value.files.filter(isChangedFile));
const groups = computed(() => [{ title: "Staged changes", staged: true, files: stagedFiles.value }, { title: "Changes", staged: false, files: changedFiles.value }]);
const mobile = useMobile();
let diffRevision = 0, alive = true;

async function openDiff(path: string, staged: boolean) {
  const keepVisible = selected.value?.path === path && selected.value?.staged === staged && !!diff.value;
  selected.value = { path, staged };
  const requestId = ++diffRevision;
  diffLoading.value = !keepVisible; diffError.value = ""; if (!keepVisible) diff.value = undefined;
  try {
    const result = await gitGet<GitDiff>(git.workspaceId(), `diff?path=${encodeURIComponent(path)}&staged=${staged}`);
    if (alive && requestId === diffRevision && JSON.stringify(diff.value) !== JSON.stringify(result)) diff.value = result;
  } catch (cause) { if (alive && requestId === diffRevision) diffError.value = errorMessage(cause); }
  finally { if (alive && requestId === diffRevision) diffLoading.value = false; }
}
function show(path: string, staged: boolean) { void openDiff(path, staged); git.detailOpen.value = true; }
/**
 * The change list's keys: ↑ ↓ Home End move, Enter opens the diff, Space
 * stages or unstages, Delete (⌘⌫ on a Mac) asks before discarding.
 */
function fileKeydown(event: KeyboardEvent, file: GitFile, staged: boolean) {
  if (event.altKey || event.ctrlKey || (event.metaKey && !isRemove(event))) return;
  const own = event.currentTarget as HTMLElement;
  const rows = Array.from(own.closest(".git-changes-list, .dock-content, body")?.querySelectorAll<HTMLElement>(".git-file-open") ?? []);
  const index = rows.indexOf(own);
  const move = event.key === "ArrowDown" ? index + 1 : event.key === "ArrowUp" ? index - 1 : event.key === "Home" ? 0 : event.key === "End" ? rows.length - 1 : undefined;
  if (move !== undefined) { event.preventDefault(); rows[Math.max(0, Math.min(rows.length - 1, move))]?.focus(); return; }
  if (event.key === " ") {
    event.preventDefault();
    if (busy.value) return;
    // The row moves to the other group; the one after it takes its place under the focus.
    (rows[index + 1] ?? rows[index - 1])?.focus();
    void stage([file], staged);
    return;
  }
  if (isRemove(event) && !staged) { event.preventDefault(); discardTarget.value = [file]; }
}
/** After every status: keep the file being read if it is still listed, else take the first. */
function reconcile() {
  const choice = selected.value;
  const all = [...conflicted.value.map(file => ({ path: file.path, staged: false })), ...changedFiles.value.map(file => ({ path: file.path, staged: false })), ...stagedFiles.value.map(file => ({ path: file.path, staged: true }))];
  // A file just staged or unstaged is followed to its other group.
  const still = choice && (all.find(entry => entry.path === choice.path && entry.staged === choice.staged) ?? all.find(entry => entry.path === choice.path));
  const next = still ?? all[0];
  if (next) void openDiff(next.path, next.staged);
  else { selected.value = undefined; diff.value = undefined; diffRevision++; }
}
watch(status, reconcile, { immediate: true });

const stage = (files: GitFile[], unstage: boolean) => files.length ? git.run(() => gitPost(git.workspaceId(), unstage ? "unstage" : "stage", { paths: pathsForGitFiles(files) })) : undefined;
/**
 * Throwing away working-tree changes cannot be undone, so it always asks
 * first, naming how many files and whether any of them are new (new files are
 * deleted, not reverted). Only the unstaged group offers it: staged work is
 * never discarded from here.
 */
const discardTarget = ref<GitFile[]>(), discardBar = ref<HTMLElement>();
// Asking takes the focus, so Enter confirms and Esc keeps the changes.
watch(discardTarget, async files => { if (!files) return; await nextTick(); discardBar.value?.querySelector<HTMLElement>(".danger")?.focus(); });
const discardNew = computed(() => (discardTarget.value ?? []).filter(file => file.worktree === "?").length);
async function discard() {
  const files = discardTarget.value;
  if (!files?.length) return;
  await git.run(async () => { await gitPost(git.workspaceId(), "discard", { paths: files.map(file => file.path) }); discardTarget.value = undefined; });
}
const resolve = (file: GitFile, side: "ours" | "theirs") => git.run(() => gitPost(git.workspaceId(), "resolve", { path: file.path, side }));

/** One hunk of the diff on screen, applied to the index or the working tree. */
async function hunk(action: string, index: number) {
  const current = diff.value, choice = selected.value;
  const patch = current && hunkPatch(current.diff, index);
  if (!patch || !choice) return;
  if (action === "discard") {
    const answer = await git.ask({ title: t("Discard this hunk?"), description: t("The lines it changed in {path} go back to what is staged. This cannot be undone.", { path: choice.path }), confirm: t("Discard"), danger: true });
    if (!answer) return;
  }
  await git.run(() => gitPost(git.workspaceId(), "apply", { path: choice.path, patch, cached: action !== "discard", reverse: action !== "stage" }));
}
const hunkActions = computed(() => {
  const choice = selected.value, file = choice && status.value.files.find(entry => entry.path === choice.path);
  if (!choice || !file || diff.value?.binary || isConflictedFile(file) || file.worktree === "?") return [];
  return choice.staged ? [{ id: "unstage", label: t("Unstage hunk") }] : [{ id: "discard", label: t("Discard"), danger: true }, { id: "stage", label: t("Stage hunk") }];
});

const canCommit = computed(() => !!message.value.trim() && (stagedFiles.value.length > 0 || amend.value) && !conflicted.value.length);
/** `stageFirst`: stage these too, in the same action, before committing. */
async function commit(then?: "push", stageFirst: GitFile[] = []) {
  if (busy.value || !message.value.trim() || conflicted.value.length || (!stageFirst.length && !canCommit.value)) return;
  const draft = viewState.value, sent = message.value, amending = amend.value;
  await git.run(async () => {
    if (stageFirst.length) await gitPost(git.workspaceId(), "stage", { paths: pathsForGitFiles(stageFirst) });
    const done = await gitPost<{ commit: string }>(git.workspaceId(), "commit", { message: sent.trim(), amend: amending });
    if (draft.message === sent) { draft.message = ""; draft.amend = false; }
    if (then === "push") await gitPost(git.workspaceId(), "push", status.value.upstream ? {} : { set_upstream: true });
    return done;
  }, done => t(then === "push" ? "Committed {hash} and pushed · {message}" : "Committed {hash} · {message}", { hash: done.commit.slice(0, 10), message: sent.split("\n")[0] }));
}
/** Stage every change, new files included, and commit them. */
const commitAll = () => commit(undefined, changedFiles.value);
/** Amending starts from the last commit's message, unless one is being written. */
watch(amend, async on => {
  if (!on || message.value.trim()) return;
  try { const head = await gitGet<GitCommitDetail>(git.workspaceId(), "commit/detail?rev=HEAD"); if (amend.value && !message.value.trim()) message.value = head.message; } catch { /* No commit yet: nothing to amend from. */ }
});
async function undoCommit() {
  let head: GitCommitDetail;
  try { head = await gitGet<GitCommitDetail>(git.workspaceId(), "commit/detail?rev=HEAD"); } catch (cause) { await git.run(() => Promise.reject(cause)); return; }
  if (!head.parents.length) { await git.run(() => Promise.reject(new Error(t("The first commit has nothing before it to go back to.")))); return; }
  const answer = await git.ask({ title: t("Undo the last commit?"), description: t("{subject} is taken off the branch. Its changes stay, staged, and its message returns to the message box.", { subject: head.message.split("\n")[0] }), confirm: t("Undo commit") });
  if (!answer) return;
  await git.run(async () => { await gitPost(git.workspaceId(), "reset", { rev: head.parents[0], mode: "soft" }); if (!message.value.trim()) message.value = head.message; }, t("Undid {hash}. Its changes are staged.", { hash: head.hash.slice(0, 10) }));
}

const commitMenu = ref<{ x: number; y: number }>();
function openCommitMenu(event: MouseEvent) { const box = (event.currentTarget as HTMLElement).getBoundingClientRect(); commitMenu.value = { x: box.left, y: box.bottom + 4 }; }
/** Right-click on a changed file, or on a group of them. */
const fileMenu = ref<{ x: number; y: number; staged: boolean; conflict?: boolean; file?: GitFile }>();
function openFileMenu(event: MouseEvent, staged: boolean, file?: GitFile, conflict = false) { event.preventDefault(); event.stopPropagation(); fileMenu.value = { x: event.clientX, y: event.clientY, staged, file, conflict }; }
/** The action reads the menu's file as it starts, so the menu closes after. */
function menuAction(run: () => unknown) { void run(); fileMenu.value = undefined; commitMenu.value = undefined; }
const ignore = (file: GitFile) => git.run(() => gitPost(git.workspaceId(), "ignore", { path: file.path }), t("Added {path} to .gitignore", { path: file.path }));
const conflictCode = (file: GitFile) => file.index + file.worktree;
onBeforeUnmount(() => { alive = false; diffRevision++; });
</script>

<template>
  <div class="git-content git-changes">
    <div class="git-sidebar">
      <form class="commit-form" @submit.prevent="commit()">
        <textarea v-model="message" data-pane-input :aria-label="t('Commit message')" :placeholder="t(amend ? 'Message for the amended commit…' : 'Commit message…')" rows="2" :disabled="busy" @keydown="matches($event, CHORDS.confirm) && ($event.preventDefault(), commit())" />
        <label class="git-amend"><input v-model="amend" type="checkbox" :disabled="busy" />{{ t('Amend last commit') }}</label>
        <div class="git-commit-buttons">
          <button class="primary-button" :title="formatChord(CHORDS.confirm)" :disabled="busy || !canCommit"><Icon name="check" :size="14" />{{ busy ? t('Working…') : amend ? t('Amend commit') : t('Commit staged ({count})', { count: stagedFiles.length }) }}</button>
          <button type="button" class="primary-button git-commit-more" :aria-label="t('More commit actions')" :disabled="busy" @click="openCommitMenu"><Icon name="chevron" :size="12" /></button>
        </div>
      </form>
      <div v-if="discardTarget" ref="discardBar" class="confirmation-bar git-discard-confirm" role="alertdialog" @keydown.esc.stop.prevent="discardTarget=undefined"><span>{{ discardTarget.length===1 ? t('Discard changes to {path}? This cannot be undone.', { path: discardTarget[0].path }) : t('Discard changes to {count} files? This cannot be undone.', { count: discardTarget.length }) }}<template v-if="discardNew"> {{ t('{count} new files will be deleted.', { count: discardNew }) }}</template></span><div class="toolbar-buttons"><button type="button" class="small-button danger" :disabled="busy" @click="discard">{{ t(busy ? 'Working…' : 'Discard') }}</button><button type="button" class="small-button" :disabled="busy" @click="discardTarget=undefined">{{ t('Cancel') }}</button></div></div>
      <div class="git-groups">
        <section v-if="conflicted.length" class="git-group git-conflicts">
          <header><strong>{{ t('Merge conflicts') }} <span>{{ conflicted.length }}</span></strong></header>
          <div v-for="file in conflicted" :key="file.path" :class="['git-file', { selected: selected?.path === file.path && !selected.staged }]" @contextmenu="openFileMenu($event, false, file, true)">
            <button class="git-file-open" :title="file.path" @click="show(file.path, false)"><span class="git-file-code conflict">{{ conflictCode(file) }}</span><span>{{ file.path }}</span></button>
            <button class="icon-button" :aria-label="t('Open changed file in editor')" :title="t('Open in editor to resolve')" @click="git.openFile(file.path)"><Icon name="edit" :size="13" /></button>
            <button class="icon-button" :aria-label="t('Mark {path} resolved', { path: file.path })" :title="t('Mark resolved (stage)')" :disabled="busy" @click="stage([file], false)"><Icon name="check" :size="13" /></button>
          </div>
        </section>
        <section v-for="group in groups" :key="group.title" class="git-group" @contextmenu="openFileMenu($event, group.staged)">
          <header><strong>{{ t(group.title) }} <span>{{ group.files.length }}</span></strong><span class="git-group-actions"><button v-if="!group.staged" class="text-button danger-text" :disabled="busy || !group.files.length" @click="discardTarget=group.files">{{ t('Discard all') }}</button><button class="text-button" :disabled="busy || !group.files.length" @click="stage(group.files, group.staged)">{{ t(group.staged ? 'Unstage all' : 'Stage all') }}</button></span></header>
          <div v-for="file in group.files" :key="file.path" :class="['git-file', { selected: selected?.path === file.path && selected.staged === group.staged }]" @contextmenu="openFileMenu($event, group.staged, file)"><button class="git-file-open" :title="file.original_path ? `${file.original_path} → ${file.path}` : file.path" @click="show(file.path, group.staged)" @keydown="fileKeydown($event, file, group.staged)"><span :class="['git-file-code', { staged: group.staged }]">{{ group.staged ? file.index : file.worktree }}</span><span>{{ file.path }}</span></button><button v-if="!group.staged" class="icon-button git-discard" :aria-label="t('Discard changes to {path}', { path: file.path })" :title="t('Discard changes')" :disabled="busy" @click="discardTarget=[file]">↺</button><button class="icon-button" :aria-label="t(group.staged ? 'Unstage {path}' : 'Stage {path}', { path: file.path })" :disabled="busy" @click="stage([file], group.staged)">{{ group.staged ? '−' : '+' }}</button></div>
          <p v-if="!group.files.length" class="group-empty">{{ t(group.staged ? 'Nothing staged' : 'Working tree clean') }}</p>
        </section>
      </div>
    </div>
    <div class="diff-view">
      <div v-if="selected" class="diff-title"><button v-if="mobile" type="button" class="icon-button git-back" :aria-label="t('Back to changes')" @click="git.detailOpen.value = false"><Icon name="chevron" :size="18" /></button><span class="truncate" :title="selected.path">{{ selected.path }}</span><span class="diff-badge">{{ t(selected.staged ? 'Staged' : 'Working tree') }}</span><button class="icon-button" :aria-label="t('Show file history')" :title="t('Show file history')" @click="git.showHistory(selected.path)"><Icon name="clock" :size="14" /></button><button class="icon-button" :aria-label="t('Open changed file in editor')" @click="git.openFile(selected.path)"><Icon name="file" :size="14" /></button></div>
      <div v-if="diffLoading" class="pane-empty"><p>{{ t('Loading diff…') }}</p></div>
      <div v-else-if="diffError" class="inline-error" role="alert">{{ diffError }}</div>
      <div v-else-if="!selected" class="pane-empty"><span class="empty-icon teal"><Icon name="check" :size="26" /></span><h3>{{ t('All clear') }}</h3><p>{{ t('File changes appear here as you and your agents work.') }}</p></div>
      <template v-else-if="diff"><div v-if="diff.binary" class="inline-notice">{{ t('Binary file changed. Open it in Explorer for a preview.') }}</div><div v-if="diff.truncated" class="inline-notice">{{ t('This diff was truncated by the server output limit.') }}</div><GitDiffLines v-if="diff.diff" :diff="diff.diff" :actions="hunkActions" :busy="busy" @hunk="hunk" /><div v-else class="pane-empty"><p>{{ t('No textual diff for this change.') }}</p></div></template>
    </div>
    <ContextMenu v-if="commitMenu" :x="commitMenu.x" :y="commitMenu.y" :width="220" :label="t('More commit actions')" @close="commitMenu = undefined">
      <button role="menuitem" :disabled="!canCommit" @click="menuAction(() => commit('push'))"><Icon name="upload" :size="13" />{{ t('Commit and push') }}</button>
      <button role="menuitem" :disabled="!message.trim() || !changedFiles.length || amend" @click="menuAction(commitAll)"><Icon name="check" :size="13" />{{ t('Stage everything and commit') }}</button>
      <hr />
      <button role="menuitem" @click="menuAction(undoCommit)"><Icon name="restore" :size="13" />{{ t('Undo last commit…') }}</button>
      <button role="menuitem" :disabled="!status.files.length" @click="menuAction(() => emit('stash'))"><Icon name="archive" :size="13" />{{ t('Stash all changes…') }}</button>
    </ContextMenu>
    <ContextMenu v-if="fileMenu" :x="fileMenu.x" :y="fileMenu.y" :label="t('Change actions')" @close="fileMenu = undefined">
      <template v-if="fileMenu.file">
        <button role="menuitem" @click="menuAction(() => show(fileMenu!.file!.path, fileMenu!.staged))">{{ t('Show diff') }}</button>
        <button role="menuitem" @click="menuAction(() => git.openFile(fileMenu!.file!.path))"><Icon name="file" :size="13" />{{ t('Open changed file in editor') }}</button>
        <button v-if="fileMenu.file.worktree !== '?'" role="menuitem" @click="menuAction(() => git.showHistory(fileMenu!.file!.path))"><Icon name="clock" :size="13" />{{ t('Show file history') }}</button>
        <button role="menuitem" @click="menuAction(() => git.copy(fileMenu!.file!.path))">{{ t('Copy path') }}</button>
        <hr />
        <template v-if="fileMenu.conflict">
          <button role="menuitem" :disabled="busy" @click="menuAction(() => resolve(fileMenu!.file!, 'ours'))">{{ t('Keep current (ours)') }}</button>
          <button role="menuitem" :disabled="busy" @click="menuAction(() => resolve(fileMenu!.file!, 'theirs'))">{{ t('Take incoming (theirs)') }}</button>
          <button role="menuitem" :disabled="busy" @click="menuAction(() => stage([fileMenu!.file!], false))"><Icon name="check" :size="13" />{{ t('Mark resolved (stage)') }}</button>
        </template>
        <template v-else>
          <button role="menuitem" :disabled="busy" @click="menuAction(() => stage([fileMenu!.file!], fileMenu!.staged))">{{ t(fileMenu.staged ? 'Unstage' : 'Stage') }}</button>
          <button v-if="!fileMenu.staged" role="menuitem" :disabled="busy" @click="menuAction(() => emit('stash', pathsForGitFiles([fileMenu!.file!])))"><Icon name="archive" :size="13" />{{ t('Stash this file…') }}</button>
          <button v-if="fileMenu.file.worktree === '?'" role="menuitem" :disabled="busy" @click="menuAction(() => ignore(fileMenu!.file!))">{{ t('Add to .gitignore') }}</button>
          <button v-if="!fileMenu.staged" role="menuitem" class="danger" :disabled="busy" @click="menuAction(() => { discardTarget = [fileMenu!.file!]; })">{{ t('Discard changes…') }}</button>
        </template>
      </template>
      <template v-else>
        <button role="menuitem" :disabled="busy || !(fileMenu.staged ? stagedFiles : changedFiles).length" @click="menuAction(() => stage(fileMenu!.staged ? stagedFiles : changedFiles, fileMenu!.staged))">{{ t(fileMenu.staged ? 'Unstage all' : 'Stage all') }}</button>
        <button role="menuitem" :disabled="busy || !status.files.length" @click="menuAction(() => emit('stash'))"><Icon name="archive" :size="13" />{{ t('Stash all changes…') }}</button>
        <button v-if="!fileMenu.staged" role="menuitem" class="danger" :disabled="busy || !changedFiles.length" @click="menuAction(() => { discardTarget = changedFiles; })">{{ t('Discard all') }}…</button>
      </template>
      <hr />
      <button role="menuitem" :disabled="busy" @click="menuAction(git.refresh)"><Icon name="refresh" :size="13" />{{ t('Refresh Git changes') }}</button>
    </ContextMenu>
  </div>
</template>

<style>
.git-changes.git-content{grid-template-columns:minmax(200px,min(32%,300px)) minmax(0,1fr)}
</style>
<style scoped>
.git-group-actions{display:flex;align-items:center;gap:10px}
.git-discard{color:var(--muted);font-size:var(--text-base)}
.git-discard:hover:not(:disabled){color:var(--danger)}
.git-discard-confirm{margin:0 0 10px}
.git-amend{display:flex;align-items:center;gap:6px;font-size:var(--text-xs);color:var(--muted);cursor:pointer;user-select:none}
.git-amend input{margin:0}
.git-commit-buttons{display:flex;gap:1px}
.git-commit-buttons>.primary-button:first-child{flex:1;min-width:0;border-top-right-radius:0;border-bottom-right-radius:0}
.git-commit-buttons>.git-commit-more{flex:none;width:26px;padding:0;border-top-left-radius:0;border-bottom-left-radius:0}
.git-commit-more :deep(svg){transform:rotate(90deg)}
.git-conflicts>header>strong{color:var(--danger-ink)!important}
.git-file-code.conflict{color:var(--danger);min-width:16px}
</style>
