<script setup lang="ts">
import TerminalEndpointSetup from "./TerminalEndpointSetup.vue";
import AgentToolsSwitch from "./AgentToolsSwitch.vue";
import { computed, defineAsyncComponent, ref, watch } from "vue";
import type { EndpointProfile, ProviderKind, Session } from "@tendly/protocol";
import { errorMessage, json, providerLabel, request } from "./api";
import { sessionConnections } from "./session-connection";
import { backendCapabilities } from "./backend-capabilities";
import { DEFAULT_CLIENT } from "./clients";
import { useI18n } from "../i18n";
import Icon from "./Icon.vue";
import ProviderIcon from "./ProviderIcon.vue";
import { useSessionMenuPosition } from "./session-menu-position";
import { escapeHatchSource } from "./terminal-reopen";
const TerminalPane=defineAsyncComponent(()=>import("../components/TerminalPane.vue"));
const { t }=useI18n();
const props=withDefaults(defineProps<{paneId?:string;session?:Session;sessions:Session[];profiles:EndpointProfile[];terminal?:boolean;consumeOpenIntent?:boolean}>(),{consumeOpenIntent:true});
const emit=defineEmits<{changed:[];create:[provider?:ProviderKind];select:[session:Session];environment:[id:string];renameRequest:[id:string];structured:[id:string];profiles:[];leave:[]}>();
const endpointSetup=ref<{prepare:()=>Promise<Session|undefined>;busy:boolean}>();
const preparing=ref(false);
const terminalView=ref<{reconnect:()=>void}>();
const ending=ref(false),confirmEnd=ref(false),error=ref("");
const menu=ref<HTMLDetailsElement>();
const { tabStyle: tabMenuStyle, panelStyle: tabMenuPanelStyle, positionMenu } = useSessionMenuPosition(() => props.paneId, menu);
const entry=computed(()=>props.session?sessionConnections.get(props.session.id):undefined);
const profile=computed(()=>props.session?.endpoint_snapshot??props.profiles.find(p=>p.id===props.session?.endpoint_profile_id));
const nativeConfig=computed(()=>profile.value?.native_config);
const unboundImportedConfig=computed(()=>props.session && !props.session.endpoint_profile_id && !props.session.native_source_id && !nativeConfig.value && props.profiles.some(profile=>profile.provider===props.session!.provider && !!profile.native_config));
const choices=computed(()=>props.sessions.filter(s=>props.terminal?s.provider==="terminal":s.provider!=="terminal"));
const running=computed(()=>entry.value?.state==="attached");
/* A terminal reopened from a conversation is a detour from it, not a session
 * of its own: when its client exits there is nothing to restart, and leaving
 * closes the tab, which goes back to that conversation (session-changes.ts). */
const escapeHatch=computed(()=>!!props.session&&!!escapeHatchSource(props.session));
/* A terminal whose process ends under the view keeps its last screen, like a
 * shell window after `exit`, with one line offering to start it again; a tap
 * or a key does the same. The empty "ended" card is only for a pane that was
 * never showing output: a restored layout, or a session stopped elsewhere. */
