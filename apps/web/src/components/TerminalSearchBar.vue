<script setup lang="ts">
import { onMounted, ref } from "vue";
import Icon from "../features/Icon.vue";
import { useI18n } from "../i18n";
const { t } = useI18n();

/**
 * Find through the terminal's whole history. It takes the keyboard when it
 * opens; Enter and Shift+Enter step through the matches and Escape closes it.
 */
const query = defineModel<string>("query", { required: true });
defineProps<{ count: string; canStep: boolean }>();
const emit = defineEmits<{ step: [direction: 1 | -1]; close: [] }>();
const field = ref<HTMLInputElement>();

/** Take the keyboard, with the query selected so typing replaces it; again when asked to find while open. */
function focus() {
  field.value?.focus();
  field.value?.select();
}
onMounted(focus);
defineExpose({ focus });

function keydown(event: KeyboardEvent) {
  if (event.key === "Escape") {
    event.preventDefault();
    event.stopPropagation();
    emit("close");
    return;
  }
  if (event.key === "Enter" && !event.isComposing) {
    event.preventDefault();
    emit("step", event.shiftKey ? -1 : 1);
  }
}
</script>

<template>
  <div class="terminal-search" role="search">
    <Icon name="search" :size="14" class="terminal-search-icon" />
    <input
      ref="field"
      v-model="query"
      class="terminal-search-input"
      type="text"
      enterkeyhint="search"
      autocomplete="off"
      spellcheck="false"
      maxlength="200"
      :placeholder="t('Find in terminal')"
      :aria-label="t('Find in terminal')"
      @keydown="keydown"
    />
    <span v-if="count" class="terminal-search-count" role="status">{{ count }}</span>
    <span class="terminal-search-divider" aria-hidden="true" />
    <button
      type="button"
      class="terminal-search-button"
      :aria-label="t('Previous match')"
      :title="t('Previous match')"
      :disabled="!canStep"
      @click="emit('step', -1)"
    >
      <Icon name="chevron" :size="14" class="terminal-search-up" />
    </button>
    <button
      type="button"
      class="terminal-search-button"
      :aria-label="t('Next match')"
      :title="t('Next match')"
      :disabled="!canStep"
      @click="emit('step', 1)"
    >
      <Icon name="chevron" :size="14" class="terminal-search-down" />
    </button>
    <button
      type="button"
      class="terminal-search-button"
      :aria-label="t('Close search')"
      :title="t('Close search')"
      @click="emit('close')"
    >
      <Icon name="close" :size="13" />
    </button>
  </div>
</template>
