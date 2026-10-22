<script setup lang="ts">
import { computed, onBeforeUnmount, provide, ref, watch } from "vue";
import type { GitBranches, GitOutcome, GitRemote, GitStash, GitStatus } from "@tendly/protocol";
import { errorMessage } from "./api";
import { placeKey } from "./checkout-place";
import { copyText } from "./clipboard";
import { GIT_CONTEXT, gitGet, gitPost, isOutcome, type GitContext, type PromptAnswer, type PromptSpec } from "./git-context";
import { isConflictedFile } from "./git-model";
import { gitViewState, type GitView } from "./git-view-state";
import GitChanges from "./GitChanges.vue";
import GitHistory from "./GitHistory.vue";
import GitStashes from "./GitStashes.vue";
import GitPrompt from "./GitPrompt.vue";
import GitRepoDialog from "./GitRepoDialog.vue";
import Icon from "./Icon.vue";
import WorkspaceBranchMenu from "./WorkspaceBranchMenu.vue";
import ContextMenu from "./ContextMenu.vue";
import { useMobile } from "./mobile";
import { useI18n } from "../i18n";

/**
 * A workspace's repository: the working tree, its history and its stashes,
 * with syncing against remotes in the toolbar. Every action goes through one
 * `run`, so one thing happens at a time and the status always follows.
 */
const { t } = useI18n();
/** `checkout`: the worktree of a session the pane was opened for; without it, the workspace directory. */
const props = defineProps<{ workspaceId: string; checkout?: string | null; refreshToken?: number }>();
const place = computed(() => placeKey(props.workspaceId, props.checkout));
const emit = defineEmits<{ changed: [status: GitStatus]; openFile: [path: string] }>();
const status = ref<GitStatus>({ branch: null, files: [] });
const remotes = ref<GitRemote[]>([]);
const viewState = computed(() => gitViewState(place.value));
const view = computed<GitView>({ get: () => viewState.value.view ?? "changes", set: value => { viewState.value.view = value; detailOpen.value = false; } });
const loading = ref(false), busy = ref(false), error = ref(""), notice = ref(""), warning = ref("");
/** Why the status itself could not be read: not a repository, or no Git. The pane then has nothing to act on. */
const unavailable = ref("");
const revision = ref(0), stashCount = ref(0);
/** A phone shows the list or one detail, not both: picking opens the detail over the list. */
const mobile = useMobile(), detailOpen = ref(false);
const conflicts = computed(() => status.value.files.filter(isConflictedFile).length);
let statusRevision = 0, alive = true;

async function refresh() {
  const requestId = ++statusRevision;
  loading.value = true; unavailable.value = "";
  try {
    const next = await gitGet<GitStatus>(place.value, "status");
    if (!alive || requestId !== statusRevision) return;
    status.value = next; emit("changed", next);
  } catch (cause) { if (alive && requestId === statusRevision) { status.value = { branch: null, files: [] }; unavailable.value = errorMessage(cause); } }
  finally { if (alive && requestId === statusRevision) loading.value = false; }
}
async function loadRepo() {
  try { remotes.value = await gitGet<GitRemote[]>(place.value, "remotes"); } catch { remotes.value = []; }
  try { stashCount.value = (await gitGet<GitStash[]>(place.value, "stashes")).length; } catch { stashCount.value = 0; }
}

/** Read everything again: after a branch moved elsewhere, or on request. */
function reload() { error.value = ""; revision.value++; void refresh(); void loadRepo(); }
async function run<T>(work: () => Promise<T>, success?: string | ((result: T) => string | undefined)): Promise<T | undefined> {
  if (busy.value) return undefined;
  busy.value = true; error.value = ""; notice.value = ""; warning.value = "";
  try {
    const result = await work();
    if (!alive) return result;
    if (isOutcome(result) && result.conflicts) { warning.value = t("Stopped on conflicts. Resolve them under Changes, then continue."); view.value = "changes"; }
    else { const done = typeof success === "function" ? success(result) : success; if (done) notice.value = done; }
    return result;
  } catch (cause) { if (alive) error.value = errorMessage(cause); return undefined; }
  finally {
    if (alive) { busy.value = false; revision.value++; await refresh(); void loadRepo(); }
  }
}