const exitedInPlace=ref(false);
const attachCount=ref(0);
watch(running,(live)=>{if(live){attachCount.value++;exitedInPlace.value=false;}});
watch(()=>props.session?.id,()=>{exitedInPlace.value=false;});
function reopenFromView(){if(exitedInPlace.value&&!running.value)void retry();}
function reopenFromKey(event:KeyboardEvent){if(!exitedInPlace.value||running.value)return;if(event.key.length===1||event.key==="Enter"){event.preventDefault();void retry();}}
const streamConnected=ref(false);
watch([()=>props.session?.id,running],()=>{streamConnected.value=false;},{flush:"sync"});
const terminalStatus=computed(()=>{const id=props.session?.id;return(connected:boolean)=>{if(props.session?.id===id)streamConnected.value=connected;};});
watch([()=>props.session?.id,()=>entry.value?.epoch,()=>props.session?.status,()=>props.session?.updated_at],async(_current,_previous,onCleanup)=>{
  let current=true;onCleanup(()=>{current=false;});
  const s=props.session;error.value="";confirmEnd.value=false;
  if(_current[0]!==_previous?.[0])ending.value=false;
  if(!s)return;
  await (props.consumeOpenIntent ? sessionConnections.ensure(s) : sessionConnections.attach(s));
  if(current&&props.session?.id===s.id)emit("changed");
},{immediate:true});
async function retry(){
  if(!props.session||preparing.value||ending.value||entry.value?.pending)return;
  const id=props.session.id;preparing.value=true;
  try{
    const s=endpointSetup.value?await endpointSetup.value.prepare():props.session;
    if(!s||props.session?.id!==id)return;
    sessionConnections.requestOpen(s.id);await sessionConnections.ensure(s);emit("changed");
  }finally{preparing.value=false;}
}
const terminalExited=computed(()=>{const id=props.session?.id;const leave=escapeHatch.value;return()=>{if(id){if(props.session?.id===id&&!leave)exitedInPlace.value=true;sessionConnections.ended(id);}emit("changed");if(leave&&props.session?.id===id)emit("leave");};});
async function endSession(){
  const s=props.session;if(!s||ending.value)return;ending.value=true;error.value="";
  try{await request(`/sessions/${encodeURIComponent(s.id)}/stop`,json("POST"));sessionConnections.ended(s.id);if(props.session?.id===s.id)confirmEnd.value=false;emit("changed");}
  catch(cause){if(props.session?.id===s.id)error.value=errorMessage(cause);}finally{if(props.session?.id===s.id)ending.value=false;}
}
function closeSessionMenu(restoreFocus=false){if(menu.value){menu.value.open=false;if(restoreFocus)menu.value.querySelector('summary')?.focus();}}
function reconnect(){closeSessionMenu();terminalView.value?.reconnect();}
</script>
<template>
  <section v-if="!session" class="pane-empty session-picker">
    <span class="empty-icon"><Icon :name="terminal?'terminal':'spark'" :size="28" /></span>
    <h3>{{ t(terminal?'A terminal, when you need it':'Make room for your next idea') }}</h3>
    <p>{{ t('Open a session to connect. Closing the pane keeps it running.') }}</p>
    <button class="primary-button" @click="emit('create',terminal?'terminal':DEFAULT_CLIENT)"><Icon name="plus" :size="15" />{{ t(terminal?'New terminal':'New agent session') }}</button>
    <div v-if="choices.length" class="session-choices"><small>{{ t('OR OPEN AN EXISTING SESSION') }}</small><button v-for="s in choices" :key="s.id" @click="emit('select',s)"><ProviderIcon :provider="s.provider" :size="15" /><span>{{ s.title }}</span><span :class="['state-dot',s.status]" /></button></div>
  </section>
  <section v-else :class="['agent-session-pane',{'is-exited':exitedInPlace&&!running}]" @pointerdown="reopenFromView" @keydown="reopenFromKey">
    <div v-if="!paneId" class="session-control">
      <div class="session-identity"><span :class="['provider-mark',session.provider]"><ProviderIcon :provider="session.provider" /></span><div><strong>{{ session.provider==='terminal'?t('Terminal'):providerLabel(session.provider) }}</strong><small :title="nativeConfig?.config_dir">{{ session.native_source_id?t('Original client configuration'):nativeConfig?t('{name} · shared host configuration',{name:profile?.name??providerLabel(session.provider)}):profile?.name||t(session.provider==='terminal'?'Host shell':'Isolated native profile') }}</small></div></div>
      <div class="session-actions">
        <span class="status-pill" :class="running?'running':session.status">{{ t(entry?.state==='opening'?'Connecting…':running?(streamConnected?'Connected':'Process running'):entry?.state==='failed'?'Connection failed':'Session ended') }}</span>
        <details :data-session-menu-pane="paneId" ref="menu" class="session-menu" @keydown.esc.stop.prevent="closeSessionMenu(true)">
          <summary class="icon-button" :aria-label="t('Session actions')">⋯</summary>
          <div class="session-menu-content"><button v-if="running" :disabled="ending" @click="reconnect">{{ t('Reconnect view') }}</button><button v-else-if="escapeHatch" @click="closeSessionMenu(); emit('leave')">{{ t('Back to conversation') }}</button><button v-else :disabled="!!entry?.pending||ending||preparing" @click="retry(); closeSessionMenu()">{{ t('Reopen session') }}</button><slot name="session-actions" :close-menu="closeSessionMenu"><button :disabled="ending" @click="emit('environment',session.id); closeSessionMenu()"><Icon name="settings" :size="14" />{{ t('Session environment') }}</button></slot><AgentToolsSwitch v-if="backendCapabilities.agentTools && session.provider !== 'terminal'" :session-id="session.id" /><button v-if="running" class="danger-text" :disabled="ending" @click="confirmEnd=true; closeSessionMenu()">{{ t('End session…') }}</button></div>
        </details>
      </div>
    </div>
    <Teleport v-if="paneId" to="body"><details :data-session-menu-pane="paneId" ref="menu" class="session-menu session-tab-menu" :style="tabMenuStyle" @toggle="positionMenu" @keydown.esc.stop.prevent="closeSessionMenu(true)">
      <summary :aria-label="t('Session actions')" :title="t('Session actions')"><Icon name="more" :size="17" /></summary>
      <div class="session-menu-content" :style="tabMenuPanelStyle"><div class="session-menu-status"><ProviderIcon :provider="session.provider" :size="14" />{{ t(entry?.state==='opening'?'Connecting…':running?(streamConnected?'Connected':'Process running'):entry?.state==='failed'?'Connection failed':'Session ended') }}</div><button v-if="running" :disabled="ending" @click="reconnect"><Icon name="refresh" :size="14" />{{ t('Reconnect view') }}</button><button v-else-if="escapeHatch" @click="closeSessionMenu(); emit('leave')"><Icon name="back" :size="14" />{{ t('Back to conversation') }}</button><button v-else :disabled="!!entry?.pending||ending||preparing" @click="retry(); closeSessionMenu()"><Icon name="play" :size="14" />{{ t('Reopen session') }}</button><hr class="session-menu-divider" /><slot name="session-actions" :close-menu="closeSessionMenu"><button :disabled="ending" @click="emit('environment',session.id); closeSessionMenu()"><Icon name="settings" :size="14" />{{ t('Session environment') }}</button></slot><AgentToolsSwitch v-if="backendCapabilities.agentTools && session.provider !== 'terminal'" :session-id="session.id" /><button v-if="running" class="danger-text" :disabled="ending" @click="confirmEnd=true; closeSessionMenu()"><Icon name="stop" :size="14" />{{ t('End session…') }}</button></div>
    </details></Teleport>
    <div v-if="error||entry?.error" class="inline-error" role="alert">{{ error||entry?.error }}</div>
    <div v-if="escapeHatch" class="inline-notice chat-upgrade-notice"><span>{{ t('This terminal reopens a conversation for commands its view cannot show. Quit the client or close this tab to go back to it.') }}</span><button class="small-button" @click="emit('leave')">{{ t('Back to conversation') }}</button></div>
    <div v-else-if="backendCapabilities.structuredChat && session.provider!=='terminal'" class="inline-notice chat-upgrade-notice"><span>{{ t(running?'This existing terminal keeps running. End it before switching to the mobile chat UI.':'Switch this stopped session to the mobile chat UI. The native CLI still owns tools and approvals.') }}</span><button class="small-button" :disabled="running||ending||!!entry?.pending" @click="emit('structured',session.id)">{{ t('Use chat UI') }}</button></div>
    <div v-if="unboundImportedConfig" class="inline-notice">{{ t('This session uses isolated configuration, not the imported host configuration. Importing does not change existing sessions; select the host profile when creating a new session.') }}</div>
    <div v-if="session.native_source_id" class="inline-notice">{{ t('Loaded history uses the original client settings. It does not take over another running terminal.') }}</div>
    <details v-else-if="nativeConfig" class="inline-notice shared-session-details"><summary>{{ t('New session · shared host configuration') }}</summary><code>{{ nativeConfig.config_dir }}</code><p>{{ t('This is a new session using the host configuration, not a resumed history session.') }}</p><p>{{ t('Account, model, endpoint, proxy and permissions follow the native client. Shared configuration is not session-isolated.') }}</p><p>{{ t('This is a live directory reference, not a snapshot of its contents. Changes to native settings affect subsequent starts.') }}</p><p>{{ t('Tendly does not copy credentials or rewrite this configuration. The native client may update its own sign-in cache and history.') }}</p></details>
    <div v-if="confirmEnd" class="confirmation-bar">{{ t('End this session? Its background process will stop; history remains.') }}<button class="small-button danger" :disabled="ending" @click="endSession">{{ t('End session') }}</button><button class="small-button" :disabled="ending" @click="confirmEnd=false">{{ t('Keep connected') }}</button></div>
    <TerminalPane v-if="running||exitedInPlace" :key="`${session.id}:${attachCount}`" ref="terminalView" :session-id="session.id" @exit="terminalExited" @status="terminalStatus" />
    <button v-if="exitedInPlace&&!running" type="button" class="terminal-exited-bar" :disabled="!!entry?.pending||ending||preparing" @click="retry"><Icon :name="entry?.state==='opening'?'refresh':'play'" :size="14" /><span>{{ t(entry?.state==='opening'?'Connecting…':'Process exited · tap or type to start it again') }}</span></button>
    <div v-else-if="!running&&escapeHatch&&entry?.state!=='opening'&&entry?.state!=='failed'" class="pane-empty"><ProviderIcon :provider="session.provider" :size="28" /><h3>{{ t('Terminal closed') }}</h3><p>{{ t('The conversation goes on where it was opened from.') }}</p><button class="secondary-button" @click="emit('leave')">{{ t('Back to conversation') }}</button></div>
    <div v-else-if="!running" class="pane-empty"><ProviderIcon :provider="session.provider" :size="28" /><h3>{{ t(entry?.state==='opening'?'Connecting…':entry?.state==='failed'?'Connection failed':'Session ended') }}</h3><p>{{ t(entry?.state==='opening'?'Connecting to the native client.':'Reopen this session to continue. No process is restarted automatically after it ends.') }}</p><TerminalEndpointSetup v-if="entry?.state!=='opening'" ref="endpointSetup" :session="session" :profiles="profiles" :disabled="ending || !!entry?.pending" @changed="emit('changed')" @profiles="emit('profiles')" /><button v-if="entry?.state!=='opening'" class="secondary-button" :disabled="!!entry?.pending||ending||preparing" @click="retry">{{ t('Reopen session') }}</button></div>
  </section>
