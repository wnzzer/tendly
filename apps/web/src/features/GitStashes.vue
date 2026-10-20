<script setup lang="ts">
import { moveInList } from "./list-keys";
import { computed, ref, watch } from "vue";
import type { GitOutcome, GitStash } from "@tendly/protocol";
import { errorMessage } from "./api";
import { gitGet, gitPost, useGit } from "./git-context";
import { relativeTime } from "./git-model";
import { gitViewState } from "./git-view-state";
import GitCommitDetail from "./GitCommitDetail.vue";
import ContextMenu from "./ContextMenu.vue";
import Icon from "./Icon.vue";
import { useMobile } from "./mobile";
import { useI18n } from "../i18n";

/** Work put aside: each stash, what it holds, and applying, popping or dropping it. */
const { t } = useI18n();
const git = useGit();
const mobile = useMobile();
const emit = defineEmits<{ stash: []; count: [count: number] }>();
const viewState = computed(() => gitViewState(git.workspaceId()));
const selected = computed({ get: () => viewState.value.stash, set: value => { viewState.value.stash = value; } });
const stashes = ref<GitStash[]>([]), loading = ref(false), error = ref("");
const current = computed(() => stashes.value.find(stash => stash.hash === selected.value));
let revision = 0;

async function load() {
  const id = ++revision;
  loading.value = true; error.value = "";
  try {
    const list = await gitGet<GitStash[]>(git.workspaceId(), "stashes");
    if (id !== revision) return;
    stashes.value = list; emit("count", list.length);
    if (!list.some(stash => stash.hash === selected.value)) selected.value = mobile.value ? undefined : list[0]?.hash;
  } catch (cause) { if (id === revision) error.value = errorMessage(cause); }
  finally { if (id === revision) loading.value = false; }
}
watch(() => git.revision.value, load, { immediate: true });

const label = (stash: GitStash) => stash.message.replace(/^(On|WIP on) [^:]+: /, "") || `stash@{${stash.index}}`;
const branchOf = (stash: GitStash) => stash.message.match(/^(?:On|WIP on) ([^:]+):/)?.[1];
async function apply(stash: GitStash, pop: boolean) {
  await git.run(() => gitPost<GitOutcome>(git.workspaceId(), "stashes/apply", { index: stash.index, hash: stash.hash, pop }), outcome => outcome.conflicts ? undefined : t(pop ? "Popped {stash}" : "Applied {stash}", { stash: label(stash) }));
}
async function drop(stash: GitStash) {
  const answer = await git.ask({ title: t("Drop this stash?"), description: t("{stash} and the changes in it are deleted. This cannot be undone.", { stash: label(stash) }), confirm: t("Drop"), danger: true });
  if (answer) await git.run(() => gitPost(git.workspaceId(), "stashes/drop", { index: stash.index, hash: stash.hash }), t("Dropped {stash}", { stash: label(stash) }));
}
const menu = ref<{ x: number; y: number; stash: GitStash }>();
function act(work: (stash: GitStash) => unknown) { const stash = menu.value?.stash; menu.value = undefined; if (stash) void work(stash); }
function choose(stash: GitStash) { selected.value = stash.hash; git.detailOpen.value = true; }
</script>

