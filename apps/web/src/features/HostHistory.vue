<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { Session } from "@tendly/protocol";
import { ApiError, providerLabel, request } from "./api";
import ProviderIcon from "./ProviderIcon.vue";
import { useI18n } from "../i18n";
import {
  RANGES,
  bin,
  busiestSessions,
  chartPaths,
  gigahertz,
  measures,
  watts,
  nearest,
  niceCeiling,
  niceRange,
  samples,
  type HostHistory,
  type RangeKey,
  type Sample,
} from "./host-history";

/**
 * The system page's longer view, from the history the server records: one
 * small chart per measure, so each keeps its own axis, sharing a cursor; and
 * the sessions that used the most CPU over the range, including ones that
 * have since ended. The page's own tiles cover the last few minutes.
 *
 * Mounted only while the system page is open. The server writes history
 * once a minute, so it is fetched again no more often than that.
 */
/** `offNoted`: the page already says monitoring is off, so this card need not. */
const props = defineProps<{
  sessions: Session[];
  cores: number;
  fanMaxRpm?: number;
  clockMaxMhz?: number;
  offNoted?: boolean;
}>();
const emit = defineEmits<{ openSession: [session: Session] }>();
const { t } = useI18n();
const range = ref<RangeKey>("1h"),
  history = ref<HostHistory>(),
  unsupported = ref(false),
  hover = ref<number>();
let timer: ReturnType<typeof setTimeout> | undefined,
  disposed = false;
const span = ref({ from: 0, to: 0 });

async function load() {
  if (timer) clearTimeout(timer);
  if (disposed || unsupported.value) return;
  if (document.visibilityState === "visible") {
    const to = Math.floor(Date.now() / 1000),
      from = to - RANGES.find(item => item.key === range.value)!.seconds;
    try {
      history.value = await request<HostHistory>(`/host/resources/history?from=${from}&to=${to}`);
      span.value = { from, to };
    } catch (cause) {
      if (cause instanceof ApiError && [404, 501].includes(cause.status)) {
        unsupported.value = true;
        return;
      }
    }
  }
  timer = setTimeout(load, 60_000);
}
onMounted(() => {
  void load();
});
onBeforeUnmount(() => {
  disposed = true;
  if (timer) clearTimeout(timer);
});
watch(range, () => {
  hover.value = undefined;
  void load();
});

const WIDTH = 300,
  HEIGHT = 56;
/** About one bucket per two or three pixels of a typical chart: smooth, but every spike still shows as a peak. */
const BINS = 120;
const gb = (bytes: number) => `${(bytes / 1024 ** 3).toFixed(1)} GB`;
const memoryTotal = computed(() => history.value?.host.at(-1)?.memory_total ?? 0);

