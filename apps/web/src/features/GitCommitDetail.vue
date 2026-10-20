<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { GitChangedFile, GitCommitDetail, GitDiff } from "@tendly/protocol";
import { errorMessage } from "./api";
import { gitGet, shortHash, useGit } from "./git-context";
import { commitRefs } from "./git-model";
import GitDiffLines from "./GitDiffLines.vue";
import Icon from "./Icon.vue";
import { useI18n } from "../i18n";

/** One commit -- or one stash, read as the commit it is -- with its message,
 * the files it changed and, for the file picked, the change itself. */
const props = defineProps<{ rev: string; stash?: boolean }>();
const emit = defineEmits<{ select: [hash: string] }>();
const { t } = useI18n();
const git = useGit();
const detail = ref<GitCommitDetail>(), error = ref(""), loading = ref(false);
const file = ref<GitChangedFile>(), diff = ref<GitDiff>(), diffError = ref(""), diffLoading = ref(false);
let revision = 0, diffRevision = 0;
const remoteNames = computed(() => git.remotes.value.map(remote => remote.name));
const refs = computed(() => commitRefs(detail.value?.refs ?? [], remoteNames.value));
const body = computed(() => detail.value?.message.split("\n").slice(1).join("\n").trim() ?? "");
const totals = computed(() => (detail.value?.files ?? []).reduce((sum, entry) => ({ added: sum.added + (entry.added ?? 0), removed: sum.removed + (entry.removed ?? 0) }), { added: 0, removed: 0 }));

async function load() {
  const id = ++revision;
  loading.value = true; error.value = "";
  try {
    const next = await gitGet<GitCommitDetail>(git.place(), `commit/detail?rev=${encodeURIComponent(props.rev)}${props.stash ? "&stash=true" : ""}`);
    if (id !== revision) return;
    detail.value = next;
    const keep = file.value && next.files.find(entry => entry.path === file.value!.path && !!entry.untracked === !!file.value!.untracked);
    if (keep) void openFile(keep); else { file.value = undefined; diff.value = undefined; diffRevision++; if (next.files.length === 1) void openFile(next.files[0]); }
  } catch (cause) { if (id === revision) { error.value = errorMessage(cause); detail.value = undefined; } }
  finally { if (id === revision) loading.value = false; }
}
async function openFile(entry: GitChangedFile) {
  file.value = entry;
  const id = ++diffRevision, hash = detail.value?.hash ?? props.rev;
  diffLoading.value = true; diffError.value = ""; diff.value = undefined;
  const query = new URLSearchParams({ rev: hash, path: entry.path });
  if (entry.original_path) query.set("original_path", entry.original_path);
  if (entry.untracked) query.set("untracked", "true");
  try { const result = await gitGet<GitDiff>(git.place(), `commit/diff?${query}`); if (id === diffRevision) diff.value = result; }
  catch (cause) { if (id === diffRevision) diffError.value = errorMessage(cause); }
  finally { if (id === diffRevision) diffLoading.value = false; }
}
watch(() => [props.rev, props.stash, git.revision.value], load, { immediate: true });
const when = (seconds: number) => new Date(seconds * 1000).toLocaleString();
const name = (path: string) => path.split("/").pop() ?? path;
const folder = (path: string) => path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";
</script>

<template>
  <div class="git-commit-detail">
    <div v-if="error" class="inline-error" role="alert">{{ error }}</div>
    <div v-else-if="!detail" class="pane-empty"><p>{{ t(loading ? 'Loading commit…' : 'Choose a commit to see what it changed.') }}</p></div>
    <template v-else>
      <header class="git-commit-head">
        <h3>{{ detail.message.split('\n')[0] }}</h3>
        <div v-if="refs.length" class="git-refs"><span v-for="ref in refs" :key="ref.kind + ref.name" :class="['git-ref', ref.kind]">{{ ref.name }}</span></div>
        <pre v-if="body" class="git-commit-body">{{ body }}</pre>
        <dl class="git-commit-meta">
          <div><dt>{{ t('Author') }}</dt><dd :title="detail.email">{{ detail.author }} · {{ when(detail.date) }}</dd></div>
          <div v-if="detail.committer !== detail.author || detail.commit_date !== detail.date"><dt>{{ t('Committer') }}</dt><dd :title="detail.committer_email">{{ detail.committer }} · {{ when(detail.commit_date) }}</dd></div>
          <div><dt>{{ t('Commit') }}</dt><dd><button type="button" class="git-hash" :title="t('Copy hash')" @click="git.copy(detail.hash)">{{ shortHash(detail.hash) }}</button></dd></div>
          <div v-if="detail.parents.length"><dt>{{ t(detail.parents.length > 1 ? 'Parents' : 'Parent') }}</dt><dd><button v-for="parent in detail.parents" :key="parent" type="button" class="git-hash link" @click="emit('select', parent)">{{ shortHash(parent) }}</button></dd></div>
        </dl>
      </header>
      <section class="git-commit-files">
        <header><strong>{{ t(detail.files.length === 1 ? '{count} file changed' : '{count} files changed', { count: detail.files.length }) }}</strong><span><b class="add">+{{ totals.added }}</b> <b class="del">−{{ totals.removed }}</b></span></header>
        <button v-for="entry in detail.files" :key="(entry.untracked ? 'u:' : '') + entry.path" type="button" :class="['git-commit-file', { selected: file?.path === entry.path && !!file?.untracked === !!entry.untracked }]" :title="entry.original_path ? `${entry.original_path} → ${entry.path}` : entry.path" @click="openFile(entry)">
          <span :class="['git-file-code', 'status-' + entry.status]">{{ entry.status }}</span>
          <span class="git-commit-file-name">{{ name(entry.path) }}<small v-if="folder(entry.path)">{{ folder(entry.path) }}</small></span>
          <small v-if="entry.untracked" class="diff-badge">{{ t('Untracked') }}</small>
          <span class="git-counts"><b v-if="entry.added" class="add">+{{ entry.added }}</b><b v-if="entry.removed" class="del">−{{ entry.removed }}</b></span>
        </button>
        <p v-if="detail.truncated" class="group-empty">{{ t('More files changed than are listed.') }}</p>
      </section>
      <section v-if="file" class="git-commit-diff">
        <div class="diff-title"><span class="truncate" :title="file.path">{{ file.path }}</span><button class="icon-button" :aria-label="t('Show file history')" :title="t('Show file history')" @click="git.showHistory(file.path)"><Icon name="clock" :size="14" /></button><button class="icon-button" :aria-label="t('Open changed file in editor')" @click="git.openFile(file.path)"><Icon name="file" :size="14" /></button></div>
        <div v-if="diffLoading" class="pane-empty"><p>{{ t('Loading diff…') }}</p></div>
        <div v-else-if="diffError" class="inline-error" role="alert">{{ diffError }}</div>
        <template v-else-if="diff"><div v-if="diff.binary" class="inline-notice">{{ t('Binary file changed. Open it in Explorer for a preview.') }}</div><GitDiffLines v-if="diff.diff" :diff="diff.diff" /><div v-else class="pane-empty"><p>{{ t('No textual diff for this change.') }}</p></div></template>
      </section>
    </template>
  </div>
