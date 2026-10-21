<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import type { AgentProviderKind, EndpointProfile, NativeHistorySource } from '@tendly/protocol';
import { accountLimitPercent, quotaLevel, quotaShortTitle, accountLoginUrl, accountResetDate, accountSignInActions, accountStatusLabel, accountResetCreditState, canResetAccountQuota, quotaResetAttempts, quotaResetOutcomeNotice, usageBackoffSeconds, usageRetryHint, QUOTA_RESET_STORAGE_WARNING, type AccountLimit, type AccountView } from './account-ui';
import { backendCapabilities } from './backend-capabilities';
import { ApiError, errorMessage, json, providerLabel, request } from './api';
import ModalDialog from './ModalDialog.vue';
import Icon from './Icon.vue';
import ProviderIcon from './ProviderIcon.vue';
import QuotaMeter from './QuotaMeter.vue';
import { loadPreferences, preferences, savePreferences } from './preferences';
import { useI18n } from '../i18n';
import { ACCOUNT_CLIENTS, AGENT_CLIENTS, clientLabel, clientTone } from './clients';

const props = defineProps<{ embedded?: boolean }>();
const embedded = computed(() => props.embedded === true);
const emit = defineEmits<{ close: []; changed: [profile?: EndpointProfile] }>();
const { t, locale } = useI18n();
const supported = computed(() => (backendCapabilities as typeof backendCapabilities & { accounts?: boolean }).accounts === true);
const accounts = ref<AccountView[]>([]), loading = ref(false), busy = ref(''), error = ref(''), notice = ref(''), unavailable = ref(false);
const selectedId = ref(''), adding = ref(false), importing = ref(false), name = ref(''), provider = ref<AgentProviderKind>(ACCOUNT_CLIENTS[0]);
const nativeSources = ref<NativeHistorySource[]>([]), nativeSourceId = ref(''), nativeName = ref(''), sharedConfirmed = ref(false), loadingSources = ref(false), sourcesLoaded = ref(false);
const selectedSource = computed(() => nativeSources.value.find(source => source.id === nativeSourceId.value));
const confirmAction = ref<'logout' | 'reset' | 'remove'>(), resetConfirmed = ref(false), resetVersion = ref(0);
const selected = computed(() => accounts.value.find(account => account.id === selectedId.value));
const loginUrl = computed(() => selected.value && accountLoginUrl(selected.value));
const signInActions = computed(() => selected.value && accountSignInActions(selected.value));
const resetAttempt = computed(() => { void resetVersion.value; return selected.value && quotaResetAttempts.get(selected.value.id); });
const showReset = computed(() => !!selected.value && (canResetAccountQuota(selected.value) || !!resetAttempt.value));
const resetStorageAvailable = computed(() => { void resetVersion.value; return quotaResetAttempts.storageAvailable(); });
/** Quote the provider's own wait hint when it sent one; otherwise stay vague rather than invent a delay. */
const usageRetryText = computed(() => {
  const hint = usageRetryHint(selected.value?.usage?.retry_after_seconds);
  if (!hint) return '';
  return hint.unit === 'seconds'
    ? t('The official Claude usage endpoint is rate limited. Try again in about {n} seconds.').replace('{n}', String(hint.value))
    : t('The official Claude usage endpoint is rate limited. Try again in about {n} minutes.').replace('{n}', String(hint.value));
});
let disposed = false, revision = 0, controller: AbortController | undefined;
function upsert(account: AccountView) { const index = accounts.value.findIndex(item => item.id === account.id); if (index < 0) accounts.value.push(account); else accounts.value[index] = account; }
function choose(id: string) { if (busy.value) return; selectedId.value = id; renaming.value = false; adding.value = false; importing.value = false; error.value = ''; notice.value = ''; confirmAction.value = undefined; resetConfirmed.value = false; }
async function toggleQuotaBadge() {
  const account = selected.value; if (!account || busy.value) return;
  busy.value = "quota-badge"; error.value = "";
  try {
    const current = await loadPreferences(true);
    const hidden = current.hidden_account_quotas ?? [];
    await savePreferences({ ...current, hidden_account_quotas: hidden.includes(account.id) ? hidden.filter(id => id !== account.id) : [...hidden, account.id] });
  } catch (cause) { error.value = errorMessage(cause); }
  finally { busy.value = ""; }
}
async function load() {
  void loadPreferences();
  if (!supported.value) return;
  const own = ++revision; controller?.abort(); controller = new AbortController(); loading.value = true; error.value = '';
  try { const items = await request<AccountView[]>('/accounts', { signal: controller.signal }); if (disposed || own !== revision) return; accounts.value = items; unavailable.value = false; if (!items.some(account => account.id === selectedId.value)) selectedId.value = items[0]?.id ?? ''; }
  catch (cause) { if (!disposed && own === revision && !(cause instanceof Error && cause.name === 'AbortError')) { unavailable.value = cause instanceof ApiError && [404, 501].includes(cause.status); error.value = unavailable.value ? '' : errorMessage(cause); } }
  finally { if (own === revision) loading.value = false; }
}
async function readAccount(id: string) { const account = await request<AccountView>('/accounts/' + encodeURIComponent(id)); if (!disposed) upsert(account); return account; }
async function create() {
  if (busy.value || !name.value.trim() || !supported.value) return;
  busy.value = 'create'; error.value = ''; notice.value = '';
  try {
    const account = await request<AccountView>('/accounts', json('POST', { name: name.value.trim(), provider: provider.value }));
    let profile: EndpointProfile | undefined;
    try { profile = await request<EndpointProfile>('/endpoint-profiles/' + encodeURIComponent(account.profile_id)); } catch { /* Parent refresh remains the fallback. */ }
    if (!disposed) { upsert(account); selectedId.value = account.id; name.value = ''; adding.value = false; notice.value = 'Account configuration created. Sign-in starts only when you choose it.'; emit('changed', profile); }
  }
  catch (cause) { if (!disposed) error.value = errorMessage(cause); }
  finally { if (!disposed) busy.value = ''; }
}
async function loadNativeSources() {
  if (!backendCapabilities.nativeConfig) return;
  loadingSources.value = true; sourcesLoaded.value = false; error.value = ''; nativeSources.value = []; nativeSourceId.value = '';
  try {
    const sources = await request<NativeHistorySource[]>('/host/native-configurations');
    if (disposed || !importing.value) return;
    nativeSources.value = sources; sourcesLoaded.value = true;
    nativeSourceId.value = sources.find(source => source.available)?.id ?? '';
    nativeName.value = selectedSource.value?.label ?? '';
  } catch (cause) { if (!disposed) error.value = errorMessage(cause); }
  finally { if (!disposed) loadingSources.value = false; }
}
function openImport() {
  if (busy.value || !backendCapabilities.nativeConfig) return;
  adding.value = true; importing.value = true; sharedConfirmed.value = false; nativeName.value = ''; error.value = ''; notice.value = '';
  void loadNativeSources();
}
async function importNative() {
  if (!backendCapabilities.accountImportNative) { error.value = 'Upgrade the backend to import existing configurations.'; return; }
  const source = selectedSource.value;
  if (!source?.available || !sharedConfirmed.value || busy.value) return;
  busy.value = 'import'; error.value = ''; notice.value = '';
  try {
    const account = await request<AccountView>('/accounts/import-native', json('POST', { source_id: source.id, ...(nativeName.value.trim() ? { name: nativeName.value.trim() } : {}), confirmed_shared_config: true }));
    let profile: EndpointProfile | undefined;
    try { profile = await request<EndpointProfile>('/endpoint-profiles/' + encodeURIComponent(account.profile_id)); } catch { /* account import already succeeded */ }
    if (!disposed) { upsert(account); selectedId.value = account.id; adding.value = false; importing.value = false; sharedConfirmed.value = false; emit('changed', profile); notice.value = 'Existing native configuration linked as an official account. Credentials remain owned by the native client.'; }
  } catch (cause) { if (!disposed) error.value = errorMessage(cause); }
  finally { if (!disposed) busy.value = ''; }
}
async function action(kind: 'refresh' | 'refresh-token' | 'device' | 'browser' | 'cancel-login' | 'logout') {
  const account = selected.value; if (!account || busy.value) return;
  if ((kind === 'device' || kind === 'browser') && !account.capabilities.login || kind === 'refresh-token' && !account.capabilities.refresh_token || kind === 'logout' && (!account.capabilities.logout || confirmAction.value !== 'logout')) return;
  busy.value = kind; error.value = ''; notice.value = '';
  const route = kind === 'refresh-token' ? 'refresh' : kind === 'device' || kind === 'browser' ? 'login' : kind;
  const body = kind === 'refresh-token' ? { refresh_token: true } : kind === 'device' || kind === 'browser' ? { mode: kind } : kind === 'logout' ? { confirmed: true } : {};
  try {
    await request('/accounts/' + encodeURIComponent(account.id) + '/' + route, json('POST', body));
    if (disposed) return;
    await readAccount(account.id); confirmAction.value = undefined; emit('changed');
    // A successful request is not proof that authentication, token refresh or
    // quota changed. Only AccountView's official status is displayed.
  } catch (cause) { if (!disposed) error.value = errorMessage(cause); }
  finally { if (!disposed) busy.value = ''; }
}
function askReset() { if (!selected.value || busy.value || !canResetAccountQuota(selected.value) && !resetAttempt.value) return; confirmAction.value = 'reset'; resetConfirmed.value = false; }
/**
 * Explicit usage check. It runs only when the user clicks, and it reports the
 * official client's own capture: an absent capture stays "not captured yet"
 * rather than becoming an estimated percentage.
 */
