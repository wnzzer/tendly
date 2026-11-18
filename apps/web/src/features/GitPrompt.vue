<script setup lang="ts">
import { computed, reactive } from "vue";
import type { PromptAnswer, PromptSpec } from "./git-context";
import ModalDialog from "./ModalDialog.vue";
import { useI18n } from "../i18n";

/** The Git pane's one dialog: a few named fields, checkboxes and choices, or
 * only a question, answered or dismissed. */
const props = defineProps<{ spec: PromptSpec }>();
const emit = defineEmits<{ answer: [value: PromptAnswer | undefined] }>();
const { t } = useI18n();
const values = reactive<PromptAnswer>(
  Object.fromEntries([
    ...(props.spec.fields ?? []).map(field => [field.key, field.value ?? ""]),
    ...(props.spec.checks ?? []).map(check => [check.key, check.value ?? false]),
    ...(props.spec.choices ?? []).map(choice => [choice.key, choice.value]),
  ]),
);
const ready = computed(() =>
  (props.spec.fields ?? []).every(field => !field.required || String(values[field.key] ?? "").trim()),
);
function submit() {
  if (ready.value) emit("answer", { ...values });
}
</script>

<template>
  <ModalDialog :title="spec.title" @close="emit('answer', undefined)">
    <form class="form-stack git-prompt" @submit.prevent="submit">
      <p v-if="spec.description" class="form-description">{{ spec.description }}</p>
      <label v-for="choice in spec.choices" :key="choice.key"
        >{{ choice.label }}
        <select v-model="values[choice.key]">
          <option v-for="option in choice.options" :key="option.value" :value="option.value">{{ option.label }}</option>
        </select>
        <small v-if="choice.options.find(option => option.value === values[choice.key])?.hint">{{
          choice.options.find(option => option.value === values[choice.key])?.hint
        }}</small>
      </label>
      <label v-for="(field, index) in spec.fields" :key="field.key"
        >{{ field.label }}
        <textarea
          v-if="field.multiline"
          v-model="values[field.key] as string"
          :placeholder="field.placeholder"
          rows="3"
          :autofocus="index === 0"
          spellcheck="false"
        />
        <input
          v-else
          v-model="values[field.key] as string"
          :placeholder="field.placeholder"
          :autofocus="index === 0"
          spellcheck="false"
          autocomplete="off"
        />
        <small v-if="field.hint">{{ field.hint }}</small>
      </label>
      <label v-for="check in spec.checks" :key="check.key" class="git-prompt-check"
        ><input v-model="values[check.key]" type="checkbox" /><span
          >{{ check.label }}<small v-if="check.hint">{{ check.hint }}</small></span
        ></label
      >
      <div class="dialog-actions">
        <button type="button" class="secondary-button" @click="emit('answer', undefined)">{{ t("Cancel") }}</button>
        <button
          type="submit"
          :class="spec.danger ? 'secondary-button git-danger-button' : 'primary-button'"
          :disabled="!ready"
        >
          {{ spec.confirm }}
        </button>
      </div>
    </form>
  </ModalDialog>
</template>

<style>
.git-prompt input:not([type="checkbox"]),
.git-prompt textarea {
  font-family: var(--mono);
}
.git-prompt textarea {
  display: block;
  width: 100%;
  min-height: 72px;
  padding: 8px 12px;
  border: 1px solid var(--line);
  border-radius: var(--radius-md);
  background: var(--surface);
  color: var(--ink);
  font-size: var(--text-sm);
  resize: vertical;
}
.git-prompt textarea:focus {
  border-color: var(--accent);
  outline: 0;
  box-shadow: 0 0 0 3px var(--accent-soft);
}
.form-stack label.git-prompt-check {
  flex-direction: row;
  align-items: flex-start;
  gap: 8px;
  font-weight: 450;
}
.form-stack .git-prompt-check input[type="checkbox"] {
  display: inline-block;
  flex: none;
  width: auto;
  min-height: 0;
  height: auto;
  padding: 0;
  margin: 3px 0 0;
  box-shadow: none;
  accent-color: var(--accent);
}
.git-prompt-check span {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.git-danger-button {
  color: var(--on-accent) !important;
  background: var(--danger) !important;
  border-color: var(--danger) !important;
}
.git-danger-button:hover:not(:disabled) {
  background: var(--danger-ink) !important;
}
</style>