interface Chart {
  key: string;
  label: string;
  scale: string;
  format: (value: number) => string;
  points: Sample[];
  resolution: number;
  line: string;
  band: string;
  /** Said instead of drawing a chart that would be a flat line at zero. */
  note?: string;
}
const charts = computed<Chart[]>(() => {
  const data = history.value;
  if (!data) return [];
  const { from, to } = span.value;
  const build = (
    key: string,
    label: string,
    raw: Sample[],
    bottom: number,
    top: number,
    scale: string,
    format: (value: number) => string,
  ): Chart => {
    const binned = bin(raw, from, to, BINS, data.resolution);
    return {
      key,
      label,
      scale,
      format,
      ...binned,
      ...chartPaths(binned.points, binned.resolution, { from, to, width: WIDTH, height: HEIGHT, top, bottom }),
    };
  };
  const peak = (points: Sample[]) => Math.max(0, ...points.map(point => point.max));
  const total = memoryTotal.value;
  const list = [
    build("cpu", "CPU", samples(data.host, measures.cpu), 0, 100, "0–100%", value => `${Math.round(value)}%`),
    // Memory is stored as a share; it reads in gigabytes, against the machine's total.
    total
      ? build(
          "memory",
          t("Memory"),
          samples(data.host, measures.memory),
          0,
          100,
          `0–${Math.round(total / 1024 ** 3)} GB`,
          value => gb((value / 100) * total),
        )
      : build(
          "memory",
          t("Memory"),
          samples(data.host, measures.memory),
          0,
          100,
          "0–100%",
          value => `${Math.round(value)}%`,
        ),
  ];
  // The GPU sits beside the CPU, before memory and the rest.
  const gpu = samples(data.host, measures.gpu);
  if (gpu.length)
    list.splice(
      1,
      0,
      build("gpu", "GPU", gpu, 0, 100, "0–100%", value => `${Math.round(value)}%`),
    );
  const clock = samples(data.host, measures.clock);
  if (clock.length) {
    // The chip's top clock where known, so a full-speed cluster reaches the top.
    const top = Math.max(props.clockMaxMhz ?? 0, peak(clock)) || niceCeiling(peak(clock), 1000);
    list.push(build("clock", t("CPU clock"), clock, 0, top, `0–${gigahertz(top)}`, gigahertz));
  }
  const power = samples(data.host, measures.power);
  if (power.length) {
    const top = niceCeiling(peak(power), 10);
    list.push(build("power", t("Power"), power, 0, top, `0–${top} W`, watts));
  }
  const temperature = samples(data.host, measures.temperature);
  if (temperature.length) {
    const { bottom, top } = niceRange(temperature, 10, 20);
    list.push(
      build(
        "temperature",
        t("Temperature"),
        temperature,
        bottom,
        top,
        `${bottom}–${top} °C`,
        value => `${Math.round(value)} °C`,
      ),
    );
  }
  const fan = samples(data.host, measures.fan);
  if (fan.length) {
    const top = niceCeiling(Math.max(peak(fan), props.fanMaxRpm ?? 0), 1000);
    const chart = build("fan", t("Fan"), fan, 0, top, `0–${top} rpm`, value => `${Math.round(value)} rpm`);
    list.push(peak(fan) > 0 ? chart : { ...chart, note: t("Stopped throughout this range") });
  }
  return list;
});
const empty = computed(() => !!history.value && !history.value.host.length);

/** Each chart's headline: the hovered bucket, else the latest one, with the peak beside it. */
const readouts = computed(
  () =>
    Object.fromEntries(
      charts.value.map(chart => {
        if (hover.value !== undefined) {
          const point = nearest(chart.points, hover.value, chart.resolution);
          return [
            chart.key,
            point
              ? { value: chart.format(point.avg), detail: t("peak {value}", { value: chart.format(point.max) }) }
              : { value: "—", detail: t("no data") },
          ];
        }
        const latest = chart.points.at(-1);
        if (!latest) return [chart.key, { value: "—", detail: "" }];
        return [
          chart.key,
          {
            value: chart.format(latest.avg),
            detail: t("peak {value}", { value: chart.format(Math.max(...chart.points.map(point => point.max))) }),
          },
        ];
      }),
    ) as Record<string, { value: string; detail: string }>,
);
const cursor = computed(() => {
  if (hover.value === undefined) return undefined;
  const { from, to } = span.value;
  return `${((hover.value - from) / Math.max(1, to - from)) * 100}%`;
});
function track(event: PointerEvent) {
  const box = (event.currentTarget as HTMLElement).getBoundingClientRect();
  const { from, to } = span.value;
  hover.value = from + Math.min(1, Math.max(0, (event.clientX - box.left) / box.width)) * (to - from);
}