/** The dialog the views ask through; one at a time. */
const prompt = ref<{ spec: PromptSpec; answer: (value: PromptAnswer | undefined) => void }>();
function ask(spec: PromptSpec) { prompt.value?.answer(undefined); return new Promise<PromptAnswer | undefined>(resolve => { prompt.value = { spec, answer: value => { prompt.value = undefined; resolve(value); } }; }); }
async function copy(text: string) { try { await copyText(text); notice.value = t("Copied"); } catch { error.value = t("Could not copy to the clipboard in this browser."); } }
function showHistory(path?: string) { viewState.value.historyPath = path; view.value = "history"; }

const context: GitContext = { place: () => place.value, status, remotes, busy, revision, detailOpen, run, refresh, ask, openFile: path => emit("openFile", path), showHistory, copy };
provide(GIT_CONTEXT, context);

// ------------------------------------------------------------------ sync
const remoteName = computed(() => remotes.value.find(remote => remote.name === "origin")?.name ?? remotes.value[0]?.name);
const fetchAll = () => run(() => gitPost(place.value, "fetch", {}), t("Fetched from every remote"));
const pull = (mode?: string) => run(() => gitPost<GitOutcome>(place.value, "pull", mode ? { mode } : {}), outcome => outcome.conflicts ? undefined : t("Pulled {upstream}", { upstream: status.value.upstream ?? "" }));
const push = () => run(() => gitPost<GitOutcome>(place.value, "push", {}), t("Pushed to {upstream}", { upstream: status.value.upstream ?? "" }));
async function publish() {
  if (!status.value.branch) return;
  let remote = remoteName.value;
  if (remotes.value.length > 1) {
    const answer = await ask({ title: t("Publish {branch}", { branch: status.value.branch }), choices: [{ key: "remote", label: t("Remote"), value: remote!, options: remotes.value.map(entry => ({ value: entry.name, label: `${entry.name} · ${entry.url}` })) }], confirm: t("Publish") });
    if (!answer) return; remote = String(answer.remote);
  }
  await run(() => gitPost(place.value, "push", { remote, set_upstream: true }), t("Published {branch} to {remote}", { branch: status.value.branch!, remote: remote! }));
}
async function forcePush() {
  const answer = await ask({ title: t("Force push {branch}?", { branch: status.value.branch ?? "" }), description: t("Replaces {upstream} with your branch, dropping commits there that you do not have. It is refused if someone pushed since you last fetched (force-with-lease).", { upstream: status.value.upstream ?? t("the remote branch") }), confirm: t("Force push"), danger: true });
  if (answer) await run(() => gitPost(place.value, "push", { force: true }), t("Force-pushed {branch}", { branch: status.value.branch ?? "" }));
}
/** Merge another branch into this one, or rebase this one onto it. */
async function integrate(kind: "merge" | "rebase") {
  let branches: GitBranches;
  try { branches = await gitGet<GitBranches>(place.value, "branches"); } catch (cause) { error.value = errorMessage(cause); return; }
  const names = [...(branches.local ?? []).map(entry => entry.name).filter(name => name !== branches.current), ...(branches.remote ?? []).map(entry => entry.name)];
  if (!names.length) { error.value = t("There is no other branch."); return; }
  const current = branches.current ?? "HEAD";
  const answer = await ask(kind === "merge"
    ? { title: t("Merge into {branch}", { branch: current }), choices: [{ key: "rev", label: t("Branch to merge"), value: names[0], options: names.map(name => ({ value: name, label: name })) }], checks: [{ key: "no_ff", label: t("Always create a merge commit (--no-ff)") }, { key: "squash", label: t("Squash: stage the changes for one commit, without merging history") }], confirm: t("Merge") }
    : { title: t("Rebase {branch}", { branch: current }), description: t("Replays the commits of {branch} on top of the branch you choose. Rewrites them: avoid it for commits already pushed and shared.", { branch: current }), choices: [{ key: "rev", label: t("Onto"), value: names[0], options: names.map(name => ({ value: name, label: name })) }], confirm: t("Rebase") });
  if (!answer) return;
  const rev = String(answer.rev);
  await run(() => gitPost<GitOutcome>(place.value, kind, kind === "merge" ? { rev, no_ff: !!answer.no_ff, squash: !!answer.squash } : { rev }), outcome => outcome.conflicts ? undefined : t(kind === "merge" ? "Merged {branch}" : "Rebased onto {branch}", { branch: rev }));
}
async function stash(paths?: string[]) {
  const answer = await ask({ title: paths?.length ? t("Stash {path}", { path: paths[0] }) : t("Stash changes"), fields: [{ key: "message", label: t("Message"), placeholder: t("What is this work?") }], checks: paths?.length ? [] : [{ key: "include_untracked", label: t("Include new (untracked) files"), value: true }, { key: "keep_index", label: t("Keep staged changes in place too") }], confirm: t("Stash") });
  if (!answer) return;
  await run(() => gitPost(place.value, "stashes", { message: String(answer.message ?? ""), include_untracked: paths?.length ? true : !!answer.include_untracked, keep_index: !!answer.keep_index, paths: paths ?? [] }), t("Changes stashed"));
}
const operationLabels: Record<string, string> = { merge: "Merge in progress", rebase: "Rebase in progress", "cherry-pick": "Cherry-pick in progress", revert: "Revert in progress" };
const operationDone: Record<string, [string, string]> = { merge: ["Merge completed", "Merge aborted"], rebase: ["Rebase completed", "Rebase aborted"], "cherry-pick": ["Cherry-pick completed", "Cherry-pick aborted"], revert: ["Revert completed", "Revert aborted"] };
function operation(action: "continue" | "abort" | "skip") {
  const [completed, aborted] = operationDone[status.value.operation ?? "merge"];
  return run(() => gitPost<GitOutcome>(place.value, "operation", { action }), outcome => outcome.conflicts ? undefined : t(action === "abort" ? aborted : completed));
}
const init = () => run(() => gitPost(place.value, "init"), t("Created a Git repository here"));

