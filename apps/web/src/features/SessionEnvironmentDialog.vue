<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { Session } from "@tendly/protocol";
import ModalDialog from "./ModalDialog.vue";
import EnvironmentEditor from "./EnvironmentEditor.vue";
import { environmentRows, parseEnvironmentRows } from "./environment-model";
import { backendCapabilities } from "./backend-capabilities";
import { errorMessage, json, request } from "./api";
import { useI18n } from "../i18n";
const { t } = useI18n();
const props = defineProps<{ session: Session }>();
const emit = defineEmits<{ close: []; saved: [session: Session] }>();
const rows = ref(environmentRows(props.session.environment)),
  busy = ref(false),
  error = ref("");
const parsed = computed(() => parseEnvironmentRows(rows.value));
const editable = computed(
  () => backendCapabilities.environment && ["stopped", "failed"].includes(props.session.status),
);
watch(
  () => props.session.id,
  () => {
    rows.value = environmentRows(props.session.environment);
    error.value = "";
  },
);
async function save() {
  if (!editable.value || busy.value || parsed.value.errors.length) return;
  busy.value = true;
  error.value = "";
  try {
    const session = await request<Session>(
      "/sessions/" + encodeURIComponent(props.session.id) + "/environment",
      json("PATCH", { environment: parsed.value.environment }),
    );
    emit("saved", session);
  } catch (cause) {
    error.value = errorMessage(cause);
  } finally {
    busy.value = false;
  }
}
</script>
<template>
  <ModalDialog :title="t('Session environment')" wide :closable="!busy" @close="emit('close')">
    <p class="form-description">{{ session.title }}</p>
    <p class="form-help">
      {{
        t(
          "These overrides apply only to this session on its next start. Saving does not restart the process or change the original native configuration.",
        )
      }}
    </p>
    <div v-if="!['stopped', 'failed'].includes(session.status)" class="inline-notice">
      {{ t("End this session before editing its environment. No running process will be stopped automatically.") }}
    </div>
    <EnvironmentEditor
      v-model="rows"
      expanded
      :disabled="busy || !editable"
      :supported="backendCapabilities.environment"
    />
    <div v-if="error" class="inline-error" role="alert">{{ error }}</div>
    <div class="dialog-actions">
      <button class="secondary-button" :disabled="busy" @click="emit('close')">{{ t("Cancel") }}</button><button
        class="primary-button"
        :disabled="busy || !editable || !!parsed.errors.length"
        @click="save"
      >
        {{ t(busy ? "Saving…" : "Save environment") }}
      </button>
    </div>
  </ModalDialog>
</template>
