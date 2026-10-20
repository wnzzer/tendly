<script setup lang="ts">
/**
 * What agents ask of Tendly, and what they changed (server activity.rs).
 *
 * A request is a card that waits for the person: what would change, and a key
 * field when the change needs one, so a key goes from here straight to the
 * server and never through the agent. A notice says what an agent already
 * changed, with an undo where the server keeps one. Every open window sees the
 * same cards; answering in one clears it in all.
 */
import { onBeforeUnmount, reactive, ref, watch } from "vue";
import type { ProviderKind } from "@tendly/protocol";
import { errorMessage, json, request } from "./api";
import ProviderIcon from "./ProviderIcon.vue";
import Icon from "./Icon.vue";
import { useI18n } from "../i18n";

interface Who { kind: "session" | "external"; session_id?: string; title?: string | null; provider?: ProviderKind | null }
interface Row { label: string; value: string }
interface AgentRequest { id: string; who: Who; title: string; values: Record<string, string>; rows: Row[]; notes: string[]; key: { label: string; optional: boolean } | null }
interface Notice { id: string; who: Who; message: string; values: Record<string, string>; undoable: boolean; undone?: boolean; open?: { session_id: string } | null }
/** Where an agent pointed the person: a file and line, a diff, a session. */
export type ShowTarget = { kind: "file"; workspace_id: string; path: string; line?: number | null; checkout?: string | null } | { kind: "changes"; workspace_id: string; checkout?: string | null } | { kind: "session"; session_id: string };

const props = defineProps<{ enabled: boolean }>();
const emit = defineEmits<{ changed: []; show: [target: ShowTarget]; canvas: [revision: number] }>();
const { t } = useI18n();
const requests = ref<AgentRequest[]>([]), notices = ref<Notice[]>([]);
const keys = reactive<Record<string, string>>({}), errors = reactive<Record<string, string>>({});
const busy = ref<string>();
const NOTICE_MS = 12000;
let socket: WebSocket | undefined, retry: ReturnType<typeof setTimeout> | undefined, backoff = 1000, stopped = false;
const timers = new Map<string, ReturnType<typeof setTimeout>>();

function who(entry: Who) {
  return entry.kind === "session" ? entry.title || t("A session") : t("An agent outside Tendly");
}
function dismiss(id: string) {
  notices.value = notices.value.filter(notice => notice.id !== id);
  clearTimeout(timers.get(id)); timers.delete(id);
}
function received(event: MessageEvent) {
  let message: { type: string; id?: string; requests?: AgentRequest[] } & Partial<AgentRequest & Notice>;
  try { message = JSON.parse(String(event.data)); } catch { return; }
  if (message.type === "snapshot") requests.value = message.requests ?? [];
  else if (message.type === "request") { const next = message as unknown as AgentRequest; requests.value = [...requests.value.filter(item => item.id !== next.id), next]; }
  else if (message.type === "resolved" && message.id) { requests.value = requests.value.filter(item => item.id !== message.id); delete keys[message.id]; delete errors[message.id]; }
  else if (message.type === "notice") {
    const notice = message as unknown as Notice;
    notices.value = [notice, ...notices.value].slice(0, 4);
    timers.set(notice.id, setTimeout(() => dismiss(notice.id), NOTICE_MS));
    emit("changed");
  }
  else if (message.type === "show") emit("show", (message as unknown as { target: ShowTarget }).target);
  else if (message.type === "canvas") emit("canvas", Number((message as unknown as { revision: number }).revision));
}
function connect() {
  if (stopped || !props.enabled || socket) return;
  const url = new URL("/api/agent-activity/ws", window.location.href);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  const current = new WebSocket(url.href);
  socket = current;
  current.onopen = () => { backoff = 1000; };
  current.onmessage = received;
  current.onclose = () => {
    if (socket === current) socket = undefined;
    if (stopped || !props.enabled) return;
    retry = setTimeout(connect, backoff); backoff = Math.min(backoff * 2, 15000);
  };
}
function disconnect() { clearTimeout(retry); const current = socket; socket = undefined; current?.close(); }
watch(() => props.enabled, on => { if (on) connect(); else disconnect(); }, { immediate: true });
onBeforeUnmount(() => { stopped = true; disconnect(); timers.forEach(clearTimeout); });

async function answer(item: AgentRequest, approve: boolean) {
  if (busy.value) return;
  busy.value = item.id; errors[item.id] = "";
  const secret = approve && item.key ? keys[item.id]?.trim() || undefined : undefined;
  try {
    await request(`/agent-activity/requests/${encodeURIComponent(item.id)}`, json("POST", { approve, ...(secret ? { secret } : {}) }));
    requests.value = requests.value.filter(entry => entry.id !== item.id); delete keys[item.id];
  } catch (cause) { errors[item.id] = errorMessage(cause); }
  finally { busy.value = undefined; }
}
async function undo(notice: Notice) {
  try {
    await request(`/agent-activity/notices/${encodeURIComponent(notice.id)}/undo`, json("POST"));
    notice.undone = true; emit("changed");
  } catch (cause) { notice.message = errorMessage(cause); notice.values = {}; notice.undoable = false; }
}
</script>

