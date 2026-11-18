<script setup lang="ts">
import { onBeforeUnmount, ref } from "vue";
import { useI18n } from "../i18n";
import { copyText } from "./clipboard";
import BottomSheet from "./BottomSheet.vue";
import Icon from "./Icon.vue";

/** `compact` is an icon-only button for places that sit beside content —
 * under a message, in a code block's corner — where a labelled button would
 * outweigh what it copies. */
const props = defineProps<{ text: string; label: string; compact?: boolean; menuItem?: boolean }>();
const { t } = useI18n();
const copied = ref(false),
  manual = ref(false),
  busy = ref(false);
const manualText = ref("");
let timer: ReturnType<typeof setTimeout> | undefined;
onBeforeUnmount(() => clearTimeout(timer));
async function copy() {
  busy.value = true;
  const text = props.text;
  try {
    await copyText(text);
    copied.value = true;
    clearTimeout(timer);
    timer = setTimeout(() => {
      copied.value = false;
    }, 1800);
  } catch {
    manualText.value = text;
    manual.value = true;
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <button
    v-if="compact"
    type="button"
    :class="['copy-icon-button', { copied }]"
    :disabled="busy || !text"
    :aria-label="t(copied ? 'Copied' : label)"
    :title="t(copied ? 'Copied' : label)"
    @click.stop="copy"
  >
    <Icon :name="copied ? 'check' : 'clipboard'" :size="14" />
  </button>
  <button
    v-else
    type="button"
    class="copy-text-button"
    :class="{ 'copy-menu-item': menuItem }"
    :role="menuItem ? 'menuitem' : undefined"
    :title="menuItem ? text : undefined"
    :disabled="busy || !text"
    :aria-label="t(label)"
    @click.stop="copy"
  >
    <Icon v-if="menuItem" :name="copied ? 'check' : 'clipboard'" :size="14" /><span role="status">{{
      t(copied ? "Copied" : label)
    }}</span>
  </button>
  <BottomSheet v-if="manual" :title="t('Select text to copy')" @close="manual = false">
    <template #default="{ close }">
      <p class="copy-help">{{ t("Automatic copy is unavailable. Long-press the text below and choose Copy.") }}</p>
      <textarea
        class="copy-manual-text"
        :value="manualText"
        readonly
        :aria-label="t('Select text to copy')"
        @contextmenu.stop
      />
      <button type="button" class="copy-text-button" @click="close">{{ t("Close") }}</button>
    </template>
  </BottomSheet>
</template>

<style scoped>
.copy-text-button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-height: 36px;
  min-width: 64px;
  padding: 6px 10px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--surface);
  color: var(--ink-soft);
  font: inherit;
  font-size: var(--text-sm);
  cursor: pointer;
  touch-action: manipulation;
  user-select: none;
}
.copy-text-button.copy-menu-item {
  display: flex;
  align-items: center;
  gap: 8px;
  justify-content: flex-start;
  width: 100%;
  min-height: 30px;
  min-width: 0;
  padding: 6px 9px;
  border: 0;
  background: none;
  border-radius: var(--radius-sm);
}
.copy-text-button.copy-menu-item:hover {
  background: var(--fill);
}
.copy-text-button:focus-visible {
  outline: 2px solid var(--focus);
  outline-offset: 2px;
}
.copy-text-button:disabled {
  opacity: 0.5;
}
.copy-icon-button {
  display: inline-grid;
  place-items: center;
  width: 26px;
  height: 26px;
  padding: 0;
  border: 0;
  border-radius: var(--radius-sm);
  background: transparent;
  color: var(--muted);
  --icon-color: currentColor;
  cursor: pointer;
  touch-action: manipulation;
  user-select: none;
}
.copy-icon-button:hover {
  background: var(--fill);
  color: var(--ink-soft);
}
.copy-icon-button.copied {
  color: var(--teal);
}
.copy-icon-button:focus-visible {
  outline: 2px solid var(--focus);
  outline-offset: 1px;
}
.copy-icon-button:disabled {
  opacity: 0.5;
}
.copy-help {
  margin: 8px 10px;
  font-size: var(--text-base);
  line-height: 1.7;
}
.copy-manual-text {
  display: block;
  width: 100%;
  height: 35dvh;
  padding: 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--surface);
  color: var(--ink);
  font: var(--text-lg)/1.6 var(--mono);
  resize: vertical;
  -webkit-user-select: text;
  user-select: text;
  -webkit-touch-callout: default;
}
@media (any-pointer: coarse) {
  .copy-text-button {
    min-height: 44px;
    min-width: 80px;
    font-size: var(--text-md);
  }
  .copy-icon-button {
    width: 32px;
    height: 32px;
  }
}
</style>
