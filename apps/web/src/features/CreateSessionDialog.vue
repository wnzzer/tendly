<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import type { EndpointProfile, ModelCatalog, ProviderKind, Session, Workspace } from "@tendly/protocol";
import { errorMessage, json, providerLabel, request, workspacePath } from "./api";
import { sessionEffortOverride, sessionModelOverride } from "./native-profiles";
import { effortLabel, modelEfforts } from "./reasoning-effort";
import { PROFILE_CHOICE_REQUIRED, isProfileSelectionValid, preferredProfileSelection as rememberedSelection, rememberProfileSelection } from "./profile-preferences";
import { loadPreferences, preferenceFor } from "./preferences";
import { useI18n } from "../i18n";
import { backendCapabilities } from "./backend-capabilities";
import { environmentRows, mergeEnvironment, parseEnvironmentRows, type EnvironmentRow } from "./environment-model";
import EnvironmentEditor from "./EnvironmentEditor.vue";
import ModalDialog from "./ModalDialog.vue";
import SessionDraftShell from "./SessionDraftShell.vue";
import ProviderIcon from "./ProviderIcon.vue";
import ModelPicker from "./ModelPicker.vue";
import Icon from "./Icon.vue";
import { offeredModels, resolveModel } from "./model-choices";
import { DEFAULT_CLIENT, PROVIDER_KINDS, agentClient, clientLabel, nativeCatalogNote } from "./clients";
import type { GitBranches } from "@tendly/protocol";
import { setupSummary, taskBranchPreview } from "./isolated-task";
import WorktreeSetupDialog from "./WorktreeSetupDialog.vue";
const { t }=useI18n();
/**
 * `inline` draws the form as a draft tab on the canvas instead of a dialog
 * over it, for a draft tab a layout saved before the dialog still holds.
 * `hidden` keeps the dialog, and what was typed into it, through a detour to
 * the settings page.
 */
const props = defineProps<{ workspace: Workspace; profiles: EndpointProfile[]; initialProvider?: ProviderKind; inline?: boolean; hidden?: boolean }>();
const emit = defineEmits<{ close: []; created: [session: Session]; profiles: []; workspace: [workspace: Workspace] }>();
const provider = ref<ProviderKind>(props.initialProvider ?? DEFAULT_CLIENT);
/** Preferences choose the account and depth a new session starts with; the form can still change both. */
const preferredProfileSelection=(value:ProviderKind,profiles:typeof props.profiles)=>rememberedSelection(value,profiles,undefined,preferenceFor(value)?.endpoint_profile_id);
const preferredEffort=()=>preferenceFor(provider.value)?.effort??"";
const title = ref(t('{provider} session',{provider:providerLabel(provider.value)}));
const profileId=ref(preferredProfileSelection(provider.value,props.profiles)), profileTouched=ref(false), model=ref(""), effort=ref(""), busy=ref(false),error=ref(""),discovering=ref(false),modelError=ref("");
const catalog=ref<ModelCatalog>();
const available = computed(() => props.profiles.filter(item => item.provider === provider.value));
const selectedProfile = computed(() => available.value.find(item => item.id === profileId.value));
const nativeConfig = computed(() => selectedProfile.value?.native_config);
const validProfile = computed(() => isProfileSelectionValid(provider.value,profileId.value,props.profiles));
const choices=computed(()=>nativeConfig.value?[]:[...Object.entries(selectedProfile.value?.model_aliases??{}).map(([id,name])=>({id,name:id+" → "+name})),...offeredModels(catalog.value?.models??[],selectedProfile.value?.models)]);
const effectiveModel=computed(()=>{if(nativeConfig.value)return undefined;const selected=model.value||selectedProfile.value?.model;return selected?(selectedProfile.value?.model_aliases?.[selected]??selected):undefined});
const effortOptions=computed(()=>provider.value==='terminal'?[]:modelEfforts(provider.value,effectiveModel.value,catalog.value,effort.value||selectedProfile.value?.effort,!!selectedProfile.value&&!selectedProfile.value.native_config));
const ephemeral=ref(false);
/**
 * Where the session works: this checkout, shared with the sessions here, or
 * an isolated task -- a new worktree on a new branch from `base`, made for it
 * alone, so it never shares files with anyone. The branch is named for the
 * session unless one is given; one already checked out anywhere is refused.
 * The session is created here, then moved into that worktree.
 */
