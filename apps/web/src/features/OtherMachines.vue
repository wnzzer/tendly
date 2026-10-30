<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import Icon from "./Icon.vue";
import ProviderIcon from "./ProviderIcon.vue";
import { errorMessage } from "./api";
import { backendCapabilities } from "./backend-capabilities";
import { allPeerSessions, sidebarMachine, type MachineRow, type MachineSessions } from "./peers";
import type { ProviderKind } from "@tendly/protocol";
import { useI18n } from "../i18n";

/**
 * The sessions of the paired machines, under this machine's own workspaces.
 *
 * They stay that machine's: listed here, and opened in a tab here when it lets
 * this machine control it. Only this list is kept up to date, each machine
 * asked every 30 seconds while the page is in view and at once when it comes
 * back; a session's own content is fetched when its tab is looked at.
 */
const props = defineProps<{ query: string }>();
const emit = defineEmits<{ settings: []; openSession: [machine: { id: string; name: string }, session: MachineRow] }>();
const { t } = useI18n();

const STORAGE_KEY = "tendly.sidebar.machines.collapsed";
const POLL = 30_000;
const machines = ref<MachineSessions[]>([]);
const loading = ref(false), error = ref("");
const collapsed = ref<Record<string, boolean>>(readCollapsed());

function readCollapsed(): Record<string, boolean> {
  try { return JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "{}") ?? {}; } catch { return {}; }
}
function toggle(id: string) {
  collapsed.value = { ...collapsed.value, [id]: !collapsed.value[id] };
  try { localStorage.setItem(STORAGE_KEY, JSON.stringify(collapsed.value)); } catch { /* private mode keeps it for the page */ }
}

let timer: ReturnType<typeof setTimeout> | undefined, disposed = false;
async function load() {
  clearTimeout(timer);
  if (!backendCapabilities.peers || disposed) return;
  loading.value = true;
  try { machines.value = (await allPeerSessions()).machines; error.value = ""; }
  catch (cause) { error.value = errorMessage(cause); }
  finally { loading.value = false; }
  if (!disposed) timer = setTimeout(() => { if (document.visibilityState === "visible") void load(); else timer = undefined; }, POLL);
}
function onVisible() { if (document.visibilityState === "visible" && !loading.value) void load(); }
onMounted(() => { void load(); document.addEventListener("visibilitychange", onVisible); });
// The page learns what the server can do after it mounts.
watch(() => backendCapabilities.peers, peers => { if (peers) void load(); });
onBeforeUnmount(() => { disposed = true; clearTimeout(timer); document.removeEventListener("visibilitychange", onVisible); });

const shown = computed(() => machines.value
  .map(machine => ({ machine, view: sidebarMachine(machine, props.query) }))
  .filter(({ view }) => !view.hidden));

const STATE_LABELS = { waiting: "Waiting for you", working: "Working", idle: "Idle", stopped: "Stopped" } as const;

/** A tab here, like one of this machine's sessions. What it holds is fetched then, not before. */
function open(machine: MachineSessions, row: MachineRow) {
  if (machine.granted === "control") emit("openSession", { id: machine.id, name: machine.name }, row);
}
</script>

<template>
  <section v-if="backendCapabilities.peers && machines.length" class="other-machines" :aria-label="t('Other machines')">
    <header class="other-machines-label">
      <span>{{ t('Other machines') }}</span><small>{{ machines.length }}</small>
      <button class="icon-button" :disabled="loading" :aria-label="t('Check again')" :title="t('Check again')" @click="load"><Icon name="refresh" :size="13" /></button>
      <button class="icon-button" :aria-label="t('Machines')" :title="t('Machines')" @click="emit('settings')"><Icon name="settings" :size="13" /></button>
    </header>
    <p v-if="error" class="other-machines-problem">{{ error }}</p>
    <div v-for="{ machine, view } in shown" :key="machine.id" class="other-machine" :data-machine="machine.name">
      <button class="other-machine-header" :aria-expanded="!collapsed[machine.id]" @click="toggle(machine.id)">
        <Icon name="chevron" :size="11" :class="['other-machine-chevron', { expanded: !collapsed[machine.id] }]" />
        <Icon name="monitor" :size="14" />
        <strong>{{ machine.name }}</strong>
        <i :class="['other-machine-dot', machine.online ? 'online' : machine.online === false ? 'offline' : '']" :title="machine.online ? t('Reachable') : machine.problem" />
        <span v-if="view.waiting" class="other-machine-count waiting" :title="t('{count} waiting for you', { count: view.waiting })">{{ view.waiting }}</span>
        <span v-else-if="view.working" class="other-machine-count" :title="t('{count} active sessions', { count: view.working })">{{ view.working }}</span>
      </button>
      <ul v-show="!collapsed[machine.id]" class="other-machine-sessions">
        <li v-if="machine.online !== true" class="other-machine-note" :title="machine.problem">{{ machine.online === null ? t('No address this machine can reach') : t('Unreachable') }}</li>
        <li v-else-if="!view.rows.length" class="other-machine-note">{{ t('No sessions there.') }}</li>
        <li v-for="row in view.rows" :key="row.id">
          <button :class="['other-machine-session', { viewonly: machine.granted !== 'control' }]" :data-state="row.state"
            :title="`${row.title}${row.workspace ? ` · ${row.workspace}` : ''}\n${machine.granted === 'control' ? t('Open here') : t('It lets this machine see its sessions only')}`"
            :aria-disabled="machine.granted !== 'control'" @click="open(machine, row)">
            <i :class="['other-machine-state', row.state]" :aria-label="t(STATE_LABELS[row.state])" />
            <ProviderIcon :provider="row.provider as ProviderKind" :size="13" />
            <span class="other-machine-title">{{ row.title }}</span>
            <Icon v-if="machine.granted === 'control'" name="arrow" :size="11" class="other-machine-open" />
          </button>
        </li>
        <li v-if="machine.online === true && view.rows.length && machine.granted !== 'control'" class="other-machine-note other-machine-viewonly">{{ t('To open these, set this machine to Full control in {name}\'s Settings → Machines.', { name: machine.name }) }}</li>
        <li v-if="view.more" class="other-machine-note"><button class="text-button" @click="emit('settings')">{{ t('{count} more in Settings → Machines', { count: view.more }) }}</button></li>
      </ul>
    </div>
  </section>
