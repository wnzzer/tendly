<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, reactive, ref, watch } from "vue";
import type { AgentProviderKind, EndpointProfile, ModelCatalog } from "@tendly/protocol";
import { errorMessage, json, providerLabel, request } from "./api";
import { parseModelAliases, formatModelAliases } from "./endpoint-models";
import { sharesHostConfig, nativeProfileRenamePayload, nativeProfileUpdatePayload, profileEnvironmentPayload, profileEnvironmentDraftChanged } from "./native-profiles";
import { environmentRows, newEnvironmentRow, type EnvironmentRow } from "./environment-model";
import { limitKey, nextLimits, validWindow, windowValue, type Limits } from "./model-limits";
import ModelSelection from "./ModelSelection.vue";
import { offeredModels } from "./model-choices";
import EndpointBalanceEditor from "./EndpointBalanceEditor.vue";
import EnvironmentEditor from "./EnvironmentEditor.vue";
import { AGENT_CLIENTS, DEFAULT_CLIENT, clientInfo, clientLabel } from "./clients";
import { SLOTS, followingRows, setSlot, slotState, slotSummary, type SlotVariable } from "./claude-slots";
import { effortLabel, modelEfforts } from "./reasoning-effort";
import { useI18n } from "../i18n";
import { backendCapabilities } from "./backend-capabilities";
import ModalDialog from "./ModalDialog.vue";
import Icon from "./Icon.vue";
import ProviderIcon from "./ProviderIcon.vue";
import ModelPicker from "./ModelPicker.vue";
import ContextMenu from "./ContextMenu.vue";
import ProfileCopyDialog from "./ProfileCopyDialog.vue";
import { loadPreferences, preferenceFor } from "./preferences";
import { showToast } from "./toasts";
import { isStored, ownStoredSecret, referencedSecret, secretNameFor, secretStatus, type SecretName } from "./secret-names";
import { profilePayload, profileSummary as describeProfile } from "./profile-form";
const { t } = useI18n();
const props = defineProps<{ profiles: EndpointProfile[]; embedded?: boolean }>();
const embedded = computed(() => props.embedded === true);
const emit = defineEmits<{ close: []; changed: [profile?: EndpointProfile]; accounts: [] }>();
// The settings shell owns the outer close, so it needs this page's unsaved guard.
defineExpose({ requestLeave: (action: () => void) => requestLeave(action) });
const balanceEditor = ref<InstanceType<typeof EndpointBalanceEditor>>();
const editing = ref<string>(), deleteId = ref<string>();
const editingRecord = ref<EndpointProfile>();
const editingProfile = computed(() => props.profiles.find(profile => profile.id === editing.value) ?? editingRecord.value);
const editingNative = computed(() => editingProfile.value?.native_config);
const editingManaged = computed(() => editingNative.value?.source_id.startsWith('account:'));
const formVisible = ref(true), busy = ref(false), discovering = ref(false);
const error = ref(""), modelError = ref("");
const models = ref<ModelCatalog>(), aliasesText = ref("");
const environmentDraft = ref<EnvironmentRow[]>([]), initialEnvironment = ref<EnvironmentRow[]>([]);
const environmentDirty = computed(() => profileEnvironmentDraftChanged(environmentDraft.value, initialEnvironment.value));
const environmentError = computed(() => { try { profileEnvironmentPayload(environmentDraft.value, backendCapabilities.environment, initialEnvironment.value); return ""; } catch (cause) { return errorMessage(cause); } });
const pendingNavigation = ref<(() => void)>();
const form = reactive({ name: "", provider: DEFAULT_CLIENT as AgentProviderKind, endpoint_url: "", api: "", model: "", effort: "", permission_mode: "native" as EndpointProfile["permission_mode"], secret_ref: "", proxy_url: "" });
const secretValid = computed(() => secretMode.value === "stored" || !form.secret_ref || /^env:TENDLY_SECRET_[A-Z0-9_]+$/.test(form.secret_ref));
/**
 * Where the endpoint's key lives. "stored": pasted here and kept by Tendly
 * in the state directory (secrets.rs), usable at once. "env": a variable set
 * on the server by hand, as before. Either way the profile only ever holds
 * the reference, and the key itself never comes back to the browser.
 */
const secretMode = ref<"stored" | "env">("stored"), apiKey = ref("");
const storedSecrets = ref<SecretName[]>([]);
const otherReferences = computed(() => props.profiles.filter(profile => profile.id !== editing.value).map(profile => profile.secret_ref));
const secret = computed(() => secretStatus(form.secret_ref, storedSecrets.value));
/** The stored key this profile uses now, shown as "saved". */
const storedKey = computed(() => secret.value.stored);
/** The variable the server sets that this profile reads, while Tendly has no copy of it. */
const adoptable = computed(() => secret.value.adoptable);
/** The server also sets the stored key's name, and wins while it does. */
const shadowed = computed(() => secret.value.shadowed);
const adopting = ref(false);
/** Keep the server's value in Tendly: read and written on the server, never sent here. */
async function adoptKey() {
  const name = adoptable.value;
  if (!name || adopting.value) return;
  adopting.value = true; error.value = "";
  try { await request<SecretName>(`/secrets/${encodeURIComponent(name)}/adopt`, json("POST")); await loadSecrets(); secretMode.value = "stored"; }
  catch (cause) { error.value = errorMessage(cause); }
  finally { adopting.value = false; }
}
async function loadSecrets() {
  if (!backendCapabilities.storedSecrets) return;
  try { storedSecrets.value = await request<SecretName[]>("/secrets"); } catch { /* The form still works with references. */ }
}
// The backend's capabilities can arrive after this page opens (a reload
// straight onto Settings); until they do, no key list is read, and the form
// would think every key lives only on the server.
watch(() => backendCapabilities.storedSecrets, available => {
  if (available) void loadSecrets().then(() => { if (!apiKey.value) chooseSecretMode(form.secret_ref); });
});
function chooseSecretMode(reference: string | null | undefined) {
  const name = referencedSecret(reference);
  secretMode.value = !backendCapabilities.storedSecrets || (name && !isStored(storedSecrets.value, name)) ? "env" : "stored";
}
/** Store a pasted key and point the form at it. Its own key is replaced in
 * place; otherwise a fresh name, so no other profile's key is overwritten. */
