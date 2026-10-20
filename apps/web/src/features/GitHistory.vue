<script setup lang="ts">
import { moveInList } from "./list-keys";
import { computed, ref, watch } from "vue";
import type { GitCommit, GitLog, GitOutcome } from "@tendly/protocol";
import { errorMessage } from "./api";
import { gitGet, gitPost, shortHash, useGit } from "./git-context";
import { commitGraph, commitRefs, relativeTime } from "./git-model";
import { gitViewState } from "./git-view-state";
import GitCommitDetail from "./GitCommitDetail.vue";
import ContextMenu from "./ContextMenu.vue";
import Icon from "./Icon.vue";
import { useMobile } from "./mobile";
import { useI18n } from "../i18n";

/**
 * The commits, as a graph: the current branch, or every branch. Picking one
 * shows what it changed; right-clicking one does something with it --
 * check it out, branch or tag it, cherry-pick, revert, or reset to it.
 */
const { t } = useI18n();
const git = useGit();
const mobile = useMobile();
const viewState = computed(() => gitViewState(git.place()));
const all = computed({ get: () => !!viewState.value.historyAll, set: value => { viewState.value.historyAll = value; } });
const path = computed({ get: () => viewState.value.historyPath, set: value => { viewState.value.historyPath = value; } });
const selected = computed({ get: () => viewState.value.commit, set: value => { viewState.value.commit = value; } });
const commits = ref<GitCommit[]>([]), more = ref(false), loading = ref(false), error = ref(""), query = ref("");
let revision = 0;
const PAGE = 150;

async function load(append = false) {
  const id = ++revision;
  loading.value = true; error.value = "";
  const params = new URLSearchParams({ limit: String(PAGE), skip: String(append ? commits.value.length : 0) });
  if (all.value) params.set("all", "true");
  if (path.value) params.set("path", path.value);
  try {
    const log = await gitGet<GitLog>(git.place(), `log?${params}`);
    if (id !== revision) return;
    commits.value = append ? [...commits.value, ...log.commits] : log.commits;
    more.value = log.more;
    if (!mobile.value && !selected.value && commits.value[0]) selected.value = commits.value[0].hash;
  } catch (cause) { if (id === revision) error.value = errorMessage(cause); }
  finally { if (id === revision) loading.value = false; }
}
watch(() => [all.value, path.value, git.revision.value, git.status.value.branch], () => load(), { immediate: true });

const graph = computed(() => commitGraph(commits.value));
const lanes = computed(() => Math.min(8, Math.max(1, ...graph.value.map(row => row.width))));
const visible = computed(() => {
  const needle = query.value.trim().toLowerCase();
  return commits.value.map((commit, index) => ({ commit, row: graph.value[index] })).filter(({ commit }) => !needle || commit.subject.toLowerCase().includes(needle) || commit.author.toLowerCase().includes(needle) || commit.hash.startsWith(needle) || commit.refs.some(ref => ref.toLowerCase().includes(needle)));
});
const remoteNames = computed(() => git.remotes.value.map(remote => remote.name));
const COLORS = ["var(--accent)", "var(--violet)", "var(--warn)", "var(--focus)", "var(--danger)", "var(--ok)"];
const LANE = 12;
/** Rows are taller on a phone, and the graph's lines must meet across them. */
const HEIGHT = computed(() => mobile.value ? 44 : 30), MID = computed(() => HEIGHT.value / 2);
const x = (lane: number) => 7 + Math.min(lane, 7) * LANE;
const color = (lane: number) => COLORS[lane % COLORS.length];
const curve = (x1: number, y1: number, x2: number, y2: number) => x1 === x2 ? `M${x1} ${y1}V${y2}` : `M${x1} ${y1}C${x1} ${(y1 + y2) / 2} ${x2} ${(y1 + y2) / 2} ${x2} ${y2}`;

function choose(commit: GitCommit) { selected.value = commit.hash; git.detailOpen.value = true; }