</template>

<style>
.git-commit-detail{display:flex;flex-direction:column;flex:1;min-height:0;overflow:auto}
.git-commit-head{padding:12px 14px 10px;border-bottom:1px solid var(--fill)}
.git-commit-head h3{margin:0;font-size:var(--text-md);font-weight:600;line-height:1.45;color:var(--ink);overflow-wrap:anywhere}
.git-commit-body{margin:8px 0 0;padding:0;white-space:pre-wrap;overflow-wrap:anywhere;font:var(--text-sm)/1.55 var(--font-body);color:var(--ink-soft)}
.git-commit-meta{display:grid;gap:3px;margin:10px 0 0;font-size:var(--text-xs);color:var(--muted)}
.git-commit-meta>div{display:flex;gap:8px;min-width:0}
.git-commit-meta dt{flex:none;width:72px;color:var(--faint)}
.git-commit-meta dd{margin:0;min-width:0;display:flex;flex-wrap:wrap;gap:6px;overflow-wrap:anywhere}
.git-hash{padding:0 4px;border:0;border-radius:var(--radius-xs);background:var(--fill);font:var(--text-xs) var(--mono);color:var(--ink-soft);cursor:pointer}
.git-hash:hover{background:var(--fill-hover);color:var(--ink)}
.git-hash.link{color:var(--accent-ink)}
.git-refs{display:flex;flex-wrap:wrap;gap:4px;margin-top:6px}
.git-ref{display:inline-flex;align-items:center;max-width:180px;padding:0 6px;height:18px;border-radius:var(--radius-full);background:var(--fill);color:var(--ink-soft);font:600 var(--text-xs)/18px var(--mono);overflow:hidden;text-overflow:ellipsis;white-space:nowrap;flex:none}
.git-ref.head{background:var(--accent-soft);color:var(--accent-ink)}
.git-ref.branch{background:var(--info-soft);color:var(--ink-soft)}
.git-ref.remote{background:var(--violet-soft);color:var(--violet)}
.git-ref.tag{background:var(--warn-soft);color:var(--warn-ink)}
.git-commit-files{padding:6px 0;border-bottom:1px solid var(--fill);flex-shrink:0}
.git-commit-files>header{display:flex;align-items:center;justify-content:space-between;padding:4px 14px 6px;font-size:var(--text-xs);color:var(--muted)}
.git-commit-files>header strong{font-weight:550}
.git-commit-file{display:flex;align-items:center;gap:8px;width:calc(100% - 8px);margin:0 4px;padding:5px 10px;border:0;border-radius:var(--radius-xs);background:none;text-align:left;font-size:var(--text-xs);color:var(--ink-soft);cursor:pointer}
.git-commit-file:hover{background:var(--fill)}
.git-commit-file.selected{background:var(--ok-soft)}
.git-commit-file-name{flex:1;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.git-commit-file-name small{margin-left:6px;color:var(--faint)}
.git-counts{display:flex;gap:6px;flex:none;font:var(--text-xs) var(--mono)}
.git-pane b.add{color:var(--ok-ink);font-weight:500}.git-pane b.del{color:var(--danger-ink);font-weight:500}
.git-file-code.status-A{color:var(--ok-ink)}.git-file-code.status-D{color:var(--danger-ink)}.git-file-code.status-R,.git-file-code.status-C{color:var(--violet)}
.git-commit-diff{display:flex;flex-direction:column;min-height:240px;flex:1}
.git-commit-diff>.diff-lines{overflow:visible;flex:none}
.git-pane.is-phone .git-commit-diff>.diff-title{padding-left:14px}
</style>