/**
 * The official endpoint allows roughly one check per window and answers 429 for
 * the rest of it. Without a cooldown every extra press spends the one allowance
 * the user is waiting on, which is why usage could look permanently unavailable.
 */
const usageCooldownUntil = ref(0), usageNow = ref(Date.now()), usageRefusals = ref(0);
let cooldownTimer: ReturnType<typeof setInterval> | undefined;
const usageCooldown = computed(() => Math.max(0, Math.ceil((usageCooldownUntil.value - usageNow.value) / 1000)));
const usageCooldownLabel = computed(() => {
  const hint = usageRetryHint(usageCooldown.value);
  if (!hint) return t('Check usage');
  return hint.unit === 'seconds' ? t('Check again in {n}s', { n: hint.value }) : t('Check again in {n} min', { n: hint.value });
});
function beginUsageCooldown(seconds: number) {
  usageCooldownUntil.value = Date.now() + seconds * 1000;
  usageNow.value = Date.now();
  clearInterval(cooldownTimer);
  cooldownTimer = setInterval(() => {
    usageNow.value = Date.now();
    if (usageCooldown.value <= 0) { clearInterval(cooldownTimer); cooldownTimer = undefined; }
  }, 1000);
}
onBeforeUnmount(() => clearInterval(cooldownTimer));