async function storePastedKey() {
  const value = apiKey.value.trim();
  if (secretMode.value !== "stored" || !value) return;
  const name = ownStoredSecret(form.secret_ref, storedSecrets.value, otherReferences.value)
    ?? secretNameFor(form.name.trim() || providerLabel(form.provider), [...storedSecrets.value.map(entry => entry.name), ...otherReferences.value.map(referencedSecret).filter((item): item is string => !!item)]);
  await request<SecretName>("/secrets/" + encodeURIComponent(name), json("PUT", { value }));
  form.secret_ref = "env:" + name; apiKey.value = "";
  await loadSecrets();
}
/** A stored key no profile points at any more is deleted with it. */
async function releaseSecret(reference: string | null | undefined, remaining: Array<string | null | undefined>) {
  const name = referencedSecret(reference);
  if (!name || !isStored(storedSecrets.value, name) || remaining.some(other => referencedSecret(other) === name)) return;
  try { await request("/secrets/" + encodeURIComponent(name), json("DELETE")); } catch { /* An orphaned key is harmless. */ }
  await loadSecrets();
}
const deletingProfile = computed(() => props.profiles.find(p => p.id === deleteId.value) ?? (editingRecord.value?.id === deleteId.value ? editingRecord.value : undefined));
const aliases = computed(() => { try { return { value: parseModelAliases(aliasesText.value), error: "" }; } catch (cause) { return { value: {} as Record<string,string>, error: errorMessage(cause) }; } });
type ProtocolCheck = { protocol: string; status: 'supported' | 'unsupported' | 'unknown'; http_status: number | null };
const protocolChecks = ref<ProtocolCheck[]>([]), checkingProtocols = ref(false), protocolError = ref("");
let protocolRevision = 0;
watch(() => [form.provider, form.endpoint_url, form.proxy_url, form.secret_ref, form.api, form.model, apiKey.value, aliasesText.value], () => {
  protocolRevision++; protocolChecks.value = []; protocolError.value = "";
});
async function checkProtocols() {
  if (checkingProtocols.value || !form.model.trim() || !secretValid.value || aliases.value.error) return;
  checkingProtocols.value = true; protocolError.value = ""; protocolChecks.value = [];
  try {
    await storePastedKey(); await nextTick();
    const revision = protocolRevision;
    try {
      const result = await request<ProtocolCheck[]>("/endpoint-profiles/check-protocols", json("POST", payload(false)));
      if (revision === protocolRevision) protocolChecks.value = result;
    } catch (cause) { if (revision === protocolRevision) protocolError.value = errorMessage(cause); }
  } catch (cause) { protocolError.value = errorMessage(cause); }
  finally { checkingProtocols.value = false; }
}
const resolved = computed(() => aliases.value.value[form.model] ?? form.model);
const offered = computed(() => offeredModels(models.value?.models ?? [], chosenModels.value));
const choices = computed(() => [...Object.entries(aliases.value.value).map(([id, actual]) => ({id,name:id + " → " + actual})), ...offered.value]);
const effortOptions = computed(() => modelEfforts(form.provider, resolved.value || undefined, models.value, form.effort, !editingNative.value));
const canSave = computed(() => !!form.name.trim() && !environmentError.value && contextValid.value && (!!editingNative.value || (secretValid.value && !aliases.value.error)));
let discoveryRevision = 0;
function resetDiscovery() { discoveryRevision++; models.value = undefined; modelError.value = ""; discovering.value = false; }
watch(() => [form.provider, form.endpoint_url, form.proxy_url, form.secret_ref, form.api, apiKey.value], resetDiscovery);

/**
 * Context windows, kept per model (server model_limits.rs) so every session
 * on that model uses them: the default model's below it, any chosen model's
 * under its advanced settings. Empty leaves the client to decide; the
 * published window is shown as a reference but never applied on its own,
 * since an account may not have all of it.
 */
