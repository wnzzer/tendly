<script setup lang="ts">
import { computed } from "vue";
import type { EnvironmentOverrides } from "@tendly/protocol";
import { newEnvironmentRow, parseEnvironmentRows, type EnvironmentRow } from "./environment-model";
import Icon from "./Icon.vue";
import { useI18n } from "../i18n";
const { t } = useI18n();
const props = withDefaults(
  defineProps<{
    modelValue: EnvironmentRow[];
    inherited?: EnvironmentOverrides;
    disabled?: boolean;
    supported?: boolean;
    expanded?: boolean;
  }>(),
  { supported: true, disabled: false, expanded: false },
);
const emit = defineEmits<{ "update:modelValue": [rows: EnvironmentRow[]] }>();
const validation = computed(() => parseEnvironmentRows(props.modelValue));
const inheritedCount = computed(() => Object.keys(props.inherited ?? {}).length);
function update(id: string, changes: Partial<EnvironmentRow>) {
  if (props.disabled || !props.supported) return;
  emit(
    "update:modelValue",
    props.modelValue.map(row => (row.id === id ? { ...row, ...changes } : row)),
  );
}
function add() {
  if (!props.disabled && props.supported && props.modelValue.length < 64)
    emit("update:modelValue", [...props.modelValue, newEnvironmentRow()]);
}
function remove(id: string) {
  if (!props.disabled && props.supported)
    emit(
      "update:modelValue",
      props.modelValue.filter(row => row.id !== id),
    );
}
</script>
<template>
  <details class="environment-editor" :open="expanded">
    <summary>
      <Icon name="settings" :size="14" />{{ t("Advanced environment")
      }}<span v-if="modelValue.length">{{ modelValue.length }}</span>
    </summary>
    <p class="form-help">
      {{
        t(
          "The native process inherits the backend host environment. These entries only override or remove selected variables; host values are never listed here.",
        )
      }}
    </p>
    <p class="form-help">
      {{
        t(
          "Exports from another terminal or an old session cannot be recovered from history. Native settings still follow the client’s own precedence.",
        )
      }}
    </p>
    <p class="form-help">
      {{
        t(
          "Plain values are stored as local metadata. Use secret references for sensitive data. Values are passed literally; shell expressions such as $PATH are not expanded.",
        )
      }}
    </p>
    <div v-if="!supported" class="inline-notice">
      {{ t("Upgrade the backend to edit environment overrides. Host environment inheritance already works.") }}
    </div>
    <details v-if="inheritedCount" class="environment-defaults">
      <summary>{{ t("{count} profile environment defaults", { count: inheritedCount }) }}</summary>
      <div v-for="(entry, name) in inherited" :key="name">
        <code>{{ name }}</code><span>{{
          entry.kind === "literal"
            ? entry.value
            : entry.kind === "secret_ref"
              ? entry.reference
              : entry.kind === "main_model"
                ? t("Follows the main model")
                : entry.kind === "main_model_context"
                  ? t("Follows the main model's context window")
                  : t("Remove inherited value")
        }}</span>
      </div>
    </details>
    <div v-for="row in modelValue" :key="row.id" class="environment-row">
      <label
        ><span>{{ t("Variable name") }}</span
        ><input
          :value="row.name"
          :disabled="disabled || !supported"
          placeholder="HTTPS_PROXY"
          autocomplete="off"
          spellcheck="false"
          @input="update(row.id, { name: ($event.target as HTMLInputElement).value })"
      /></label>
      <label
        ><span>{{ t("Value type") }}</span
        ><select
          :value="row.kind"
          :disabled="disabled || !supported"
          @change="update(row.id, { kind: ($event.target as HTMLSelectElement).value as EnvironmentRow['kind'] })"
        >
          <option value="literal">{{ t("Plain value") }}</option>
          <option value="secret_ref">{{ t("Secret reference") }}</option>
          <option value="unset">{{ t("Remove inherited value") }}</option>
          <option value="main_model">{{ t("Main model") }}</option>
          <option value="main_model_context">{{ t("Main model's context window") }}</option>
        </select></label
      >
      <label class="environment-value"
        ><span>{{ t("Value") }}</span
        ><input
          :value="row.value"
          :disabled="
            disabled ||
            !supported ||
            row.kind === 'unset' ||
            row.kind === 'main_model' ||
            row.kind === 'main_model_context'
          "
          :placeholder="
            row.kind === 'secret_ref'
              ? 'env:TENDLY_SECRET_NAME'
              : row.kind === 'unset'
                ? t('No value needed')
                : row.kind === 'main_model'
                  ? t('Follows the main model')
                  : row.kind === 'main_model_context'
                    ? t('Follows the main model\'s context window')
                    : 'http://127.0.0.1:7890'
          "
          autocomplete="off"
          spellcheck="false"
          @input="update(row.id, { value: ($event.target as HTMLInputElement).value })"
      /></label>
      <button
        type="button"
        class="icon-button"
        :disabled="disabled || !supported"
        :aria-label="t('Remove environment override')"
        @click="remove(row.id)"
      >
        <Icon name="close" :size="14" />
      </button>
    </div>
    <div v-for="message in validation.errors" :key="message" class="inline-error" role="alert">{{ t(message) }}</div>
    <button
      type="button"
      class="small-button"
      :disabled="disabled || !supported || modelValue.length >= 64"
      @click="add"
    >
      <Icon name="plus" :size="13" />{{ t("Add environment variable") }}
    </button>
  </details>
</template>
<style scoped>
.environment-editor {
  border: 1px solid var(--accent-soft);
  border-radius: var(--radius-md);
  padding: 10px;
  background: var(--sunken);
}
.environment-editor > summary {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: var(--text-sm);
  color: var(--accent-ink);
  cursor: pointer;
}
.environment-editor > summary > span {
  font-size: var(--text-xs);
  background: var(--accent-soft);
  border-radius: var(--radius-xs);
  padding: 1px 5px;
}
.environment-editor .form-help {
  margin: 9px 0;
  line-height: 18px;
}
.environment-row {
  display: grid;
  grid-template-columns: minmax(90px, 1fr) 120px minmax(110px, 1.6fr) 24px;
  gap: 7px;
  align-items: end;
  margin-bottom: 8px;
}
.environment-row label {
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
  font-size: var(--text-xs);
  color: var(--muted);
}
.environment-row input,
.environment-row select {
  min-width: 0;
  width: 100%;
  padding: 7px;
  border: 1px solid var(--accent-line);
  border-radius: var(--radius-sm);
  background: var(--surface);
  font-family: inherit;
  font-size: var(--text-xs);
}
.environment-row > .icon-button {
  margin-bottom: 4px;
}
.environment-defaults {
  font-size: var(--text-xs);
  color: var(--muted);
  padding: 7px 0;
}
.environment-defaults > div {
  display: flex;
  gap: 8px;
  padding: 5px;
  overflow-wrap: anywhere;
}
.environment-defaults code {
  flex: 1;
}
.environment-defaults span {
  flex: 2;
}
.environment-editor > .small-button {
  margin-top: 5px;
}
@media (max-width: 620px) {
  .environment-row {
    grid-template-columns: minmax(80px, 1fr) 110px 24px;
  }
  .environment-value {
    grid-column: 1/3;
    grid-row: auto;
  }
  .environment-row > .icon-button {
    grid-column: 3;
    grid-row: 1/3;
  }
  .environment-row label:first-child {
    grid-column: 1;
  }
  .environment-row label:nth-child(2) {
    grid-column: 2;
  }
}
</style>