<template>
  <div v-if="requests.length || notices.length" class="agent-activity" aria-live="polite">
    <section v-for="item in requests" :key="item.id" class="agent-request" role="alertdialog" :aria-label="t(item.title, item.values)">
      <header>
        <ProviderIcon v-if="item.who.provider && item.who.provider !== 'terminal'" :provider="item.who.provider" :size="15" />
        <Icon v-else name="spark" :size="15" />
        <span class="agent-who">{{ who(item.who) }}</span><small>{{ t('asks') }}</small>
      </header>
      <strong class="agent-title">{{ t(item.title, item.values) }}</strong>
      <dl v-if="item.rows.length"><template v-for="row in item.rows" :key="row.label"><dt>{{ t(row.label) }}</dt><dd>{{ row.value }}</dd></template></dl>
      <p v-for="note in item.notes" :key="note" class="agent-note">{{ t(note) }}</p>
      <label v-if="item.key" class="agent-key">{{ t(item.key.label) }}
        <input v-model="keys[item.id]" type="password" autocomplete="new-password" spellcheck="false" :placeholder="t('Paste the key here')" @keydown.enter.prevent="answer(item, true)" />
        <small>{{ t('Saved in Tendly only. The agent never sees it.') }}</small>
      </label>
      <p v-if="errors[item.id]" class="agent-error" role="alert">{{ errors[item.id] }}</p>
      <footer>
        <button type="button" class="small-button" :disabled="busy === item.id" @click="answer(item, false)">{{ t('Decline') }}</button>
        <button type="button" class="small-button primary" :disabled="busy === item.id || (!!item.key && !item.key.optional && !keys[item.id]?.trim())" @click="answer(item, true)">{{ t('Approve') }}</button>
      </footer>
    </section>
    <div v-for="notice in notices" :key="notice.id" class="agent-notice" role="status">
      <Icon :name="notice.undone ? 'refresh' : 'check'" :size="14" />
      <span><b>{{ who(notice.who) }}</b> · {{ notice.undone ? t('Undone') : t(notice.message, notice.values) }}</span>
      <button v-if="notice.open" type="button" class="text-button" @click="emit('show', { kind: 'session', session_id: notice.open.session_id }); dismiss(notice.id)">{{ t('Open') }}</button>
      <button v-if="notice.undoable && !notice.undone" type="button" class="text-button" @click="undo(notice)">{{ t('Undo') }}</button>
      <button type="button" class="icon-button" :aria-label="t('Dismiss')" @click="dismiss(notice.id)"><Icon name="close" :size="13" /></button>
    </div>
  </div>
</template>

<style scoped>
.agent-activity{position:fixed;right:16px;bottom:40px;z-index:85;display:flex;flex-direction:column-reverse;gap:8px;width:min(380px,calc(100vw - 32px));max-height:calc(100dvh - 90px);overflow-y:auto;pointer-events:none}
.agent-activity>*{pointer-events:auto}
.agent-request{display:flex;flex-direction:column;gap:8px;padding:14px 15px 12px;border:1px solid var(--border);border-radius:var(--radius-lg);background:var(--surface);box-shadow:var(--shadow-lg);animation:agent-in .22s ease-out}
.agent-request>header{display:flex;align-items:center;gap:7px;font-size:var(--text-xs);color:var(--ink-soft)}
.agent-who{flex:0 1 auto;min-width:0;font-weight:600;color:var(--ink);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.agent-request>header>small,.agent-request>header>svg,.agent-request>header>span:first-child{flex:none}
.agent-title{font-size:var(--text-base);font-weight:600;color:var(--ink);line-height:1.4}
.agent-request dl{display:grid;grid-template-columns:auto 1fr;gap:4px 12px;margin:0;font-size:var(--text-sm)}
.agent-request dt{color:var(--ink-soft)}
.agent-request dd{margin:0;color:var(--ink);overflow-wrap:anywhere;font-family:var(--mono);font-size:var(--text-sm)}
.agent-note{margin:0;padding:6px 9px;border-radius:var(--radius-md);background:var(--warn-soft);color:var(--warn-ink);font-size:var(--text-sm);line-height:1.5}
.agent-key{display:flex;flex-direction:column;gap:5px;font-size:var(--text-sm);font-weight:600;color:var(--ink)}
.agent-key input{height:34px;padding:0 10px;border:1px solid var(--border);border-radius:var(--radius-md);font:inherit;font-weight:400}
.agent-key input:focus{outline:2px solid var(--focus,var(--focus));outline-offset:0;border-color:transparent}
.agent-key small{font-weight:400;color:var(--muted);font-size:var(--text-xs)}
.agent-error{margin:0;color:var(--danger);font-size:var(--text-sm)}
.agent-request footer{display:flex;justify-content:flex-end;gap:8px;margin-top:2px}
.agent-notice{display:flex;align-items:center;gap:8px;padding:9px 10px 9px 12px;border:1px solid var(--accent-line);border-radius:var(--radius-md);background:var(--ok-soft);color:var(--accent-ink);font-size:var(--text-sm);box-shadow:var(--shadow-lg);animation:agent-in .22s ease-out}
.agent-notice>svg{flex:none;color:var(--teal)}
.agent-notice>span{flex:1;min-width:0;line-height:1.45}
.agent-notice .text-button{font-size:var(--text-sm)}
@keyframes agent-in{from{opacity:0;transform:translateY(8px)}}
@media (max-width:760px){
  .agent-activity{left:0;right:0;bottom:0;width:auto;max-height:85dvh;padding:0 8px calc(8px + env(safe-area-inset-bottom))}
  .agent-request{border-radius:var(--radius-xl);padding:16px}
  .agent-title{font-size:var(--text-lg)}
  .agent-request dl,.agent-key{font-size:var(--text-md)}
  .agent-key input{height:44px;font-size:var(--input-text)}
  .agent-request footer .small-button{min-height:44px;flex:1;font-size:var(--text-base)}
  .agent-notice{font-size:var(--text-md)}
}
@media (prefers-reduced-motion:reduce){.agent-request,.agent-notice{animation:none}}
</style>
