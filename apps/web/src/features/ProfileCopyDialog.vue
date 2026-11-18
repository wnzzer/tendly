<script setup lang="ts">
/**
 * A new endpoint profile started from another, asking only what usually
 * differs: its name, client, URL, model and key. The rest comes from the
 * source (profile-copy.ts), and is all in the full form once it is created.
 */
import { computed, reactive, ref, watch } from "vue";
import type { AgentProviderKind, EndpointProfile, ModelCatalog } from "@tendly/protocol";
import { errorMessage, json, providerLabel, request } from "./api";
import { AGENT_CLIENTS, clientLabel } from "./clients";
import { copiedProfilePayload, copyName } from "./profile-copy";
import { referencedSecret, secretNameFor, type SecretName } from "./secret-names";
import { backendCapabilities } from "./backend-capabilities";
import { useI18n } from "../i18n";
import ModalDialog from "./ModalDialog.vue";
import ModelPicker from "./ModelPicker.vue";
import Icon from "./Icon.vue";

const props = defineProps<{ source: EndpointProfile; profiles: EndpointProfile[] }>();
const emit = defineEmits<{ close: []; created: [profile: EndpointProfile] }>();
const { t } = useI18n();

const form = reactive({
  name: copyName(
    props.profiles.map(profile => profile.name),
    n => t(n === 1 ? "{name} copy" : "{name} copy {n}", { name: props.source.name, n }),
  ),
  provider: props.source.provider as AgentProviderKind,
  endpoint_url: props.source.endpoint_url ?? "",
  model: props.source.model ?? "",
});
/** Keep the source's key (shared by reference, never read back), or paste one for this profile alone. */
const keyMode = ref<"keep" | "new">(props.source.secret_ref ? "keep" : "new"),
  apiKey = ref("");
const canPaste = computed(() => backendCapabilities.storedSecrets);
const busy = ref(false),
  error = ref("");
const discovering = ref(false),
  catalog = ref<ModelCatalog>(),
  modelError = ref("");

// Another client speaks to its endpoint differently: its model is chosen again.
watch(
  () => form.provider,
  provider => {
    form.model = provider === props.source.provider ? (props.source.model ?? "") : "";
  },
);
watch(
  () => [form.provider, form.endpoint_url, keyMode.value],
  () => {
    catalog.value = undefined;
    modelError.value = "";
  },
);
watch(apiKey, () => {
  pastedRef = undefined;
  catalog.value = undefined;
});

/** A pasted key is stored once, under a new name, so the source keeps its own. */
let pastedRef: string | undefined;
async function secretRef(): Promise<string | null> {
  if (keyMode.value === "keep") return props.source.secret_ref;
  const value = apiKey.value.trim();
  if (!value) return null;
  if (pastedRef) return pastedRef;
  const stored = await request<SecretName[]>("/secrets").catch(() => [] as SecretName[]);
  const taken = [
    ...stored.map(entry => entry.name),
    ...props.profiles.map(profile => referencedSecret(profile.secret_ref)).filter((name): name is string => !!name),
  ];
  const name = secretNameFor(form.name.trim() || providerLabel(form.provider), taken);
  await request<SecretName>("/secrets/" + encodeURIComponent(name), json("PUT", { value }));
  apiKey.value = "";
  pastedRef = "env:" + name;
  return pastedRef;
}
async function payload(withEnvironment: boolean) {
  return copiedProfilePayload(
    props.source,
    { ...form, secret_ref: await secretRef() },
    withEnvironment && backendCapabilities.environment,
  );
}