const moreMenu = ref<{ x: number; y: number }>();
function openMore(event: MouseEvent) { const box = (event.currentTarget as HTMLElement).getBoundingClientRect(); moreMenu.value = { x: box.right - 240, y: box.bottom + 4 }; }
function menuAction(work: () => unknown) { moreMenu.value = undefined; void work(); }
const showRepo = ref(false);

watch(place, () => { statusRevision++; busy.value = false; status.value = { branch: null, files: [] }; notice.value = ""; error.value = ""; warning.value = ""; void refresh(); void loadRepo(); }, { immediate: true, flush: "sync" });
// Files change often while agents work: the status follows every change,
// history and stashes only while they are on screen.
watch(() => props.refreshToken, () => { if (busy.value) return; void refresh(); if (view.value !== "changes") revision.value++; });
onBeforeUnmount(() => { alive = false; statusRevision++; prompt.value?.answer(undefined); });
const tabs = computed(() => [
  { id: "changes" as const, label: t("Changes"), count: status.value.files.length },
  { id: "history" as const, label: t("History") },
  { id: "stashes" as const, label: t("Stashes"), count: stashCount.value },
]);
</script>

<template>
  <section :class="['git-pane', { 'is-phone': mobile, 'show-diff': mobile && detailOpen }]">
    <div class="content-toolbar git-toolbar">
      <span class="git-branch-bar"><span v-if="checkout" class="git-worktree" :title="t('Works in worktree {path}', { path: checkout })"><svg viewBox="0 0 16 16" width="11" height="11" aria-hidden="true"><path d="M5 3v7M5 10a2 2 0 1 0 0 4 2 2 0 0 0 0-4Zm0-7a2 2 0 1 0 0-.01M11 5a2 2 0 1 0 0-.01M11 7c0 2-2 3-6 3" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg><span>{{ status.branch || t('Detached HEAD') }}</span><small>{{ t('Own directory') }}</small></span><WorkspaceBranchMenu v-else-if="!unavailable" :workspace-id="workspaceId" :branch="status.branch" @switched="reload" @changed="reload" /><span v-if="status.upstream" class="git-upstream" :title="t('Tracks {upstream}', { upstream: status.upstream })">{{ status.upstream }}</span></span>
      <span v-if="!unavailable" class="toolbar-buttons git-sync">
        <template v-if="status.upstream">
          <button type="button" :class="['git-sync-button', { due: status.behind }]" :disabled="busy" :title="t('Pull from {upstream}', { upstream: status.upstream })" :aria-label="t('Pull from {upstream}', { upstream: status.upstream })" @click="pull()"><Icon name="download" :size="14" /><span v-if="status.behind">{{ status.behind }}</span></button>
          <button type="button" :class="['git-sync-button', { due: status.ahead }]" :disabled="busy" :title="t('Push to {upstream}', { upstream: status.upstream })" :aria-label="t('Push to {upstream}', { upstream: status.upstream })" @click="push"><Icon name="upload" :size="14" /><span v-if="status.ahead">{{ status.ahead }}</span></button>
        </template>
        <button v-else-if="status.branch && remotes.length" type="button" class="small-button" :disabled="busy" :title="t('Push {branch} and track it', { branch: status.branch })" @click="publish"><Icon name="upload" :size="13" />{{ t('Publish branch') }}</button>
        <button v-if="remotes.length" type="button" class="icon-button" :disabled="busy" :title="t('Fetch from every remote')" :aria-label="t('Fetch from every remote')" @click="fetchAll"><Icon name="sync" :size="15" /></button>
        <button type="button" class="icon-button" :aria-label="t('More Git actions')" :title="t('More Git actions')" @click="openMore"><Icon name="more" :size="15" /></button>
        <button class="icon-button" :disabled="busy || loading" :aria-label="t('Refresh Git changes')" :title="t('Refresh Git changes')" @click="reload"><Icon name="refresh" :size="15" /></button>
      </span>
    </div>
    <nav v-if="!unavailable" class="git-tabs" role="tablist" :aria-label="t('Git views')">
      <button v-for="tab in tabs" :key="tab.id" type="button" role="tab" :aria-selected="view === tab.id" :class="{ selected: view === tab.id }" @click="view = tab.id">{{ tab.label }}<span v-if="tab.count" class="count-badge">{{ tab.count }}</span></button>
      <span v-if="busy" class="git-working"><i class="state-dot running" />{{ t('Working…') }}</span>
    </nav>
    <div v-if="status.operation" class="confirmation-bar git-operation" role="status">
      <span><strong>{{ t(operationLabels[status.operation]) }}</strong> · {{ conflicts === 1 ? t('One conflicted file. Resolve it, mark it resolved, then continue.') : conflicts ? t('{count} conflicted files. Resolve them, mark them resolved, then continue.', { count: conflicts }) : t('No conflicts left. Continue to finish.') }}</span>
      <span class="flex-spacer" />
      <button type="button" class="small-button primary" :disabled="busy || !!conflicts" @click="operation('continue')">{{ t('Continue') }}</button>
      <button v-if="status.operation !== 'merge'" type="button" class="small-button" :disabled="busy" @click="operation('skip')">{{ t('Skip') }}</button>
      <button type="button" class="small-button danger" :disabled="busy" @click="operation('abort')">{{ t('Abort') }}</button>
    </div>
    <div v-if="error" class="inline-error git-message" role="alert"><span>{{ error }}</span><button type="button" class="icon-button" :aria-label="t('Dismiss')" @click="error = ''"><Icon name="close" :size="13" /></button></div>
    <div v-else-if="warning && !status.operation" class="inline-notice git-message" role="alert"><span>{{ warning }}</span><button type="button" class="icon-button" :aria-label="t('Dismiss')" @click="warning = ''"><Icon name="close" :size="13" /></button></div>
    <div v-else-if="notice" class="inline-success git-message" role="status"><span>{{ notice }}</span><button type="button" class="icon-button" :aria-label="t('Dismiss')" @click="notice = ''"><Icon name="close" :size="13" /></button></div>
    <div v-if="unavailable" class="pane-empty git-unavailable"><span class="empty-icon"><Icon name="git" :size="24" /></span><h3>{{ t('Git unavailable') }}</h3><p>{{ t('This directory is not a Git repository yet. Create one here to track changes, or register a repository as the workspace.') }}</p><button type="button" class="primary-button" :disabled="busy" @click="init"><Icon name="plus" :size="14" />{{ t('Initialize repository') }}</button><details><summary>{{ t('Details') }}</summary><code>{{ unavailable }}</code></details></div>
    <GitChanges v-else-if="view === 'changes'" @stash="stash" />
    <GitHistory v-else-if="view === 'history'" />
    <GitStashes v-else @stash="stash()" @count="count => stashCount = count" />
    <ContextMenu v-if="moreMenu" :x="moreMenu.x" :y="moreMenu.y" :width="240" :label="t('More Git actions')" @close="moreMenu = undefined">
      <template v-if="remotes.length">
        <div class="context-menu-label">{{ t('Sync') }}</div>
        <button role="menuitem" :disabled="busy" @click="menuAction(fetchAll)"><Icon name="sync" :size="13" />{{ t('Fetch from every remote') }}</button>
        <button role="menuitem" :disabled="busy || !status.upstream" @click="menuAction(() => pull('merge'))"><Icon name="download" :size="13" />{{ t('Pull (merge)') }}</button>
        <button role="menuitem" :disabled="busy || !status.upstream" @click="menuAction(() => pull('rebase'))">{{ t('Pull (rebase)') }}</button>
        <button role="menuitem" :disabled="busy || !status.upstream" @click="menuAction(() => pull('ff-only'))">{{ t('Pull (fast-forward only)') }}</button>
        <button role="menuitem" :disabled="busy || !status.upstream" @click="menuAction(push)"><Icon name="upload" :size="13" />{{ t('Push') }}</button>
        <button role="menuitem" :disabled="busy || !status.branch" @click="menuAction(publish)">{{ t('Publish branch to…') }}</button>
        <button role="menuitem" class="danger" :disabled="busy || !status.upstream" @click="menuAction(forcePush)">{{ t('Force push…') }}</button>
        <hr />
      </template>
      <div class="context-menu-label">{{ t('Branch') }}</div>
      <button role="menuitem" :disabled="busy || !!status.operation" @click="menuAction(() => integrate('merge'))">{{ t('Merge a branch into this one…') }}</button>
      <button role="menuitem" :disabled="busy || !!status.operation || !status.branch" @click="menuAction(() => integrate('rebase'))">{{ t('Rebase this branch onto…') }}</button>
      <button role="menuitem" :disabled="busy || !status.files.length" @click="menuAction(() => stash())"><Icon name="archive" :size="13" />{{ t('Stash all changes…') }}</button>
      <hr />
      <button role="menuitem" @click="menuAction(() => { showRepo = true; })"><Icon name="settings" :size="13" />{{ t('Remotes and tags…') }}</button>
    </ContextMenu>
    <GitPrompt v-if="prompt" :key="prompt.spec.title" :spec="prompt.spec" @answer="prompt.answer" />
    <GitRepoDialog v-if="showRepo" @close="showRepo = false" />
  </section>