const place=ref<'here'|'isolated'>('here'), worktreeBranch=ref(''), base=ref(''), repo=ref<GitBranches>(), editingSetup=ref(false);
const setup=ref(props.workspace.worktree_setup);
watch(()=>props.workspace.worktree_setup,value=>{setup.value=value;});
watch(place,async value=>{ if(value!=='isolated'||repo.value)return; try{repo.value=await request<GitBranches>(`${workspacePath(props.workspace.id)}/git/branches`);base.value=repo.value.current??repo.value.branches[0]??'';}catch(cause){error.value=errorMessage(cause);place.value='here';} });
const branchPreview=computed(()=>worktreeBranch.value.trim()||taskBranchPreview(title.value));
/** Where a named branch is checked out already, which isolation refuses. */
const branchTaken=computed(()=>{const name=worktreeBranch.value.trim();return name?repo.value?.worktrees.find(tree=>tree.branch===name)?.path:undefined;});
const prepared=computed(()=>setupSummary(setup.value));
function setupSaved(workspace: Workspace){setup.value=workspace.worktree_setup;editingSetup.value=false;emit('workspace',workspace);}
const ephemeralSupported=computed(()=>backendCapabilities.ephemeralSessions);
const environmentDrafts=ref(new Map<string,EnvironmentRow[]>());
const environmentScope=computed(()=>JSON.stringify([props.workspace.id,provider.value,profileId.value]));
const environmentDraft=computed({get:()=>environmentDrafts.value.get(environmentScope.value)??[],set:(rows:EnvironmentRow[])=>{environmentDrafts.value.set(environmentScope.value,rows);}});
const parsedEnvironment=computed(()=>parseEnvironmentRows(environmentDraft.value));
const mergedEnvironment=computed(()=>parseEnvironmentRows(environmentRows(mergeEnvironment(selectedProfile.value?.environment,parsedEnvironment.value.environment))));
const environmentErrors=computed(()=>[...new Set([...parsedEnvironment.value.errors,...mergedEnvironment.value.errors])]);
const unsupportedEnvironment=computed(()=>!backendCapabilities.environment&&(environmentDraft.value.length>0||Object.keys(selectedProfile.value?.environment??{}).length>0));
const validEnvironment=computed(()=>!unsupportedEnvironment.value&&!environmentErrors.value.length);
let revision=0;
watch(provider,(value,previous)=>{if(title.value===t('{provider} session',{provider:providerLabel(previous)})) title.value=t('{provider} session',{provider:providerLabel(value)});profileTouched.value=false;profileId.value=preferredProfileSelection(value,props.profiles);model.value="";effort.value="";},{flush:"sync"});
watch(available,()=>{
  if(busy.value)return;
  if(profileId.value && profileId.value!==PROFILE_CHOICE_REQUIRED){
    if(!validProfile.value){profileTouched.value=true;profileId.value=PROFILE_CHOICE_REQUIRED;}
    return; // Refreshes must not replace a valid account already displayed in the form.
  }
  if(!profileTouched.value)profileId.value=preferredProfileSelection(provider.value,props.profiles);
},{flush:"sync"});
watch([provider,profileId,()=>nativeConfig.value?.source_id,()=>nativeConfig.value?.config_dir],()=>{catalog.value=undefined;modelError.value="";model.value="";effort.value=selectedProfile.value?.effort??preferredEffort();discovering.value=false;revision++;},{flush:"sync"});
void loadPreferences().then(()=>{if(busy.value)return;if(!profileTouched.value)profileId.value=preferredProfileSelection(provider.value,props.profiles);if(!effort.value)effort.value=selectedProfile.value?.effort??preferredEffort();});
async function discover(){
  if(!validProfile.value||!backendCapabilities.models||nativeConfig.value||provider.value==="terminal")return;
  const p=selectedProfile.value ?? {name:"Native catalog",provider:provider.value,endpoint_url:null,model:null,permission_mode:"native",secret_ref:null,proxy_url:null,effort:null,model_aliases:{}};if(discovering.value)return;
  discovering.value=true;modelError.value="";const own=++revision;
  try{const result=await request<ModelCatalog>("/endpoint-profiles/discover-models",json("POST",{name:p.name,provider:p.provider,endpoint_url:p.endpoint_url,model:p.model,permission_mode:p.permission_mode,secret_ref:p.secret_ref,proxy_url:p.proxy_url??null,effort:p.effort??null,model_aliases:p.model_aliases??{}}));if(own===revision)catalog.value=result;}
  catch(cause){if(own===revision)modelError.value=errorMessage(cause);}
  finally{if(own===revision)discovering.value=false;}
}
async function create() {
  if (!title.value.trim() || busy.value || !validProfile.value || !validEnvironment.value || place.value === 'isolated' && (!!branchTaken.value || !base.value)) return;
  const selectedProvider=provider.value, selectedProfileId=profileId.value;
  busy.value = true; error.value = "";
  try {
    let session = await request<Session>(`${workspacePath(props.workspace.id)}/sessions`, json("POST", { title: title.value.trim(), provider: selectedProvider, endpoint_profile_id: selectedProfileId || null, ...(backendCapabilities.structuredChat&&selectedProvider!=='terminal'?{interaction_mode:'structured'}:{}), ...sessionModelOverride(selectedProvider,selectedProfile.value,resolveModel(model.value,catalog.value?.models??[])), ...sessionEffortOverride(selectedProvider,selectedProfile.value,effort.value), ...(selectedProvider!=='terminal'&&!nativeConfig.value&&!effort.value.trim()?{effort:null}:{}), ...(backendCapabilities.environment&&Object.keys(parsedEnvironment.value.environment).length?{environment:parsedEnvironment.value.environment}:{}), ...(ephemeralSupported.value&&ephemeral.value?{ephemeral:true}:{}) })); if (place.value === 'isolated') { const branch = worktreeBranch.value.trim(); session = await request<Session>(`/sessions/${encodeURIComponent(session.id)}/checkout`, json("POST", { isolated: true, base: base.value, ...(branch ? { branch } : {}) })); }
    rememberProfileSelection(selectedProvider,selectedProfileId); emit("created", session); }
  catch (cause) { error.value = errorMessage(cause); }
  finally { busy.value = false; }
}
onBeforeUnmount(()=>{revision++;});
/** Anything chosen beyond the defaults, which closing by accident would throw away. */
const dirty = computed(() => title.value !== t('{provider} session', { provider: providerLabel(provider.value) }) || !!model.value || !!effort.value || profileTouched.value || ephemeral.value || place.value !== 'here'
  || [...environmentDrafts.value.values()].some(rows => rows.some(row => row.name || row.value)));
