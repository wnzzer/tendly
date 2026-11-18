<script setup lang="ts">
import { ref } from "vue";
import Icon from "./Icon.vue";
import { useI18n } from "../i18n";
const { t } = useI18n();
/**
 * The new-session form as a tab on the canvas rather than a dialog over it: it
 * takes the same props as ModalDialog, so the form renders in either. A tab
 * outlives a trip to settings and back, which a dialog closed to make way could
 * not.
 */
const props = withDefaults(defineProps<{ title: string; closable?: boolean; dirty?: boolean }>(), {
  closable: true,
  dirty: false,
});
const emit = defineEmits<{ close: [] }>();
const confirming = ref(false);
function requestClose() {
  if (!props.closable) return;
  if (props.dirty && !confirming.value) {
    confirming.value = true;
    return;
  }
  emit("close");
}
</script>
<template>
  <div class="session-draft">
    <section
      class="session-draft-card"
      :aria-label="title"
      @keydown.esc.prevent="confirming ? (confirming = false) : requestClose()"
    >
      <header class="session-draft-header">
        <span class="session-draft-badge">{{ t("Draft") }}</span>
        <h2>{{ title }}</h2>
        <button
          v-if="closable"
          type="button"
          class="icon-button"
          :aria-label="t('Close {title}', { title })"
          @click="requestClose"
        >
          <Icon name="close" />
        </button>
      </header>
      <div v-if="confirming" class="confirmation-bar" role="alertdialog">
        <span>{{ t("Discard your changes?") }}</span><span class="flex-spacer" /><button
          type="button"
          class="small-button"
          @click="confirming = false"
        >
          {{ t("Keep editing") }}</button
        ><button type="button" class="small-button danger" @click="emit('close')">{{ t("Discard") }}</button>
      </div>
      <div class="session-draft-body"><slot /></div>
    </section>
  </div>
</template>
<style scoped>
.session-draft {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: var(--space-5) var(--space-4);
  background: var(--sunken);
}
.session-draft-card {
  max-width: 560px;
  margin: 0 auto;
  border: 1px solid var(--line);
  border-radius: var(--radius-lg);
  background: var(--surface);
  box-shadow: var(--shadow-sm);
}
.session-draft-header {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: 14px 16px 12px;
  border-bottom: 1px solid var(--line);
}
.session-draft-header h2 {
  flex: 1;
  margin: 0;
  font-size: var(--text-lg);
  font-weight: 600;
  color: var(--ink);
}
.session-draft-badge {
  padding: 1px 7px;
  border-radius: var(--radius-full);
  background: var(--accent-soft);
  color: var(--accent-ink);
  font-size: var(--text-xs);
  font-weight: 550;
}
.session-draft .confirmation-bar {
  margin: 10px 16px 0;
}
.session-draft-body {
  padding: 14px 16px 16px;
}
@media (max-width: 650px) {
  .session-draft {
    padding: 0;
    background: var(--surface);
  }
  .session-draft-card {
    border: 0;
    border-radius: 0;
    box-shadow: none;
  }
}
</style>
