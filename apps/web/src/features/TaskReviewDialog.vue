<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { GitDiff, Session, TaskFile, TaskState } from "@tendly/protocol";
import { errorMessage, inCheckout, json, request, sessionPath, workspacePath } from "./api";
import { taskActions, taskTotals } from "./isolated-task";
import GitDiffLines from "./GitDiffLines.vue";
import ModalDialog from "./ModalDialog.vue";
import { useI18n } from "../i18n";
const { t } = useI18n();
/**
 * Reviewing an isolated task and bringing it back: everything it changed
 * since it left its base, committed or not, and the ways to finish it --
 * merge into the base, push the branch for a pull request, or throw it away.
 * Merging or putting it away removes the worktree and branch and archives
 * the session (`tasks.rs`).
 */
const props = defineProps<{ session: Session }>();
const emit = defineEmits<{ close: []; done: [session: Session] }>();
const task = ref<TaskState>(), error = ref(""), busy = ref(false), squash = ref(false), confirmDiscard = ref(false);
const file = ref<TaskFile>(), diff = ref<GitDiff>(), diffError = ref(""), diffLoading = ref(false);
let diffRevision = 0;
const actions = computed(() => task.value ? taskActions(task.value) : undefined);
const totals = computed(() => task.value ? taskTotals(task.value) : { files: 0, added: 0, removed: 0 });
async function load() {
  error.value = "";
  try {
    const next = await request<TaskState>(`${sessionPath(props.session.id)}/task`);
    task.value = next;
    const keep = file.value && next.files.find(entry => entry.path === file.value!.path);
    if (keep) void openFile(keep); else if (next.files.length) void openFile(next.files[0]!); else { file.value = undefined; diff.value = undefined; }
  } catch (cause) { error.value = errorMessage(cause); }
}
async function openFile(entry: TaskFile) {
  file.value = entry; diff.value = undefined; diffError.value = "";
  if (entry.untracked) return;
  const id = ++diffRevision;
  diffLoading.value = true;
  try { const result = await request<GitDiff>(`${sessionPath(props.session.id)}/task/diff?path=${encodeURIComponent(entry.path)}`); if (id === diffRevision) diff.value = result; }
  catch (cause) { if (id === diffRevision) diffError.value = errorMessage(cause); }
  finally { if (id === diffRevision) diffLoading.value = false; }
}
watch(() => props.session.id, load, { immediate: true });
async function act(run: () => Promise<Session | void>) {
  if (busy.value) return;
  busy.value = true; error.value = "";
  try { const session = await run(); if (session) emit("done", session); else await load(); }
  catch (cause) { error.value = errorMessage(cause); }
  finally { busy.value = false; }
}
const merge = () => act(() => request<Session>(`${sessionPath(props.session.id)}/task/merge`, json("POST", { squash: squash.value })));
const finish = (discard: boolean) => act(() => request<Session>(`${sessionPath(props.session.id)}/task/close`, json("POST", { discard })));
const push = () => act(async () => { await request(inCheckout(`${workspacePath(props.session.workspace_id)}/git/push`, task.value?.worktree), json("POST", { set_upstream: true })); });
const name = (path: string) => path.split("/").pop() ?? path;
const folder = (path: string) => path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";
</script>
<template>
  <ModalDialog :title="t('Review task')" wide :closable="!busy" @close="emit('close')">
    <div class="task-review">
      <div v-if="error" class="inline-error" role="alert">{{ error }}</div>
      <div v-if="!task" class="pane-empty"><p>{{ t(error ? 'This task could not be read.' : 'Reading the task…') }}</p></div>
      <template v-else>
        <header class="task-review-head">
          <p class="task-review-branches"><code>{{ task.branch }}</code><span>→</span><code>{{ task.base }}</code></p>
          <p class="task-review-counts">{{ t('{ahead} commit(s) to merge', { ahead: task.ahead }) }}<template v-if="task.behind"> · {{ t('{base} has {behind} newer commit(s)', { base: task.base, behind: task.behind }) }}</template> · <b class="add">+{{ totals.added }}</b> <b class="del">−{{ totals.removed }}</b></p>
          <p v-if="task.preparing" class="inline-notice">{{ t('Its worktree is still being prepared.') }}</p>
          <p v-else-if="task.uncommitted" class="inline-notice">{{ t('{count} file(s) are not committed yet. Ask the agent to commit what it wants to keep; a merge takes only commits.', { count: task.uncommitted }) }}</p>
          <p v-else-if="!task.ahead" class="inline-notice">{{ t('Nothing left to merge: everything here is on {base} already.', { base: task.base }) }}</p>
        </header>
        <div class="task-review-body">
          <section class="git-commit-files task-review-files">
            <header><strong>{{ t(totals.files === 1 ? '{count} file changed' : '{count} files changed', { count: totals.files }) }}</strong></header>
            <button v-for="entry in task.files" :key="(entry.untracked ? 'u:' : '') + entry.path" type="button" :class="['git-commit-file', { selected: file?.path === entry.path }]" :title="entry.path" @click="openFile(entry)">
              <span class="git-commit-file-name">{{ name(entry.path) }}<small v-if="folder(entry.path)">{{ folder(entry.path) }}</small></span>
              <small v-if="entry.untracked" class="diff-badge">{{ t('Untracked') }}</small>
              <span class="git-counts"><b v-if="entry.added" class="add">+{{ entry.added }}</b><b v-if="entry.removed" class="del">−{{ entry.removed }}</b></span>
            </button>
            <p v-if="task.files_truncated" class="group-empty">{{ t('More files changed than are listed.') }}</p>
            <p v-if="!task.files.length" class="group-empty">{{ t('No changes yet.') }}</p>
          </section>
          <section class="task-review-diff">
            <div v-if="!file" class="pane-empty"><p>{{ t('Choose a file to see what changed.') }}</p></div>
            <div v-else-if="file.untracked" class="pane-empty"><p>{{ t('A new file, not committed yet.') }}</p></div>
            <div v-else-if="diffLoading" class="pane-empty"><p>{{ t('Loading diff…') }}</p></div>
            <div v-else-if="diffError" class="inline-error" role="alert">{{ diffError }}</div>
            <template v-else-if="diff"><div v-if="diff.binary" class="inline-notice">{{ t('Binary file changed. Open it in Explorer for a preview.') }}</div><GitDiffLines v-if="diff.diff" :diff="diff.diff" /><div v-else-if="!diff.binary" class="pane-empty"><p>{{ t('No textual diff for this change.') }}</p></div></template>
          </section>
        </div>
        <div v-if="confirmDiscard" class="confirmation-bar" role="alertdialog"><span>{{ t('Throw away this task? Its worktree, its uncommitted changes and the branch {branch} are deleted.', { branch: task.branch }) }}</span><span class="flex-spacer" /><button type="button" class="small-button" :disabled="busy" @click="confirmDiscard=false">{{ t('Keep it') }}</button><button type="button" class="small-button danger" :disabled="busy" @click="finish(true)">{{ t('Discard task') }}</button></div>
        <footer v-else class="task-review-actions">
          <button type="button" class="text-button danger" :disabled="busy || !actions?.discard" @click="confirmDiscard=true">{{ t('Discard task…') }}</button>
          <span class="flex-spacer" />
          <a v-if="task.compare_url" class="secondary-button" :href="task.compare_url" target="_blank" rel="noopener noreferrer">{{ t('Open pull request') }}</a>
          <button v-else-if="actions?.push" type="button" class="secondary-button" :disabled="busy" :title="t('Push {branch} and track it, to open a pull request', { branch: task.branch })" @click="push">{{ t('Push branch') }}</button>
          <button v-if="actions?.finish" type="button" class="primary-button" :disabled="busy" @click="finish(false)">{{ t('Finish task') }}</button>
          <template v-else>
            <label class="task-review-squash"><input v-model="squash" type="checkbox" :disabled="busy || !actions?.merge" />{{ t('As one commit') }}</label>
            <button type="button" class="primary-button" :disabled="busy || !actions?.merge" @click="merge">{{ busy ? t('Merging…') : t('Merge into {base}', { base: task.base }) }}</button>
          </template>
        </footer>
        <p v-if="!confirmDiscard" class="form-help">{{ t(task.base_checkout ? (task.base_is_workspace ? 'Merges into the workspace directory, where {base} is checked out. The worktree and branch are then removed and this session archived.' : 'Merges where {base} is checked out, {path}. The worktree and branch are then removed and this session archived.') : 'Moves {base}, which is checked out nowhere, to the merged commit. The worktree and branch are then removed and this session archived.', { base: task.base, path: task.base_checkout ?? '' }) }}</p>
      </template>
    </div>
  </ModalDialog>
