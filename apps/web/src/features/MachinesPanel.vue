<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import Icon from './Icon.vue';
import CopyButton from './CopyButton.vue';
import { errorMessage, providerLabel } from './api';
import {
  GRANTS, GRANT_LABELS, GRANT_NOTES, addPeer, discoverPeers, invitePeer, looksLikeInvite, openPeer, peerSessions, readPeers, regrantPeer, removePeer,
  type Discovered, type Grant, type Invite, type Peer, type RemoteSessions,
} from './peers';
import { useI18n } from '../i18n';

/**
 * Other machines running Tendly, paired with this one.
 *
 * Nothing sits between them: each machine keeps its own sessions and decides
 * what each other machine may do there. This page is where that is decided,
 * and where the other machines' sessions are seen and opened.
 */
const emit = defineEmits<{ remote: [] }>();
const { t } = useI18n();

const name = ref(''), peers = ref<Peer[]>(), error = ref('');
const loading = ref(false);

// Inviting: this machine makes a link the other one redeems.
const inviteGrant = ref<Grant>('view'), invite = ref<Invite>(), inviteError = ref(''), inviting = ref(false);
// Adding: the link another machine made, and what that machine may do here.
const link = ref(''), addGrant = ref<Grant>('view'), addError = ref(''), adding = ref(false);
const linkOk = computed(() => looksLikeInvite(link.value));

const found = ref<Discovered[]>(), finding = ref(false), findError = ref('');
const open = ref<Record<string, RemoteSessions | 'loading' | { error: string }>>({});
const confirmRemove = ref<string>(), busy = ref<string>();

let timer: ReturnType<typeof setTimeout> | undefined, disposed = false;
async function load(quiet = false) {
  if (!quiet) loading.value = true;
  try {
    const next = await readPeers();
    name.value = next.name; peers.value = next.peers;
    if (!quiet) error.value = '';
  } catch (cause) { if (!quiet) error.value = errorMessage(cause); }
  finally { loading.value = false; }
  clearTimeout(timer);
  if (!disposed) timer = setTimeout(() => void load(true), 30000);
}
onMounted(() => load());
onBeforeUnmount(() => { disposed = true; clearTimeout(timer); });

async function makeInvite() {
  inviting.value = true; inviteError.value = '';
  try { invite.value = await invitePeer(inviteGrant.value); }
  catch (cause) { inviteError.value = errorMessage(cause); }
  finally { inviting.value = false; }
}
async function pair() {
  adding.value = true; addError.value = '';
  try { await addPeer(link.value.trim(), addGrant.value); link.value = ''; await load(true); }
  catch (cause) { addError.value = errorMessage(cause); }
  finally { adding.value = false; }
}
async function regrant(peer: Peer, grant: Grant) {
  busy.value = peer.id;
  try { Object.assign(peer, await regrantPeer(peer.id, grant)); }
  catch (cause) { error.value = errorMessage(cause); }
  finally { busy.value = undefined; }
}
async function remove(peer: Peer) {
  if (confirmRemove.value !== peer.id) { confirmRemove.value = peer.id; return; }
  confirmRemove.value = undefined; busy.value = peer.id;
  try { await removePeer(peer.id); delete open.value[peer.id]; await load(true); }
  catch (cause) { error.value = errorMessage(cause); }
  finally { busy.value = undefined; }
}
async function toggleSessions(peer: Peer) {
  if (open.value[peer.id]) { delete open.value[peer.id]; return; }
  open.value[peer.id] = 'loading';
  try { open.value[peer.id] = await peerSessions(peer.id); }
  catch (cause) { open.value[peer.id] = { error: errorMessage(cause) }; }
}
/**
 * The other machine's own window, signed in by a ticket good once. The tab is
 * opened before the ticket is asked for, so no popup blocker takes it.
 */
async function openWindow(peer: Peer, sessionId?: string) {
  const tab = window.open('', '_blank');
  try {
    const { url } = await openPeer(peer.id, sessionId);
    if (tab) { tab.opener = null; tab.location.href = url; } else window.location.href = url;
  } catch (cause) { tab?.close(); error.value = errorMessage(cause); }
}
async function find() {
  finding.value = true; findError.value = '';
  try { found.value = (await discoverPeers()).filter(machine => machine.tendly); }
  catch (cause) { findError.value = errorMessage(cause); }
  finally { finding.value = false; }
}

const sessionsOf = (listing: RemoteSessions, workspace: string) => listing.sessions.filter(session => session.workspace_id === workspace);
const host = (url: string | null) => { try { return url ? new URL(url).host : ''; } catch { return url ?? ''; } };
const state = (peer: Peer) => peer.online === true ? 'online' : peer.online === false ? 'offline' : 'one-way';
const STATE_LABELS = { online: 'Reachable', offline: 'Unreachable', 'one-way': 'Calls here only' } as const;
</script>

