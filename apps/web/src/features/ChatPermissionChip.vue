<script setup lang="ts">
/**
 * The composer's approval chip: how this session's tools are approved. The
 * unattended mode is the one that stops asking, so the chip looks different
 * whenever it is in force.
 *
 * It only says which mode was picked; changing it is the session's.
 */
import { computed, ref } from "vue";
import ChipMenu from "./ChipMenu.vue";
import Icon from "./Icon.vue";
import { useMobile } from "./mobile";
import { PERMISSION_LABELS, PERMISSION_NOTES, permissionName as permissionNameFor } from "./permission-modes";
import { useI18n } from "../i18n";

const props = defineProps<{
  /** The modes the client offers. */
  choices: string[];
  /** The mode in force, as the client reports it. */
  live: string;
  /** No turn or approval is in the way of a change. */
  changeable: boolean;
}>();
const emit = defineEmits<{ pick: [mode: string] }>();
const { t } = useI18n();
const mobile = useMobile();
const menu = ref<InstanceType<typeof ChipMenu>>();

const permissionName = computed(() => permissionNameFor(props.live, t));
const permissionDanger = computed(() => props.live === "danger");
/** A phone's card has room for a mark beside each mode, which is what a glance finds first. */
const PERMISSION_ICONS: Record<string, string> = {
  ask: "shield",
  accept_edits: "edit",
  plan: "clipboard",
  danger: "shield",
};
function pick(mode: string) {
  menu.value?.close(true);
  if (props.changeable && mode !== props.live) emit("pick", mode);
}
</script>

<template>
  <ChipMenu
    ref="menu"
    class="chat-permission"
    :caption="t('How tools are approved')"
    :class="{ danger: permissionDanger }"
    :label="permissionName"
    :tone="permissionDanger ? 'danger' : undefined"
    :disabled="!changeable"
    :title="
      changeable
        ? t('How tools are approved')
        : t('Wait for the current turn and approvals to finish before changing this.')
    "
    ><template #mark><Icon name="shield" :size="12" /></template>
    <div class="chat-permission-body">
      <header>{{ t("How tools are approved") }}</header>
      <nav :aria-label="t('How tools are approved')">
        <button
          v-for="mode in choices"
          :key="mode"
          type="button"
          :class="{ selected: mode === live, danger: mode === 'danger' }"
          @click="pick(mode)"
        >
          <span
            ><Icon
              v-if="mobile"
              class="chat-permission-mark"
              :name="PERMISSION_ICONS[mode] ?? 'shield'"
              :size="15" /><strong>{{ t(PERMISSION_LABELS[mode] ?? mode) }}</strong
            ><Icon v-if="mode === live" name="check" :size="13" /></span
          ><small v-if="PERMISSION_NOTES[mode]">{{ t(PERMISSION_NOTES[mode]) }}</small>
        </button>
      </nav>
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
.chat-permission :deep(.chip-menu-panel) {
  min-width: 190px;
  width: max-content;
  max-width: min(292px, 78cqw);
  max-height: min(340px, 66vh);
  overflow-y: auto;
  padding: 5px;
}
/* The unattended mode is the one that stops asking, so it is the one that has
   to look different from every other chip rather than merely selected. */
.chat-permission {
  max-width: 150px;
}
/* Tall enough that a fixed list of four does not scroll by two pixels, and
   still bounded for a short screen. */
.chat-permission :deep(.chip-menu-panel) {
  min-width: 214px;
}
/* A panel is as wide as its longest line, within reason. This one was left out
   of the rules above and fell back to the browser's own buttons, in a column so
   narrow that a sentence wrapped every three characters. */
.chat-permission-body > header {
  padding: 8px 10px 6px;
  font-size: var(--text-xs);
  color: var(--muted);
}
.chat-permission nav button {
  display: block;
  width: 100%;
  padding: 8px 10px;
  border: 0;
  border-radius: var(--radius-md);
  background: none;
  text-align: left;
  white-space: normal;
  cursor: pointer;
}
.chat-permission nav button + button {
  margin-top: 1px;
}
.chat-permission nav button > span {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
.chat-permission nav button strong {
  font-size: var(--text-sm);
  font-weight: 600;
  color: var(--ink-soft);
}
.chat-permission nav button small {
  display: block;
  margin-top: 3px;
  font-size: var(--text-xs);
  line-height: 1.6;
  color: var(--muted);
}
.chat-permission nav button:hover {
  background: var(--fill);
}
.chat-permission nav button.selected {
  background: var(--selected);
}
.chat-permission nav button.selected strong {
  color: var(--teal);
}
.chat-permission nav button.danger strong {
  color: #a85c4e;
}
.chat-permission nav button.danger:hover,
.chat-permission nav button.danger.selected {
  background: var(--danger-soft);
}
.chat-permission nav button.danger small {
  color: #b07a70;
}
/* Approval is a mark alone, and red when nothing is asked: it is the one
   setting worth seeing without opening anything. */
.chat-composer.is-phone .chat-permission {
  order: -1;
}
.chat-composer.is-phone .chat-permission :deep(summary) {
  width: 38px;
  padding: 0;
  justify-content: center;
}
.chat-composer.is-phone .chat-permission :deep(summary > .chip-menu-label),
.chat-composer.is-phone .chat-permission :deep(summary > .chip-menu-caret) {
  display: none;
}
.chat-composer.is-phone .chat-permission :deep(summary > svg) {
  width: 17px;
  height: 17px;
}
.chat-composer.is-phone .chat-permission.danger :deep(summary) {
  color: var(--danger);
}
/* Approval modes: a mark, the name, what it does. */
.chat-composer.is-phone .chat-permission-body > header {
  margin: 8px 12px 4px;
  padding: 0;
  font-size: var(--text-xs);
  font-weight: 600;
  letter-spacing: 0.6px;
  text-transform: uppercase;
}
.chat-composer.is-phone .chat-permission nav button > span {
  justify-content: flex-start;
  gap: 10px;
}
.chat-composer.is-phone .chat-permission nav button > span > svg:last-child:not(.chat-permission-mark) {
  margin-left: auto;
}
.chat-composer.is-phone .chat-permission nav button small {
  margin-left: 41px;
}
.chat-permission-mark {
  flex: none;
  width: 31px;
  height: 31px;
  padding: 8px;
  border-radius: var(--radius-md);
  background: var(--fill);
  color: var(--ink-soft);
}
.chat-permission nav button.danger .chat-permission-mark {
  background: var(--danger-soft);
  color: var(--danger);
}
</style>
