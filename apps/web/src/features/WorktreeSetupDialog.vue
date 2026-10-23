<script setup lang="ts">
import { computed, ref } from "vue";
import type { Workspace } from "@tendly/protocol";
import { errorMessage, json, request, workspacePath } from "./api";
import { setupPayload } from "./isolated-task";
import ModalDialog from "./ModalDialog.vue";
import { useI18n } from "../i18n";
const { t } = useI18n();
/**
 * How a worktree Tendly makes for a session of this workspace is prepared:
 * what Git does not carry, copied from the workspace directory, then a
 * command run inside it. The person's setting, kept by Tendly -- nothing in
 * the repository can decide what runs.
 */
const props = defineProps<{ workspace: Workspace }>();
const emit = defineEmits<{ close: []; saved: [workspace: Workspace] }>();
const initialCopy = (props.workspace.worktree_setup?.copy ?? []).join("\n");
const initialCommand = props.workspace.worktree_setup?.command ?? "";
const copy = ref(initialCopy), command = ref(initialCommand), busy = ref(false), error = ref("");
const dirty = computed(() => copy.value !== initialCopy || command.value !== initialCommand);
async function save() {
  if (busy.value) return;
  busy.value = true; error.value = "";
  try {
    const saved = await request<Workspace>(workspacePath(props.workspace.id), json("PATCH", { worktree_setup: setupPayload(copy.value, command.value) }));
    emit("saved", saved);
  } catch (cause) { error.value = errorMessage(cause); }
  finally { busy.value = false; }
}
</script>
<template>
  <ModalDialog :title="t('Prepare new worktrees')" :closable="!busy" :dirty="dirty" @close="emit('close')">
    <form class="form-stack" @submit.prevent="save">
      <p class="form-description">{{ t('A new worktree has only the files Git tracks. Before a session starts in one Tendly made for it, this fills in the rest.') }}</p>
      <label>{{ t('Copy from the workspace directory') }}<textarea v-model="copy" class="setup-field" rows="4" spellcheck="false" autocomplete="off" :disabled="busy" :placeholder="'.env\nnode_modules'" /></label>
      <p class="form-help">{{ t('One path per line, relative to {path}. Paths it does not have are skipped. Where the file system supports it (APFS, Btrfs), directories are cloned and take almost no space.', { path: workspace.root_path }) }}</p>
      <label>{{ t('Then run') }}<textarea v-model="command" class="setup-field" rows="2" spellcheck="false" autocomplete="off" :disabled="busy" :placeholder="'pnpm install --prefer-offline'" /></label>
      <p class="form-help">{{ t('Runs in the new worktree through your shell, as a terminal there would. TENDLY_WORKSPACE_ROOT, TENDLY_WORKTREE and TENDLY_BRANCH are set. Its output appears in the session.') }}</p>
      <div v-if="error" class="inline-error" role="alert">{{ error }}</div>
      <div class="dialog-actions"><button type="button" class="secondary-button" :disabled="busy" @click="emit('close')">{{ t('Cancel') }}</button><button class="primary-button" :disabled="busy || !dirty">{{ busy ? t('Saving…') : t('Save') }}</button></div>
    </form>
  </ModalDialog>
</template>
<style scoped>.setup-field{width:100%;padding:8px 9px;border:1px solid var(--line);border-radius:var(--radius-sm);font:var(--text-sm) var(--mono);line-height:1.5;resize:vertical;background:var(--surface);color:var(--ink)}</style>