<template>
  <section class="machines-page">
    <div class="settings-lead"><p>{{ t('Pair other computers that run Tendly, to see their sessions here, open them, and let agents start work on them. Nothing sits in between: each machine keeps its own sessions and decides for itself what the others may do there.') }}</p></div>
    <p v-if="error" class="inline-error" role="alert">{{ error }}</p>

    <article class="machine-card">
      <header>
        <span class="machine-mark"><Icon name="monitor" :size="17"/></span>
        <strong>{{ t('Paired machines') }}</strong>
        <button type="button" class="small-button machine-refresh" :disabled="loading" :aria-label="t('Check again')" @click="load()"><Icon name="refresh" :size="13"/></button>
      </header>
      <p v-if="!peers" class="machine-hint">{{ t('Loading…') }}</p>
      <p v-else-if="!peers.length" class="machine-hint">{{ t('None yet. Invite one from this machine, or add the invite link another machine made.') }}</p>
      <ul v-else class="machine-list">
        <li v-for="peer in peers" :key="peer.id" class="machine-row">
          <div class="machine-line">
            <span :class="['machine-dot', state(peer)]" aria-hidden="true"/>
            <div class="machine-name">
              <strong>{{ peer.name }}</strong>
              <small>{{ host(peer.url) || t('No address this machine can reach') }}<template v-if="peer.version"> · {{ peer.version }}</template> · {{ t(STATE_LABELS[state(peer)]) }}</small>
            </div>
            <div class="machine-buttons">
              <button v-if="peer.url" type="button" class="small-button" :aria-expanded="!!open[peer.id]" @click="toggleSessions(peer)"><Icon name="chevron" :size="12" :class="{ turned: open[peer.id] }"/>{{ t('Sessions') }}</button>
              <button v-if="peer.granted === 'control'" type="button" class="small-button" @click="openWindow(peer)"><Icon name="arrow" :size="12"/>{{ t('Open its window') }}</button>
            </div>
          </div>
          <p v-if="peer.problem" class="machine-problem">{{ peer.problem }}</p>
          <div class="machine-grants">
            <label>
              <span>{{ t('It may, here') }}</span>
              <select :value="peer.grant" :disabled="busy === peer.id" @change="regrant(peer, ($event.target as HTMLSelectElement).value as Grant)">
                <option v-for="grant in GRANTS" :key="grant" :value="grant">{{ t(GRANT_LABELS[grant]) }}</option>
              </select>
            </label>
            <span class="machine-granted">{{ t('You may, there') }}: <b>{{ peer.granted ? t(GRANT_LABELS[peer.granted]) : t('Unknown') }}</b></span>
            <span class="machine-remove">
              <button type="button" :class="['small-button', { danger: confirmRemove === peer.id }]" :disabled="busy === peer.id" @click="remove(peer)">{{ confirmRemove === peer.id ? t('Remove, on both sides') : t('Remove') }}</button>
              <button v-if="confirmRemove === peer.id" type="button" class="small-button" @click="confirmRemove = undefined">{{ t('Cancel') }}</button>
            </span>
          </div>
          <div v-if="open[peer.id]" class="machine-sessions">
            <p v-if="open[peer.id] === 'loading'" class="machine-hint">{{ t('Loading…') }}</p>
            <p v-else-if="'error' in (open[peer.id] as object)" class="inline-error">{{ (open[peer.id] as { error: string }).error }}</p>
            <template v-else>
              <p v-if="!(open[peer.id] as RemoteSessions).sessions.length" class="machine-hint">{{ t('No sessions there.') }}</p>
              <div v-for="workspace in (open[peer.id] as RemoteSessions).workspaces" :key="workspace.id" v-show="sessionsOf(open[peer.id] as RemoteSessions, workspace.id).length" class="machine-workspace">
                <span class="machine-workspace-name"><Icon name="folder" :size="13"/>{{ workspace.name }}</span>
                <ul>
                  <li v-for="session in sessionsOf(open[peer.id] as RemoteSessions, workspace.id)" :key="session.id">
                    <span :class="['machine-activity', session.activity]" :title="session.activity"/>
                    <span class="machine-session-title">{{ session.title }}</span>
                    <small>{{ providerLabel(session.provider) }}<template v-if="session.branch"> · {{ session.branch }}</template></small>
                    <button v-if="peer.granted === 'control'" type="button" class="small-button" @click="openWindow(peer, session.id)">{{ t('Open') }}</button>
                  </li>
                </ul>
              </div>
            </template>
          </div>
        </li>
      </ul>
    </article>

    <div class="machine-pair">
      <article class="machine-card">
        <header><span class="machine-mark"><Icon name="upload" :size="16"/></span><strong>{{ t('Invite a machine') }}</strong></header>
        <p class="machine-hint">{{ t('Make a link here and paste it into Settings → Machines on the other machine. It works once, for 10 minutes.') }}</p>
        <label class="machine-field">
          <span>{{ t('What it may do on {name}', { name: name || t('this machine') }) }}</span>
          <select v-model="inviteGrant"><option v-for="grant in GRANTS" :key="grant" :value="grant">{{ t(GRANT_LABELS[grant]) }}</option></select>
          <small>{{ t(GRANT_NOTES[inviteGrant]) }}</small>
        </label>
        <button type="button" class="primary-button" :disabled="inviting" @click="makeInvite"><Icon name="plus" :size="14"/>{{ t('Make invite link') }}</button>
        <div v-if="invite" class="machine-invite">
          <code>{{ invite.link }}</code><CopyButton :text="invite.link" :label="t('Copy link')"/>
        </div>
        <p v-if="inviteError" class="inline-error" role="alert">{{ inviteError }} <button type="button" class="link-button" @click="emit('remote')">{{ t('Remote access') }}</button></p>
      </article>

      <article class="machine-card">
        <header><span class="machine-mark"><Icon name="download" :size="16"/></span><strong>{{ t('Add a machine') }}</strong></header>
        <p class="machine-hint">{{ t('Paste the invite link the other machine made.') }}</p>
        <input v-model="link" class="machine-link" type="url" spellcheck="false" autocomplete="off" placeholder="https://…#tendly-pair=…" @keydown.enter="linkOk && !adding && pair()"/>
        <label class="machine-field">
          <span>{{ t('What it may do here') }}</span>
          <select v-model="addGrant"><option v-for="grant in GRANTS" :key="grant" :value="grant">{{ t(GRANT_LABELS[grant]) }}</option></select>
          <small>{{ t(GRANT_NOTES[addGrant]) }}</small>
        </label>
        <button type="button" class="primary-button" :disabled="!linkOk || adding" @click="pair"><Icon name="check" :size="14"/>{{ adding ? t('Pairing…') : t('Pair') }}</button>
        <p v-if="addError" class="inline-error" role="alert">{{ addError }}</p>
      </article>
    </div>

    <article class="machine-card">
      <header>
        <span class="machine-mark"><Icon name="globe" :size="16"/></span><strong>{{ t('On your tailnet') }}</strong>
        <button type="button" class="small-button machine-refresh" :disabled="finding" @click="find"><Icon name="search" :size="13"/>{{ finding ? t('Looking…') : t('Find Tendly') }}</button>
      </header>
      <p class="machine-hint">{{ t('Machines on your tailnet that run Tendly with Remote access on. Open one to make an invite there.') }}</p>
      <p v-if="findError" class="inline-error" role="alert">{{ findError }}</p>
      <p v-else-if="found && !found.length" class="machine-hint">{{ t('No other machine on the tailnet answers as Tendly.') }}</p>
      <ul v-if="found?.length" class="machine-found">
        <li v-for="machine in found" :key="machine.url">
          <strong>{{ machine.name }}</strong><small>{{ machine.os }}<template v-if="machine.version"> · {{ machine.version }}</template></small>
          <span v-if="machine.paired" class="machine-tag">{{ t('Paired') }}</span>
          <span v-else-if="!machine.pairs" class="machine-tag">{{ t('Update it to pair') }}</span>
          <a v-else class="small-button" :href="`${machine.url}settings/machines`" target="_blank" rel="noopener">{{ t('Open') }}</a>
        </li>
      </ul>
    </article>
  </section>
