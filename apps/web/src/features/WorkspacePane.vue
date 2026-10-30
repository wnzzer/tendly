<script setup lang="ts">
import { computed } from "vue";
import type { EndpointProfile, GitStatus, PaneNode, ProviderKind, Session, Workspace } from "@tendly/protocol";
import { paneCheckout, paneString, paneSession, paneWorkspace } from "./pane-context";
import { placeKey } from "./checkout-place";
import NativeSessionPane from "./NativeSessionPane.vue";
import CreateSessionDialog from "./CreateSessionDialog.vue";
import SessionPane from "./SessionPane.vue";
import RemoteSessionPane from "./RemoteSessionPane.vue";
import FilePane from "./FilePane.vue";
import GitPane from "./GitPane.vue";
import Icon from "./Icon.vue";
import { useI18n } from "../i18n";
const { t } = useI18n();
const props = defineProps<{ pane: PaneNode; workspaces: Workspace[]; sessions: Session[]; profiles: EndpointProfile[]; gitRefresh: Record<string, number> }>();
const emit = defineEmits<{
  openRemoteSession: [machine: { id: string; name: string }, session: Session];
  sessionChanged: []; newSession: [id: string | undefined, provider?: ProviderKind]; openSession: [session: Session]; renameRequest: [id: string]; sessionEnvironment: [id: string];
  structuredSession: [id: string]; profiles: [];
  revealSession: [id: string]; keepSession: [id: string];
  /** `place`: the workspace and checkout (`placeKey`) a Git or file pane shows. */
  gitChanged: [place: string, status: GitStatus]; openFile: [id: string, path: string, checkout: string | null]; openReference: [id: string, reference: { path: string; line?: number; checkout?: string | null }]; filesSaved: [place: string];
  browse: [id: string | undefined];
  draftCreated: [paneId: string, session: Session]; closeDraft: [paneId: string];
  /** An escape-hatch terminal is done: its tab closes, which returns to its conversation. */
  leaveTerminal: [paneId: string];
}>();
const workspace = computed(() => paneWorkspace(props.pane, props.workspaces, props.sessions));
const session = computed(() => paneSession(props.pane, props.sessions));
const checkout = computed(() => paneCheckout(props.pane, props.sessions));
const place = computed(() => workspace.value ? placeKey(workspace.value.id, checkout.value) : undefined);
/** A worktree's branch, as the sessions working there last recorded it, or its directory's name. */
const checkoutBranch = computed(() => checkout.value ? props.sessions.find(item => item.checkout_path === checkout.value)?.checkout_branch ?? checkout.value.split(/[\\/]/).pop() : undefined);
const invalid = computed(() => (!!paneString(props.pane, "workspace_id") && !workspace.value) || (!!paneString(props.pane, "session_id") && !session.value) || (!!paneString(props.pane, "path") && !workspace.value));
const choices = computed(() => workspace.value ? props.sessions.filter(session => session.workspace_id === workspace.value!.id) : props.sessions);
const isAgent = computed(() => props.pane.kind === "agent_chat" || props.pane.kind === "terminal");
/** A file names its workspace in its own toolbar, so the bar above would say it twice. */
const fileShown = computed(() => !invalid.value && !!workspace.value && !isAgent.value && props.pane.kind !== "git_diff" && !!paneString(props.pane, "path"));
/** A session still being set up: a tab with the form, until Create binds it to the new session. */
const draft = computed(() => isAgent.value && !session.value && props.pane.metadata?.draft === true);
const draftProvider = computed(() => paneString(props.pane, "provider") as ProviderKind | undefined);
const callbacks = computed(() => {
  const id = workspace.value?.id, here = place.value, tree = checkout.value;
  return {
    newSession: (provider?: ProviderKind) => emit("newSession", id, provider),
    gitChanged: (status: GitStatus) => { if (here) emit("gitChanged", here, status); },
    openFile: (path: string) => { if (id) emit("openFile", id, path, tree); },
    filesSaved: () => { if (here) emit("filesSaved", here); },
    browse: () => emit("browse", id),
  };
});
function renameSession(id: string) { emit("renameRequest", id); }
</script>
<template>
  <RemoteSessionPane v-if="paneString(pane,'remote_session_id')" :key="`remote:${paneString(pane,'remote_session_id')}`" :pane="pane" @open-session="(machine, session) => emit('openRemoteSession', machine, session)" />
  <section v-else :class="['workspace-pane', { 'agent-pane': isAgent && (!!session || draft) }]" :data-workspace-id="workspace?.id">
    <div v-if="!fileShown" class="pane-workspace-bar"><span :title="checkout ?? workspace?.root_path"><Icon name="folder" :size="12" />{{ workspace?.name || t('Unassigned workspace') }}<template v-if="checkoutBranch"> · ⎇ {{ checkoutBranch }}</template></span><span class="pane-scope-label">{{ t('Fixed to this pane') }}</span></div>
    <div v-if="invalid" class="pane-empty"><Icon name="info" :size="26"/><h3>{{ t('Pane source unavailable') }}</h3><p>{{ t('This pane cannot resolve its original workspace or session. It will not use another workspace automatically.') }}</p></div>
    <SessionPane v-else-if="isAgent && session" :key="`session:${session.id}`" :pane-id="pane.id" :session="session" :sessions="choices" :profiles="profiles" @changed="emit('sessionChanged')" @create="callbacks.newSession" @select="emit('openSession',$event)" @rename-request="renameSession" @environment="emit('sessionEnvironment',$event)" @profiles="emit('profiles')" @reveal="emit('revealSession',$event)" @keep="emit('keepSession',$event)" @leave="emit('leaveTerminal',pane.id)" @open-file="(reference: { path: string; line?: number; checkout?: string | null }) => workspace && emit('openReference', workspace.id, reference)" />
    <CreateSessionDialog v-else-if="draft && workspace" inline :workspace="workspace" :profiles="profiles" :initial-provider="draftProvider" @close="emit('closeDraft', pane.id)" @created="emit('draftCreated', pane.id, $event)" @profiles="emit('profiles')" />
    <NativeSessionPane v-else-if="isAgent" :key="`${pane.id}:${workspace?.id??''}`" :session="session" :sessions="choices" :profiles="profiles" :terminal="pane.kind==='terminal'" @changed="emit('sessionChanged')" @create="callbacks.newSession" @select="emit('openSession',$event)" @environment="emit('sessionEnvironment',$event)" @structured="emit('structuredSession',$event)" @profiles="emit('profiles')" />
    <GitPane v-else-if="pane.kind==='git_diff' && workspace" :key="`${pane.id}:${place}`" :workspace-id="workspace.id" :checkout="checkout" :refresh-token="gitRefresh[place!]??0" @changed="callbacks.gitChanged" @open-file="callbacks.openFile" />
    <FilePane v-else-if="workspace" :key="`${pane.id}:${place}`" :workspace-id="workspace.id" :checkout="checkout" :path="paneString(pane,'path')" :workspace-name="checkoutBranch ? `${workspace.name} ⎇ ${checkoutBranch}` : workspace.name" @saved="callbacks.filesSaved" @browse="callbacks.browse" />
    <div v-else class="pane-empty"><Icon name="folder" :size="28"/><h3>{{ t('Choose a workspace source') }}</h3><p>{{ t('Open files or Git using a workspace shortcut in the sidebar.') }}</p></div>
  </section>
</template>
<style scoped>
.workspace-pane{display:flex;flex:1;min-height:0;min-width:0;flex-direction:column;height:100%;overflow:hidden}.pane-workspace-bar{display:flex;align-items:center;justify-content:space-between;gap:8px;flex:none;min-height:25px;padding:4px 10px;border-bottom:1px solid var(--accent-soft);background:var(--sunken);color:var(--ink-soft);font-size:var(--text-xs)}.pane-workspace-bar>span:first-child{display:flex;align-items:center;gap:5px;min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.pane-scope-label{font-size:var(--text-xs);color:var(--faint);white-space:nowrap}.workspace-pane.agent-pane>.pane-workspace-bar{display:none}@media(max-width:650px){.pane-scope-label{display:none}}
</style>
