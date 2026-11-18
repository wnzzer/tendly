<script setup lang="ts">
import { computed, onMounted, onBeforeUnmount, ref } from "vue";
import type { BalanceReading } from "./balance-model";
import { balanceLabel, balanceIsStale } from "./balance-model";
import { useI18n } from "../i18n";
const props = defineProps<{ reading?: BalanceReading | null; stale?: boolean; error?: string | null }>();
const { t } = useI18n();
const now = ref(Date.now());
let timer: ReturnType<typeof setInterval> | undefined;
onMounted(() => {
  timer = setInterval(() => {
    now.value = Date.now();
  }, 15_000);
});
onBeforeUnmount(() => clearInterval(timer));
const old = computed(() => balanceIsStale(props.reading ?? null, props.stale || !!props.error, now.value));
const percentage = computed(() =>
  props.reading?.unit.kind === "percentage" ? Math.max(0, Math.min(100, Number(props.reading.value))) : undefined,
);
</script>
<template>
  <div class="balance-result" :class="{ stale: old }" aria-live="polite">
    <div v-if="reading" class="balance-value-row">
      <svg v-if="percentage !== undefined" class="balance-ring" viewBox="0 0 40 40" aria-hidden="true">
        <circle cx="20" cy="20" r="16" />
        <circle cx="20" cy="20" r="16" pathLength="100" :stroke-dasharray="`${percentage} 100`" />
      </svg>
      <div>
        <small
          >{{ t(reading.unit.kind === "money" ? "Balance" : "Remaining allowance")
          }}<span v-if="old" class="balance-stale">{{ t("Outdated") }}</span></small
        ><strong>{{ balanceLabel(reading) }}</strong>
      </div>
    </div>
    <p v-else class="balance-empty">{{ t("No balance reading yet") }}</p>
    <small v-if="reading" class="balance-time"
      >{{ t("Last successful query") }} ·
      <time :datetime="reading.checked_at">{{ new Date(reading.checked_at).toLocaleString() }}</time></small
    >
    <p v-if="error" class="inline-error" role="alert">{{ t(error) }}</p>
  </div>
</template>
<style scoped>
.balance-value-row {
  display: flex;
  align-items: center;
  gap: 12px;
}
.balance-value-row > div {
  display: grid;
  gap: 4px;
  min-width: 0;
}
.balance-value-row small {
  display: flex;
  align-items: center;
  gap: 6px;
}
.balance-value-row small,
.balance-time,
.balance-empty {
  color: var(--muted);
  font-size: var(--text-xs);
}
.balance-value-row strong {
  font-size: var(--text-xl);
  font-variant-numeric: tabular-nums;
  overflow-wrap: anywhere;
}
.balance-time {
  display: block;
  margin-top: 6px;
  font-variant-numeric: tabular-nums;
}
.balance-stale {
  padding: 1px 7px;
  border-radius: var(--radius-full);
  background: var(--warn-soft);
  color: var(--warn-ink);
  font-size: var(--text-xs);
  line-height: 16px;
}
.balance-ring {
  width: 44px;
  height: 44px;
  flex-shrink: 0;
  transform: rotate(-90deg);
}
.balance-ring circle {
  fill: none;
  stroke: var(--border);
  stroke-width: 4;
}
.balance-ring circle + circle {
  stroke: var(--accent);
  stroke-linecap: round;
}
.stale .balance-ring circle + circle {
  stroke: var(--muted);
}
.inline-error {
  margin-bottom: 0;
}
</style>