const proxyDraft = ref('');
watch(() => selected.value?.id, () => { proxyDraft.value = selected.value?.proxy_url ?? ''; }, { immediate: true });
const renaming = ref(false), nameDraft = ref('');
function beginRename() { if (busy.value) return; nameDraft.value = selected.value?.name ?? ''; renaming.value = true; }
async function saveName() {
  const account = selected.value;
  if (!account || busy.value || !nameDraft.value.trim()) return;
  busy.value = 'rename'; error.value = ''; notice.value = '';
  try {
    const view = await request<AccountView>('/accounts/' + encodeURIComponent(account.id) + '/rename', json('POST', { name: nameDraft.value.trim() }));
    if (disposed) return;
    upsert(view); renaming.value = false;
    // The endpoint profile shares this name, so the parent list must refresh too.
    emit('changed');
  } catch (cause) { if (!disposed) error.value = errorMessage(cause); }
  finally { if (!disposed) busy.value = ''; }
}
async function removeAccount() {
  const account = selected.value;
  if (!account || busy.value || confirmAction.value !== 'remove') return;
  busy.value = 'remove'; error.value = ''; notice.value = '';
  try {
    await request('/accounts/' + encodeURIComponent(account.id) + '/remove', json('POST', { confirmed: true }));
    if (disposed) return;
    accounts.value = accounts.value.filter(item => item.id !== account.id);
    selectedId.value = accounts.value[0]?.id ?? '';
    confirmAction.value = undefined;
    notice.value = account.native_source_id
      ? t('Account removed. The host configuration directory was left untouched.')
      : t('Account removed, along with the configuration directory it owned.');
    emit('changed');
  } catch (cause) { if (!disposed) error.value = errorMessage(cause); }
  finally { if (!disposed) busy.value = ''; }
}
async function saveProxy() {
  const account = selected.value;
  if (!account || busy.value) return;
  busy.value = 'proxy'; error.value = ''; notice.value = '';
  try {
    const view = await request<AccountView>('/accounts/' + encodeURIComponent(account.id) + '/proxy', json('POST', { proxy_url: proxyDraft.value.trim() || null }));
    if (disposed) return;
    upsert(view);
    proxyDraft.value = view.proxy_url ?? '';
    notice.value = view.proxy_url ? t('Proxy saved. It applies to the next sign-in or usage check.') : t('Proxy cleared. This account now connects directly.');
  } catch (cause) { if (!disposed) error.value = errorMessage(cause); }
  finally { if (!disposed) busy.value = ''; }
}
async function checkUsage() {
  const account = selected.value;
  if (!account || busy.value || !account.capabilities.usage || usageCooldown.value > 0) return;
  busy.value = 'usage'; error.value = ''; notice.value = '';
  try {
    const view = await request<AccountView>('/accounts/' + encodeURIComponent(account.id) + '/usage', json('POST', {}));
    if (disposed) return;
    upsert(view);
    if (view.usage?.availability === 'capture_disabled') notice.value = t('Usage capture is turned off, so nothing has been recorded.');
    else if (view.usage?.availability === 'capture_missing') notice.value = t('No local usage capture yet. Start an official Claude Code session once, then check again.');
    else if (view.usage?.availability === 'not_reported') notice.value = t('The official client did not report usage for this account. No estimate is shown.');
    else if (view.usage?.availability === 'credential_unavailable') notice.value = t('The official Claude OAuth credential could not be read from its native storage. Sign in with Claude Code, then query again.');
    else if (view.usage?.availability === 'query_failed') notice.value = t('The official Claude usage endpoint could not be reached. Check the network, then query again.');
    else if (view.usage?.availability === 'unauthorized') notice.value = t('The official Claude usage endpoint rejected the stored credential. Sign in again with Claude Code, then query again.');
    else if (view.usage?.availability === 'rate_limited') notice.value = usageRetryText.value || t('The official Claude usage endpoint is rate limited right now. Wait a moment, then query again.');
    else if (view.usage?.availability === 'stale') notice.value = t('This capture predates its reset window, so it is shown as stale rather than current.');
    // Observed behaviour: repeated probing while refused does not recover, and
    // appears to push the allowance further out. So each consecutive refusal
    // doubles the wait instead of retrying on a fixed interval.
    if (view.usage?.availability === 'rate_limited') {
      usageRefusals.value += 1;
      beginUsageCooldown(usageBackoffSeconds(usageRefusals.value, view.usage.retry_after_seconds));
    } else { usageRefusals.value = 0; beginUsageCooldown(usageBackoffSeconds(0)); }
  } catch (cause) { if (!disposed) { error.value = errorMessage(cause); beginUsageCooldown(60); } }
  finally { if (!disposed) busy.value = ''; }
}
async function resetQuota() {
  const account = selected.value; if (!account || busy.value || confirmAction.value !== 'reset' || !resetConfirmed.value) return;
  let committed = false; busy.value = 'reset'; error.value = ''; notice.value = '';
  try {
    const attempt = quotaResetAttempts.begin(account, true); resetVersion.value++;
    const result = await request<AccountView | undefined>('/accounts/' + encodeURIComponent(account.id) + '/reset-quota', json('POST', { confirmed: true, idempotency_key: attempt.idempotency_key, ...(attempt.credit_id ? { credit_id: attempt.credit_id } : {}) }));
    const refreshed = await readAccount(account.id);
    committed = quotaResetAttempts.complete(account.id); resetVersion.value++;
    if (disposed) return;
    confirmAction.value = undefined; resetConfirmed.value = false;
    notice.value = committed ? quotaResetOutcomeNotice(result?.reset_outcome ?? refreshed.reset_outcome) : QUOTA_RESET_STORAGE_WARNING;
    emit('changed');
  } catch (cause) {
    if (!committed) quotaResetAttempts.unknown(account.id);
    resetVersion.value++;
    if (!disposed) { error.value = t(errorMessage(cause)); if (!committed) notice.value = quotaResetAttempts.storageAvailable() ? 'The reset result is uncertain. Its original request key is kept; any explicit retry uses that same key, never a new credit request.' : QUOTA_RESET_STORAGE_WARNING; }
  } finally { if (!disposed) busy.value = ''; }
}
function resetTime(value?: string | number) { const date = accountResetDate(value); return date ? new Intl.DateTimeFormat(locale.value, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }).format(date) : t('Reset time unavailable'); }
const creditDetails = computed(() => selected.value?.limits?.reset_credits_source === 'official' ? selected.value.limits.reset_credit_details ?? [] : []);
function creditTime(seconds: number) { const date = new Date(seconds * 1000); return new Intl.DateTimeFormat(locale.value, { ...(date.getFullYear() !== new Date().getFullYear() ? { year: 'numeric' } : {}), month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }).format(date); }
function creditExpiresSoon(seconds: number | null) { return seconds !== null && seconds * 1000 - Date.now() < 3 * 86400_000; }
function creditLeft(seconds: number) { const hours = (seconds * 1000 - Date.now()) / 3600_000; return hours <= 0 ? t('Expired') : hours < 24 ? t('{count} h left',{count:Math.ceil(hours)}) : t('{count} d left',{count:Math.floor(hours / 24)}); }
function shortTitle(slot: 'primary' | 'secondary', limit?: AccountLimit) { const name = quotaShortTitle(slot, limit); return t(name.key, name.values); }
function limitTitle(slot: 'primary' | 'secondary', limit?: AccountLimit) {
  if (limit?.window_kind === 'five_hour') return t('5-hour quota window');
  if (limit?.window_kind === 'weekly') return t('Weekly quota window');
  return t(slot === 'primary' ? 'Primary window' : 'Secondary window');
}
onMounted(() => { void load(); });
// Opened straight from a link, the page mounts before the server has said what it
// supports; load once it has, instead of showing an empty list until a revisit.
watch(supported, (now, before) => { if (now && !before) void load(); });
onBeforeUnmount(() => { disposed = true; revision++; controller?.abort(); });
</script>