</template>
<style>
.task-review{display:flex;flex-direction:column;gap:10px;min-height:min(560px,70vh)}
.task-review-head p{margin:0}
.task-review-head{display:flex;flex-direction:column;gap:6px}
.task-review-branches{display:flex;align-items:center;gap:8px;flex-wrap:wrap;font-size:var(--text-sm)}
.task-review-branches code{padding:2px 7px;border-radius:var(--radius-sm);background:var(--fill);font:var(--text-sm) var(--mono)}
.task-review-branches span{color:var(--muted)}
.task-review-counts{font-size:var(--text-xs);color:var(--muted)}
.task-review b.add{color:var(--ok-ink);font-weight:500}.task-review b.del{color:var(--danger-ink);font-weight:500}
.task-review-body{display:grid;grid-template-columns:minmax(180px,260px) 1fr;gap:10px;flex:1;min-height:0}
.task-review-files{border:1px solid var(--line);border-radius:var(--radius-md);overflow:auto;max-height:52vh}
.task-review-diff{display:flex;flex-direction:column;min-width:0;border:1px solid var(--line);border-radius:var(--radius-md);overflow:auto;max-height:52vh}
.task-review-diff>.diff-lines{overflow:visible;flex:none}
.task-review-actions{display:flex;align-items:center;gap:8px;flex-wrap:wrap}
.task-review-actions .text-button.danger{color:var(--danger-ink)}
.task-review-squash{display:flex;flex-direction:row;align-items:center;gap:6px;font-size:var(--text-sm);color:var(--ink-soft);white-space:nowrap}
.task-review-squash input{width:auto;margin:0;accent-color:var(--teal)}
@media (max-width:640px){.task-review-body{grid-template-columns:1fr}}
</style>