const savedLimits = ref<Limits>(), contextDrafts = reactive<Record<string, string>>({});
const publishedWindows = reactive<Record<string, number | null>>({});
async function loadLimits() {
  for (const key of Object.keys(contextDrafts)) delete contextDrafts[key];
  try {
    const saved = await request<{ models: Record<string, { context_window: number }> }>("/models/limits");
    savedLimits.value = Object.fromEntries(Object.entries(saved.models).map(([model, limit]) => [model, limit.context_window]));
  } catch { savedLimits.value = undefined; /* An older backend: no windows to set. */ }
}
function windowText(model: string) { const key = limitKey(model); return contextDrafts[key] ?? (savedLimits.value?.[key] ? String(savedLimits.value[key]) : ""); }
function setWindow(model: string, text: string) { contextDrafts[limitKey(model)] = text; }
async function lookPublished(model: string) {
  const key = limitKey(model);
  if (!key || key in publishedWindows) return;
  publishedWindows[key] = null;
  try { publishedWindows[key] = (await request<{ published: number | null }>(`/models/context?model=${encodeURIComponent(model.trim())}`)).published; } catch { /* Unknown stays unknown. */ }
}
function windowPlaceholder(model: string) { const published = publishedWindows[limitKey(model)]; return published ? t("Published: {tokens} tokens", { tokens: published.toLocaleString() }) : t("Unknown · the client decides"); }
watch(() => form.model, model => { if (model.trim()) void lookPublished(model); });
const contextValid = computed(() => Object.values(contextDrafts).every(validWindow));
/** Write the windows set here, and where the client reads the window from a variable (Claude Code), make sure the default model's is passed, as one listed below. */
async function saveContextWindows() {
  if (!savedLimits.value || !contextValid.value) return;
  const variable = client.value.contextWindow.variable;
  if (windowValue(windowText(form.model)) && variable && !environmentDraft.value.some(row => row.name.trim() === variable)) {
    environmentDraft.value = [...environmentDraft.value, { ...newEnvironmentRow(), name: variable, kind: "main_model_context" }];
  }
  const next = nextLimits(savedLimits.value, contextDrafts);
  if (!next) return;
  await request("/models/limits", json("PUT", { models: Object.fromEntries(Object.entries(next).map(([model, window]) => [model, { context_window: window }])) }));
  savedLimits.value = next;
  for (const key of Object.keys(contextDrafts)) delete contextDrafts[key];
}
/** The endpoint models this profile offers in a session's menu; none chosen offers them all. */
const chosenModels = ref<string[]>([]);
/** What the form offers and explains for the chosen client (clients.ts). */
const client = computed(() => clientInfo(form.provider));

/**
 * Claude Code's model slots, edited here and kept as the environment
 * variables listed below: the same rows, so nothing about them is hidden.
 */
const slotsOpen = ref(false);
const slotMode = (variable: SlotVariable) => slotState(environmentDraft.value, variable).mode;
function slotModel(variable: SlotVariable) { const state = slotState(environmentDraft.value, variable); return state.mode === "model" ? state.model : ""; }
function chooseSlot(variable: SlotVariable, mode: string) {
  environmentDraft.value = setSlot(environmentDraft.value, variable, mode === "model" ? { mode: "model", model: slotModel(variable) } : mode === "main" ? { mode: "main" } : { mode: "client" });
}
function pinSlot(variable: SlotVariable, model: string) { environmentDraft.value = setSlot(environmentDraft.value, variable, { mode: "model", model }); }
const slotsText = computed(() => t({
  main: "Opus, Sonnet and Haiku follow the main model.",
  client: "Opus, Sonnet and Haiku use Claude Code's own models.",
  mixed: "Opus, Sonnet and Haiku are set one by one.",
}[slotSummary(environmentDraft.value)]));
// A new profile's slots follow its main model on a client that has them
// (Claude Code), and are not there at all on one with a single model (Codex).
watch(() => form.provider, provider => {
  if (editing.value || editingRecord.value) return;
  const plain = environmentDraft.value.filter(row => !SLOTS.some(slot => slot.variable === row.name.trim()));
  environmentDraft.value = clientInfo(provider).modelSlots ? followingRows(plain) : plain;
  initialEnvironment.value = environmentDraft.value.map(row => ({ ...row }));
});
function payload(includeEnvironment = true) {
  if (editingNative.value) return includeEnvironment ? nativeProfileUpdatePayload(form.name, environmentDraft.value, backendCapabilities.environment, initialEnvironment.value) : nativeProfileRenamePayload(form.name);
  const environment = includeEnvironment ? profileEnvironmentPayload(environmentDraft.value, backendCapabilities.environment, initialEnvironment.value) : {};
  return { ...profilePayload(form, editingProfile.value, aliases.value.value, chosenModels.value, !!client.value.apis?.length), ...environment };
}
function requestLeave(action: () => void) {
  if (busy.value || balanceEditor.value?.busy) return;
  if (formVisible.value && (environmentDirty.value || balanceEditor.value?.dirty)) { pendingNavigation.value = action; return; }
  action();
}
function discardEnvironmentDraft() { balanceEditor.value?.reset(); const action = pendingNavigation.value; pendingNavigation.value = undefined; environmentDraft.value = initialEnvironment.value.map(row => ({ ...row })); action?.(); }
function edit(p?: EndpointProfile) {
  pendingNavigation.value = undefined;
  // A new profile starts on the default client, whose slots follow its main model.
  environmentDraft.value = p ? environmentRows(p.environment) : clientInfo(DEFAULT_CLIENT).modelSlots ? followingRows() : [];
  initialEnvironment.value = environmentDraft.value.map(row => ({ ...row }));
  resetDiscovery(); protocolRevision++; protocolChecks.value = []; protocolError.value = ""; editingRecord.value=p;
  editing.value=p?.id; formVisible.value=true; error.value=""; deleteId.value=undefined;
  Object.assign(form,{name:p?.name??"",provider:p?.provider??DEFAULT_CLIENT,endpoint_url:p?.endpoint_url??"",api:p?.api??"",model:p?.model??"",effort:p?.effort??"",permission_mode:p?.permission_mode??"native",secret_ref:p?.secret_ref??"",proxy_url:p?.proxy_url??""});
  apiKey.value = ""; chooseSecretMode(p?.secret_ref);
  aliasesText.value=formatModelAliases(p?.model_aliases); models.value=undefined; modelError.value="";
  chosenModels.value=[...(p?.models??[])]; void loadLimits();
}
async function discover() {
  if (!backendCapabilities.models || editingNative.value || discovering.value || !secretValid.value || aliases.value.error) return;
  // A pasted key is stored first: discovery resolves the reference on the
  // server, and storing it changes the reference, which resets discovery.
  if (secretMode.value === "stored" && apiKey.value.trim()) {
    try { await storePastedKey(); await nextTick(); } catch (cause) { modelError.value = errorMessage(cause); return; }
  }
  const revision=++discoveryRevision; discovering.value=true; modelError.value="";
  // Model discovery is not a child-process launch: never send draft environment
  // values or secret references to that endpoint.
  try { const result=await request<ModelCatalog>("/endpoint-profiles/discover-models",json("POST",payload(false))); if(revision===discoveryRevision) models.value=result; }
  catch(cause){if(revision===discoveryRevision)modelError.value=errorMessage(cause);}
  finally {if(revision===discoveryRevision)discovering.value=false;}
}
async function save() {
  if(busy.value||!canSave.value)return;
  busy.value=true;error.value="";
  try{
    const previous=editingProfile.value?.secret_ref;
    await storePastedKey();
    await saveContextWindows();
    const saved=await request<EndpointProfile>("/endpoint-profiles"+(editing.value?"/"+encodeURIComponent(editing.value):""),json(editing.value?"PATCH":"POST",payload()));
    await loadPreferences(true);
    if(previous!==saved.secret_ref)await releaseSecret(previous,[saved.secret_ref,...otherReferences.value]);
    const message=editingNative.value?(backendCapabilities.environment?"Profile updated. Process environment overrides were saved without changing the native configuration.":"Profile name updated. The native configuration was not changed."):editing.value?"Profile updated. Existing session snapshots are unchanged.":preferenceFor(saved.provider)?.endpoint_profile_id === saved.id ? "Profile created and set as the default endpoint in Preferences." : "Profile created. Select it when creating a session.";
    // Stay on what was just saved; the toast says it took, wherever the form is scrolled.
    pendingNavigation.value=undefined;edit(saved);showToast(t(message));emit("changed",saved);
  }catch(cause){error.value=errorMessage(cause);}finally{busy.value=false;}
}
async function remove(){
  if(!deleteId.value||busy.value)return;const id=deleteId.value,native=!!deletingProfile.value?.native_config;busy.value=true;error.value="";
  const reference=deletingProfile.value?.secret_ref;
  try{await request("/endpoint-profiles/"+encodeURIComponent(id),json("DELETE"));await releaseSecret(reference,props.profiles.filter(p=>p.id!==id).map(p=>p.secret_ref));deleteId.value=undefined;if(editing.value===id){formVisible.value=false;editing.value=undefined;editingRecord.value=undefined;}showToast(t(native?"Profile reference removed. Native settings, credentials and history remain on the host.":"Profile deleted; no credentials were deleted from the host."));emit("changed");}
  catch(cause){error.value=errorMessage(cause);}finally{busy.value=false;}
}
/** One line per profile: the provider is already its icon, and the directory
 * is shown once the profile is opened. */