</template>

<style scoped>
.machines-page{min-width:0;display:grid;gap:14px}
.settings-lead{margin:2px 0 2px}
.settings-lead p{margin:0;font-size:var(--text-sm);line-height:1.7;color:var(--ink-soft)}
.machine-card{border:1px solid var(--border);border-radius:var(--radius-lg);padding:13px 14px 14px;background:var(--surface);min-width:0;display:flex;flex-direction:column;gap:8px}
.machine-card header{display:flex;align-items:center;gap:10px}
.machine-card header strong{font-size:var(--text-md);font-weight:600;color:var(--ink)}
.machine-mark{display:grid;place-items:center;width:30px;height:30px;border-radius:var(--radius-md);background:var(--fill);color:var(--ink-soft);flex-shrink:0}
.machine-refresh{margin-left:auto;display:inline-flex;align-items:center;gap:5px}
.machine-hint{margin:0;font-size:var(--text-xs);line-height:1.6;color:var(--muted)}
.machine-list{list-style:none;margin:0;padding:0;display:grid}
.machine-row{padding:10px 0;border-top:1px solid var(--border);display:grid;gap:8px}
.machine-line{display:flex;align-items:center;gap:10px;min-width:0}
.machine-dot{width:9px;height:9px;border-radius:50%;flex-shrink:0;background:var(--line-strong)}
.machine-dot.online{background:var(--teal)}
.machine-dot.offline{background:var(--danger-ink)}
.machine-name{display:flex;flex-direction:column;min-width:0;flex:1}
.machine-name strong{font-size:var(--text-sm);font-weight:600;color:var(--ink);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.machine-name small{font-size:var(--text-xs);color:var(--muted);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.machine-buttons{display:flex;gap:6px;flex-shrink:0}
.machine-buttons .small-button{display:inline-flex;align-items:center;gap:5px}
.machine-buttons svg{transition:transform .15s}
.machine-buttons svg.turned{transform:rotate(90deg)}
.machine-problem{margin:0 0 0 19px;font-size:var(--text-xs);line-height:1.5;color:var(--danger-ink)}
.machine-grants{display:flex;flex-wrap:wrap;align-items:center;gap:8px 16px;margin-left:19px;font-size:var(--text-xs);color:var(--ink-soft)}
.machine-grants label{display:inline-flex;align-items:center;gap:6px}
.machine-granted b{font-weight:600;color:var(--ink)}
.machine-remove{margin-left:auto;display:inline-flex;gap:6px}
.small-button.danger{color:var(--danger-ink);border-color:var(--danger-line)}
.machine-sessions{margin-left:19px;padding:8px 10px;border-radius:var(--radius-md);background:var(--fill);display:grid;gap:8px}
.machine-workspace-name{display:inline-flex;align-items:center;gap:6px;font-size:var(--text-xs);font-weight:600;color:var(--ink-soft)}
.machine-workspace ul{list-style:none;margin:4px 0 0;padding:0;display:grid;gap:2px}
.machine-workspace li{display:flex;align-items:center;gap:8px;min-height:30px;min-width:0;font-size:var(--text-sm)}
.machine-session-title{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;color:var(--ink)}
.machine-workspace li small{flex-shrink:0;font-size:var(--text-xs);color:var(--muted)}
.machine-workspace li .small-button{margin-left:auto}
.machine-activity{width:7px;height:7px;border-radius:50%;flex-shrink:0;background:var(--line-strong)}
.machine-activity.working,.machine-activity.running{background:var(--accent)}
.machine-activity.idle{background:var(--teal)}
.machine-activity.waiting_for_approval{background:var(--warn)}
.machine-pair{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:14px}
.machine-pair .primary-button{align-self:flex-start}
.machine-field{display:grid;gap:4px;font-size:var(--text-xs);color:var(--ink-soft)}
.machine-field small{color:var(--muted);line-height:1.5}
.machine-card select{height:32px;padding:0 8px;border:1px solid var(--line);border-radius:var(--radius-md);background:var(--surface);font:inherit;font-size:var(--text-sm);color:var(--ink)}
.machine-field select{width:100%;max-width:260px}
.machine-grants select{height:28px;font-size:var(--text-xs)}
.machine-link{width:100%;min-width:0;height:32px;padding:0 9px;border:1px solid var(--line);border-radius:var(--radius-md);background:var(--surface);font-family:var(--mono);font-size:var(--text-xs);color:var(--ink)}
.machine-link:focus-visible,.machine-card select:focus-visible{outline:2px solid var(--focus);outline-offset:1px}
.machine-invite{display:flex;align-items:center;gap:8px;min-width:0}
.machine-invite code{flex:1;min-width:0;overflow-x:auto;white-space:nowrap;padding:7px 9px;border-radius:var(--radius-sm);background:var(--fill);font-size:var(--text-xs);color:var(--ink)}
.link-button{border:0;background:none;padding:0;font:inherit;color:var(--accent);text-decoration:underline;cursor:pointer}
.machine-found{list-style:none;margin:0;padding:0;display:grid}
.machine-found li{display:flex;align-items:center;gap:10px;min-height:36px;border-top:1px solid var(--border);font-size:var(--text-sm)}
.machine-found li strong{font-weight:600;color:var(--ink)}
.machine-found li small{font-size:var(--text-xs);color:var(--muted)}
.machine-found li > :last-child{margin-left:auto}
.machine-tag{font-size:var(--text-xs);color:var(--muted)}
@media(max-width:700px){
  .machine-pair{grid-template-columns:minmax(0,1fr)}
  .machine-remove{margin-left:0}
}
</style>
