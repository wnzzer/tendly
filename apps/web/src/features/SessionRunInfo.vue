<script setup lang="ts">
import type { Session } from "@tendly/protocol";
import { computed } from "vue";
import ModalDialog from "./ModalDialog.vue";
import CopyButton from "./CopyButton.vue";
import { useI18n } from "../i18n";
import { isEphemeralSession, resumeCommand } from "./session-list";
import { clientLabel } from "./clients";
const props = defineProps<{ session: Session }>();
const emit = defineEmits<{ close: [] }>();
const { t } = useI18n();
const isEphemeral = computed(() => isEphemeralSession(props.session));
</script>
<template>
  <ModalDialog :title="t('Run information')" @close="emit('close')">
    <dl class="session-runtime-info" :data-session-id="session.id">
      <div>
        <dt>{{ t("Tendly session ID") }}</dt>
        <dd>
          <code>{{ session.id }}</code><CopyButton :text="session.id" label="Copy" compact /><small>{{
            t("Names this session in Tendly, with its client, endpoint and checkout.")
          }}</small>
        </dd>
      </div>
      <div v-if="session.provider !== 'terminal'">
        <dt>{{ t("{client} session ID", { client: clientLabel(session.provider) }) }}</dt>
        <dd>
          <code>{{ session.provider_session_id || "—" }}</code><CopyButton
            v-if="session.provider_session_id"
            :text="session.provider_session_id"
            label="Copy"
            compact
          /><small>{{
            session.provider_session_id
              ? t(
                  "The client's own conversation ID. Alone it opens nothing: the conversation lives in this session's configuration directory.",
                )
              : t("Assigned once the first message is sent.")
          }}</small>
        </dd>
      </div>
      <div v-if="resumeCommand(session)">
        <dt>{{ t("Resume in a terminal") }}</dt>
        <dd>
          <code>{{ resumeCommand(session) }}</code><CopyButton
            :text="resumeCommand(session)!"
            label="Copy"
            compact
          /><small>{{
            t("Run on the machine Tendly runs on. The key stays there; only the session ID is in the command.")
          }}</small>
        </dd>
      </div>
      <div>
        <dt>{{ t("Mode") }}</dt>
        <dd>{{ session.interaction_mode || "pty" }}</dd>
      </div>
      <div v-if="isEphemeral">
        <dt>{{ t("Temporary window") }}</dt>
        <dd>{{ t("Discarded when its window is closed") }}</dd>
      </div>
      <div>
        <dt>{{ t("Workspace") }}</dt>
        <dd>
          <code>{{ session.workspace_id }}</code>
        </dd>
      </div>
      <div>
        <dt>{{ t("Configuration revision") }}</dt>
        <dd>{{ session.configuration_revision ?? 0 }}</dd>
      </div>
    </dl>
  </ModalDialog>
</template>
<style scoped>
.session-runtime-info {
  display: grid;
  gap: 16px;
  margin: 0;
  color: var(--text, var(--ink));
  font-size: var(--text-md);
}
.session-runtime-info > div {
  display: grid;
  grid-template-columns: minmax(100px, 1fr) minmax(0, 2fr);
  align-items: start;
  gap: 8px 16px;
}
.session-runtime-info dt {
  color: var(--ink-soft);
  font-size: var(--text-sm);
}
.session-runtime-info dd {
  margin: 0;
  min-width: 0;
  overflow-wrap: anywhere;
}
.session-runtime-info code {
  font: var(--text-sm)/1.6 var(--mono);
}
.session-runtime-info small {
  display: block;
  margin-top: 4px;
  font-size: var(--text-xs);
  line-height: 1.6;
  color: var(--muted);
}
@media (max-width: 520px) {
  .session-runtime-info > div {
    grid-template-columns: 1fr;
    gap: 5px;
  }
}
</style>