const profileSummary = (p: EndpointProfile) => describeProfile(p, t);
/** Cancelling goes back to a profile rather than to an empty page. */
function cancelForm() {
  requestLeave(() => {
    const back = props.profiles.find(profile => profile.id === editing.value) ?? props.profiles[0];
    if (back) edit(back); else formVisible.value = false;
  });
}
/** A new profile started from another, in a short form of its own (ProfileCopyDialog). */
const copySource = ref<EndpointProfile>();
function duplicate(p: EndpointProfile) {
  menu.value = undefined;
  if (!p.native_config) requestLeave(() => { copySource.value = p; });
}
function copied(saved: EndpointProfile) {
  copySource.value = undefined;
  // Opened in the full form, for whatever the short one did not ask.
  pendingNavigation.value = undefined; edit(saved);
  showToast(t("Profile created. Select it when creating a session."));
  emit("changed", saved);
}

/** Right-click on a profile in the list: rename it in place, copy it, or delete it. */
const menu = ref<{ x: number; y: number; profile: EndpointProfile }>();
const renamingId = ref<string>(), renameText = ref("");
const renameInput = ref<HTMLInputElement[]>();
function openMenu(event: MouseEvent, p: EndpointProfile) {
  event.preventDefault();
  if (busy.value) return;
  menu.value = { x: event.clientX, y: event.clientY, profile: p };
}
const removable = (p: EndpointProfile) => !p.native_config?.source_id.startsWith("account:");
function startRename(p: EndpointProfile) {
  menu.value = undefined; renamingId.value = p.id; renameText.value = p.name;
  void nextTick(() => { renameInput.value?.[0]?.focus(); renameInput.value?.[0]?.select(); });
}
async function commitRename() {
  const id = renamingId.value, name = renameText.value.trim();
  // Cleared first: the input's blur follows Enter and must not save twice.
  renamingId.value = undefined;
  const p = props.profiles.find(profile => profile.id === id);
  if (!id || !p || !name || name === p.name || busy.value) return;
  busy.value = true; error.value = "";
  try {
    const saved = await request<EndpointProfile>("/endpoint-profiles/" + encodeURIComponent(id), json("PATCH", { name }));
    if (editing.value === id) { form.name = saved.name; editingRecord.value = saved; }
    showToast(t("Renamed to {name}", { name: saved.name }));
    emit("changed", saved);
  } catch (cause) { error.value = errorMessage(cause); }
  finally { busy.value = false; }
}
function askDelete(p: EndpointProfile) { menu.value = undefined; deleteId.value = p.id; }

// The form can be scrolled far from its top, where errors and the delete
// confirmation appear; bring them into view rather than leave them unseen.
const banner = ref<HTMLElement>();
watch(() => [error.value, deleteId.value], ([message, id]) => { if (message || id) void nextTick(() => banner.value?.scrollIntoView?.({ block: "nearest", behavior: "smooth" })); });

