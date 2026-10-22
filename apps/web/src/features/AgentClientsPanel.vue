<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import type { AgentProviderKind } from '@tendly/protocol';
import { backendCapabilities } from './backend-capabilities';
import { ApiError, errorMessage, json, providerLabel, request } from './api';
import Icon from './Icon.vue';
import ProviderIcon from './ProviderIcon.vue';
import { useI18n } from '../i18n';
import { clientTone } from './clients';

/** Where the program a session launches was resolved from. */
type ProgramSource = 'override' | 'path' | 'managed' | 'missing';
interface ClientView {
  provider: AgentProviderKind;
  command: string;
  npm_package: string;
  program: string;
  source: ProgramSource;
  installed: boolean;
  version?: string | null;
  managed_path?: string | null;
  install_available: boolean;
  shadowed: boolean;
}

const { t } = useI18n();
const supported = computed(() => (backendCapabilities as typeof backendCapabilities & { clients?: boolean }).clients === true);
const clients = ref<ClientView[]>([]), loading = ref(false), busy = ref(''), error = ref(''), notice = ref(''), unavailable = ref(false);
const confirming = ref<AgentProviderKind>();
let disposed = false, revision = 0, controller: AbortController | undefined;

async function load() {
  if (!supported.value) return;
  const own = ++revision; controller?.abort(); controller = new AbortController(); loading.value = true; error.value = '';
  try {
    const items = await request<ClientView[]>('/clients', { signal: controller.signal });
    if (disposed || own !== revision) return;
    clients.value = items; unavailable.value = false;
  } catch (cause) {
    if (disposed || own !== revision || (cause instanceof Error && cause.name === 'AbortError')) return;
    unavailable.value = cause instanceof ApiError && [404, 501].includes(cause.status);
    error.value = unavailable.value ? '' : errorMessage(cause);
  } finally { if (own === revision) loading.value = false; }
}
async function install(client: ClientView) {
  if (busy.value || !client.install_available || confirming.value !== client.provider) return;
  busy.value = client.provider; error.value = ''; notice.value = '';
  try {
    const updated = await request<ClientView>('/clients/' + encodeURIComponent(client.provider) + '/install', json('POST', { confirmed: true }));
    if (disposed) return;
    clients.value = clients.value.map(item => item.provider === updated.provider ? updated : item);
    confirming.value = undefined;
    // An install can succeed and still change nothing a session will run.
    notice.value = updated.shadowed
      ? 'Installed, but sessions keep using the copy already on the host PATH.'
      : updated.installed ? 'Installed. New sessions will use this client.' : 'The install finished but the client still does not report a version.';
  } catch (cause) { if (!disposed) error.value = errorMessage(cause); }
  finally { if (!disposed) busy.value = ''; }
}
/** Claude reports "2.1.280 (Claude Code)" and Codex "codex-cli 0.154.0"; the
 * card already says which client it is, so only the number is kept. */
const versionLabel = (client: ClientView) => client.version?.replace(/\s*\([^)]*\)\s*$/, '').replace(/^[a-z-]+\s+(?=\d)/i, '') || t('No version reported');
function sourceLabel(client: ClientView) {
  return client.source === 'override' ? t('Server override')
    : client.source === 'path' ? t('Found on host PATH')
    : client.source === 'managed' ? t('Installed by Tendly')
    : t('Not found');
}
onMounted(() => { void load(); });
// The backend's capabilities can arrive after this panel opens (a reload straight onto it).
watch(supported, ready => { if (ready && !clients.value.length) void load(); });
onBeforeUnmount(() => { disposed = true; controller?.abort(); });
</script>