const menu = ref<{ x: number; y: number; commit: GitCommit }>();
function openMenu(event: MouseEvent, commit: GitCommit) { event.preventDefault(); menu.value = { x: event.clientX, y: event.clientY, commit }; }
function act(work: (commit: GitCommit) => unknown) { const commit = menu.value?.commit; menu.value = undefined; if (commit) void work(commit); }
const conflictNote = (outcome: GitOutcome | undefined, done: string) => outcome?.conflicts ? undefined : done;
const subject = (commit: GitCommit) => commit.subject.length > 60 ? commit.subject.slice(0, 57) + "…" : commit.subject;

async function checkout(commit: GitCommit) {
  const answer = await git.ask({ title: t("Check out {hash}?", { hash: shortHash(commit.hash) }), description: t("The workspace directory moves to this commit on no branch (detached HEAD). Commits made there belong to no branch until you create one."), confirm: t("Check out") });
  if (answer) await git.run(() => gitPost(git.place(), "switch", { branch: commit.hash, detach: true }), t("Checked out {hash}", { hash: shortHash(commit.hash) }));
}
async function branchHere(commit: GitCommit) {
  const answer = await git.ask({ title: t("Create a branch at {hash}", { hash: shortHash(commit.hash) }), description: subject(commit), fields: [{ key: "name", label: t("Branch name"), required: true, placeholder: "feature/…" }], confirm: t("Create and switch") });
  const name = String(answer?.name ?? "").trim();
  if (name) await git.run(() => gitPost(git.place(), "switch", { branch: name, create: true, start_point: commit.hash }), t("Switched to new branch {branch}", { branch: name }));
}
async function tagHere(commit: GitCommit) {
  const answer = await git.ask({ title: t("Tag {hash}", { hash: shortHash(commit.hash) }), description: subject(commit), fields: [{ key: "name", label: t("Tag name"), required: true, placeholder: "v1.0.0" }, { key: "message", label: t("Message"), multiline: true, hint: t("With a message the tag is annotated; without one it is a lightweight tag.") }], checks: git.remotes.value.length ? [{ key: "push", label: t("Push the tag to {remote}", { remote: git.remotes.value.find(remote => remote.name === "origin")?.name ?? git.remotes.value[0].name }) }] : [], confirm: t("Create tag") });
  const name = String(answer?.name ?? "").trim();
  if (!name) return;
  await git.run(async () => {
    await gitPost(git.place(), "tags", { name, rev: commit.hash, message: String(answer!.message ?? "") || undefined });
    if (answer!.push) await gitPost(git.place(), "push", { tag: name });
  }, t("Tagged {hash} as {tag}", { hash: shortHash(commit.hash), tag: name }));
}
async function cherryPick(commit: GitCommit) {
  await git.run(() => gitPost<GitOutcome>(git.place(), "cherry-pick", { rev: commit.hash }), outcome => conflictNote(outcome, t("Cherry-picked {hash}", { hash: shortHash(commit.hash) })));
}
async function revert(commit: GitCommit) {
  const answer = await git.ask({ title: t("Revert {hash}?", { hash: shortHash(commit.hash) }), description: t("A new commit on the current branch undoes what {subject} changed. History is kept.", { subject: subject(commit) }), confirm: t("Revert") });
  if (answer) await git.run(() => gitPost<GitOutcome>(git.place(), "revert", { rev: commit.hash }), outcome => conflictNote(outcome, t("Reverted {hash}", { hash: shortHash(commit.hash) })));
}
async function reset(commit: GitCommit) {
  const branch = git.status.value.branch ?? "HEAD";
  const answer = await git.ask({
    title: t("Reset {branch} to {hash}", { branch, hash: shortHash(commit.hash) }),
    description: t("Commits after this one leave the branch. What happens to their changes:"),
    choices: [{ key: "mode", label: t("Mode"), value: "mixed", options: [
      { value: "soft", label: t("Soft · keep the changes staged") },
      { value: "mixed", label: t("Mixed · keep the changes, unstaged") },
      { value: "hard", label: t("Hard · discard the changes and every uncommitted edit"), hint: t("Hard reset deletes uncommitted work. It cannot be undone from here.") },
    ] }],
    confirm: t("Reset"), danger: true,
  });
  if (answer) await git.run(() => gitPost(git.place(), "reset", { rev: commit.hash, mode: answer.mode }), t("{branch} is now at {hash}", { branch, hash: shortHash(commit.hash) }));
}
</script>