const clock = (seconds: number, withDate: boolean) =>
  new Date(seconds * 1000).toLocaleString(
    undefined,
    withDate
      ? { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" }
      : { hour: "2-digit", minute: "2-digit" },
  );
const longRange = computed(() => range.value !== "1h");
/** The window, or the moment under the cursor while one is hovered. */
const timeLabel = computed(() =>
  hover.value !== undefined
    ? clock(hover.value, longRange.value)
    : `${clock(span.value.from, longRange.value)} – ${t("now")}`,
);

const busiest = computed(() =>
  busiestSessions(history.value?.sessions ?? [], props.cores).map(row => ({
    ...row,
    session: props.sessions.find(item => item.id === row.session_id),
  })),
);
const percent = (value: number) => (value < 0.5 ? "<1%" : `${Math.round(value)}%`);
</script>

<template>
  <section v-if="!unsupported" class="host-history" :aria-label="t('Trends')">
    <header>
      <h2>{{ t("Trends") }}</h2>
      <small v-if="history && !empty" :class="['host-history-time', { hovering: hover !== undefined }]">{{
        timeLabel
      }}</small>
      <div class="host-history-ranges" role="group" :aria-label="t('Time range')">
        <button
          v-for="item in RANGES"
          :key="item.key"
          type="button"
          :aria-pressed="range === item.key"
          @click="range = item.key"
        >
          {{ item.key }}
        </button>
      </div>
    </header>
    <p v-if="history?.monitoring === false && !offNoted" class="host-history-off">
      {{
        t(
          empty
            ? "Resource monitoring is turned off."
            : "Resource monitoring is turned off; this is what was kept before.",
        )
      }}
      <RouterLink :to="{ name: 'settings', params: { section: 'preferences' } }">{{ t("Settings") }}</RouterLink>
    </p>
    <p v-else-if="history?.monitoring === false && empty" class="host-history-empty">
      {{ t("Nothing was kept before monitoring was turned off.") }}
    </p>
    <p v-else-if="empty" class="host-history-empty">
      {{ t("History builds up while Tendly runs; the first points arrive within a minute.") }}
    </p>
    <template v-if="history && !empty">
      <div class="host-trends">
        <div v-for="chart in charts" :key="chart.key" class="host-trend">
          <div class="host-trend-head">
            <span class="host-trend-name"
              >{{ chart.label }}<small v-if="!chart.note">{{ chart.scale }}</small></span
            >
            <span class="host-trend-value"
              ><strong>{{ readouts[chart.key].value }}</strong
              ><small>{{ readouts[chart.key].detail }}</small></span
            >
          </div>
          <p v-if="chart.note" class="host-trend-note">{{ chart.note }}</p>
          <div v-else class="host-trend-plot" @pointermove="track" @pointerleave="hover = undefined">
            <svg
              :viewBox="`0 0 ${WIDTH} ${HEIGHT}`"
              preserveAspectRatio="none"
              role="img"
              :aria-label="`${chart.label}: ${readouts[chart.key].value}, ${readouts[chart.key].detail}`"
            >
              <line class="host-trend-grid" x1="0" :x2="WIDTH" y1="0.5" y2="0.5" />
              <line class="host-trend-grid" x1="0" :x2="WIDTH" :y1="HEIGHT / 2" :y2="HEIGHT / 2" />
              <path class="host-trend-band" :d="chart.band" />
              <path class="host-trend-line" :d="chart.line" />
            </svg>
            <i v-if="cursor" class="host-trend-cursor" :style="{ left: cursor }" />
          </div>
        </div>
      </div>
      <!-- Without the core count a share of the machine cannot be worked out. -->
      <template v-if="busiest.length && cores">
        <h3>{{ t("Busiest sessions") }}</h3>
        <ul>
          <li class="host-history-columns" aria-hidden="true">
            <span /><span /><span>{{ t("Average") }}</span
            ><span>{{ t("Peak") }}</span>
          </li>
          <li
            v-for="row in busiest"
            :key="row.session_id"
            :class="{ link: !!row.session }"
            @click="row.session && emit('openSession', row.session)"
          >
            <ProviderIcon v-if="row.session" :provider="row.session.provider" :size="13" />
            <span v-else />
            <span class="host-history-name" :title="row.session?.title">{{
              row.session?.title ?? (row.session ? providerLabel(row.session.provider) : row.session_id.slice(0, 8))
            }}</span>
            <span class="host-history-figure" :title="t('Average CPU')">{{ percent(row.cpu_avg) }}</span>
            <span class="host-history-figure" :title="t('Peak CPU')">{{ percent(row.cpu_max) }}</span>
          </li>
        </ul>
        <small class="host-history-note">{{
          t("Average and peak CPU over the range, as a share of the machine.")
        }}</small>
      </template>
    </template>
  </section>
</template>

<style scoped>
.host-history {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  padding: var(--space-4);
  border-radius: var(--radius-lg);
  background: var(--surface);
  box-shadow: var(--shadow-card);
  min-width: 0;
}
.host-history header {
  display: flex;
  align-items: center;
  gap: var(--space-2);
}
.host-history h2 {
  font-size: var(--text-md);
  font-weight: 600;
  color: var(--ink);
}
.host-history-time {
  color: var(--muted);
  font-size: var(--text-sm);
  font-variant-numeric: tabular-nums;
}
.host-history-time.hovering {
  color: var(--ink);
}
.host-history-ranges {
  display: flex;
  gap: 2px;
  margin-left: auto;
  padding: 2px;
  border-radius: var(--radius-sm);
  background: var(--fill);
}
.host-history-ranges button {
  min-width: 34px;
  padding: 2px var(--space-2);
  border: 0;
  border-radius: var(--radius-xs);
  background: none;
  color: var(--muted);
  font: inherit;
  font-size: var(--text-xs);
  cursor: pointer;
}
.host-history-ranges button[aria-pressed="true"] {
  background: var(--surface);
  color: var(--ink);
  box-shadow: var(--shadow-sm);
}
.host-history-empty {
  color: var(--muted);
  font-size: var(--text-sm);
}
.host-history-off {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-3);
  border-radius: var(--radius-md);
  background: var(--sunken);
  color: var(--ink-soft);
  font-size: var(--text-sm);
}
.host-history-off a {
  color: var(--accent);
  font-weight: 550;
  text-decoration: none;
}
.host-history-off a:hover {
  text-decoration: underline;
}
.host-trends {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
  gap: var(--space-5) var(--space-6);
}
.host-trend {
  min-width: 0;
}
.host-trend-head {
  display: flex;
  align-items: flex-end;
  justify-content: space-between;
  gap: var(--space-3);
  margin-bottom: var(--space-2);
}
.host-trend-name {
  display: flex;
  flex-direction: column;
  gap: 2px;
  font-weight: 600;
  color: var(--ink);
  font-size: var(--text-sm);
}
.host-trend-name small,
.host-trend-value small {
  font-weight: 400;
  color: var(--muted);
  font-size: var(--text-xs);
  font-variant-numeric: tabular-nums;
}
.host-trend-value {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 2px;
  min-width: 0;
}
.host-trend-value strong {
  font-size: var(--text-lg);
  font-weight: 650;
  color: var(--ink);
  font-variant-numeric: tabular-nums;
  line-height: 1.1;
  white-space: nowrap;
}
.host-trend-note {
  margin: 0;
  padding: var(--space-3) 0;
  border-bottom: 1px solid var(--line);
  color: var(--ink-soft);
  font-size: var(--text-sm);
}
.host-trend-plot {
  position: relative;
  height: 56px;
  border-bottom: 1px solid var(--line);
  cursor: crosshair;
  touch-action: none;
}
.host-trend-plot svg {
  display: block;
  width: 100%;
  height: 100%;
  overflow: visible;
}
.host-trend-grid {
  stroke: var(--line);
  stroke-width: 1;
  stroke-dasharray: 2 3;
  vector-effect: non-scaling-stroke;
}
.host-trend-band {
  fill: var(--accent);
  opacity: 0.18;
}
.host-trend-line {
  fill: none;
  stroke: var(--accent);
  stroke-width: 2;
  stroke-linejoin: round;
  stroke-linecap: round;
  vector-effect: non-scaling-stroke;
}
.host-trend-cursor {
  position: absolute;
  top: 0;
  bottom: 0;
  width: 1px;
  background: var(--muted);
  pointer-events: none;
}
.host-history h3 {
  font-size: var(--text-sm);
  font-weight: 600;
  color: var(--ink);
}
.host-history ul {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.host-history li {
  display: grid;
  grid-template-columns: 14px minmax(0, 1fr) 48px 48px;
  align-items: center;
  gap: var(--space-2);
  padding: 6px var(--space-2);
  border-radius: var(--radius-sm);
  font-size: var(--text-sm);
}
.host-history li.link {
  cursor: pointer;
}
.host-history li:not(.host-history-columns):hover {
  background: var(--sunken);
}
.host-history-columns {
  padding-block: 0 !important;
  color: var(--muted);
  font-size: var(--text-xs) !important;
}
.host-history-columns span:nth-child(n + 3) {
  text-align: right;
}
.host-history-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--ink);
}
.host-history-figure {
  text-align: right;
  font-variant-numeric: tabular-nums;
  color: var(--ink-soft);
}
.host-history-note {
  font-size: var(--text-xs);
  color: var(--muted);
}
</style>
