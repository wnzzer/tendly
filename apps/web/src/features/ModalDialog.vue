<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import Icon from "./Icon.vue";
import { useI18n } from "../i18n";
const { t } = useI18n();
/** `embedded` renders the content alone, for a dialog shown as one page inside
 * another dialog. The outer shell then owns the backdrop, heading and focus. */
/** `dirty` makes Esc, the close button and a click outside ask before throwing edits away. */
/** `hidden` takes the dialog off screen without closing it, so what it holds survives until it is shown again. */
const props = withDefaults(defineProps<{ title: string; wide?: boolean; closable?: boolean; embedded?: boolean; dirty?: boolean; hidden?: boolean }>(), { closable: true, dirty: false });
const emit = defineEmits<{ close: [] }>();
const dialog = ref<HTMLElement>();
const confirming = ref(false);
function requestClose() { if (!props.closable) return; if (props.dirty && !confirming.value) { confirming.value = true; return; } emit("close"); }
let previousFocus: HTMLElement | null = null;
function onKey(event: KeyboardEvent) {
  if (event.key === "Escape" && props.closable) { event.preventDefault(); if (confirming.value) confirming.value = false; else requestClose(); }
  if (event.key !== "Tab") return;
  const targets = Array.from(dialog.value?.querySelectorAll<HTMLElement>('button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), a[href]') ?? []).filter(el => el.offsetParent !== null);
  const first = targets[0], last = targets[targets.length - 1];
  if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
  else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
}
onMounted(() => { if (props.embedded) return; previousFocus = document.activeElement as HTMLElement; // What asks for focus first, wherever it is; a field kept out of the tab order (a password form's hidden username) never.
  const box = dialog.value; (box?.querySelector<HTMLElement>('[autofocus]') ?? box?.querySelector<HTMLElement>('input:not([readonly]):not([tabindex="-1"]), button'))?.focus(); });
onUnmounted(() => { if (!props.embedded) previousFocus?.focus(); });
</script>
<template>
  <div v-if="embedded" class="dialog-content embedded"><slot /></div>
  <Teleport v-else to="body"><div v-show="!hidden" class="modal-backdrop" @mousedown.self="requestClose"><section ref="dialog" :class="['dialog', { wide }]" role="dialog" aria-modal="true" :aria-label="title" @keydown="onKey"><header class="dialog-header"><h2>{{ title }}</h2><button v-if="closable" class="icon-button" :aria-label="t('Close {title}', { title })" @click="requestClose"><Icon name="close" /></button></header><div v-if="confirming" class="confirmation-bar dialog-discard" role="alertdialog"><span>{{ t('Discard your changes?') }}</span><span class="flex-spacer" /><button type="button" class="small-button" @click="confirming = false">{{ t('Keep editing') }}</button><button type="button" class="small-button danger" @click="emit('close')">{{ t('Discard') }}</button></div><div class="dialog-content"><slot /></div></section></div></Teleport>
</template>
<style>.dialog-content.embedded{padding:0;overflow:visible}.dialog .dialog-discard{margin:8px 22px 0}</style>
