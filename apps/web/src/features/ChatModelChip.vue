<script setup lang="ts">
/**
 * The composer's model chip: the models this session's client and endpoint
 * offer, narrowed by what is typed, and any other ID entered by hand. On a
 * phone its card also holds the endpoint list, passed in as the default slot,
 * since the models depend on the endpoint.
 *
 * It only says which model was picked; switching is the session's.
 */
import { computed, ref } from "vue";
import type { Session } from "@tendly/protocol";
import ChipMenu from "./ChipMenu.vue";
import Icon from "./Icon.vue";
import ProviderIcon from "./ProviderIcon.vue";
import { providerLabel } from "./api";
import type { NativeModel } from "./chat-model";
import { filterModels, resolveModel } from "./model-choices";
import { describeModel, resolvedDefaultName } from "./model-description";
import { useMobile } from "./mobile";
import { useI18n } from "../i18n";

const props = defineProps<{
  session: Session;
  /** What the client and the endpoint offer, merged. */
  models: NativeModel[];
  /** The model in use, or the one the session will start with. */
  current?: string;
  /** There is a model to name or choose: the chip shows one rather than the endpoint. */
  tunable: boolean;
  /** A model can be switched to now. */
  pickable: boolean;
  /** A phone's card can open for its endpoint list even when no model can be picked. */
  openable: boolean;
  endpointName: string;
  /** The endpoint's own models, under the IDs this client runs them by: what a typed name resolves against. */
  endpointCatalog: NativeModel[];
  /** The endpoint answered with a catalog of its own, so the list is not only the client's. */
  fromEndpoint: boolean;
}>();
const emit = defineEmits<{ pick: [id: string] }>();
const { t } = useI18n();
const mobile = useMobile();
const menu = ref<InstanceType<typeof ChipMenu>>();
/** The session closes the card after a pick, wherever the pick was made in it. */
defineExpose({ close: (restoreFocus?: boolean) => menu.value?.close(restoreFocus) });

const currentEntry = computed(() => props.models.find(entry => entry.id === props.current));
/**
 * A model typed by hand rather than picked from the list.
 *
 * The list is whatever the host's client version knows about, so a model newer
 * than that client never appears in it even when the account can run it. The
 * client still accepts any ID and refuses one it cannot serve, so typing the ID
 * is the way onto it without waiting for a client upgrade.
 */
const customModel = ref("");
const customPicked = ref("");
/** The menu's list, narrowed by what is typed below it: with an endpoint's hundred models, typing finds one. */
const visibleModels = computed(() => filterModels(props.models, customModel.value));
const modelDefault = computed(() =>
  currentEntry.value
    ? currentEntry.value.isDefault === true || currentEntry.value.id === "default"
    : !customPicked.value || props.current !== customPicked.value,
);
/**
 * The model chip always names the model in use, the default one too: which
 * model is answering is the first thing a reader of the conversation wants to
 * know. Whether it was chosen shows in the chip's emphasis, not its words.
 */
const shownModel = computed(() => currentEntry.value);
const modelName = computed(
  () =>
    (shownModel.value && resolvedDefaultName(shownModel.value, props.models)) ??
    shownModel.value?.name ??
    props.current ??
    t("Model"),
);
function pickCustomModel() {
  // A model's name typed by hand goes out as its ID, which is what the endpoint knows.
  const id = resolveModel(customModel.value, props.endpointCatalog);
  if (!id) return;
  customPicked.value = id;
  customModel.value = "";
  emit("pick", id);
}
</script>

<template>
  <ChipMenu
    ref="menu"
    class="chat-model"
    :caption="t('Model')"
    :label="tunable ? modelName : endpointName"
    :active="tunable && !modelDefault"
    :disabled="mobile ? !pickable && !openable : !pickable"
    :title="pickable ? t('Model') : t('Model: {model}', { model: modelName })"
    ><template #mark
      ><ProviderIcon v-if="mobile" :provider="session.provider" :size="13" /><Icon
        v-else
        name="spark"
        :size="12" /></template
    ><template v-if="tunable"
      ><p v-if="mobile" class="chat-card-group">
        <span>{{ endpointName }}</span><em>{{ t("Current endpoint") }}</em>
      </p>
      <p v-if="mobile && !pickable" class="chat-menu-note">
        {{ t("Wait for the current turn and approvals to finish before changing this.") }}
      </p>
      <nav :aria-label="t('Model')">
        <button
          v-for="entry in visibleModels"
          :key="entry.id"
          type="button"
          :class="{ selected: entry.id === current }"
          :disabled="!pickable"
          @click="emit('pick', entry.id)"
        >
          <strong>{{ entry.name }}</strong><code
            v-if="entry.id.toLocaleLowerCase() !== entry.name.toLocaleLowerCase()"
            class="chat-model-id"
            >{{ entry.id }}</code
          ><small v-if="entry.description">{{ describeModel(entry.description, t) }}</small></button
        ><button
          v-if="customPicked && !models.some(entry => entry.id === customPicked)"
          type="button"
          :class="{ selected: customPicked === current }"
          @click="emit('pick', customPicked)"
        >
          <strong>{{ customPicked }}</strong><small>{{ t("Entered by hand") }}</small>
        </button>
      </nav>
      <div class="chat-model-custom">
        <input
          v-model="customModel"
          type="text"
          autocomplete="off"
          spellcheck="false"
          maxlength="128"
          :placeholder="t('Find a model, or enter an ID')"
          :aria-label="t('Other model ID')"
          @keydown.enter.prevent.stop="pickCustomModel"
        /><button type="button" :disabled="!customModel.trim()" @click="pickCustomModel">{{ t("Use") }}</button>
      </div>
      <p class="chat-model-hint">
        {{
          t(
            fromEndpoint
              ? "From the {provider} client on the host and from this endpoint. Type to find a model, or enter an ID."
              : "This list comes from the {provider} client on the host. Upgrade it there to see newer models, or enter an ID.",
            { provider: providerLabel(session.provider) },
          )
        }}
      </p></template
    >
    <slot />
  </ChipMenu>