</script>
<template>
  <component :is="inline ? SessionDraftShell : ModalDialog" :title="t('New session')" :closable="!busy" :dirty="dirty" :hidden="hidden" @close="!busy && emit('close')">
    <!-- The four choices most sessions differ by, and the rest folded under "More settings". -->
    <form class="form-stack" @submit.prevent="create">
      <div class="session-kinds" role="radiogroup" :aria-label="t('Provider')">
        <label v-for="kind in PROVIDER_KINDS" :key="kind" :class="{ selected: provider === kind }"><input v-model="provider" type="radio" name="session-provider" :value="kind" :disabled="busy" /><Icon v-if="kind === 'terminal'" name="terminal" :size="15" /><ProviderIcon v-else :provider="kind" :size="15" />{{ kind === 'terminal' ? t('Terminal') : clientLabel(kind) }}</label>
      </div>
      <label>{{ t('Session name') }}<input v-model="title" autofocus required :disabled="busy" maxlength="120" :placeholder="t('What are you working on?')" /></label>
      <template v-if="provider!=='terminal'">
        <label>{{ t('Endpoint profile') }}<select v-model="profileId" :disabled="busy" @change="profileTouched=true"><option v-if="profileId===PROFILE_CHOICE_REQUIRED" :value="PROFILE_CHOICE_REQUIRED" disabled>{{ t('Choose a session configuration') }}</option><option value="">{{ t('Isolated configuration · separate sign-in may be required') }}</option><option v-for="profile in available" :key="profile.id" :value="profile.id">{{ profile.name }} · {{ profile.native_config ? t('Host configuration · shared sign-in') : profile.model || t('default model') }}</option></select>
          <small v-if="nativeConfig" class="form-help">{{ t('Shares sign-in with {path}', { path: nativeConfig.config_dir }) }}</small>
          <small v-else-if="selectedProfile" class="form-help">{{ selectedProfile.endpoint_url || t('Official endpoint') }} · {{ t(selectedProfile.permission_mode) }}</small>
        </label>
        <p v-if="!validProfile" class="inline-notice">{{ t('Choose a configuration explicitly when several host accounts are available. No account will be selected silently.') }}</p>
        <div class="session-place"><span>{{ t('Works in') }}</span><div class="session-place-options"><label><input v-model="place" type="radio" value="here" :disabled="busy" />{{ t('This checkout') }}</label><label><input v-model="place" type="radio" value="isolated" :disabled="busy" />{{ t('Isolated task') }}</label></div>
          <small v-if="place==='here'">{{ t('Shares this directory, and every change in it, with the other sessions here.') }}</small>
          <template v-else>
            <small>{{ t('A new worktree and branch for this session alone. When it is done, you review what it changed and merge it back.') }}</small>
            <div class="session-place-fields"><label>{{ t('Starts from') }}<select v-model="base" :disabled="busy||!repo"><option v-for="name in repo?.branches??[]" :key="name" :value="name">{{ name }}</option></select></label><label>{{ t('New branch') }}<input v-model="worktreeBranch" :disabled="busy" maxlength="200" spellcheck="false" autocomplete="off" :placeholder="taskBranchPreview(title)" /></label></div>
            <small v-if="branchTaken" class="session-place-error" role="alert">{{ t('{branch} is checked out in {path} already. An isolated task needs a branch of its own.', { branch: worktreeBranch.trim(), path: branchTaken }) }}</small>
            <small v-else>{{ t(repo?.branches.includes(branchPreview) ? 'Opens a worktree on the existing branch {branch}.' : 'Creates branch {branch} from {current} in a new worktree.', { branch: branchPreview, current: base || 'HEAD' }) }}</small>
            <div class="session-place-setup"><small v-if="prepared.kind==='none'">{{ t('Nothing is prepared: the worktree gets no installed dependencies and no .env.') }}</small><small v-else>{{ t('Prepared with') }} <code v-for="path in prepared.copy" :key="path">{{ path }}</code><code v-if="prepared.command">{{ prepared.command }}</code></small><button type="button" class="text-button" :disabled="busy" @click="editingSetup=true">{{ t(prepared.kind==='none'?'Set up…':'Change…') }}</button></div>
          </template></div>
      </template>
      <details class="session-more">
        <summary><Icon name="chevron" :size="12" class="session-more-caret" />{{ t('More settings') }}</summary>
        <div class="form-stack">
          <template v-if="provider!=='terminal' && validProfile && !nativeConfig">
            <div class="session-model-bar"><label>{{ t('Session model') }}<ModelPicker v-model="model" :models="choices" :placeholder="selectedProfile?.model || t('Use profile or native default')" /></label><button v-if="selectedProfile || agentClient(provider)?.nativeCatalog" type="button" class="small-button" :disabled="discovering||!backendCapabilities.models" :title="!backendCapabilities.models?t('Backend upgrade required'):undefined" @click="discover">{{ discovering?t('Loading models…'):t('Load models') }}</button></div>
            <p v-if="nativeCatalogNote(catalog?.source_url)" class="form-help">{{ t(nativeCatalogNote(catalog?.source_url)!) }}</p>
            <div v-if="modelError" class="inline-error" role="alert">{{ modelError }}</div>
            <p v-if="effectiveModel" class="form-help">{{ t('Effective model') }}: <code>{{ effectiveModel }}</code></p>
            <label v-if="effortOptions.length" :title="selectedProfile ? t('Actual effort support depends on the endpoint and model.') : undefined">{{ t('Thinking depth') }}<select v-model="effort"><option value="">{{ t('Automatic · provider default') }}</option><option v-for="level in effortOptions" :key="level" :value="level">{{ effortLabel(level) }}</option></select></label>
          </template>
          <label v-if="ephemeralSupported" class="session-ephemeral-choice"><input v-model="ephemeral" type="checkbox" :disabled="busy" /><span><strong>{{ t('Temporary window') }}</strong><small>{{ t('Discarded when its window is closed, and cleared on restart. It still appears in its workspace, marked temporary, so you can keep it at any time.') }}</small></span></label>
          <EnvironmentEditor v-model="environmentDraft" :inherited="selectedProfile?.environment" :supported="backendCapabilities.environment" :disabled="busy||!validProfile" />
          <button v-if="provider!=='terminal'" type="button" class="text-button" :disabled="busy" @click="emit('profiles')">{{ t('Manage endpoint profiles') }}</button>
        </div>
      </details>
      <!-- Outside the fold: whatever keeps Create disabled has to be in sight. -->
      <div v-for="message in environmentErrors" :key="message" class="inline-error" role="alert">{{ t(message) }}</div>
      <div v-if="unsupportedEnvironment" class="inline-notice" role="alert">{{ t('Environment overrides require an updated backend. Your draft has not been discarded.') }}<button v-if="environmentDraft.length" type="button" class="text-button" :disabled="busy" @click="environmentDraft=[]">{{ t('Discard environment draft') }}</button></div>
      <div v-if="error" class="inline-error" role="alert">{{ error }}</div>
      <div class="dialog-actions"><button type="button" class="secondary-button" :disabled="busy" @click="emit('close')">{{ t('Cancel') }}</button><button class="primary-button" :disabled="busy || !title.trim() || !validProfile || !validEnvironment || place==='isolated' && (!!branchTaken || !base)">{{ busy?t('Creating…'):t('Create session') }}</button></div>
    </form>
    <WorktreeSetupDialog v-if="editingSetup" :workspace="{ ...workspace, worktree_setup: setup }" @close="editingSetup=false" @saved="setupSaved" />
  </component>
