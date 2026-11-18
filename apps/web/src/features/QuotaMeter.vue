<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import type { AccountLimit } from "./account-ui";
import {
  accountLimitExpired,
  accountLimitPercent,
  accountResetDate,
  quotaLevel,
  quotaWindowTitle,
  untilLabel,
} from "./account-ui";
import { useI18n } from "../i18n";

/**
 * One official quota window, drawn as a ring of what is left.
 *
 * Everything here reads the same way: the ring is the remaining quota, full
 * when fresh and shrinking as it is spent, like a battery; the number inside
 * it is that remainder, and the account list quotes the same figure. The line
 * beside it says when it comes back — "resets in 2h 18m" rather than a
 * timestamp to subtract in one's head — and the colour turns as it runs low.
 */
const props = defineProps<{ position: "primary" | "secondary"; limit?: AccountLimit }>();
const { t, locale } = useI18n();
const now = ref(Date.now());
let timer: ReturnType<typeof setInterval> | undefined;
onMounted(() => {
  timer = setInterval(() => {
    now.value = Date.now();
  }, 30_000);
});
onBeforeUnmount(() => {
  if (timer) clearInterval(timer);
});

const expired = computed(() => accountLimitExpired(props.limit, now.value));
const percent = computed(() => accountLimitPercent(props.limit, now.value));
const level = computed(() => quotaLevel(percent.value));
const title = computed(() => {
  const name = quotaWindowTitle(props.position, props.limit);
  return t(name.key, name.values);
});
const resetsAt = computed(() => accountResetDate(props.limit?.resets_at));
const until = computed(() => (resetsAt.value ? untilLabel(resetsAt.value.getTime() - now.value) : undefined));
const resetLine = computed(() =>
  expired.value
    ? t("Reset · starts again with your next message")
    : !until.value
      ? t("Reset time unavailable")
      : until.value.soon
        ? t("Resetting now")
        : t("Resets in {time}", { time: t(until.value.key, until.value.values) }),
);
const resetExact = computed(() =>
  resetsAt.value && !expired.value
    ? new Intl.DateTimeFormat(locale.value, {
        month: "short",
        day: "numeric",
        weekday: "short",
        hour: "2-digit",
        minute: "2-digit",
      }).format(resetsAt.value)
    : "",
);
// r = 26 on a 64 box: circumference ≈ 163.4.
const CIRCUMFERENCE = 2 * Math.PI * 26;
const left = computed(() => (percent.value === undefined ? undefined : Math.max(0, Math.round(100 - percent.value))));
const dash = computed(() =>
  left.value === undefined || left.value === 0 ? 0 : CIRCUMFERENCE * Math.max(0.015, left.value / 100),
);
</script>

<template>
  <div :class="['quota-meter', level]" role="group" :aria-label="title">
    <svg class="quota-ring" viewBox="0 0 64 64" aria-hidden="true">
      <circle class="quota-track" cx="32" cy="32" r="26" />
      <circle
        v-if="dash > 0"
        class="quota-value"
        cx="32"
        cy="32"
        r="26"
        :stroke-dasharray="`${dash} ${CIRCUMFERENCE}`"
      />
    </svg>
    <span class="quota-figure"
      ><span
        ><strong>{{ left === undefined ? "—" : left }}</strong
        ><small v-if="left !== undefined">%</small></span
      ><em v-if="left !== undefined">{{ t("left") }}</em></span
    >
    <div class="quota-copy">
      <span class="quota-title">{{ title }}</span>
      <span v-if="left === undefined" class="quota-left">{{ t("Unavailable") }}</span>
      <span class="quota-reset" :title="resetExact">{{ resetLine }}</span>
      <small v-if="resetExact" class="quota-exact">{{ resetExact }}</small>
    </div>
  </div>
</template>

<style scoped>
.quota-meter {
  --quota: var(--teal);
  --quota-soft: var(--teal-soft);
  position: relative;
  display: grid;
  grid-template-columns: 64px minmax(0, 1fr);
  align-items: center;
  gap: 14px;
  padding: 14px 16px;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: linear-gradient(180deg, var(--surface), var(--sunken));
  min-width: 0;
}
.quota-meter.warn {
  --quota: #c88a1c;
  --quota-soft: #fdf3e1;
}
.quota-meter.danger {
  --quota: #c2415a;
  --quota-soft: #fdecef;
}
.quota-meter.unknown {
  --quota: var(--muted);
}
.quota-ring {
  grid-row: 1;
  grid-column: 1;
  width: 64px;
  height: 64px;
  transform: rotate(-90deg);
}
.quota-track {
  fill: none;
  stroke: var(--quota-soft);
  stroke-width: 7;
}
.quota-value {
  fill: none;
  stroke: var(--quota);
  stroke-width: 7;
  stroke-linecap: round;
  transition: stroke-dasharray 0.6s cubic-bezier(0.2, 0.8, 0.2, 1);
}
.quota-figure {
  grid-row: 1;
  grid-column: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  line-height: 1;
  color: var(--quota);
  pointer-events: none;
}
.quota-figure > span {
  display: flex;
  align-items: baseline;
}
.quota-figure em {
  font-style: normal;
  font-size: var(--text-xs);
  font-weight: 500;
  color: var(--muted);
  margin-top: 3px;
}
.quota-figure strong {
  font-size: var(--text-xl);
  font-weight: 650;
  letter-spacing: -0.4px;
  font-variant-numeric: tabular-nums;
}
.quota-figure small {
  font-size: var(--text-xs);
  font-weight: 600;
  margin-left: 1px;
}
.quota-copy {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.quota-title {
  font-size: var(--text-xs);
  font-weight: 600;
  letter-spacing: 0.3px;
  color: var(--ink-soft);
  text-transform: uppercase;
}
.quota-left {
  font-size: var(--text-base);
  font-weight: 600;
  color: var(--muted);
}
.quota-reset {
  font-size: var(--text-base);
  font-weight: 600;
  color: var(--ink);
  margin-top: 3px;
}
.quota-exact {
  font-size: var(--text-xs);
  color: var(--muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
@media (prefers-reduced-motion: reduce) {
  .quota-value {
    transition: none;
  }
}
</style>
