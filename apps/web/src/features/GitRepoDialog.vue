<script setup lang="ts">
import { onMounted, ref } from "vue";
import type { GitRemote, GitTag } from "@tendly/protocol";
import { errorMessage } from "./api";
import { gitGet, gitPost, shortHash, useGit } from "./git-context";
import { relativeTime } from "./git-model";
import ModalDialog from "./ModalDialog.vue";
import Icon from "./Icon.vue";
import { useI18n } from "../i18n";

/** The repository's remotes and tags: add, edit, fetch, push and remove them. */
const emit = defineEmits<{ close: [] }>();
const { t } = useI18n();
const git = useGit();
const tab = ref<"remotes" | "tags">("remotes");
const tags = ref<GitTag[]>([]), busy = ref(false), error = ref(""), notice = ref("");
const adding = ref({ name: "", url: "" });
const editing = ref<{ name: string; to: string; url: string }>();
const confirming = ref<{ kind: "remote" | "tag" | "remote-tag"; name: string }>();

async function act<T>(work: () => Promise<T>, done?: string) {
  if (busy.value) return;
  busy.value = true; error.value = ""; notice.value = "";
  try { await work(); if (done) notice.value = done; git.revision.value++; void git.refresh(); }
  catch (cause) { error.value = errorMessage(cause); }
  finally { busy.value = false; }
}
async function loadTags() { try { tags.value = await gitGet<GitTag[]>(git.place(), "tags"); } catch (cause) { error.value = errorMessage(cause); } }
const addRemote = () => act(async () => { git.remotes.value = await gitPost<GitRemote[]>(git.place(), "remotes", { name: adding.value.name.trim(), url: adding.value.url.trim() }); adding.value = { name: "", url: "" }; }, t("Remote added"));
const saveRemote = () => act(async () => { const edit = editing.value!; git.remotes.value = await gitPost<GitRemote[]>(git.place(), "remotes/edit", { name: edit.name, to: edit.to.trim(), url: edit.url.trim() }); editing.value = undefined; }, t("Remote saved"));
const fetchRemote = (name: string) => act(() => gitPost(git.place(), "fetch", { remote: name }), t("Fetched {remote}", { remote: name }));
const pushTag = (name: string) => act(() => gitPost(git.place(), "push", { tag: name }), t("Pushed tag {tag}", { tag: name }));
async function confirm() {
  const target = confirming.value; if (!target) return;
  if (target.kind === "remote") await act(async () => { git.remotes.value = await gitPost<GitRemote[]>(git.place(), "remotes/remove", { name: target.name }); }, t("Removed remote {remote}", { remote: target.name }));
  else if (target.kind === "tag") await act(async () => { tags.value = await gitPost<GitTag[]>(git.place(), "tags/delete", { name: target.name }); }, t("Deleted tag {tag}", { tag: target.name }));
  else await act(() => gitPost(git.place(), "push", { tag: target.name, delete: true }), t("Deleted tag {tag} from the remote", { tag: target.name }));
  confirming.value = undefined;
}
onMounted(loadTags);
</script>