</template>
<style scoped>
.session-menu.session-tab-menu>summary{min-height:28px}.session-menu-content>button>svg{flex:none}
.session-menu-divider{border:0;border-top:1px solid var(--line);margin:4px 6px}.session-menu-status{display:flex;align-items:center;gap:6px;font-size:var(--text-xs);color:var(--muted);padding:8px 9px;border-bottom:1px solid var(--border)}.session-menu summary:focus-visible,.session-menu button:focus-visible{outline:2px solid var(--focus);outline-offset:2px}
@media(max-width:520px){.session-menu.session-tab-menu>summary{min-width:32px;min-height:32px;width:32px;height:32px}}
.terminal-exited-bar{display:flex;flex-shrink:0;align-items:center;justify-content:center;gap:8px;min-height:40px;padding:8px 12px;border:0;border-top:1px solid var(--line);background:var(--sunken);color:var(--ink-soft);font:inherit;font-size:var(--text-sm);cursor:pointer}
.terminal-exited-bar:hover{background:var(--accent-soft);color:var(--accent-ink)}.terminal-exited-bar:disabled{cursor:default;opacity:.7}
@media(pointer:coarse){.terminal-exited-bar{min-height:48px}}
.is-exited :deep(.native-terminal){opacity:.72}
</style>
<style scoped>.session-menu{position:relative}.session-menu>summary{list-style:none;cursor:pointer;min-width:44px;min-height:44px}.session-menu>summary::-webkit-details-marker{display:none}.session-menu-content{position:absolute;right:0;top:44px;z-index:25;width:190px;background:var(--surface);border:1px solid var(--line);border-radius:var(--radius-md);padding:5px;box-shadow:var(--shadow-lg)}.session-menu-content>button{display:flex;align-items:center;gap:8px;width:100%;min-height:30px;border:0;border-radius:var(--radius-sm);background:none;padding:6px 9px;text-align:left;font-size:var(--text-sm);color:var(--ink-soft);cursor:pointer}
@media(pointer:coarse){.session-menu-content>button{min-height:44px}}.session-menu-content>button:hover{background:var(--ok-soft)}.session-tab-menu{position:fixed;z-index:60;display:inline-flex}.session-tab-menu>summary{min-width:28px;width:28px;height:28px;border-radius:var(--radius-sm);display:grid;place-items:center;font-size:var(--text-xl)}.session-tab-menu>summary:hover,.session-tab-menu[open]>summary{background:var(--accent-soft);color:var(--accent-ink)}.session-tab-menu .session-menu-content{top:32px}.shared-session-details{flex-shrink:0;max-height:40%;overflow:auto}.shared-session-details>summary{cursor:pointer;color:var(--accent-ink)}.shared-session-details>code{display:block;overflow-wrap:anywhere;margin-top:8px;font-size:var(--text-xs)}.shared-session-details p{margin-top:7px;line-height:17px}</style>