// Open on something to read rather than on a blank page that asks for a click.
if (props.profiles.length) edit(props.profiles[0]);
void loadSecrets().then(() => chooseSecretMode(form.secret_ref));
onBeforeUnmount(()=>{discoveryRevision++;protocolRevision++;});
</script>

<template>
  <ModalDialog :title="t('Endpoint profiles')" wide :closable="!busy" :embedded="embedded" @close="requestLeave(() => emit('close'))">
    <div class="profiles-layout">
      <section class="profiles-list">
        <button class="secondary-button" :disabled="busy" @click="requestLeave(() => edit())"><Icon name="plus" :size="14" />{{ t('New profile') }}</button>
        <template v-for="p in profiles" :key="p.id">
          <div v-if="renamingId===p.id" class="profile-card selected profile-renaming">
            <span :class="['provider-mark',p.provider]"><ProviderIcon :provider="p.provider" /></span>
            <input ref="renameInput" v-model="renameText" maxlength="120" :aria-label="t('Profile name')" @keydown.enter.prevent="commitRename" @keydown.esc.prevent="renamingId=undefined" @blur="commitRename" />
          </div>
          <button v-else :class="['profile-card',{selected:editing===p.id&&formVisible}]" :disabled="busy" @click="requestLeave(() => edit(p))" @contextmenu="openMenu($event, p)">
            <span :class="['provider-mark',p.provider]"><ProviderIcon :provider="p.provider" /></span>
            <span><strong>{{ p.name }}</strong><small :class="{'shared-profile-label':!!p.native_config}">{{ profileSummary(p) }}</small></span>
          </button>
        </template>
        <p v-if="!profiles.length" class="small-empty">{{ t('No profiles yet. Native, isolated sessions also work without one.') }}</p>
        <p v-if="embedded" class="small-empty profiles-accounts-hint">{{ t('Already signed in to Claude Code or Codex on this host?') }} <button type="button" class="profiles-accounts-link" @click="requestLeave(() => emit('accounts'))">{{ t('Link it in Official accounts') }} →</button></p>
      </section>
      <div class="profile-details">
        <div ref="banner" class="profile-banner">
          <div v-if="error" class="inline-error" role="alert">{{ error }}</div>
          <div v-if="deleteId" class="confirmation-bar">{{ t(deletingProfile?.native_config?'Remove this profile reference? Native settings, sign-in and history stay on the host. Existing sessions retain their reference.':'Delete this profile? Existing session snapshots remain.') }} {{ deletingProfile?.name }}<div class="toolbar-buttons"><button class="small-button danger" :disabled="busy" @click="remove">{{ t('Confirm delete') }}</button><button class="small-button" :disabled="busy" @click="deleteId=undefined">{{ t('Keep profile') }}</button></div></div>
        </div>
        <div v-if="pendingNavigation" class="confirmation-bar" role="alert">{{ t('Discard unsaved configuration changes?') }}<div class="toolbar-buttons"><button type="button" class="small-button danger" @click="discardEnvironmentDraft">{{ t('Discard changes') }}</button><button type="button" class="small-button" @click="pendingNavigation=undefined">{{ t('Keep editing') }}</button></div></div>
        <form v-if="formVisible" class="form-stack" @submit.prevent="save">
          <h3>{{ editing ? t('Edit profile') : t('Create profile') }}</h3>
          <div class="form-columns">
            <label>{{ t('Name') }}<input v-model="form.name" required :disabled="busy" :placeholder="t('Personal {client}', { client: providerLabel(form.provider) })" maxlength="120" /></label>
            <label>{{ t('Provider') }}<select v-model="form.provider" :disabled="!!editing||busy"><option v-for="id in AGENT_CLIENTS" :key="id" :value="id">{{ clientLabel(id) }}</option></select></label>
          </div>
          <template v-if="editingNative">
            <div class="shared-config-panel"><strong><ProviderIcon :provider="editingProfile!.provider" :size="17" />{{ sharesHostConfig(editingNative) ? t('Host configuration · shared sign-in') : t('Isolated configuration · this account only') }}</strong><code>{{ editingNative.config_dir }}</code><p v-if="sharesHostConfig(editingNative)">{{ t('Shared with the host: sign-in, model and permissions come from this directory, and anything else that edits it changes this profile too. Tendly never copies or rewrites it.') }}</p><p v-else>{{ t('Tendly created this directory for this account, and nothing else signs in through it.') }}</p></div>
          </template>
          <template v-else>
          <label>{{ t('Endpoint URL') }}<input v-model="form.endpoint_url" type="url" :placeholder="t('Official endpoint when empty')" autocomplete="off" /></label>
          <label v-if="client.apis?.length">{{ t('Endpoint API') }}<select v-model="form.api"><option value="">{{ t('Guess from the URL') }}</option><option v-for="api in client.apis" :key="api.value" :value="api.value">{{ api.label }}</option></select></label>
          <label>{{ t('Proxy URL') }}<input v-model="form.proxy_url" type="url" placeholder="http://127.0.0.1:7890" autocomplete="off" /></label>
          <p class="form-help">{{ t('Applied on the server. Empty uses the host network.') }}<template v-if="client.proxyNote"> {{ t(client.proxyNote) }}</template></p>
          <template v-if="secretMode==='stored'">
            <label>{{ t('API key') }}<input v-model="apiKey" type="password" autocomplete="new-password" spellcheck="false" :placeholder="storedKey ? t('Saved · leave empty to keep it') : t('Empty for an endpoint without a key')" /></label>
            <p class="form-help">{{ storedKey ? t('Kept as {name} in the state directory, readable only by you. It never returns to the browser.', { name: storedKey }) : t('Kept in the state directory, readable only by you, and usable at once. It never returns to the browser.') }}<button v-if="storedKey" type="button" class="text-button danger-text secret-clear" @click="form.secret_ref=''">{{ t('Stop using this key') }}</button></p>
            <p v-if="shadowed" class="form-help">{{ t("The server's environment also sets {name}, and that value is used until it is removed there.", { name: storedKey ?? '' }) }}</p>
            <button v-if="backendCapabilities.storedSecrets" type="button" class="text-button secret-switch" @click="secretMode='env'">{{ t('Use a server environment variable instead') }}</button>
          </template>
          <template v-else>
            <div v-if="adoptable && backendCapabilities.storedSecrets" class="inline-notice secret-adopt"><span>{{ t("This profile reads its key from {name} in the server's environment.", { name: adoptable }) }}</span><button type="button" class="small-button" :disabled="adopting" @click="adoptKey">{{ t('Keep it in Tendly') }}</button></div>
            <label>{{ t('Secret reference') }} <small>{{ t('never an API key') }}</small><input v-model="form.secret_ref" class="field-mono" placeholder="env:TENDLY_SECRET_PROVIDER_KEY" autocomplete="off" spellcheck="false" /></label>
            <p :class="secretValid?'form-help':'inline-error'">{{ secretValid?t('Name a server environment variable such as env:TENDLY_SECRET_WORK. Its value never reaches the browser.'):t('Enter an environment reference, not the secret itself.') }}</p>
            <button v-if="backendCapabilities.storedSecrets" type="button" class="text-button secret-switch" @click="secretMode='stored'">{{ t('Save the key in Tendly instead') }}</button>
          </template>
          <div class="model-fetch"><strong>{{ t('Model selection') }}</strong><button type="button" class="small-button" :disabled="discovering||!backendCapabilities.models||!secretValid||!!aliases.error" :title="!backendCapabilities.models?t('Backend upgrade required'):t('Reads the endpoint\'s model list. No model is run.')" @click="discover"><Icon name="refresh" :size="14" />{{ discovering?t('Loading models…'):t('Load models from endpoint') }}</button></div>
          <div v-if="modelError" class="inline-error" role="alert">{{ modelError }}<p>{{ t('You can still enter a model ID or alias manually.') }}</p></div>
          <p v-if="models" class="form-help">{{ t('{count} models returned', {count:models.models.length}) }} · <span>{{ models.source_url }}</span><br v-if="models.has_more" /><span v-if="models.has_more">{{ t('The endpoint has more models; this is the first page.') }}</span></p>
          <ModelSelection v-if="!editingNative && (models?.models.length || chosenModels.length)" v-model:selected="chosenModels" :catalog="models?.models ?? []" :window-text="windowText" :placeholder="windowPlaceholder" @window="setWindow" @look="lookPublished" />
          <label>{{ t('Default model') }}<ModelPicker v-model="form.model" :models="choices" :placeholder="t('Choose a model or enter an ID')" /></label>
          <div class="protocol-check">
            <button type="button" class="small-button" :disabled="checkingProtocols || busy || !form.model.trim() || !secretValid || !!aliases.error" @click="checkProtocols">{{ t(checkingProtocols ? 'Checking protocols…' : 'Check model and protocol support') }}</button>
            <p class="form-help">{{ t('Sends a tiny request for this model to each of Messages, Responses and Chat Completions. May use a small amount of credit.') }}</p>
            <p class="form-help">{{ t('Claude Code requires Messages; Codex requires Responses; Pi uses the selected endpoint API.') }}</p>
            <ul v-if="protocolChecks.length" aria-live="polite"><li v-for="check in protocolChecks" :key="check.protocol">{{ check.protocol }} · {{ t(check.status === 'supported' ? 'Supported' : check.status === 'unsupported' ? 'Unsupported' : 'Unable to confirm') }}<span v-if="check.http_status"> (HTTP {{ check.http_status }})</span></li></ul>
            <p v-if="protocolChecks.some(check => check.status === 'unknown')" class="form-help">{{ t('Authentication, model access, missing routes, request parameters or rate limits can prevent verification. An unverified protocol is not necessarily unsupported.') }}</p>
            <p v-if="protocolError" class="inline-error" role="alert">{{ protocolError }}</p>
          </div>
          <label v-if="form.model.trim() && savedLimits">{{ t('Context window') }} <small>{{ t('tokens, for this model in every profile') }}</small><input :value="windowText(form.model)" class="field-mono" inputmode="numeric" autocomplete="off" spellcheck="false" :placeholder="windowPlaceholder(form.model)" @input="setWindow(form.model, ($event.target as HTMLInputElement).value)" /></label>
          <p v-if="form.model.trim() && savedLimits" :class="validWindow(windowText(form.model)) ? 'form-help' : 'inline-error'">{{ !validWindow(windowText(form.model)) ? t('A context window is between 1,000 and 100,000,000 tokens.') : t(client.contextWindow.help) }}</p>
          <div v-if="client.modelSlots" class="model-slots">
            <p class="form-help">{{ slotsText }} <button type="button" class="text-button" :aria-expanded="slotsOpen" @click="slotsOpen=!slotsOpen">{{ t(slotsOpen ? 'Hide' : 'Advanced') }}</button></p>
            <template v-if="slotsOpen">
              <p class="form-help">{{ t('Claude Code also runs these slots, for background work, subagents and its Opus, Sonnet and Haiku choices. Each is an environment variable, listed with the others below.') }}</p>
              <div v-for="slot in SLOTS" :key="slot.key" class="model-slot">
                <span>{{ slot.label }}</span>
                <select :value="slotMode(slot.variable)" :aria-label="slot.label" @change="chooseSlot(slot.variable, ($event.target as HTMLSelectElement).value)">
                  <option value="main">{{ t('Follow the main model') }}</option>
                  <option value="client">{{ t("Claude Code's own model") }}</option>
                  <option value="model">{{ t('A model of its own') }}</option>
                </select>
                <ModelPicker v-if="slotMode(slot.variable)==='model'" :model-value="slotModel(slot.variable)" :models="choices" :placeholder="t('Choose a model or enter an ID')" @update:model-value="pinSlot(slot.variable, $event)" />
                <code v-else>{{ slot.variable }}</code>
              </div>
            </template>
          </div>
          <label v-if="effortOptions.length" :title="!editingNative ? t('Actual effort support depends on the endpoint and model.') : undefined">{{ t('Thinking depth') }}<select v-model="form.effort"><option value="">{{ t('Automatic · provider default') }}</option><option v-for="effort in effortOptions" :key="effort" :value="effort">{{ effortLabel(effort) }}</option></select></label>
          <p v-if="effortOptions.length && !editingNative" class="form-help">{{ t('Actual effort support depends on the endpoint and model.') }}</p>
          <p v-else class="form-help">{{ t('Load a model list to show the reasoning levels supported by this model.') }}</p>
          <label>{{ t('Model aliases') }}<textarea v-model="aliasesText" rows="3" spellcheck="false" placeholder="fast = actual-model-id&#10;review = another-model-id" /></label>
          <p v-if="aliases.error" class="inline-error">{{ t(aliases.error) }}</p>
          <p v-else class="form-help">{{ t('One alias = model-id per line. Aliases change display selection, not the provider protocol.') }}</p>
          <p v-if="form.model" class="model-effective">{{ t('Effective model') }}: <code>{{ resolved }}</code></p>
          <label>{{ t('Permission intent') }}<select v-model="form.permission_mode"><option value="native">{{ t('Native · provider defaults') }}</option><template v-if="client.approvals"><option value="interactive">{{ t('Interactive · conservative native prompts') }}</option><option value="trusted">{{ t('Trusted · native file-edit trust') }}</option></template><option v-if="client.profilePlan" value="plan">{{ t('Plan · review before edits') }}</option><option value="blocked">{{ t('Blocked · do not start') }}</option></select></label>
          </template>
          <p v-if="backendCapabilities.customBalances && !editing" class="form-help">{{ t('Save this profile first to configure a custom balance query.') }}</p>
          <EnvironmentEditor v-model="environmentDraft" :supported="backendCapabilities.environment" :disabled="busy" />
          <p v-if="environmentError" class="inline-error" role="alert">{{ t(environmentError) }}</p>
          <p v-if="editingManaged" class="form-help">{{ t('Manage this account from Official accounts. Its profile cannot be removed separately from its account.') }}</p>
          <div class="dialog-actions"><button v-if="editing && !editingManaged" type="button" class="text-button danger-text" :disabled="busy" @click="deleteId=editing">{{ t(editingNative?'Remove profile reference':'Delete profile') }}</button><button v-if="editing && !editingNative" type="button" class="text-button" :disabled="busy" @click="duplicate(editingProfile!)">{{ t('Duplicate as new profile') }}</button><span class="flex-spacer" /><button type="button" class="secondary-button" :disabled="busy" @click="cancelForm">{{ t('Cancel') }}</button><button class="primary-button" :disabled="busy||!canSave">{{ busy?t('Saving…'):t('Save profile') }}</button></div>
        </form>
        <div v-else class="pane-empty"><p>{{ t('Select a profile to edit, or create one for a different account or endpoint.') }}</p></div>
        <EndpointBalanceEditor v-if="backendCapabilities.customBalances && formVisible && editing && !editingNative" :key="editing" ref="balanceEditor" :profile-id="editing" :disabled="busy" />
      </div>
    </div>
    <ContextMenu v-if="menu" :x="menu.x" :y="menu.y" :label="t('Actions for {profile}', { profile: menu.profile.name })" @close="menu = undefined">
      <button role="menuitem" @click="startRename(menu!.profile)"><Icon name="edit" :size="14" />{{ t('Rename') }}</button>
      <button v-if="!menu.profile.native_config" role="menuitem" @click="duplicate(menu!.profile)"><Icon name="clipboard" :size="14" />{{ t('Duplicate as new profile') }}</button>
      <template v-if="removable(menu.profile)"><hr /><button role="menuitem" class="danger" @click="askDelete(menu!.profile)">{{ t(menu.profile.native_config ? 'Remove profile reference' : 'Delete profile') }}</button></template>
    </ContextMenu>
    <ProfileCopyDialog v-if="copySource" :source="copySource" :profiles="profiles" @close="copySource = undefined" @created="copied" />
  </ModalDialog>
