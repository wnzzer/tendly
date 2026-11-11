<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { EndpointProfile, Session } from "@tendly/protocol";
import { backendCapabilities } from "./backend-capabilities";
import { errorMessage, json, request } from "./api";
import { useI18n } from "../i18n";
const props = defineProps<{ session: Session; profiles: EndpointProfile[]; disabled?: boolean }>();
const emit = defineEmits<{ profiles: []; changed: [] }>();
const { t } = useI18n();
const selected = ref(props.session.endpoint_profile_id ?? "");
const confirmed = ref(false), busy = ref(false), error = ref("");
const matching = computed(() => props.profiles.filter(profile => profile.provider === props.session.provider));
const available = computed(() => backendCapabilities.sessionConfiguration && props.session.provider !== "terminal" && !props.session.native_source_id && !props.session.resume_source_id);
const changed = computed(() => selected.value !== (props.session.endpoint_profile_id ?? ""));
const needsConfirmation = computed(() => changed.value && !!props.session.provider_session_id);
watch(() => [props.session.id, props.session.endpoint_profile_id], () => { selected.value = props.session.endpoint_profile_id ?? ""; confirmed.value = false; error.value = ""; });
watch(selected, () => { confirmed.value = false; error.value = ""; });
async function prepare(): Promise<Session | undefined> {
  if (busy.value || props.disabled) return undefined;
  if (!changed.value || !available.value) return props.session;
  if (needsConfirmation.value && !confirmed.value) return undefined;
  const id = props.session.id;
  busy.value = true; error.value = "";
  try {
    const session = await request<Session>(`/sessions/${encodeURIComponent(id)}/configuration`, json("PATCH", { endpoint_profile_id: selected.value || null, confirmed: true }));
    if (props.session.id !== id) return undefined;
    emit("changed");
    return session;
  } catch (cause) { if (props.session.id === id) error.value = errorMessage(cause); return undefined; }
  finally { busy.value = false; }
}
defineExpose({ prepare, busy });
</script>
<template>
  <div v-if="available" class="terminal-endpoint-setup">
    <label>{{ t('Endpoint profile') }}<select v-model="selected" :disabled="disabled || busy" :aria-label="t('Endpoint profile')">
      <option v-if="!session.endpoint_profile_id" value="" disabled>{{ t('Blank configuration · separate sign-in') }}</option>
      <option v-for="profile in matching" :key="profile.id" :value="profile.id">{{ profile.name }}</option>
      <option v-if="session.endpoint_profile_id && !matching.some(profile => profile.id === session.endpoint_profile_id)" :value="session.endpoint_profile_id">{{ session.endpoint_snapshot?.name || session.endpoint_profile_id }}</option>
    </select></label>
    <button type="button" class="text-button" :disabled="disabled || busy" @click="emit('profiles')">{{ t('Manage endpoint profiles') }}</button>
    <p>{{ t('The selected endpoint is applied before the terminal starts.') }}</p>
    <label v-if="needsConfirmation" class="terminal-endpoint-confirm"><input v-model="confirmed" type="checkbox" :disabled="disabled || busy" />{{ t('Changing the endpoint starts a new native context. Earlier messages are not sent to it.') }}</label>
    <p v-if="error" class="inline-error" role="alert">{{ error }}</p>
  </div>
  <div v-else-if="session.resume_source_id && session.provider !== 'terminal'" class="terminal-endpoint-setup terminal-endpoint-pinned">
    <strong>{{ t('Endpoint: {name}', { name: session.endpoint_snapshot?.name || t('Blank configuration · separate sign-in') }) }}</strong>
    <p>{{ t('This terminal continues the original conversation with the endpoint it used. To use another endpoint, switch it on the original session, then open the terminal again.') }}</p>
  </div>
</template>
<style scoped>
.terminal-endpoint-setup{display:grid;gap:8px;width:min(100%,360px);margin:4px 0 16px;text-align:left}.terminal-endpoint-setup>label{display:grid;gap:6px;font-size:var(--text-sm);color:var(--ink-soft)}select{width:100%;min-height:36px;padding:6px 9px;border:1px solid var(--border);border-radius:var(--radius-sm);background:var(--surface);color:var(--ink);font:inherit}.terminal-endpoint-setup>button{justify-self:start;font-size:var(--text-sm)}.terminal-endpoint-setup>p{margin:0;font-size:var(--text-xs);line-height:1.6}.terminal-endpoint-setup>.terminal-endpoint-confirm{display:flex;align-items:flex-start;gap:8px;line-height:1.6}.terminal-endpoint-confirm>input{margin-top:4px;flex:none}.terminal-endpoint-pinned>strong{font-size:var(--text-sm);font-weight:550;color:var(--ink-soft)}@media(pointer:coarse){select{min-height:44px}}
</style>