</template>
<style scoped>.session-kinds{display:flex;flex-wrap:wrap;gap:6px}.session-kinds label{flex:1 0 auto;flex-direction:row;align-items:center;justify-content:center;gap:7px;min-height:36px;padding:0 10px;border:1px solid var(--line);border-radius:var(--radius-md);font-size:var(--text-sm);color:var(--ink-soft);cursor:pointer;white-space:nowrap}.session-kinds label:hover{background:var(--fill)}.session-kinds label.selected{border-color:var(--accent);background:var(--accent-soft);color:var(--ink);font-weight:550}.session-kinds label:has(:focus-visible){box-shadow:0 0 0 2px var(--focus)}.session-kinds input{position:absolute;opacity:0;width:1px;height:1px;pointer-events:none}
.session-place{display:flex;flex-direction:column;gap:7px;font-size:var(--text-sm)}.session-place>span{font-weight:550;color:var(--ink-soft)}.session-place-options{display:flex;gap:16px}.session-place-options label{flex-direction:row;align-items:center;gap:6px;font-size:var(--text-sm);color:var(--ink);white-space:nowrap}.session-place-options input[type=radio]{display:inline-block;width:auto;margin:0;padding:0;accent-color:var(--teal)}.session-place input[type=text],.session-place input:not([type]){height:34px;padding:0 9px;border:1px solid var(--line);border-radius:var(--radius-sm);font:var(--text-sm) var(--mono)}.session-place small{font-size:var(--text-xs);color:var(--muted);line-height:1.5}.session-place-fields{display:flex;gap:8px}.session-place-fields>label{flex:1;min-width:0;font-size:var(--text-xs)}.session-place .session-place-error{color:var(--danger-ink)}.session-place-setup{display:flex;align-items:baseline;justify-content:space-between;gap:8px;padding-top:6px;border-top:1px solid var(--line)}.session-place-setup code{margin-left:4px;padding:1px 5px;border-radius:var(--radius-sm);background:var(--fill);font:var(--text-xs) var(--mono);overflow-wrap:anywhere}.session-place-setup .text-button{flex-shrink:0}.session-ephemeral-choice{flex-direction:row;align-items:flex-start;gap:9px;padding:9px 11px;border:1px solid var(--violet-soft);border-radius:var(--radius-sm);background:var(--bg)}.session-ephemeral-choice input[type=checkbox]{width:14px;height:14px;min-width:14px;margin:2px 0 0;flex:0 0 14px;accent-color:var(--violet)}.session-ephemeral-choice strong{display:block;font-size:var(--text-xs);font-weight:550;color:#6f5c93}.session-ephemeral-choice small{display:block;margin-top:3px;font-size:var(--text-xs);line-height:15px;color:var(--muted);font-weight:400}
.session-model-bar{display:flex;align-items:end;gap:8px}.session-model-bar>label{flex:1;min-width:0}.session-model-bar>button{margin-bottom:1px;white-space:nowrap}label>small.form-help{font-weight:400;overflow-wrap:anywhere}
.session-more>summary{display:flex;align-items:center;gap:6px;width:max-content;list-style:none;font-size:var(--text-sm);font-weight:550;color:var(--ink-soft);cursor:pointer}.session-more>summary::-webkit-details-marker{display:none}.session-more-caret{transition:transform var(--duration-fast,.12s)}.session-more[open]>summary .session-more-caret{transform:rotate(90deg)}.session-more>.form-stack{margin-top:12px}</style>
