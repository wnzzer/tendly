<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { accountQuota, accountResetDate, quotaShortTitle, quotaWindowTitle } from './account-ui';
import { balanceIsStale, balanceLabel, balanceLevel, balanceShort, type BalanceReading } from './balance-model';
import { accountFor, balanceFor, balances } from './balance-store';
import { preferences } from './preferences';
import { useI18n } from '../i18n';

/**
 * What is left on an endpoint, as a short figure coloured as it runs low: a
 * custom balance, or an official account's allowance (its tightest window).
 * In the message box's endpoint chip, where it is being spent, and beside
 * each endpoint in its menu. Nothing renders without a reading.
 */
const props = defineProps<{ profileId?: string | null; compact?: boolean }>();
const { t, locale } = useI18n();
const now = ref(Date.now());
let timer: ReturnType<typeof setInterval> | undefined;
onMounted(() => { timer = setInterval(() => { now.value = Date.now(); }, 30_000); });
onBeforeUnmount(() => clearInterval(timer));

const row = computed(() => balanceFor(props.profileId));
const quota = computed(() => { const account = row.value ? undefined : accountFor(props.profileId); return account && !preferences.value.hidden_account_quotas?.includes(account.id) ? accountQuota(account, now.value) : undefined; });
/** An allowance reads like a remaining-percentage balance, so both share one look and the same thresholds. */
const reading = computed<BalanceReading | null>(() => row.value?.reading
  ?? (quota.value ? { value: String(quota.value.remaining), unit: { kind: 'percentage' }, checked_at: '' } : null));
const busy = computed(() => !!row.value && balances.busy.includes(row.value.id));
const stale = computed(() => !!row.value && balanceIsStale(reading.value, row.value.stale || !!row.value.error, now.value));
const windowName = computed(() => { if (!quota.value) return ''; const name = quotaShortTitle(quota.value.slot, quota.value.limit); return t(name.key, name.values); });
const title = computed(() => {
  if (quota.value) {
    const name = quotaWindowTitle(quota.value.slot, quota.value.limit), resets = accountResetDate(quota.value.limit.resets_at);
    return [`${t(name.key, name.values)} · ${t('{percent}% left', { percent: quota.value.remaining })}`, ...(resets ? [t('Resets {time}', { time: resets.toLocaleString(locale.value) })] : [])].join('\n');
  }
  if (!row.value || !reading.value) return '';
  const lines = [`${t('Balance')} · ${balanceLabel(reading.value)}`, t('Checked {time}', { time: new Date(reading.value.checked_at).toLocaleString(locale.value) })];
  if (row.value.error) lines.push(t(row.value.error)); else if (stale.value) lines.push(t('Outdated'));
  return lines.join('\n');
});
</script>

<template>
  <span v-if="reading" :class="['balance-badge', balanceLevel(reading), { stale, busy }]" :title="title">{{ windowName ? `${windowName} ${balanceShort(reading, locale, compact)}` : balanceShort(reading, locale, compact) }}</span>
</template>

<style scoped>
.balance-badge{font:600 var(--text-xs)/1 var(--mono);font-variant-numeric:tabular-nums;color:var(--accent-ink);white-space:nowrap}
.balance-badge.warn{color:var(--warn-ink)}.balance-badge.danger{color:var(--danger-ink)}.balance-badge.stale:not(.warn,.danger){color:var(--muted)}
.balance-badge.busy{animation:balance-pulse 1s ease-in-out infinite}
@keyframes balance-pulse{50%{opacity:.35}}
@media(prefers-reduced-motion:reduce){.balance-badge.busy{animation:none}}
</style>
