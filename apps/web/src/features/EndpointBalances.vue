<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue';
import { balanceAutoRefreshOn, balanceIsStale, balanceLabel, balanceLevel, balanceShort, type EndpointBalance } from './balance-model';
import { preferences } from './preferences';
import { balances, loadBalances, refreshBalance } from './balance-store';
import Icon from './Icon.vue';
import { useI18n } from '../i18n';

/** Custom endpoint balances at the top of the Usage page, drawn like the official allowance cards beside them. */
const { t, locale } = useI18n();
const now = ref(Date.now());
let timer: ReturnType<typeof setInterval> | undefined;
onMounted(() => { void loadBalances(); timer = setInterval(() => { now.value = Date.now(); }, 30_000); });
onBeforeUnmount(() => clearInterval(timer));
defineExpose({ load: loadBalances });

const percent = (row: EndpointBalance) => row.reading?.unit.kind === 'percentage' ? Math.max(0, Math.min(100, Number(row.reading.value))) : undefined;
const stale = (row: EndpointBalance) => balanceIsStale(row.reading, row.stale || !!row.error, now.value);
function checked(row: EndpointBalance): string {
  if (!row.reading) return t('No balance reading yet');
  const minutes = Math.round((now.value - Date.parse(row.reading.checked_at)) / 60_000);
  return minutes < 1 ? t('Checked just now') : minutes < 60 ? t('Checked {count} min ago', { count: minutes }) : t('Checked {time}', { time: new Date(row.reading.checked_at).toLocaleString(locale.value) });
}
</script>

<template>
  <section v-if="balances.rows.length || balances.error" class="balance-row" :aria-label="t('Custom endpoint balances')">
    <p v-if="balances.error" class="inline-error" role="alert">{{ balances.error }}</p>
    <article v-for="row in balances.rows" :key="row.id" :class="['balance-card', row.reading ? balanceLevel(row.reading) : 'ok', { stale: stale(row) }]">
      <header>
        <Icon :name="row.reading?.unit.kind === 'percentage' ? 'gauge' : 'wallet'" :size="15" />
        <strong :title="row.name">{{ row.name }}</strong>
        <span class="balance-kind">{{ t(row.reading?.unit.kind === 'percentage' ? 'Remaining allowance' : 'Balance') }}</span>
        <button type="button" :class="['icon-button', 'balance-refresh', { busy: balances.busy.includes(row.id) }]" :disabled="balances.busy.includes(row.id)" :aria-label="t('Refresh balance for {name}', { name: row.name })" :title="t('Refresh balance for {name}', { name: row.name })" @click="refreshBalance(row.id)"><Icon name="refresh" :size="13" /></button>
      </header>
      <div class="balance-figure" :title="row.reading ? balanceLabel(row.reading) : ''">{{ row.reading ? balanceShort(row.reading, locale) : '—' }}</div>
      <span v-if="percent(row) !== undefined" class="balance-bar"><i :style="{ width: percent(row) + '%' }" /></span>
      <small class="balance-meta">{{ checked(row) }}<template v-if="balanceAutoRefreshOn(row, preferences)"> · {{ t('Refreshes automatically') }}</template><template v-if="row.error"> · <span class="danger-text">{{ t(row.error) }}</span></template><template v-else-if="stale(row)"> · {{ t('Outdated') }}</template></small>
    </article>
  </section>
</template>

<style scoped>
.balance-row{display:grid;grid-template-columns:repeat(auto-fit,minmax(220px,1fr));gap:var(--space-3);margin-bottom:var(--space-3)}
.balance-row>.inline-error{grid-column:1 / -1;margin:0}
.balance-card{--balance:var(--teal);display:flex;flex-direction:column;gap:6px;padding:12px 14px;border-radius:var(--radius-lg);background:var(--surface);box-shadow:var(--shadow-card);border-top:3px solid var(--balance)}
.balance-card.warn{--balance:var(--warn)}.balance-card.danger{--balance:var(--danger)}
.balance-card.stale:not(.warn,.danger){--balance:var(--line-strong)}
.balance-card header{display:flex;align-items:center;gap:8px;font-size:var(--text-sm)}
.balance-card header .app-icon{--icon-color:var(--balance)}
.balance-card header strong{font-weight:600;color:var(--ink);min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.balance-kind{font-size:var(--text-xs);color:var(--muted);white-space:nowrap}
.balance-refresh{margin-left:auto;color:var(--muted)}
.balance-refresh.busy svg{animation:balance-spin 1s linear infinite}@keyframes balance-spin{to{transform:rotate(360deg)}}
.balance-figure{font:650 var(--text-2xl)/1.15 var(--mono);font-variant-numeric:tabular-nums;color:var(--ink);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.balance-card.warn .balance-figure{color:var(--warn-ink)}.balance-card.danger .balance-figure{color:var(--danger-ink)}.balance-card.stale:not(.warn,.danger) .balance-figure{color:var(--ink-soft)}
.balance-bar{height:6px;border-radius:var(--radius-full);background:var(--fill);overflow:hidden}
.balance-bar i{display:block;height:100%;border-radius:inherit;background:var(--balance);transition:width .6s ease}
.balance-meta{font-size:var(--text-xs);color:var(--muted)}
@media(prefers-reduced-motion:reduce){.balance-bar i{transition:none}.balance-refresh.busy svg{animation:none}}
@media(pointer:coarse){.balance-refresh{min-width:40px;height:40px}}
</style>