</template>

<style scoped>
/* The pane's defaults for its controls (ChatSessionPane). Its scoped rules do
   not reach into this component, so they are repeated here, first, where the
   pane has them. */
.chat-pane button:disabled {
  cursor: not-allowed;
  opacity: 0.5;
}
.chat-pane button:focus-visible,
.chat-pane summary:focus-visible,
.chat-pane select:focus-visible,
.chat-pane input:focus-visible,
.chat-pane textarea:focus-visible {
  outline: 2px solid var(--focus);
  outline-offset: 3px;
}
@media (prefers-reduced-motion: reduce) {
  .chat-pane * {
    scroll-behavior: auto;
  }
}
.chat-model {
  max-width: min(34%, 170px);
}
.chat-model :deep(.chip-menu-panel) {
  min-width: 190px;
  width: max-content;
  max-width: min(292px, 78cqw);
  max-height: min(340px, 66vh);
  overflow-y: auto;
  padding: 5px;
}
.chat-model nav button strong {
  display: block;
  font-size: var(--text-sm);
  font-weight: 600;
}
.chat-model nav button small {
  display: block;
  margin-top: 2px;
  font-size: var(--text-xs);
  color: var(--muted);
  white-space: normal;
  line-height: 1.5;
}
.chat-model nav button {
  white-space: normal;
}
.chat-model nav button {
  display: block;
  width: 100%;
  min-height: 38px;
  padding: 8px 10px;
  border: 0;
  border-radius: var(--radius-md);
  background: none;
  text-align: left;
  font-size: var(--text-sm);
  color: var(--ink-soft);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  cursor: pointer;
}
.chat-model nav button:hover:not(:disabled) {
  background: var(--fill);
}
.chat-model nav button.selected {
  color: var(--accent-ink);
  font-weight: 600;
  background: var(--selected);
}
.chat-model-custom {
  display: flex;
  gap: 6px;
  margin: 5px 3px 3px;
  padding-top: 6px;
  border-top: 1px solid var(--line);
  position: sticky;
  bottom: -5px;
  background: var(--surface);
  padding-bottom: 5px;
}
/* The ID a model goes out as, under the name it is known by. */
.chat-model nav button .chat-model-id {
  display: block;
  margin-top: 2px;
  font-family: var(--mono);
  font-size: var(--text-xs);
  color: var(--muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.chat-model-custom input {
  flex: 1;
  min-width: 0;
  height: 30px;
  padding: 0 8px;
  border: 1px solid var(--line);
  border-radius: var(--radius-sm);
  font: inherit;
  font-size: var(--text-xs);
  background: var(--surface);
  color: inherit;
}
.chat-model-custom button {
  height: 30px;
  padding: 0 10px;
  border: 0;
  border-radius: var(--radius-sm);
  background: var(--teal);
  color: var(--on-accent);
  font-size: var(--text-xs);
  cursor: pointer;
}
.chat-model-hint {
  margin: 4px 5px 3px;
  font-size: var(--text-xs);
  line-height: 1.5;
  color: var(--muted);
  white-space: normal;
}
.chat-composer.is-phone .chat-model {
  min-width: 0;
  flex: 0 1 auto;
}
.chat-composer.is-phone .chat-model :deep(.chip-menu-label) {
  max-width: 30vw;
}
/* The model's card: find a model first, then its models, then the endpoints they come from. */
.chat-composer.is-phone .chat-model :deep(.chip-menu-panel) {
  display: flex;
  flex-direction: column;
}
.chat-composer.is-phone .chat-model .chat-model-custom {
  order: -1;
  position: static;
  margin: 4px 6px 4px;
  padding: 0;
  border-top: 0;
  background: none;
}
.chat-composer.is-phone .chat-model .chat-model-custom input {
  height: 44px;
  padding: 0 12px;
  border-radius: var(--radius-md);
  font-size: var(--input-text);
}
.chat-composer.is-phone .chat-model .chat-model-custom button {
  height: 44px;
  padding: 0 16px;
  border-radius: var(--radius-md);
  font-size: var(--text-base);
}
.chat-composer.is-phone .chat-model .chat-model-hint {
  margin: 4px 12px 6px;
  font-size: var(--text-sm);
}
</style>