<template>
  <ModalDialog :title="t('Official accounts')" wide :closable="!busy" :embedded="embedded" @close="emit('close')">
    <p class="accounts-lead">{{ t('Sign in to official Claude Code or Codex accounts and watch their usage. Each account can be picked as a session configuration.') }}</p>
    <div v-if="!supported||unavailable" class="accounts-unavailable"><Icon name="settings" :size="27" /><h3>{{ t('Account management needs an updated backend') }}</h3><p>{{ t('Existing native configuration references still work. No login, refresh or quota result is being simulated.') }}</p></div>
    <template v-else>
      <div v-if="error" class="account-error" role="alert">{{ error }}</div><p v-if="notice" class="account-notice" role="status">{{ t(notice) }}</p>
      <div v-if="!accounts.length&&!adding&&!loading" class="accounts-first"><span class="accounts-first-marks"><span v-for="client in AGENT_CLIENTS" :key="client" :class="['account-provider',client]" :style="clientTone(client)"><ProviderIcon :provider="client" :size="20" /></span></span><h3>{{ t('Keep your accounts in one place') }}</h3><p>{{ t('Add an account to sign in, or link a Claude Code or Codex already set up on this host.') }}</p><div><button class="account-add" :disabled="!!busy" @click="adding=true;importing=false;confirmAction=undefined;error='';notice=''"><Icon name="plus" :size="16" />{{ t('Add account') }}</button><button class="account-add account-import" :disabled="!!busy||!backendCapabilities.nativeConfig" @click="openImport"><Icon name="folder" :size="16" />{{ t('Import existing configuration') }}</button></div></div>
      <div v-else class="accounts-layout"><aside class="accounts-list"><div class="accounts-list-heading"><span>{{ t('Accounts') }} <small>{{ accounts.length }}</small></span><button :aria-label="t('Refresh account list')" :disabled="loading||!!busy" @click="load"><Icon name="refresh" :size="16" /></button></div><button v-for="account in accounts" :key="account.id" :class="['account-choice',{selected:selectedId===account.id&&!adding}]" :disabled="!!busy" @click="choose(account.id)"><span :class="['account-provider',account.provider]" :style="clientTone(account.provider)"><ProviderIcon :provider="account.provider" :size="19" /></span><span class="account-choice-text"><strong>{{ account.name }}</strong><small>{{ account.email??providerLabel(account.provider) }}<template v-if="account.native_source_id"> · {{ t('Imported native config') }}</template></small><span v-if="account.limits?.primary||account.limits?.secondary" class="account-mini" :title="t('Left in each quota window')"><template v-for="slot in (['primary','secondary'] as const)" :key="slot"><span v-if="accountLimitPercent(account.limits?.[slot])!==undefined" :class="quotaLevel(accountLimitPercent(account.limits?.[slot]))">{{ shortTitle(slot, account.limits?.[slot]) }} <b>{{ Math.max(0, Math.round(100 - accountLimitPercent(account.limits?.[slot])!)) }}%</b></span></template></span></span><span :class="['account-dot',account.status]" /></button><p v-if="loading" class="accounts-empty">{{ t('Loading accounts…') }}</p><button class="account-add" :disabled="!!busy" @click="adding=true;importing=false;confirmAction=undefined;error='';notice=''"><Icon name="plus" :size="16" />{{ t('Add account') }}</button><button class="account-add account-import" :disabled="!!busy||!backendCapabilities.nativeConfig" @click="openImport"><Icon name="folder" :size="16" />{{ t('Import existing configuration') }}</button></aside>
        <main class="account-detail"><form v-if="adding&&!importing" class="account-create" @submit.prevent="create"><h3>{{ t('A separate home for this account') }}</h3><p>{{ t('Creating a configuration does not sign in, copy host credentials, or send a model request.') }}</p><label>{{ t('Account name') }}<input v-model="name" maxlength="120" required :disabled="!!busy" :placeholder="t('Personal account or work account')" /></label><label>{{ t('Native client') }}<select v-model="provider" :disabled="!!busy"><option v-for="client in ACCOUNT_CLIENTS" :key="client" :value="client">{{ clientLabel(client) }}</option></select></label><div class="account-buttons"><button type="button" :disabled="!!busy" @click="adding=false">{{ t('Cancel') }}</button><button class="account-primary" :disabled="!!busy||!name.trim()">{{ t(busy?'Creating…':'Create account configuration') }}</button></div></form>
        <form v-else-if="adding&&importing" class="account-create account-native-import" @submit.prevent="importNative"><h3>{{ t('Import existing configuration') }}</h3><p>{{ t('Link an existing Claude Code or Codex home as an official account. Credentials and native settings remain in place.') }}</p><label>{{ t('Host configuration source') }}<select v-model="nativeSourceId" :disabled="!!busy||loadingSources"><option value="" disabled>{{ t('Select a host configuration') }}</option><option v-for="source in nativeSources" :key="source.id" :value="source.id" :disabled="!source.available">{{ source.label }} · {{ t(source.available?'Directory available':'Not found on this host') }}</option></select></label><p v-if="loadingSources" class="form-help" role="status">{{ t('Loading host configurations…') }}</p><p v-else-if="sourcesLoaded&&!nativeSources.some(source=>source.available)" class="form-help">{{ t('No usable native configuration directories were found. Configure the native client on this host, then refresh.') }}</p><template v-if="selectedSource"><div class="shared-config-panel"><strong><ProviderIcon :provider="selectedSource.provider" :size="17" />{{ t('Host configuration · shared sign-in') }}</strong><code>{{ selectedSource.path }}</code><p>{{ t('Directory availability does not confirm sign-in. The native client may still ask you to log in.') }}</p></div><label>{{ t('Account name') }}<input v-model="nativeName" maxlength="120" :disabled="!!busy" :placeholder="selectedSource.label" /></label><div class="shared-config-consent"><p>{{ t('Tendly only stores a reference. It does not copy credentials, read tokens, or start the native client.') }}</p><label><input v-model="sharedConfirmed" type="checkbox" :disabled="!!busy" />{{ t('I agree to reuse this shared host configuration for the account.') }}</label></div></template><div class="account-buttons"><button type="button" :disabled="!!busy" @click="adding=false;importing=false">{{ t('Cancel') }}</button><button class="account-primary" :disabled="!!busy||loadingSources||!selectedSource?.available||!sharedConfirmed">{{ t(busy?'Importing…':'Import configuration') }}</button></div></form>
        <template v-else-if="selected"><header class="account-detail-header"><span :class="['account-provider large',selected.provider]" :style="clientTone(selected.provider)"><ProviderIcon :provider="selected.provider" :size="27" /></span><div><form v-if="renaming" class="account-rename" @submit.prevent="saveName"><input v-model="nameDraft" maxlength="120" :disabled="!!busy" :aria-label="t('Account name')" /><button class="small-button" :disabled="!!busy||!nameDraft.trim()">{{ t(busy==='rename'?'Saving…':'Save') }}</button><button type="button" class="small-button" :disabled="!!busy" @click="renaming=false">{{ t('Cancel') }}</button></form><h3 v-else>{{ selected.name }} <button type="button" class="account-rename-button" :aria-label="t('Rename account')" :title="t('Rename account')" :disabled="!!busy" @click="beginRename"><Icon name="edit" :size="13" /></button></h3><p>{{ selected.email??providerLabel(selected.provider) }}<span v-if="selected.plan" class="account-plan">{{ selected.plan }}</span></p></div><span :class="['account-state',selected.status]">{{ t(accountStatusLabel(selected.status)) }}</span></header>
          <section class="account-section"><div class="account-section-title"><h4>{{ t('Official usage limits') }}</h4><button v-if="selected.capabilities.usage" type="button" :disabled="!!busy||usageCooldown>0" :title="usageCooldown>0?t('The official endpoint allows about one check per window. Waiting avoids spending the next allowance.'):undefined" @click="checkUsage"><Icon name="refresh" :size="13" />{{ busy==='usage'?t('Checking usage…'):usageCooldown>0?usageCooldownLabel:t('Check usage') }}</button><button v-else-if="selected.capabilities.quota" type="button" :disabled="!!busy" :title="t('Reported by client')" @click="action('refresh')"><Icon name="refresh" :size="13" />{{ busy==='refresh'?t('Checking usage…'):t('Check usage') }}</button><span v-else class="account-source-label">{{ t('Reported by client') }}</span></div>
            <p v-if="!selected.capabilities.quota&&!selected.capabilities.usage" class="account-quiet">{{ t('This client does not expose account quota through a supported interface. No estimated percentage is shown.') }}</p>
            <div v-else-if="selected.usage?.availability==='capture_disabled'" class="account-native-help"><strong>{{ t('Usage capture is off') }}</strong><p>{{ t('No OAuth credential was available for the official usage query, and local status-line capture is disabled. Sign in with Claude Code, then query again.') }}</p><code>TENDLY_CLAUDE_USAGE_CAPTURE=1</code></div>
            <p v-else-if="selected.usage?.availability==='credential_unavailable'" class="account-quiet">{{ t('The official Claude OAuth credential was not available to Tendly. Sign in with Claude Code, then query again. No token is shown or stored in the browser.') }}</p>
            <p v-else-if="selected.usage?.availability==='query_failed'" class="account-quiet">{{ t('The official Claude usage endpoint could not be reached. Check the server network or proxy, then query again.') }}</p>
            <p v-else-if="selected.usage?.availability==='rate_limited'" class="account-quiet">{{ usageRetryText || t('The official Claude usage endpoint is rate limited right now. Wait a moment, then query again.') }} {{ t('This limit is applied by the provider; Tendly does not retry on its own.') }}</p>
            <p v-else-if="selected.usage?.availability==='unauthorized'" class="account-quiet">{{ t('The official Claude usage endpoint rejected the stored credential. Sign in again with Claude Code, then query again. Tendly does not refresh or replace official tokens.') }}</p>
            <p v-else-if="selected.usage?.availability==='capture_missing'" class="account-quiet">{{ t('No local usage capture yet. Start an official Claude Code session once, then press Check usage. Tendly reads only the local status line capture and never your credentials.') }}</p>
            <p v-else-if="selected.usage?.availability==='stale'" class="account-quiet">{{ t('The stored capture predates its reset window. It is shown as stale rather than current.') }}</p>
            <template v-if="selected.capabilities.quota">
              <div class="account-limits"><QuotaMeter v-for="slot in (['primary','secondary'] as const)" :key="slot" :position="slot" :limit="selected.limits?.[slot]" /></div><template v-if="showReset"><p v-if="!resetStorageAvailable" class="account-notice" role="status">{{ t(QUOTA_RESET_STORAGE_WARNING) }}</p><div class="account-reset"><span class="account-reset-count" :class="{empty:!selected.limits?.reset_credits}">{{ selected.limits?.reset_credits ?? '—' }}</span><div><strong>{{ t('Official reset credits') }}</strong><p v-if="selected.limits?.reset_credits_source==='official'&&selected.limits?.reset_credits!==undefined">{{ t('Granted by the provider') }}<template v-if="creditDetails.length"> · <span :class="{'account-credit-soon':creditExpiresSoon(creditDetails[0].expires_at)}">{{ creditDetails[0].expires_at===null?t('Never expires'):t('Earliest expires {time}',{time:creditTime(creditDetails[0].expires_at)}) }}</span></template></p><p v-else>{{ selected.limits?.reset_credits===undefined?t('Not reported'):t('{count} available · source not confirmed',{count:selected.limits.reset_credits}) }}</p></div><button :disabled="!!busy||(!canResetAccountQuota(selected)&&!resetAttempt)||(!resetStorageAvailable&&!resetAttempt)" @click="askReset">{{ t(resetAttempt?'Review pending reset':'Reset quota…') }}</button><details v-if="creditDetails.length" class="account-credit-list"><summary>{{ t('Each credit ({count})',{count:creditDetails.length}) }}<small v-if="creditDetails.length<(selected.limits?.reset_credits??0)">{{ t('The provider listed {shown} of {total}',{shown:creditDetails.length,total:selected.limits?.reset_credits??0}) }}</small></summary><ol><li v-for="(credit, index) in creditDetails" :key="index"><div><strong>{{ credit.title || t('Reset credit {n}',{n:index+1}) }}</strong><small v-if="credit.granted_at!==undefined">{{ t('Granted {time}',{time:creditTime(credit.granted_at)}) }}</small></div><div class="account-credit-expiry"><span>{{ credit.expires_at===null?t('Never expires'):t('Expires {time}',{time:creditTime(credit.expires_at)}) }}</span><em v-if="credit.expires_at!==null" :class="{soon:creditExpiresSoon(credit.expires_at)}">{{ creditLeft(credit.expires_at) }}</em></div></li></ol></details></div><p class="account-quiet">{{ t('This may consume a real account resource; there is no local quota bypass.') }}</p></template></template>
            <template v-else-if="selected.capabilities.usage&&(selected.limits?.primary||selected.limits?.secondary)"><div class="account-limits"><QuotaMeter v-for="slot in (['primary','secondary'] as const)" :key="slot" :position="slot" :limit="selected.limits?.[slot]" /></div><p v-if="selected.usage?.captured_at" class="account-checked">{{ t('Captured {time}',{time:resetTime(selected.usage.captured_at)}) }}</p><p v-if="selected.usage?.availability==='cached'" class="account-checked" role="status">{{ t('Cached from {time}',{time:resetTime(selected.usage.retained_checked_at)}) }}</p><p v-else-if="selected.usage?.retained_checked_at" class="account-notice" role="status">{{ t('Query failed; showing the earlier result from {time}.',{time:resetTime(selected.usage.retained_checked_at)}) }} {{ usageRetryText||'' }}</p><p v-if="selected.usage?.source!=='claude_oauth_usage'" class="account-quiet">{{ t('From the official client\'s own status-line snapshot.') }}</p></template></section>
          <label class="account-quota-toggle"><input type="checkbox" :checked="!preferences.hidden_account_quotas?.includes(selected.id)" :disabled="!!busy" @change="toggleQuotaBadge" />{{ t('Show remaining quota in endpoint picker') }}</label>
          <div v-if="selected.error" class="account-error" role="alert">{{ selected.error }}</div>
          <section class="account-section"><div class="account-section-title"><h4>{{ t('Sign-in and connection') }}</h4><button :disabled="!!busy" @click="action('refresh')"><Icon name="refresh" :size="14" />{{ t('Refresh official status') }}</button></div><p v-if="selected.checked_at" class="account-checked">{{ t('Last checked') }} · {{ resetTime(selected.checked_at) }}</p>
            <div v-if="selected.status==='login_pending'||selected.login" class="account-login"><strong>{{ t('Finish sign-in with the official provider') }}</strong><p>{{ t('Open the official link yourself. The browser may be on another machine; this does not send prompts or grant tool permissions.') }}</p><div v-if="selected.login?.user_code" class="account-device-code"><small>{{ t('One-time device code') }}</small><code>{{ selected.login.user_code }}</code></div><a v-if="loginUrl" class="account-login-link" :href="loginUrl" target="_blank" rel="noopener noreferrer">{{ t('Open official sign-in') }} ↗</a><p v-else-if="selected.login?.url">{{ t('The sign-in URL is not a recognized official HTTPS address. Use the native instructions instead.') }}</p><button :disabled="!!busy" @click="action('cancel-login')">{{ t('Cancel sign-in') }}</button></div>
            <div class="account-buttons"><template v-if="signInActions?.signIn"><button class="account-primary" :disabled="!!busy" @click="action('device')">{{ t('Sign in with device code') }}</button><button :disabled="!!busy" @click="action('browser')">{{ t('Browser sign-in') }}</button></template><button v-if="signInActions?.refreshToken" :disabled="!!busy" @click="action('refresh-token')">{{ t('Refresh login token') }}</button><button v-if="signInActions?.signOut" class="account-danger" :disabled="!!busy" @click="confirmAction='logout';resetConfirmed=false">{{ t('Sign out…') }}</button><button class="account-danger" :disabled="!!busy" @click="confirmAction='remove';resetConfirmed=false">{{ t('Remove account…') }}</button></div>
          </section>
          <details class="account-more"><summary>{{ t('Directory and proxy') }}</summary><div class="account-meta"><span :title="t('Native account storage')"><code>{{ selected.storage_path }}</code></span></div><p v-if="selected.shared_configuration" class="account-shared" role="note">{{ t('Shared configuration: this account signs in through a directory the host already had. Anything else that edits it — a profile switcher, a native re-login — changes this account too.') }}</p><div class="account-proxy-row"><label>{{ t('Proxy') }}<input v-model="proxyDraft" :disabled="!!busy" maxlength="400" placeholder="http://127.0.0.1:7890" @keydown.enter.prevent="saveProxy" /></label><button class="small-button" :disabled="!!busy||proxyDraft.trim()===(selected.proxy_url??'')" @click="saveProxy">{{ t(busy==='proxy'?'Saving…':'Save') }}</button><small>{{ t('Used for sign-in, usage checks and sessions from this account. Empty means direct.') }}</small></div></details>
          <section v-if="confirmAction==='logout'" class="account-confirm"><strong>{{ t('Sign out of this account?') }}</strong><p>{{ t('The official client will remove this account sign-in. Sessions using it may need to authenticate again; other account configurations are unchanged.') }}</p><div class="account-buttons"><button :disabled="!!busy" @click="confirmAction=undefined">{{ t('Keep signed in') }}</button><button class="account-danger" :disabled="!!busy" @click="action('logout')">{{ t('Confirm sign out') }}</button></div></section>
          <section v-if="confirmAction==='reset'" class="account-confirm"><strong>{{ t(resetAttempt?'Review the previous reset request':'Use an official quota reset credit?') }}</strong><p>{{ t(resetAttempt?'The previous result is uncertain. A retry will use its existing request key, not create a second credit request.':'This request can consume an official reset credit with real resource cost. It cannot be undone by refreshing this page.') }}</p><label><input v-model="resetConfirmed" type="checkbox" :disabled="!!busy" />{{ t('I confirm the possible credit cost and want to submit this reset request.') }}</label><div class="account-buttons"><button :disabled="!!busy" @click="confirmAction=undefined">{{ t('Cancel') }}</button><button class="account-primary" :disabled="!!busy||!resetConfirmed" @click="resetQuota">{{ t(resetAttempt?'Retry the same reset request':'Confirm quota reset') }}</button></div></section>
          <section v-if="confirmAction==='remove'" class="account-confirm"><strong>{{ t('Remove this account?') }}</strong><p>{{ selected.native_source_id ? t('Only the Tendly reference is removed. The host configuration directory, its sign-in and its history are left untouched.') : t('This account has its own configuration directory, and removing it deletes the sign-in stored there. Existing sessions keep their saved configuration snapshot.') }}</p><div class="account-buttons"><button type="button" :disabled="!!busy" @click="confirmAction=undefined">{{ t('Cancel') }}</button><button type="button" class="account-danger" :disabled="!!busy" @click="removeAccount">{{ t(busy==='remove'?'Removing…':'Confirm removal') }}</button></div></section>
        </template><p v-else class="accounts-empty">{{ t('Select an account to inspect its official sign-in and usage state.') }}</p></main>
      </div>
    </template>
  </ModalDialog>
