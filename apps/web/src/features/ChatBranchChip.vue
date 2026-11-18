<script setup lang="ts">
/**
 * The composer's branch chip: which branch this session works on, and moving
 * it -- only it -- to another. An isolated task names the branch it started
 * from and offers its review instead. On a phone the chip is hidden and the
 * top bar opens its card (`toggle`).
 *
 * It only says what was asked for; moving is the session's.
 */
import { ref } from "vue";
import type { Session } from "@tendly/protocol";
import ChipMenu from "./ChipMenu.vue";
import { useI18n } from "../i18n";

defineProps<{
  session: Session;
  /** The branch the session is on; none when its checkout is detached. */
  branch?: string;
  /** The workspace directory's branch, marked in the list. */
  directoryBranch?: string;
  choices: string[];
  /** A turn or another action is in the way of a move. */
  busy: boolean;
  /** An isolated task's base: the branch it started from and goes back to. */
  isolatedBase?: string;
  /** The workspace's Git pane can be opened from here. */
  canOpenGit: boolean;
}>();
const emit = defineEmits<{ opened: []; pick: [name: string]; review: []; openGit: [] }>();
const { t } = useI18n();
const menu = ref<InstanceType<typeof ChipMenu>>();
/** A new branch's name, typed below the list. */
const branchTarget = ref("");
defineExpose({
  close: (restoreFocus?: boolean) => menu.value?.close(restoreFocus),
  toggle: () => menu.value?.toggle(),
  /** The session moved: the name typed for it is done with. */
  clearTarget: () => {
    branchTarget.value = "";
  },
});
</script>

<template>
  <ChipMenu
    ref="menu"
    class="chat-branch"
    placement="down"
    @opened="emit('opened')"
    :caption="t('Branch')"
    :label="branch || t('Detached HEAD')"
    :active="!!session.checkout_path"
    :disabled="busy"
    :title="
      isolatedBase
        ? t('Isolated task from {base}, in {path}', { base: isolatedBase, path: session.checkout_path ?? '' })
        : session.checkout_path
          ? t('Works in worktree {path}', { path: session.checkout_path })
          : t('Works in the workspace directory')
    "
    ><template #mark
      ><svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
        <path
          d="M5 3v7M5 10a2 2 0 1 0 0 4 2 2 0 0 0 0-4Zm0-7a2 2 0 1 0 0-.01M11 5a2 2 0 1 0 0-.01M11 7c0 2-2 3-6 3"
          fill="none"
          stroke="currentColor"
          stroke-width="1.5"
          stroke-linecap="round"
        /></svg
    ></template>
    <div class="chat-branch-body">
      <template v-if="isolatedBase"
        ><header>{{ t("Isolated task") }}</header>
        <p>
          {{
            t("Its own worktree and branch, from {base}. When it is done, review what it changed and merge it back.", {
              base: isolatedBase,
            })
          }}
        </p>
        <button
          type="button"
          class="chat-branch-review"
          @click="
            menu?.close(true);
            emit('review');
          "
        >
          {{ t("Review and finish…") }}
        </button></template
      ><template v-else
        ><header>{{ t("Move only this session") }}</header>
        <p>
          {{ t("Moves only this session, into that branch's own worktree. Other sessions keep their branches.") }}
        </p></template
      ><component :is="isolatedBase ? 'details' : 'div'" class="chat-branch-choices"
        ><summary v-if="isolatedBase">{{ t("Move to another branch") }}</summary>
        <nav>
          <button
            v-for="name in choices"
            :key="name"
            type="button"
            :class="{ selected: name === branch }"
            @click="emit('pick', name)"
          >
            <span>{{ name }}</span><small v-if="name === branch">{{ t("Current") }}</small
            ><small v-else-if="name === directoryBranch">{{ t("Workspace directory") }}</small>
          </button>
        </nav>
        <form @submit.prevent="emit('pick', branchTarget)">
          <input
            v-model="branchTarget"
            :placeholder="t('New branch name')"
            maxlength="200"
            spellcheck="false"
            autocomplete="off"
          /><button type="submit" :disabled="!branchTarget.trim()">{{ t("Create") }}</button>
        </form></component
      ><button v-if="canOpenGit" type="button" class="chat-branch-workspace" @click="emit('openGit')">
        {{ t("Switch the whole workspace's branch")
        }}<small>{{ t("Every session in the workspace directory moves with it") }}</small>
      </button>
    </div></ChipMenu
  >
</template>

<style scoped>
/* The pane's defaults for its controls (ChatSessionPane). Its scoped rules do
   not reach into this component, so they are repeated here, first, where the
   pane has them. */