<template>
  <section class="clients-page">
    <div class="settings-lead"><p>{{ t('Sessions run the official clients installed on this host. A missing one can be installed from npm.') }}</p><button v-if="supported&&!unavailable" type="button" class="settings-lead-action" :disabled="!!busy||loading" :aria-label="t('Re-check')" :title="t('Re-check')" @click="load"><Icon name="refresh" :size="14"/></button></div>

    <div v-if="!supported||unavailable" class="clients-unavailable"><Icon name="settings" :size="25"/><h3>{{ t('Client management needs an updated backend') }}</h3><p>{{ t('Sessions still launch whatever client is already on this host. Nothing is installed or changed here.') }}</p></div>
    <template v-else>
      <p v-if="error" class="inline-error client-message" role="alert">{{ error }}</p>
      <p v-if="notice" class="inline-success client-message" role="status">{{ notice }}</p>
      <p v-if="loading&&!clients.length" class="clients-quiet">{{ t('Checking this host…') }}</p>

      <article v-for="client in clients" :key="client.provider" class="client-card">
        <header>
          <span :class="['client-mark',client.provider]" :style="clientTone(client.provider)"><ProviderIcon :provider="client.provider" :size="19"/></span>
          <div>
            <strong>{{ providerLabel(client.provider) }} <em>{{ versionLabel(client) }}</em></strong>
            <small :title="client.program"><code>{{ client.program }}</code> · {{ sourceLabel(client) }}</small>
          </div>
          <span :class="['client-state',client.installed?'installed':'missing']">{{ t(client.installed?'Installed':'Not installed') }}</span>
        </header>
        <p v-if="client.managed_path&&client.shadowed" class="clients-quiet">{{ t('Tendly has its own copy, but the host PATH copy wins. Installing again will not change which client sessions run.') }}</p>
        <div v-if="confirming===client.provider" class="client-confirm" role="group" :aria-label="t('Install {name} from npm?', { name: providerLabel(client.provider) })">
          <strong>{{ t('Install {name} from npm?', { name: providerLabel(client.provider) }) }}</strong>
          <p>{{ t('This downloads {package} from the npm registry and runs its install scripts on this host. It installs into Tendly\'s own directory, not globally, and never replaces a client already on PATH.', { package: client.npm_package }) }}</p>
          <div class="client-confirm-actions"><button type="button" class="small-button" :disabled="!!busy" @click="confirming=undefined">{{ t('Cancel') }}</button><button type="button" class="small-button primary" :disabled="!!busy" @click="install(client)">{{ t(busy===client.provider?'Installing…':'Install now') }}</button></div>
        </div>
        <div v-else-if="client.install_available" class="client-actions">
          <button type="button" class="client-link" :disabled="!!busy" @click="confirming=client.provider;notice=''">{{ t(client.installed?'Install or update Tendly copy…':'Install from npm…') }}</button>
        </div>
      </article>
    </template>
  </section>
</template>

<style scoped>
.clients-page{min-width:0}
.settings-lead{display:flex;align-items:center;gap:12px;margin:2px 0 16px}
.settings-lead p{flex:1;margin:0;font-size:var(--text-sm);line-height:1.7;color:var(--ink-soft)}
.settings-lead-action{display:grid;place-items:center;width:30px;height:30px;flex-shrink:0;border:1px solid var(--border);border-radius:var(--radius-md);background:var(--surface);color:var(--ink-soft);cursor:pointer}
.settings-lead-action:hover:not(:disabled){color:var(--teal);border-color:var(--teal-line)}
.client-message{margin:0 0 10px}
.clients-quiet{font-size:var(--text-xs);line-height:1.7;color:var(--muted);margin:8px 0 0}
.client-card{border:1px solid var(--border);border-radius:var(--radius-lg);padding:13px 14px;margin-bottom:10px;background:var(--surface)}
.client-card header{display:flex;align-items:center;gap:11px}
.client-card header>div{flex:1;min-width:0}
.client-card strong{display:block;font-size:var(--text-md);font-weight:600;color:var(--ink)}
.client-card strong em{font-style:normal;font-weight:450;font-size:var(--text-xs);color:var(--muted);margin-left:4px}
.client-card small{display:block;margin-top:3px;font-size:var(--text-xs);color:var(--muted);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.client-card small code{font-size:var(--text-xs);color:var(--ink-soft)}
.client-mark{display:grid;place-items:center;width:34px;height:34px;border-radius:var(--radius-md);background:var(--client-soft);color:var(--client-ink);flex-shrink:0}
.client-state{border-radius:var(--radius-xl);padding:4px 9px;font-size:var(--text-xs);background:var(--fill);color:var(--ink-soft);flex-shrink:0}
.client-state.installed{background:var(--ok-soft);color:var(--accent-ink)}
.client-state.missing{background:var(--warn-soft);color:#9c844c}
.client-actions{margin:8px 0 0 45px}
/* Under the client's name, as its actions are: a quiet panel, not page-sized text. */
.client-confirm{margin:10px 0 0 45px;padding:11px 13px;border:1px solid var(--line);border-radius:var(--radius-md);background:var(--sunken)}
.client-confirm strong{font-size:var(--text-sm);font-weight:600}
.client-confirm p{margin:4px 0 0;font-size:var(--text-xs);line-height:1.7;color:var(--ink-soft);overflow-wrap:anywhere}
.client-confirm-actions{display:flex;justify-content:flex-end;gap:8px;margin-top:10px}
@media(max-width:520px){.client-confirm,.client-actions{margin-left:0}}
.client-link{border:0;background:none;padding:2px 0;font:inherit;font-size:var(--text-sm);color:var(--teal);cursor:pointer}
.client-link:hover:not(:disabled){text-decoration:underline}
.client-link:disabled{opacity:.5;cursor:not-allowed}
.clients-unavailable{text-align:center;padding:34px 20px;color:var(--muted)}
.clients-unavailable>svg{margin:auto}
.clients-unavailable h3{font-size:var(--text-xl);font-weight:550;margin:15px 0 9px;color:var(--ink)}
.clients-unavailable p{font-size:var(--text-sm);line-height:1.9;color:var(--ink-soft);max-width:430px;margin:auto}
@media(pointer:coarse){.settings-lead-action{width:44px;height:44px}.client-link{min-height:44px}}
</style>