<template>
  <ModalDialog :title="t('Remotes and tags')" wide :closable="!busy" @close="emit('close')">
    <div class="segmented git-repo-tabs" role="tablist"><button type="button" role="tab" :aria-selected="tab === 'remotes'" :class="{ selected: tab === 'remotes' }" @click="tab = 'remotes'">{{ t('Remotes') }} · {{ git.remotes.value.length }}</button><button type="button" role="tab" :aria-selected="tab === 'tags'" :class="{ selected: tab === 'tags' }" @click="tab = 'tags'; loadTags()">{{ t('Tags') }} · {{ tags.length }}</button></div>
    <div v-if="error" class="inline-error" role="alert">{{ error }}</div>
    <div v-else-if="notice" class="inline-success" role="status">{{ notice }}</div>
    <div v-if="confirming" class="confirmation-bar" role="alertdialog">
      <span>{{ confirming.kind === 'remote' ? t('Remove remote {remote}? Its remote-tracking branches go with it; nothing on the server changes.', { remote: confirming.name }) : confirming.kind === 'tag' ? t('Delete tag {tag} here? A copy already pushed stays on the remote.', { tag: confirming.name }) : t('Delete tag {tag} from the remote? Others who fetched it keep their copy.', { tag: confirming.name }) }}</span>
      <span class="flex-spacer" /><button type="button" class="small-button danger" :disabled="busy" @click="confirm">{{ t('Delete') }}</button><button type="button" class="small-button" @click="confirming = undefined">{{ t('Cancel') }}</button>
    </div>
    <section v-if="tab === 'remotes'" class="git-repo-list">
      <div v-for="remote in git.remotes.value" :key="remote.name" class="git-repo-row">
        <form v-if="editing?.name === remote.name" class="git-repo-edit" @submit.prevent="saveRemote">
          <input v-model="editing.to" :aria-label="t('Remote name')" spellcheck="false" autocomplete="off" />
          <input v-model="editing.url" :aria-label="t('Remote URL')" spellcheck="false" autocomplete="off" />
          <button type="submit" class="small-button primary" :disabled="busy || !editing.to.trim() || !editing.url.trim()">{{ t('Save') }}</button><button type="button" class="small-button" @click="editing = undefined">{{ t('Cancel') }}</button>
        </form>
        <template v-else>
          <span class="git-repo-main"><strong>{{ remote.name }}</strong><small :title="remote.url">{{ remote.url }}</small><small v-if="remote.push_url" :title="remote.push_url">{{ t('push') }} · {{ remote.push_url }}</small></span>
          <span class="toolbar-buttons">
            <button type="button" class="icon-button" :title="t('Fetch')" :aria-label="t('Fetch {remote}', { remote: remote.name })" :disabled="busy" @click="fetchRemote(remote.name)"><Icon name="download" :size="14" /></button>
            <button type="button" class="icon-button" :title="t('Edit')" :aria-label="t('Edit {remote}', { remote: remote.name })" :disabled="busy" @click="editing = { name: remote.name, to: remote.name, url: remote.url }"><Icon name="edit" :size="14" /></button>
            <button type="button" class="icon-button danger-text" :title="t('Remove')" :aria-label="t('Remove {remote}', { remote: remote.name })" :disabled="busy" @click="confirming = { kind: 'remote', name: remote.name }"><Icon name="close" :size="14" /></button>
          </span>
        </template>
      </div>
      <p v-if="!git.remotes.value.length" class="form-help">{{ t('No remotes yet. Add one to fetch, pull and push.') }}</p>
      <form class="git-repo-edit git-repo-add" @submit.prevent="addRemote">
        <input v-model="adding.name" :placeholder="git.remotes.value.length ? t('Name') : 'origin'" :aria-label="t('Remote name')" spellcheck="false" autocomplete="off" />
        <input v-model="adding.url" placeholder="git@github.com:owner/repo.git" :aria-label="t('Remote URL')" spellcheck="false" autocomplete="off" />
        <button type="submit" class="small-button primary" :disabled="busy || !adding.name.trim() || !adding.url.trim()"><Icon name="plus" :size="13" />{{ t('Add remote') }}</button>
      </form>
      <p class="form-help">{{ t('Sign-in for a remote comes from Git on the host: an SSH key or a credential helper. Tendly never asks for a password.') }}</p>
    </section>
    <section v-else class="git-repo-list">
      <div v-for="tag in tags" :key="tag.name" class="git-repo-row">
        <span class="git-repo-main"><strong>{{ tag.name }}</strong><small>{{ shortHash(tag.hash) }} · {{ relativeTime(tag.date, t) }}<template v-if="tag.annotated"> · {{ tag.subject || t('annotated') }}</template></small></span>
        <span class="toolbar-buttons">
          <button type="button" class="icon-button" :title="t('Copy hash')" :aria-label="t('Copy hash')" @click="git.copy(tag.hash)"><Icon name="clipboard" :size="14" /></button>
          <button v-if="git.remotes.value.length" type="button" class="icon-button" :title="t('Push to remote')" :aria-label="t('Push {tag}', { tag: tag.name })" :disabled="busy" @click="pushTag(tag.name)"><Icon name="upload" :size="14" /></button>
          <button v-if="git.remotes.value.length" type="button" class="small-button" :disabled="busy" @click="confirming = { kind: 'remote-tag', name: tag.name }">{{ t('Delete remote') }}</button>
          <button type="button" class="icon-button danger-text" :title="t('Delete')" :aria-label="t('Delete {tag}', { tag: tag.name })" :disabled="busy" @click="confirming = { kind: 'tag', name: tag.name }"><Icon name="close" :size="14" /></button>
        </span>
      </div>
      <p v-if="!tags.length" class="form-help">{{ t('No tags. Create one from a commit in History.') }}</p>
    </section>
  </ModalDialog>
</template>

<style>
.git-repo-tabs{margin-bottom:12px}
.git-repo-list{display:flex;flex-direction:column;gap:6px}
.git-repo-row{display:flex;align-items:center;gap:10px;padding:8px 10px;border:1px solid var(--fill-hover);border-radius:var(--radius-md)}
.git-repo-main{flex:1;min-width:0;display:flex;flex-direction:column;gap:2px}
.git-repo-main strong{font:600 var(--text-sm) var(--mono);color:var(--ink)}
.git-repo-main small{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font:var(--text-xs) var(--mono);color:var(--muted)}
.git-repo-edit{flex:1;display:flex;flex-wrap:wrap;gap:6px}
.git-repo-edit input{flex:1;min-width:120px;height:30px;padding:0 8px;border:1px solid var(--line);border-radius:var(--radius-sm);background:var(--surface);color:var(--ink);font:var(--text-sm) var(--mono)}
.git-repo-edit input:first-child{flex:0 1 120px}
.git-repo-add{margin-top:6px;padding-top:10px;border-top:1px solid var(--fill)}
</style>