</template>

<style scoped>
.account-quota-toggle{display:flex;align-items:center;gap:8px;min-height:44px;font-size:var(--text-sm);color:var(--ink-soft);cursor:pointer}
.account-quota-toggle input{flex:none;margin:0;accent-color:var(--teal)}
.accounts-intro{display:flex;align-items:flex-start;gap:13px;margin-bottom:20px;color:var(--ink)}.accounts-intro>span{display:grid;place-items:center;width:43px;height:43px;border-radius:var(--radius-lg);background:var(--teal-soft);flex-shrink:0}.accounts-intro strong{font-size:var(--text-lg);font-weight:550}.accounts-intro p{font-size:var(--text-sm);color:var(--muted);line-height:1.8;margin:6px 0 0}.accounts-layout{display:grid;grid-template-columns:220px minmax(0,1fr);gap:22px;min-height:420px}.accounts-list{border-right:1px solid var(--border);padding-right:17px}.accounts-list-heading{display:flex;align-items:center;justify-content:space-between;min-height:44px;font-size:var(--text-xs);color:var(--ink-soft);letter-spacing:.5px}.accounts-list-heading small{background:var(--fill-hover);border-radius:var(--radius-sm);padding:2px 5px;margin-left:5px}.accounts-list-heading button{width:44px;height:44px;display:grid;place-items:center;border:0;background:none;color:var(--muted)}.account-choice{width:100%;display:flex;align-items:center;gap:9px;text-align:left;padding:12px 9px;margin:5px 0;border:1px solid transparent;border-radius:var(--radius-md);background:none;cursor:pointer;min-height:64px}.account-choice.selected{border-color:var(--accent-line);background:var(--teal-soft)}.account-provider{width:35px;height:35px;border-radius:var(--radius-md);display:grid;place-items:center;background:var(--client-soft);color:var(--client-ink);flex-shrink:0}.account-provider.large{width:49px;height:49px;border-radius:var(--radius-lg)}.account-choice-text{flex:1;min-width:0}.account-choice strong{display:block;font-size:var(--text-sm);font-weight:550;color:var(--ink)}.account-choice small{display:block;margin-top:5px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:var(--text-xs);color:var(--muted)}.account-dot{width:6px;height:6px;background:var(--faint);border-radius:50%;flex-shrink:0}.account-dot.signed_in{background:var(--focus)}.account-dot.login_pending{background:#c9a351}.account-dot.error{background:var(--danger-ink)}.account-add{width:100%;display:flex;align-items:center;justify-content:center;gap:8px;min-height:44px;margin-top:12px;border:1px dashed var(--line);border-radius:var(--radius-md);background:var(--sunken);color:var(--teal);font-size:var(--text-sm);cursor:pointer}.account-detail{min-width:0}.account-detail-header{display:flex;align-items:center;gap:12px;flex-wrap:wrap;margin:8px 0 18px}.account-detail-header>div{flex:1;min-width:0}.account-detail-header h3{font-weight:550;font-size:var(--text-xl);overflow-wrap:anywhere;margin:0}.account-detail-header p{font-size:var(--text-sm);color:var(--muted);overflow-wrap:anywhere;margin:6px 0 0}.account-state{border-radius:var(--radius-xl);padding:6px 10px;font-size:var(--text-xs);background:var(--fill);color:var(--ink-soft)}.account-state.signed_in{background:var(--ok-soft);color:var(--accent-ink)}.account-state.error{background:var(--danger-soft);color:var(--danger-ink)}.account-state.login_pending{background:var(--warn-soft);color:#9c844c}.account-meta{border:1px solid var(--border);background:var(--sunken);border-radius:var(--radius-md);padding:12px;margin-bottom:18px;font-size:var(--text-xs);color:var(--ink-soft)}.account-meta span{display:flex;gap:10px;align-items:center;flex-wrap:wrap}.account-meta code{font-size:var(--text-xs);overflow-wrap:anywhere;color:var(--muted)}.account-meta details summary{min-height:44px;display:flex;align-items:center;cursor:pointer}.account-meta p{font-size:var(--text-xs);line-height:1.8}.account-rename{display:flex;align-items:center;gap:8px;flex-wrap:wrap}.account-rename input{flex:1;min-width:150px;min-height:36px;padding:7px 9px;border:1px solid var(--line);border-radius:var(--radius-md);font-size:var(--text-lg);color:var(--ink);background:var(--surface)}.account-rename-button{border:0;background:none;color:var(--muted);padding:4px;cursor:pointer;vertical-align:middle}.account-rename-button:hover{color:var(--teal)}.account-proxy-row{display:flex;align-items:center;gap:9px;flex-wrap:wrap;margin-bottom:18px}.account-proxy-row label{display:flex;align-items:center;gap:8px;flex:1;min-width:210px;font-size:var(--text-xs);color:var(--ink-soft)}.account-proxy-row input{flex:1;min-width:0;min-height:36px;padding:7px 9px;border:1px solid var(--line);border-radius:var(--radius-md);background:var(--surface);color:var(--ink);font-size:var(--text-sm)}.account-proxy-row small{flex-basis:100%;font-size:var(--text-xs);line-height:1.7;color:var(--muted)}.account-section{border-top:1px solid var(--border);padding-top:17px;margin-top:18px}.account-section-title{display:flex;align-items:center;justify-content:space-between;gap:8px}.account-section-title h4{font-size:var(--text-sm);font-weight:600;color:var(--ink);margin:0}.account-section-title button{border:0;background:none;color:var(--teal);font-size:var(--text-xs);display:flex;align-items:center;gap:6px;min-height:44px;padding:8px 3px;cursor:pointer}.account-checked,.account-shared{margin:6px 0 0;padding:7px 9px;border-radius:var(--radius-sm);background:#fffaee;border:1px solid var(--warn-line);color:#86713e;font-size:var(--text-xs);line-height:1.7}
.account-quiet{font-size:var(--text-xs);line-height:1.8;color:var(--muted);margin:6px 0 12px}.account-source-label{font-size:var(--text-xs);color:var(--muted)}.account-buttons{display:flex;align-items:center;flex-wrap:wrap;gap:9px;margin-top:15px}.account-buttons button,.account-reset button,.account-login>button{min-height:44px;border:1px solid var(--line);background:var(--surface);border-radius:var(--radius-md);padding:8px 12px;color:var(--ink-soft);font-size:var(--text-xs);cursor:pointer}.account-buttons .account-primary{background:var(--teal);border-color:var(--teal);color:var(--on-accent)}.account-buttons .account-danger{border-color:var(--danger-line);color:var(--danger);background:var(--danger-soft)}.account-login,.account-native-help{border:1px solid var(--accent-line);border-radius:var(--radius-lg);padding:14px;background:var(--sunken);margin:12px 0;font-size:var(--text-sm);color:var(--teal)}.account-login p,.account-native-help p{font-size:var(--text-xs);line-height:1.8;color:var(--ink-soft)}.account-native-help code{display:block;background:var(--fill);padding:12px;border-radius:var(--radius-sm);font-size:var(--text-xs);line-height:1.8;color:var(--ink-soft);overflow-wrap:anywhere;white-space:pre-wrap;user-select:all}.account-login-link{display:inline-flex;align-items:center;min-height:44px;margin:8px 10px 5px 0;padding:8px 14px;border-radius:var(--radius-md);background:var(--teal);color:var(--on-accent);font-size:var(--text-sm);text-decoration:none}.account-device-code{display:flex;align-items:center;gap:12px;flex-wrap:wrap}.account-device-code small{font-size:var(--text-xs);color:var(--muted)}.account-device-code code{font:var(--text-2xl) var(--mono);letter-spacing:3px;user-select:all;padding:9px 13px;background:var(--surface);border:1px dashed var(--line);border-radius:var(--radius-md);color:var(--teal)}.account-limits{display:grid;grid-template-columns:1fr 1fr;gap:13px;margin:15px 0}.account-limit{border:1px solid var(--border);border-radius:var(--radius-md);background:var(--sunken);padding:13px;min-width:0}.account-limit header{display:flex;justify-content:space-between;flex-wrap:wrap;gap:8px;font-size:var(--text-xs);color:var(--ink-soft);margin-bottom:11px}.account-limit strong{font-weight:500;color:var(--teal)}.account-limit small{font-size:var(--text-xs);color:var(--muted)}.account-limit progress{width:100%;height:6px;display:block;appearance:none;border:0;border-radius:var(--radius-xs);margin:10px 0;background:var(--fill-hover);overflow:hidden;accent-color:var(--teal)}.account-limit progress::-webkit-progress-bar{background:var(--fill-hover);border-radius:var(--radius-xs)}.account-limit progress::-webkit-progress-value{background:var(--teal);border-radius:var(--radius-xs)}.account-limit progress::-moz-progress-bar{background:var(--teal);border-radius:var(--radius-xs)}.account-no-meter{height:6px;border-radius:var(--radius-xs);background:repeating-linear-gradient(120deg,var(--fill-hover),var(--fill-hover) 7px,var(--sunken) 7px,var(--sunken) 14px);margin:10px 0}.account-reset{display:flex;justify-content:space-between;align-items:center;gap:10px;font-size:var(--text-xs);color:var(--ink-soft);margin-top:12px}.account-reset strong{font-weight:500}.account-reset p{font-size:var(--text-xs);color:var(--muted);margin:6px 0}.account-reset button{color:var(--teal);border-color:var(--line)}.account-confirm{margin-top:17px;border:1px solid var(--warn-line);background:#fffaee;border-radius:var(--radius-lg);padding:16px;font-size:var(--text-sm);color:#86713e}.account-confirm p{font-size:var(--text-xs);line-height:1.8}.account-confirm label{display:flex;align-items:flex-start;gap:9px;min-height:44px;font-size:var(--text-xs);line-height:1.8;padding-top:8px}.account-confirm input{accent-color:var(--teal);width:18px;height:18px;flex-shrink:0;margin-top:2px}.account-create h3{font-size:var(--text-xl);font-weight:550;color:var(--ink)}.account-create p{font-size:var(--text-sm);color:var(--ink-soft);line-height:1.8}.account-create label{display:flex;flex-direction:column;gap:8px;font-size:var(--text-sm);color:var(--ink-soft);margin:17px 0}.account-create input,.account-create select{min-height:44px;font-size:var(--input-text);width:100%;padding:10px;border:1px solid var(--line);border-radius:var(--radius-md);background:var(--surface);color:var(--ink)}.account-error{padding:12px 14px;background:var(--danger-soft);border:1px solid var(--danger-line);color:var(--danger-ink);border-radius:var(--radius-md);font-size:var(--text-sm);line-height:1.8;overflow-wrap:anywhere;margin:10px 0}.account-notice{padding:12px 14px;background:var(--teal-soft);border:1px solid var(--accent-line);color:var(--accent-ink);border-radius:var(--radius-md);font-size:var(--text-sm);line-height:1.8}.accounts-empty{font-size:var(--text-xs);color:var(--muted);text-align:center;line-height:1.8;padding:15px 5px}.accounts-unavailable{text-align:center;padding:38px 20px;color:var(--muted)}.accounts-unavailable>svg{margin:auto}.accounts-unavailable h3{font-size:var(--text-xl);font-weight:550;margin:17px 0 10px}.accounts-unavailable p{font-size:var(--text-sm);line-height:1.9;color:var(--ink-soft);max-width:440px;margin:auto}.accounts-layout button:disabled{opacity:.45;cursor:not-allowed}.accounts-layout button:focus-visible,.accounts-layout a:focus-visible,.accounts-layout summary:focus-visible,.accounts-layout input:focus-visible,.accounts-layout select:focus-visible{outline:2px solid var(--focus);outline-offset:3px}@media(max-width:680px){.accounts-layout{grid-template-columns:minmax(0,1fr);gap:15px}.accounts-list{border-right:0;border-bottom:1px solid var(--border);padding-right:0;padding-bottom:13px;display:flex;gap:8px;overflow-x:auto;align-items:center}.accounts-list-heading{flex-shrink:0}.accounts-list-heading>span{display:none}.account-choice{width:190px;min-width:170px;margin:0;min-height:60px}.account-add{min-width:125px;width:125px;margin:0;padding:8px}.accounts-intro strong{font-size:var(--text-base)}.account-detail-header{gap:10px}.account-state{font-size:var(--text-xs)}.account-limits{gap:9px}.account-limit{padding:10px}.account-section-title button,.account-buttons button{font-size:var(--text-sm)}.account-device-code code{font-size:var(--text-2xl)}.account-confirm label{font-size:var(--text-sm)}.account-native-help code{font-size:var(--text-sm)}.account-meta code{font-size:var(--text-xs)}.account-create label{font-size:var(--text-base)}}
.accounts-intro p,.account-choice small,.account-detail-header p,.account-meta,.account-meta code,.account-checked,.account-quiet,.account-source-label,.account-native-help p,.account-login p,.account-limit small,.account-limit header,.account-reset p,.account-create p,.accounts-empty,.accounts-unavailable p{color:var(--ink-soft)}.account-detail-header h3,.account-create h3,.account-section-title h4,.account-choice strong,.account-reset strong,.account-create label,.accounts-unavailable h3{color:var(--ink)}.account-buttons button,.account-reset button,.account-section-title button,.account-add{color:var(--teal)}.account-buttons .account-primary{color:var(--on-accent);background:var(--teal);border-color:var(--teal)}.account-login-link{background:var(--teal)}.account-notice{color:var(--teal)}.account-error{color:var(--danger-ink)}.account-confirm{color:#86713e}
.account-create .shared-config-consent>label{display:flex;flex-direction:row;align-items:flex-start;gap:8px;line-height:17px}
.account-create .shared-config-consent input[type="checkbox"]{width:14px;height:14px;min-width:14px;min-height:14px;padding:0;margin:2px 0 0;flex:0 0 14px;accent-color:var(--teal)}
.account-limit header>div{display:flex;flex-direction:column;gap:4px;min-width:0}

.accounts-lead{margin:2px 0 16px;font-size:var(--text-sm);line-height:1.7;color:var(--ink-soft)}
.accounts-first{display:flex;flex-direction:column;align-items:center;text-align:center;gap:10px;padding:44px 20px;border:1px dashed var(--line);border-radius:var(--radius-lg);background:var(--sunken)}
.accounts-first-marks{display:flex;gap:8px}
.accounts-first h3{margin:6px 0 0;font-size:var(--text-lg);font-weight:600;color:var(--ink)}
.accounts-first p{margin:0;max-width:380px;font-size:var(--text-sm);line-height:1.7;color:var(--ink-soft)}
.accounts-first>div{display:flex;flex-wrap:wrap;justify-content:center;gap:10px;margin-top:10px}
.accounts-first .account-add{width:auto;min-width:160px;margin:0}
.accounts-layout{min-height:0}
.account-choice strong{font-size:var(--text-md)}
.account-choice small{font-size:var(--text-xs)}
.account-quiet,.account-checked{font-size:var(--text-xs)}

/* Quota first: it is what this page is opened for. */
.account-detail-header+.account-section{border-top:0;padding-top:0;margin-top:2px}
.account-limits{grid-template-columns:repeat(auto-fit,minmax(210px,1fr));gap:12px;margin:12px 0 8px}
.account-section-title h4{font-size:var(--text-md)}
.account-section-title button{font-size:var(--text-sm);font-weight:550}
.account-checked{background:none;border:0;padding:0;margin:4px 0 0;font-size:var(--text-xs);color:var(--muted)}
.account-mini{display:block;margin-top:4px;font-size:var(--text-xs);color:var(--muted);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.account-mini>span+span::before{content:"·";margin:0 5px;color:var(--line-strong)}
.account-mini b{font-weight:600;color:var(--teal);font-variant-numeric:tabular-nums}
.account-mini .warn b{color:var(--warn)}.account-mini .danger b{color:var(--danger)}
.account-more{margin-top:18px;border-top:1px solid var(--border);padding-top:6px}
.account-more>summary{display:flex;align-items:center;min-height:40px;font-size:var(--text-sm);font-weight:600;color:var(--ink-soft);cursor:pointer;list-style:none}
.account-more>summary::before{content:"›";display:inline-block;width:14px;transition:transform .15s ease;color:var(--muted)}
.account-more[open]>summary::before{transform:rotate(90deg)}
.account-more>summary::-webkit-details-marker{display:none}
.account-more .account-meta{margin:6px 0 12px}
.account-more .account-shared{margin:0 0 12px}

.account-detail-header p{display:flex;align-items:center;flex-wrap:wrap;gap:6px}
.account-plan{display:inline-block;padding:2px 8px;border-radius:var(--radius-xl);background:var(--violet-soft);color:var(--violet);font-size:var(--text-xs);font-weight:600;text-transform:capitalize;white-space:nowrap}
.account-section-title button{border:1px solid var(--teal-line);border-radius:var(--radius-md);min-height:32px;padding:5px 10px;background:var(--surface)}
.account-section-title button:hover:not(:disabled){background:var(--teal-soft)}
@media(pointer:coarse){.account-section-title button{min-height:44px}}

.account-reset{justify-content:flex-start;gap:14px;padding:12px 14px;border:1px solid var(--border);border-radius:var(--radius-lg);background:var(--sunken);margin-top:0}
.account-reset>div{flex:1;min-width:0}
.account-reset strong{display:block;font-size:var(--text-md);font-weight:600;color:var(--ink);white-space:nowrap}
.account-reset p{margin:3px 0 0;font-size:var(--text-xs)}
.account-reset-count{display:grid;place-items:center;flex-shrink:0;width:40px;height:40px;border-radius:var(--radius-lg);background:var(--violet-soft);color:var(--violet);font-size:var(--text-xl);font-weight:650;font-variant-numeric:tabular-nums}
.account-reset{flex-wrap:wrap}
.account-credit-soon{color:var(--warn-ink)}
.account-credit-list{flex-basis:100%;border-top:1px solid var(--border);padding-top:6px}
.account-credit-list>summary{display:flex;align-items:center;gap:8px;min-height:32px;font-size:var(--text-xs);font-weight:550;color:var(--ink-soft);cursor:pointer;list-style:none}
.account-credit-list>summary::before{content:"›";display:inline-block;width:10px;transition:transform .15s ease;color:var(--muted)}
.account-credit-list[open]>summary::before{transform:rotate(90deg)}
.account-credit-list>summary::-webkit-details-marker{display:none}
.account-credit-list>summary small{font-weight:400;color:var(--muted)}
.account-credit-list ol{list-style:none;margin:4px 0 2px;padding:0;display:flex;flex-direction:column;gap:6px}
.account-credit-list li{display:flex;align-items:center;justify-content:space-between;gap:12px;padding:9px 12px;border:1px solid var(--border);border-radius:var(--radius-md);background:var(--surface)}
.account-credit-list li strong{font-size:var(--text-xs);font-weight:550;color:var(--ink);white-space:normal}
.account-credit-list li small{display:block;margin-top:2px;font-size:var(--text-xs);color:var(--muted)}
.account-credit-expiry{display:flex;align-items:center;gap:8px;flex-shrink:0;font-size:var(--text-xs);color:var(--ink-soft);font-variant-numeric:tabular-nums}
.account-credit-expiry em{font-style:normal;padding:2px 7px;border-radius:var(--radius-xl);background:var(--fill);color:var(--muted)}
.account-credit-expiry em.soon{background:var(--warn-soft);color:var(--warn-ink)}
.account-reset-count.empty{background:var(--fill);color:var(--muted)}
.account-reset button{min-height:34px;border-radius:var(--radius-md)}
.account-reset+.account-quiet{margin:6px 2px 0;font-size:var(--text-xs)}
</style>