</template>
<style scoped>
.protocol-check{padding:12px;border:1px solid var(--border);border-radius:var(--radius-md);background:var(--sunken)}
.protocol-check ul{margin:8px 0;padding-left:20px;font-size:var(--text-sm);line-height:1.8;color:var(--ink-soft)}
.secret-switch{align-self:flex-start;margin-top:-6px}
.secret-adopt{display:flex;flex-wrap:wrap;align-items:center;justify-content:space-between;gap:8px}
.model-slots{display:flex;flex-direction:column;gap:6px;margin-top:-4px}
.model-slots .text-button{display:inline;margin-left:4px}
.model-slot{display:grid;grid-template-columns:64px minmax(0,190px) minmax(0,1fr);align-items:center;gap:8px}
.model-slot>span{font-size:var(--text-sm);font-weight:550;color:var(--ink)}
.model-slot code{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:var(--text-xs);color:var(--muted)}
@media(max-width:520px){.model-slot{grid-template-columns:56px minmax(0,1fr)}.model-slot>:last-child{grid-column:1/-1}}
.secret-mode{display:flex;flex-wrap:wrap;gap:6px 16px;font-size:var(--text-xs);color:var(--ink-soft)}
.secret-mode>label{display:inline-flex;align-items:center;gap:6px;cursor:pointer}
.secret-clear{margin-left:8px;font-size:inherit}
.model-fetch {display:flex;align-items:center;justify-content:space-between;gap:12px;border-top:1px solid var(--fill-hover);padding-top:12px;}
.model-fetch strong {font-size:var(--text-sm);}.model-effective {margin:0;color:var(--accent-ink);font-size:var(--text-sm);}.form-help {overflow-wrap:anywhere;}
.shared-config-panel{border:1px solid var(--accent-line);border-radius:var(--radius-md);background:var(--sunken);padding:12px}.shared-config-panel>strong{display:flex;align-items:center;gap:7px;font-size:var(--text-xs);font-weight:550;color:var(--accent-ink)}.shared-config-panel>code{display:block;margin-top:8px;font-size:var(--text-xs);overflow-wrap:anywhere;color:var(--ink-soft)}.shared-config-panel p{font-size:var(--text-xs);line-height:18px;color:var(--ink-soft);margin-top:8px}.shared-config-consent{padding:12px;border:1px solid var(--warn-line);border-radius:var(--radius-md);background:var(--warn-soft)}.shared-config-consent p{font-size:var(--text-xs);line-height:18px;color:#837e66;margin:0 0 8px}.shared-config-consent>label{flex-direction:row;align-items:flex-start;gap:8px;line-height:17px;color:var(--ink-soft)}.shared-config-consent input{width:14px;height:14px;margin-top:2px;flex-shrink:0;accent-color:var(--accent)}.profile-card .shared-profile-label{color:var(--accent-ink)}.profile-card .profile-path{max-width:143px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;color:var(--faint)}

/* Settings-page type scale: the shared form styles run from 8px to 11px in a
   pale grey, which read as faint rather than calm at this density. */
.profiles-layout{margin-top:0;grid-template-columns:210px minmax(0,1fr)}
.profile-card{align-items:center;padding:9px 10px;border-radius:var(--radius-md);margin-bottom:3px}
.profile-card.selected{background:var(--teal-soft);border-color:var(--teal-line)}
.profile-card strong{font-size:var(--text-md);line-height:18px;color:var(--ink);font-weight:550;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.profile-card small{font-size:var(--text-xs);line-height:15px;color:var(--muted);margin-top:1px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.profile-card>span:last-child{flex:1}
.profile-renaming{display:flex;gap:10px;border:1px solid var(--teal-line)}
.profile-renaming>input{flex:1;min-width:0;padding:4px 6px;border:1px solid var(--border);border-radius:var(--radius-sm);background:var(--surface);font-size:var(--text-md);color:var(--ink)}
.profile-banner{display:flex;flex-direction:column;gap:8px;scroll-margin-top:8px}
.profile-banner:empty{display:none}
.profiles-list>.secondary-button{font-size:var(--text-sm);min-height:36px}
.profiles-accounts-hint{margin-top:14px;font-size:var(--text-xs);line-height:1.6;color:var(--muted);text-align:left}
.profiles-accounts-link{border:0;background:none;padding:0;font:inherit;color:var(--teal);cursor:pointer}
.profiles-accounts-link:hover{text-decoration:underline}
.form-stack{gap:14px}
.form-stack>h3{font-size:var(--text-base);font-weight:600;color:var(--ink)}
.form-stack label{font-size:var(--text-sm);color:var(--ink-soft)}
.form-stack label>small{font-size:var(--text-xs);color:var(--muted)}
.form-stack input,.form-stack select,.form-stack textarea{font-size:var(--text-md);color:var(--ink);border-color:var(--border);background:var(--surface);border-radius:var(--radius-md)}
.form-stack textarea{display:block;width:100%;padding:9px 11px;border:1px solid var(--border);line-height:1.6;font-family:var(--mono);font-size:var(--text-sm);resize:vertical;outline:0}
.form-stack textarea:focus{border-color:var(--ok-line)}
.form-stack textarea::placeholder{color:var(--faint)}
.form-help{font-size:var(--text-xs);line-height:1.7;color:var(--muted)}
.model-fetch strong{font-size:var(--text-md);color:var(--ink)}
.shared-config-panel>strong{font-size:var(--text-sm)}
.shared-config-panel>code{font-size:var(--text-xs)}
.shared-config-panel p,.shared-config-consent p{font-size:var(--text-sm);line-height:1.7}
.pane-empty p{font-size:var(--text-sm);color:var(--muted)}
/* The list stays put while a long form scrolls beside it. */
.profiles-list{position:sticky;top:0;align-self:start}
.form-stack input,.form-stack select{font-weight:400}
.form-stack :deep(.model-picker-input>input){width:auto;flex:1 1 0;min-width:0;font-weight:400;font-family:var(--mono);font-size:var(--text-sm);color:var(--ink);background:var(--surface);border-color:var(--border);border-radius:var(--radius-md)}
.form-stack :deep(.model-picker-input>input::placeholder){font-family:var(--font-ui)}
@media(max-width:760px){
  .profiles-layout{grid-template-columns:minmax(0,1fr)}
  .profiles-list{position:static;border-right:0;padding-right:0;border-bottom:1px solid var(--border);padding-bottom:10px}
}
</style>