.chat-pane button:disabled {
  cursor: not-allowed;
  opacity: 0.5;
}
.chat-pane button:focus-visible,
.chat-pane summary:focus-visible,
.chat-pane select:focus-visible,
.chat-pane input:focus-visible,
.chat-pane textarea:focus-visible {
  outline: 2px solid var(--focus);
  outline-offset: 3px;
}
@media (prefers-reduced-motion: reduce) {
  .chat-pane * {
    scroll-behavior: auto;
  }
}
.chat-branch-review {
  display: block;
  width: calc(100% - 20px);
  margin: 2px 10px 6px;
  min-height: 32px;
  border: 1px solid var(--line);
  border-radius: var(--radius-md);
  background: var(--surface);
  color: var(--accent-ink);
  font-size: var(--text-sm);
  font-weight: 550;
  cursor: pointer;
}
.chat-branch-review:hover {
  background: var(--fill);
}
details.chat-branch-choices > summary {
  padding: 6px 10px;
  font-size: var(--text-xs);
  color: var(--muted);
  cursor: pointer;
}

.chat-branch {
  max-width: min(30%, 160px);
}
.chat-branch :deep(.chip-menu-panel) {
  min-width: 220px;
  width: max-content;
  max-width: min(300px, 78cqw);
  max-height: min(420px, 66vh);
  overflow-y: auto;
  padding: 5px;
}
/* Reads as the other chips' menus: a quiet heading, rows that lift on hover,
   and the current one in the selected tone rather than a colour of its own. */
.chat-branch-body header {
  padding: 8px 10px 2px;
  font-size: var(--text-xs);
  font-weight: 600;
  color: var(--ink-soft);
}
.chat-branch-body p {
  font-size: var(--text-xs);
  line-height: 1.6;
  color: var(--muted);
  margin: 0 10px 6px;
  white-space: normal;
}
.chat-branch-body nav button {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  width: 100%;
  min-height: 36px;
  padding: 8px 10px;
  border: 0;
  border-radius: var(--radius-md);
  background: none;
  cursor: pointer;
  text-align: left;
}
.chat-branch-body nav button + button {
  margin-top: 1px;
}
.chat-branch-body nav button:hover {
  background: var(--fill);
}
.chat-branch-body nav button.selected {
  background: var(--selected);
}
.chat-branch-body nav button.selected span {
  color: var(--accent-ink);
  font-weight: 600;
}
.chat-branch-body nav span {
  font-size: var(--text-sm);
  color: var(--ink-soft);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.chat-branch-body nav small {
  flex-shrink: 0;
  font-size: var(--text-xs);
  color: var(--muted);
}
.chat-branch-workspace {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 2px;
  width: 100%;
  margin-top: 5px;
  padding: 8px 10px;
  border: 0;
  border-top: 1px solid var(--line);
  background: none;
  font-size: var(--text-sm);
  color: var(--accent-ink);
  text-align: left;
  cursor: pointer;
}
.chat-branch-workspace:hover {
  background: var(--fill);
}
.chat-branch-workspace small {
  font-size: var(--text-xs);
  color: var(--muted);
}
.chat-branch-body form {
  display: flex;
  gap: 6px;
  margin: 5px 3px 3px;
  padding-top: 6px;
  border-top: 1px solid var(--line);
}
.chat-branch-body input {
  flex: 1;
  min-width: 0;
  height: 30px;
  padding: 0 8px;
  border: 1px solid var(--line);
  border-radius: var(--radius-sm);
  font: inherit;
  font-size: var(--text-xs);
  background: var(--surface);
  color: var(--ink);
}
.chat-branch-body form button {
  height: 30px;
  padding: 0 10px;
  border: 0;
  border-radius: var(--radius-sm);
  background: var(--teal);
  color: var(--on-accent);
  font-size: var(--text-xs);
  cursor: pointer;
}
.chat-branch-body form button:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}
/* The branch chip lives in the top bar; its card stays here, with its state. */
.chat-composer.is-phone .chat-branch {
  position: absolute;
  width: 0;
  height: 0;
  overflow: visible;
}
.chat-composer.is-phone .chat-branch :deep(summary) {
  display: none;
}
/* Branches: the card opens down from the top bar. */
.chat-composer.is-phone .chat-branch-body header {
  margin: 8px 12px 2px;
  padding: 0;
  font-size: var(--text-xs);
  letter-spacing: 0.6px;
  text-transform: uppercase;
  color: var(--muted);
}
.chat-composer.is-phone .chat-branch-body p {
  font-size: var(--text-sm);
  margin: 0 12px 6px;
}
.chat-composer.is-phone .chat-branch-body nav span {
  font-size: var(--text-lg);
}
.chat-composer.is-phone .chat-branch-body form {
  margin: 8px 6px 4px;
}
.chat-composer.is-phone .chat-branch-body input {
  height: 44px;
  font-size: var(--input-text);
}
.chat-composer.is-phone .chat-branch-body form button {
  height: 44px;
  padding: 0 16px;
  font-size: var(--text-base);
}
</style>
