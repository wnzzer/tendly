<script setup lang="ts">
import { computed } from "vue";
import { parseUnifiedDiff } from "./git-model";
import { useI18n } from "../i18n";

/** A unified diff, line-numbered. With `actions`, each hunk's header offers
 * them -- stage, unstage, discard -- for that hunk alone. */
const props = defineProps<{
  diff: string;
  actions?: { id: string; label: string; danger?: boolean }[];
  busy?: boolean;
}>();
const emit = defineEmits<{ hunk: [action: string, index: number] }>();
const { t } = useI18n();
const rows = computed(() => parseUnifiedDiff(props.diff));
</script>

<template>
  <div class="diff-lines" tabindex="0" :aria-label="t('Unified Git diff')">
    <div v-for="(line, index) in rows" :key="index" :class="['diff-line', line.kind]">
      <span class="line-number">{{ line.before }}</span><span class="line-number">{{ line.after }}</span
      ><code>{{ line.text || " " }}</code>
      <span v-if="line.kind === 'hunk' && actions?.length" class="diff-hunk-actions">
        <button
          v-for="action in actions"
          :key="action.id"
          type="button"
          :class="{ danger: action.danger }"
          :disabled="busy"
          @click="emit('hunk', action.id, line.hunk!)"
        >
          {{ action.label }}
        </button>
      </span>
    </div>
  </div>
</template>

<style>
.diff-line.hunk {
  position: relative;
}
.diff-hunk-actions {
  position: sticky;
  right: 6px;
  margin-left: auto;
  display: inline-flex;
  gap: 4px;
  padding-left: 12px;
  font-family: var(--font-body);
}
.diff-hunk-actions button {
  height: 20px;
  padding: 0 7px;
  border: 1px solid var(--line);
  border-radius: var(--radius-xs);
  background: var(--surface);
  color: var(--ink-soft);
  font-size: var(--text-xs);
  line-height: 1;
  cursor: pointer;
  opacity: 0.75;
}
.diff-line.hunk:hover .diff-hunk-actions button,
.diff-hunk-actions button:focus-visible {
  opacity: 1;
}
.diff-hunk-actions button:hover:not(:disabled) {
  background: var(--fill);
  color: var(--ink);
}
.diff-hunk-actions button.danger:hover:not(:disabled) {
  background: var(--danger-soft);
  border-color: var(--danger-line);
  color: var(--danger-ink);
}
.diff-hunk-actions button:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
@media (pointer: coarse) {
  .diff-hunk-actions button {
    height: 30px;
    padding: 0 10px;
    opacity: 1;
  }
}
</style>