<template>
  <div class="git-content git-history">
    <div class="git-sidebar git-list">
      <div class="git-list-tools">
        <div class="segmented" role="group" :aria-label="t('Which history')"><button type="button" :class="{ selected: !all }" @click="all = false">{{ t('Current branch') }}</button><button type="button" :class="{ selected: all }" @click="all = true">{{ t('All branches') }}</button></div>
        <label class="git-search"><Icon name="search" :size="13" /><input v-model="query" :placeholder="t('Filter commits…')" :aria-label="t('Filter commits')" spellcheck="false" /></label>
      </div>
      <div v-if="path" class="git-path-filter"><Icon name="file" :size="12" /><span class="truncate" :title="path">{{ path }}</span><button type="button" class="icon-button" :aria-label="t('Show all files')" @click="path = undefined"><Icon name="close" :size="12" /></button></div>
      <div v-if="error" class="inline-error" role="alert">{{ error }}</div>
      <div class="git-log" role="list">
        <div v-for="({ commit, row }, index) in visible" :key="commit.hash" role="listitem" :tabindex="selected === commit.hash || (!selected && index === 0) ? 0 : -1" :class="['git-log-row', { selected: selected === commit.hash }]" @click="choose(commit)" @keydown="moveInList($event, ($event.currentTarget as HTMLElement).parentElement, '.git-log-row') || (($event.key === 'Enter' || $event.key === ' ') && ($event.preventDefault(), choose(commit)))" @contextmenu="openMenu($event, commit)">
          <svg class="git-graph" :width="x(lanes - 1) + 7" :height="HEIGHT" aria-hidden="true">
            <path v-for="lane in row.lanes.filter(lane => !row.into.includes(lane))" :key="'p' + lane" :d="curve(x(lane), 0, x(lane), HEIGHT)" :stroke="color(lane)" />
            <path v-for="lane in row.into" :key="'i' + lane" :d="curve(x(lane), 0, x(row.column), MID)" :stroke="color(lane)" />
            <path v-for="lane in row.out" :key="'o' + lane" :d="curve(x(row.column), MID, x(lane), HEIGHT)" :stroke="color(lane)" />
            <circle :cx="x(row.column)" :cy="MID" :r="commit.parents.length > 1 ? 3 : 3.5" :fill="commit.parents.length > 1 ? 'var(--surface)' : color(row.column)" :stroke="color(row.column)" />
          </svg>
          <span class="git-log-text">
            <span v-for="ref in commitRefs(commit.refs, remoteNames)" :key="ref.kind + ref.name" :class="['git-ref', ref.kind]" :title="ref.name">{{ ref.name }}</span>
            <span class="git-log-subject" :title="commit.subject">{{ commit.subject }}</span>
          </span>
          <span class="git-log-meta" :title="`${commit.author} <${commit.email}> · ${new Date(commit.date * 1000).toLocaleString()}`">{{ commit.author }} · {{ relativeTime(commit.date, t) }}</span>
        </div>
        <button v-if="more" type="button" class="text-button git-more" :disabled="loading" @click="load(true)">{{ t(loading ? 'Loading…' : 'Load more commits') }}</button>
        <p v-if="!loading && !commits.length && !error" class="group-empty">{{ t(path ? 'No commits changed this file.' : 'No commits yet.') }}</p>
        <p v-else-if="commits.length && !visible.length" class="group-empty">{{ t('No loaded commit matches.') }}</p>
      </div>
    </div>
    <div class="diff-view">
      <div v-if="mobile && selected" class="diff-title"><button type="button" class="icon-button git-back" :aria-label="t('Back to history')" @click="git.detailOpen.value = false"><Icon name="chevron" :size="18" /></button><span class="truncate">{{ shortHash(selected) }}</span></div>
      <GitCommitDetail v-if="selected" :rev="selected" @select="hash => selected = hash" />
      <div v-else class="pane-empty"><span class="empty-icon"><Icon name="clock" :size="24" /></span><p>{{ t('Choose a commit to see what it changed.') }}</p></div>
    </div>
    <ContextMenu v-if="menu" :x="menu.x" :y="menu.y" :width="230" :label="subject(menu.commit)">
      <button role="menuitem" @click="act(commit => git.copy(commit.hash))"><Icon name="clipboard" :size="13" />{{ t('Copy hash') }}</button>
      <button role="menuitem" @click="act(commit => git.copy(commit.subject))">{{ t('Copy message') }}</button>
      <hr />
      <button role="menuitem" :disabled="git.busy.value" @click="act(checkout)">{{ t('Check out this commit…') }}</button>
      <button role="menuitem" :disabled="git.busy.value" @click="act(branchHere)"><Icon name="plus" :size="13" />{{ t('Create branch here…') }}</button>
      <button role="menuitem" :disabled="git.busy.value" @click="act(tagHere)">{{ t('Create tag here…') }}</button>
      <hr />
      <button role="menuitem" :disabled="git.busy.value || !!git.status.value.operation" @click="act(cherryPick)">{{ t('Cherry-pick onto current branch') }}</button>
      <button role="menuitem" :disabled="git.busy.value || !!git.status.value.operation" @click="act(revert)"><Icon name="restore" :size="13" />{{ t('Revert this commit…') }}</button>
      <button role="menuitem" class="danger" :disabled="git.busy.value || !!git.status.value.operation" @click="act(reset)">{{ t('Reset current branch to here…') }}</button>
    </ContextMenu>
  </div>