async function discover() {
  if (discovering.value || !backendCapabilities.models) return;
  discovering.value = true;
  modelError.value = "";
  // Discovery is not a launch: no environment goes to the endpoint.
  try {
    catalog.value = await request<ModelCatalog>(
      "/endpoint-profiles/discover-models",
      json("POST", await payload(false)),
    );
  } catch (cause) {
    modelError.value = errorMessage(cause);
  } finally {
    discovering.value = false;
  }
}
async function create() {
  if (busy.value || !form.name.trim()) return;
  busy.value = true;
  error.value = "";
  try {
    emit("created", await request<EndpointProfile>("/endpoint-profiles", json("POST", await payload(true))));
  } catch (cause) {
    error.value = errorMessage(cause);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <ModalDialog :title="t('New profile from {name}', { name: source.name })" :closable="!busy" @close="emit('close')">
    <form class="form-stack profile-copy" @submit.prevent="create">
      <div v-if="error" class="inline-error" role="alert">{{ error }}</div>
      <label
        >{{ t("Name")
        }}<input
          v-model="form.name"
          required
          autofocus
          maxlength="120"
          :disabled="busy"
          @focus="($event.target as HTMLInputElement).select()"
      /></label>
      <label
        >{{ t("Provider")
        }}<select v-model="form.provider" :disabled="busy">
          <option v-for="id in AGENT_CLIENTS" :key="id" :value="id">{{ clientLabel(id) }}</option>
        </select></label
      >
      <label
        >{{ t("Endpoint URL")
        }}<input
          v-model="form.endpoint_url"
          type="url"
          :placeholder="t('Official endpoint when empty')"
          autocomplete="off"
          :disabled="busy"
      /></label>
      <fieldset v-if="source.secret_ref || canPaste" class="profile-copy-key" :disabled="busy">
        <legend>{{ t("API key") }}</legend>
        <label v-if="source.secret_ref"
          ><input v-model="keyMode" type="radio" value="keep" />{{
            t("Use the key of {name}", { name: source.name })
          }}</label
        >
        <label v-if="source.secret_ref && canPaste"
          ><input v-model="keyMode" type="radio" value="new" />{{ t("Paste a new key for this profile") }}</label
        >
        <input
          v-if="keyMode === 'new' && canPaste"
          v-model="apiKey"
          type="password"
          autocomplete="new-password"
          spellcheck="false"
          :aria-label="t('API key')"
          :placeholder="t('Empty for an endpoint without a key')"
        />
      </fieldset>
      <div class="profile-copy-model">
        <label
          >{{ t("Default model")
          }}<ModelPicker
            :key="form.provider"
            v-model="form.model"
            :models="catalog?.models ?? []"
            :placeholder="t('Choose a model or enter an ID')"
            :disabled="busy"
        /></label>
        <button
          v-if="backendCapabilities.models"
          type="button"
          class="small-button"
          :disabled="discovering || busy"
          :title="t('Reads the endpoint\'s model list. No model is run.')"
          @click="discover"
        >
          <Icon name="refresh" :size="14" />{{ discovering ? t("Loading models…") : t("Load models") }}
        </button>
      </div>
      <p v-if="modelError" class="inline-error" role="alert">{{ modelError }}</p>
      <p v-else-if="catalog" class="form-help">{{ t("{count} models returned", { count: catalog.models.length }) }}</p>
      <p class="form-help">
        {{
          form.provider === source.provider
            ? t(
                "Proxy, permission intent and environment variables come from {name}. Change them in the full form after it is created.",
                { name: source.name },
              )
            : t(
                "Proxy, permission intent and your environment variables come from {name}; settings only {client} reads are left out. Change them in the full form after it is created.",
                { name: source.name, client: clientLabel(source.provider as AgentProviderKind) },
              )
        }}
      </p>
      <div class="dialog-actions">
        <span class="flex-spacer" /><button
          type="button"
          class="secondary-button"
          :disabled="busy"
          @click="emit('close')"
        >
          {{ t("Cancel") }}</button
        ><button class="primary-button" :disabled="busy || !form.name.trim()">
          {{ busy ? t("Saving…") : t("Create profile") }}
        </button>
      </div>
    </form>
  </ModalDialog>
</template>

<style scoped>
.profile-copy {
  display: flex;
  flex-direction: column;
  gap: 12px;
  min-width: 0;
}
.profile-copy label {
  display: flex;
  flex-direction: column;
  gap: 6px;
  font-size: var(--text-sm);
  color: var(--ink-soft);
}
.profile-copy-key {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin: 0;
  padding: 0;
  border: 0;
}
.profile-copy-key > legend {
  margin-bottom: 6px;
  padding: 0;
  font-size: var(--text-sm);
  color: var(--ink-soft);
}
.profile-copy-key > label {
  flex-direction: row;
  align-items: center;
  gap: 8px;
  cursor: pointer;
}
/* Not the full-width text box every .form-stack input gets (workflows.css). */
.profile-copy-key input[type="radio"] {
  display: inline-block;
  flex: none;
  width: 14px;
  height: 14px;
  min-height: 0;
  margin: 0;
  padding: 0;
  border: 0;
  box-shadow: none;
  accent-color: var(--accent);
}
.profile-copy-key input[type="radio"]:focus {
  box-shadow: none;
}
.profile-copy-key input[type="radio"]:focus-visible {
  outline: 2px solid var(--accent-soft);
  outline-offset: 2px;
}
.profile-copy-model {
  display: flex;
  align-items: flex-end;
  gap: 8px;
}
.profile-copy-model > label {
  flex: 1;
  min-width: 0;
}
.profile-copy-model > .small-button {
  flex: none;
  min-height: 36px;
}
.form-help {
  font-size: var(--text-xs);
  line-height: 1.7;
  color: var(--muted);
}
.dialog-actions {
  display: flex;
  gap: 8px;
  margin-top: 6px;
}
</style>
