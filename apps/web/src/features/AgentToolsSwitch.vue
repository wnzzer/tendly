<script setup lang="ts">
/**
 * One session's choice about Tendly's agent tools (server agent.rs):
 * follow the preference for every session, or on, or off for this one. The
 * tools are injected at launch, so a change applies at the next start.
 */
import { computed, onMounted, ref } from "vue";
import MenuFlyout from "./MenuFlyout.vue";
import Icon from "./Icon.vue";
import { errorMessage, json, request } from "./api";
import { useI18n } from "../i18n";

interface State {
  choice: boolean | null;
  default: boolean;
  enabled: boolean;
}
const props = defineProps<{ sessionId: string }>();
const { t } = useI18n();
const state = ref<State>(),
  busy = ref(false),
  error = ref("");
const path = () => `/sessions/${encodeURIComponent(props.sessionId)}/agent-tools`;

onMounted(async () => {
  try {
    state.value = await request<State>(path());
  } catch {
    /* An older server: nothing to show. */
  }
});
const options = computed(() =>
  state.value
    ? [
        { choice: null, label: t(state.value.default ? "Default · on" : "Default · off") },
        { choice: true, label: t("On") },
        { choice: false, label: t("Off") },
      ]
    : [],
);
const current = computed(() => options.value.find(option => option.choice === state.value?.choice)?.label);
async function choose(choice: boolean | null) {
  if (busy.value || state.value?.choice === choice) return;
  busy.value = true;
  error.value = "";
  try {
    state.value = await request<State>(path(), json("PUT", { choice }));
  } catch (cause) {
    error.value = errorMessage(cause);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <!-- A row like the menu's others: the choice opens beside it, with when it applies. -->
  <MenuFlyout
    v-if="state"
    class="agent-tools-row"
    :label="t('Tendly tools')"
    icon="spark"
    :value="current"
    :width="220"
  >
    <button
      v-for="option in options"
      :key="String(option.choice)"
      type="button"
      role="menuitemradio"
      :aria-checked="state.choice === option.choice"
      :disabled="busy"
      @click.stop="choose(option.choice)"
    >
      <Icon v-if="state.choice === option.choice" name="check" :size="14" /><span v-else class="agent-tools-space" />{{
        option.label
      }}
    </button>
    <p>{{ error || t("Takes effect when this session next starts.") }}</p>
  </MenuFlyout>
</template>
<style scoped>
.agent-tools-space {
  flex: none;
  width: 14px;
}
</style>