</template>

<style>
.git-history.git-content{grid-template-columns:minmax(260px,42%) minmax(0,1fr)}
.git-sidebar.git-list{display:flex;flex-direction:column;overflow:hidden}
.git-list-tools{display:flex;flex-wrap:wrap;align-items:center;gap:6px;padding:8px 9px;border-bottom:1px solid var(--fill)}
.git-search{flex:1;min-width:110px;display:flex;align-items:center;gap:5px;height:26px;padding:0 8px;border:1px solid var(--line);border-radius:var(--radius-full);color:var(--faint)}
.git-search input{flex:1;min-width:0;border:0;background:none;outline:0;font-size:var(--text-xs);color:var(--ink)}
.git-search:focus-within{border-color:var(--accent);box-shadow:0 0 0 3px var(--accent-soft)}
.git-path-filter{display:flex;align-items:center;gap:5px;margin:6px 9px 0;padding:2px 2px 2px 8px;border-radius:var(--radius-sm);background:var(--info-soft);font:var(--text-xs) var(--mono);color:var(--ink-soft)}
.git-path-filter>span{flex:1}
.git-path-filter .icon-button{width:22px;height:22px}
.git-log{flex:1;min-height:0;overflow:auto;padding:4px 0 10px}
.git-log-row{display:flex;align-items:center;gap:6px;height:30px;padding:0 8px 0 4px;cursor:pointer;font-size:var(--text-xs);color:var(--ink-soft)}
.git-log-row:hover{background:var(--fill)}
.git-log-row.selected{background:var(--ok-soft)}
.git-graph{flex:none;overflow:visible}
.git-graph path{fill:none;stroke-width:1.6}
.git-graph circle{stroke-width:1.6}
.git-log-text{flex:1;min-width:0;display:flex;align-items:center;gap:4px;overflow:hidden}
.git-log-subject{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:var(--ink)}
.git-log-meta{flex:none;max-width:40%;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:var(--faint)}
.git-more{display:block;margin:8px auto 0}
.git-history .diff-view{overflow:hidden}
@container git (max-width: 720px){.git-log-meta{display:none}}
@container git (max-width: 570px){.git-history.git-content{grid-template-columns:minmax(0,1fr);grid-template-rows:minmax(150px,45%) minmax(100px,1fr)}.git-sidebar.git-list{display:flex}}
.git-pane.is-phone .git-log-row{height:44px;font-size:var(--text-sm)}
.git-pane.is-phone .git-sidebar.git-list .git-list-tools{padding:8px 12px}
</style>
