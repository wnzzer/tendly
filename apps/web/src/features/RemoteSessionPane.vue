<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, provide, ref } from "vue";
import type { EndpointProfile, PaneNode, Session } from "@tendly/protocol";
import Icon from "./Icon.vue";
import SessionPane from "./SessionPane.vue";
import { errorMessage, rememberRemote, request, sessionPath } from "./api";
import { paneString } from "./pane-context";
import { openPeerWindow } from "./peers";
import { IS_REMOTE_PANE } from "./remote-session";
import { useI18n } from "../i18n";

/**
 * A session of a paired machine, in a tab here. Only the tab in front is
 * mounted, so nothing is asked of that machine until this tab is looked at,
 * and its connection ends when another tab is. Everything below is the same
 * SessionPane as here: `api.ts` sends what it asks to the right machine.
 */
const props = defineProps<{ pane: PaneNode }>();
const emit = defineEmits<{ openSession: [machine: { id: string; name: string }, session: Session] }>();
const { t } = useI18n();
provide(IS_REMOTE_PANE, true);

const REFRESH = 15_000;
const machine = computed(() => ({ id: paneString(props.pane, "machine_id")!, name: paneString(props.pane, "machine_name") ?? "" }));
const sessionId = computed(() => paneString(props.pane, "remote_session_id")!);
const session = ref<Session>(), profiles = ref<EndpointProfile[]>([]), problem = ref("");

let timer: ReturnType<typeof setTimeout> | undefined, disposed = false;
async function load() {
  clearTimeout(timer);
  rememberRemote(machine.value.id, [sessionId.value]);
  try {
    const found = await request<Session>(sessionPath(sessionId.value));
    if (disposed) return;
    rememberRemote(machine.value.id, [found.workspace_id]);
    session.value = found; problem.value = "";
  } catch (cause) { if (!disposed) problem.value = errorMessage(cause); }
  if (!disposed) timer = setTimeout(() => { if (document.visibilityState === "visible") void load(); else timer = setTimeout(load, REFRESH); }, REFRESH);
}
async function loadProfiles() {
  try {
    const list = await request<EndpointProfile[]>(`/machines/${encodeURIComponent(machine.value.id)}/endpoint-profiles`);
    rememberRemote(machine.value.id, list.map(profile => profile.id));
    if (!disposed) profiles.value = list;
  } catch { /* The pane works without its list of endpoints. */ }
}
onMounted(() => { void load(); void loadProfiles(); });
onBeforeUnmount(() => { disposed = true; clearTimeout(timer); });

/** The other way there, for a machine too old to be used from here. */
function openWindow() { openPeerWindow(machine.value.id, sessionId.value).catch(cause => { problem.value = errorMessage(cause); }); }
</script>

<template>
  <section class="workspace-pane remote-pane" :data-machine="machine.name">
    <div class="remote-pane-bar"><Icon name="monitor" :size="12" /><span>{{ machine.name }}</span></div>
    <SessionPane v-if="session" :key="`session:${session.id}`" :pane-id="pane.id" :session="session" :sessions="[session]" :profiles="profiles" @changed="load" @select="emit('openSession', machine, $event)" />
    <div v-else-if="problem" class="pane-empty"><Icon name="info" :size="26" /><h3>{{ t('This session cannot be reached') }}</h3><p>{{ problem }}</p><div class="remote-pane-actions"><button type="button" class="small-button" @click="load">{{ t('Check again') }}</button><button type="button" class="small-button" @click="openWindow">{{ t('Open its window') }}</button></div></div>
    <div v-else class="pane-empty"><p>{{ t('Loading…') }}</p></div>
  </section>
</template>

<style scoped>
.remote-pane{display:flex;flex:1;min-height:0;min-width:0;flex-direction:column;height:100%;overflow:hidden}
.remote-pane-bar{display:flex;align-items:center;gap:5px;flex:none;min-height:25px;padding:4px 10px;border-bottom:1px solid var(--accent-soft);background:var(--sunken);color:var(--ink-soft);font-size:var(--text-xs)}
.remote-pane-bar>span{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.remote-pane-actions{display:flex;gap:8px}
</style>