</template>

<style>
.git-toolbar{gap:6px}
.git-branch-bar{display:flex;align-items:center;gap:8px;min-width:0;font-size:var(--text-sm);font-weight:600}
.git-worktree{display:inline-flex;align-items:center;gap:5px;min-width:0;padding:1px 6px;border-radius:var(--radius-sm);color:var(--accent-ink)}
.git-worktree>span{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.git-worktree small{flex:none;font-size:var(--text-xs);font-weight:400;color:var(--muted)}
.git-upstream{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font:var(--text-xs) var(--mono);color:var(--faint)}
.git-sync{gap:2px}
.git-sync-button{display:inline-flex;align-items:center;gap:3px;height:28px;min-width:28px;padding:0 7px;border:0;border-radius:var(--radius-sm);background:none;color:var(--muted);font:600 var(--text-xs) var(--mono);cursor:pointer}
.git-sync-button:hover:not(:disabled){background:var(--fill);color:var(--ink)}
.git-sync-button.due{color:var(--accent-ink);background:var(--accent-soft)}
.git-sync-button:disabled{opacity:.5;cursor:not-allowed}
.git-tabs{display:flex;align-items:center;gap:2px;padding:4px 8px;border-bottom:1px solid var(--fill);flex-shrink:0}
.git-tabs>button{display:inline-flex;align-items:center;gap:6px;height:26px;padding:0 10px;border:0;border-radius:var(--radius-full);background:none;color:var(--muted);font-size:var(--text-xs);font-weight:550;cursor:pointer}
.git-tabs>button:hover{background:var(--fill);color:var(--ink)}
.git-tabs>button.selected{background:var(--fill-hover);color:var(--ink)}
.git-tabs .count-badge{min-width:17px;padding:1px 4px}
.git-working{display:inline-flex;align-items:center;gap:6px;margin-left:auto;font-size:var(--text-xs);color:var(--muted)}
.git-operation{flex-shrink:0}
.git-message{display:flex;align-items:flex-start;gap:8px;white-space:pre-wrap}
.git-message>span{flex:1;min-width:0;max-height:160px;overflow:auto}
.git-message>.icon-button{width:22px;height:22px;color:inherit;opacity:.7}
.git-pane .segmented{display:inline-flex;padding:2px;border-radius:var(--radius-full);background:var(--fill)}
.git-pane .segmented button,.git-repo-tabs button{height:22px;padding:0 9px;border:0;border-radius:var(--radius-full);background:none;color:var(--muted);font-size:var(--text-xs);cursor:pointer;white-space:nowrap}
.git-pane .segmented button.selected,.git-repo-tabs button.selected{background:var(--surface);color:var(--ink);box-shadow:var(--shadow-sm)}
.git-repo-tabs{display:inline-flex;padding:2px;border-radius:var(--radius-full);background:var(--fill)}
.git-repo-tabs button{height:26px;padding:0 12px}
.git-unavailable details{margin-top:var(--space-3);max-width:420px;font-size:var(--text-xs);color:var(--muted)}
.git-unavailable summary{cursor:pointer}
.git-unavailable code{display:block;margin-top:var(--space-2);padding:var(--space-2) var(--space-3);border-radius:var(--radius-sm);background:var(--fill);font-family:var(--mono);text-align:left;overflow-wrap:anywhere;white-space:pre-wrap}
@container git (max-width: 420px){.git-upstream{display:none}}
.git-pane.is-phone .git-tabs{padding:6px 10px}
.git-pane.is-phone .git-tabs>button{height:34px;padding:0 14px;font-size:var(--text-sm)}
.git-pane.is-phone.show-diff>.git-tabs,.git-pane.is-phone.show-diff>.git-operation{display:none}
</style>