<template>
  <div class="git-content git-stashes">
    <div class="git-sidebar git-list">
      <div class="git-list-tools"><button type="button" class="small-button" :disabled="git.busy.value || !git.status.value.files.length" @click="emit('stash')"><Icon name="archive" :size="13" />{{ t('Stash changes…') }}</button></div>
      <div v-if="error" class="inline-error" role="alert">{{ error }}</div>
      <div class="git-log" role="list">
        <div v-for="(stash, index) in stashes" :key="stash.hash" role="listitem" :tabindex="selected === stash.hash || (!selected && index === 0) ? 0 : -1" :class="['git-stash-row', { selected: selected === stash.hash }]" @click="choose(stash)" @keydown="moveInList($event, ($event.currentTarget as HTMLElement).parentElement, '.git-stash-row') || (($event.key === 'Enter' || $event.key === ' ') && ($event.preventDefault(), choose(stash)))" @contextmenu.prevent="menu = { x: $event.clientX, y: $event.clientY, stash }">
          <span class="git-stash-text"><strong :title="stash.message">{{ label(stash) }}</strong><small>{{ branchOf(stash) ? branchOf(stash) + ' · ' : '' }}{{ relativeTime(stash.date, t) }}<template v-if="stash.untracked"> · {{ t('with untracked files') }}</template></small></span>
          <span class="git-stash-actions"><button type="button" class="icon-button" :title="t('Apply · keep the stash')" :aria-label="t('Apply {stash}', { stash: label(stash) })" :disabled="git.busy.value" @click.stop="apply(stash, false)"><Icon name="download" :size="13" /></button></span>
        </div>
        <div v-if="!loading && !stashes.length && !error" class="pane-empty"><span class="empty-icon"><Icon name="archive" :size="24" /></span><h3>{{ t('No stashes') }}</h3><p>{{ t('Stash changes to set them aside and get a clean working tree, then bring them back later.') }}</p></div>
      </div>
    </div>
    <div class="diff-view">
      <div v-if="current" class="diff-title git-stash-title">
        <button v-if="mobile" type="button" class="icon-button git-back" :aria-label="t('Back to stashes')" @click="git.detailOpen.value = false"><Icon name="chevron" :size="18" /></button>
        <span class="truncate">stash@{{ '{' + current.index + '}' }}</span><span class="flex-spacer" />
        <button type="button" class="small-button" :disabled="git.busy.value" @click="apply(current, false)">{{ t('Apply') }}</button>
        <button type="button" class="small-button primary" :disabled="git.busy.value" @click="apply(current, true)">{{ t('Pop') }}</button>
        <button type="button" class="small-button danger" :disabled="git.busy.value" @click="drop(current)">{{ t('Drop') }}</button>
      </div>
      <GitCommitDetail v-if="current" :rev="current.hash" stash />
      <div v-else-if="stashes.length" class="pane-empty"><p>{{ t('Choose a stash to see what it holds.') }}</p></div>
    </div>
    <ContextMenu v-if="menu" :x="menu.x" :y="menu.y" :label="label(menu.stash)" @close="menu = undefined">
      <button role="menuitem" :disabled="git.busy.value" @click="act(stash => apply(stash, false))">{{ t('Apply · keep the stash') }}</button>
      <button role="menuitem" :disabled="git.busy.value" @click="act(stash => apply(stash, true))">{{ t('Pop · apply and drop') }}</button>
      <hr />
      <button role="menuitem" class="danger" :disabled="git.busy.value" @click="act(drop)">{{ t('Drop…') }}</button>
    </ContextMenu>
  </div>
</template>

<style>
.git-stashes.git-content{grid-template-columns:minmax(220px,36%) minmax(0,1fr)}
.git-stash-row{display:flex;align-items:center;gap:6px;margin:1px 4px;padding:6px 6px 6px 10px;border-radius:var(--radius-xs);cursor:pointer}
.git-stash-row:hover{background:var(--fill)}
.git-stash-row.selected{background:var(--ok-soft)}
.git-stash-text{flex:1;min-width:0;display:flex;flex-direction:column;gap:2px}
.git-stash-text strong{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:var(--text-xs);font-weight:550;color:var(--ink)}
.git-stash-text small{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:var(--text-xs);color:var(--faint)}
.git-stash-actions{opacity:0}
.git-stash-row:hover .git-stash-actions,.git-stash-row.selected .git-stash-actions{opacity:1}
.git-stash-title{font-family:var(--mono)}
.git-stash-title>.small-button{font-family:var(--font-body)}
@container git (max-width: 570px){.git-stashes.git-content{grid-template-columns:minmax(0,1fr);grid-template-rows:minmax(120px,35%) minmax(100px,1fr)}}
.git-pane.is-phone .git-stash-row{min-height:52px;margin:0 6px}
.git-pane.is-phone .git-stash-actions{opacity:1}
@media(pointer:coarse){.git-stash-actions{opacity:1}}
</style>