</template>

<style scoped>
.other-machines{margin-top:8px;padding-top:8px;border-top:1px solid var(--fill-hover);min-width:0}
.other-machines-label{display:flex;align-items:center;gap:7px;padding:8px 8px 6px 10px;font-size:var(--text-xs);letter-spacing:.4px;color:var(--muted)}
.other-machines-label>span{font-weight:600}
.other-machines-label>small{color:var(--faint)}
.other-machines-label>.icon-button{width:24px;height:24px}
.other-machines-label>.icon-button:first-of-type{margin-left:auto}
.other-machines-label>.icon-button>svg{color:var(--muted)}
.other-machines-problem{margin:0 10px 6px;font-size:var(--text-xs);line-height:1.5;color:var(--danger-ink)}
.other-machine{min-width:0;margin-bottom:4px}
.other-machine-header{display:flex;align-items:center;gap:6px;width:100%;min-height:32px;padding:4px 6px 4px 2px;border:0;border-radius:var(--radius-sm);background:none;text-align:left;color:var(--ink)}
.other-machine-header:hover{background:var(--fill)}
.other-machine-header>svg{flex-shrink:0;color:var(--faint)}
.other-machine-chevron{transition:transform 120ms ease}
.other-machine-chevron.expanded{transform:rotate(90deg)}
.other-machine-header>strong{min-width:0;font-size:var(--text-md);font-weight:650;letter-spacing:-.015em;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.other-machine-dot{flex-shrink:0;width:7px;height:7px;border-radius:50%;background:var(--line-strong)}
.other-machine-dot.online{background:var(--teal)}
.other-machine-dot.offline{background:var(--danger-ink)}
.other-machine-count{margin-left:auto;flex-shrink:0;font-size:var(--text-xs);padding:1px 6px;border-radius:var(--radius-sm);background:var(--ok-soft);color:var(--accent-ink)}
.other-machine-count.waiting{color:var(--warn-ink);font-weight:600}
.other-machine-sessions{list-style:none;margin:2px 0 4px 12px;padding:0}
.other-machine-session{display:flex;align-items:center;gap:7px;width:100%;min-height:30px;padding:4px 8px;border:0;border-radius:var(--radius-sm);background:none;text-align:left;font-size:var(--text-sm);color:var(--ink)}
.other-machine-session:hover{background:var(--fill)}
.other-machine-session.viewonly{cursor:default}
.other-machine-session.viewonly:hover{background:none}
.other-machine-session>svg{flex-shrink:0}
.other-machine-title{min-width:0;flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.other-machine-open{color:var(--faint);opacity:0}
.other-machine-session:hover>.other-machine-open,.other-machine-session:focus-visible>.other-machine-open{opacity:1}
.other-machine-state{flex-shrink:0;width:6px;height:6px;border-radius:50%;background:var(--line-strong)}
.other-machine-state.working{background:var(--accent)}
.other-machine-state.idle{background:var(--teal)}
.other-machine-state.waiting{background:var(--warn)}
.other-machine-note{padding:5px 8px;font-size:var(--text-xs);line-height:1.5;color:var(--faint)}
.other-machine-note>.text-button{font-size:var(--text-xs)}
@media(pointer:coarse){.other-machine-header{min-height:44px}.other-machine-session{min-height:44px}.other-machine-open{opacity:1}}
@media(prefers-reduced-motion:reduce){.other-machine-chevron{transition:none}}
</style>
