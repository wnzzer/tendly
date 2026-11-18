<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { Session } from "@tendly/protocol";
import { ApiError, errorMessage, providerLabel, request } from "./api";
import PageShell from "./PageShell.vue";
import ProviderIcon from "./ProviderIcon.vue";
import Icon from "./Icon.vue";
import HostHistory from "./HostHistory.vue";
import { gigahertz, watts } from "./host-history";
import { preferences } from "./preferences";
import { useI18n } from "../i18n";
import { formatBytes, formatDuration, formatRate, levelFor, pushSample, sparkPath } from "./system-format";

/**
 * The machine Tendly runs on: what it is, how busy it is, and how much room
 * is left. It polls while the page is open and visible, and keeps a short
 * history here in the browser so each figure shows which way it is heading.
 */
interface SystemDetail {
  host: {
    hostname: string | null;
    os: string | null;
    kernel: string | null;
    arch: string;
    uptime_seconds: number;
    tendly_version: string;
    tendly_uptime_seconds: number;
  };
  load: { one: number; five: number; fifteen: number } | null;
  cpu: { percent: number; count: number; brand: string | null; frequency_mhz: number | null; cores: number[] };
  memory: {
    used_bytes: number;
    total_bytes: number;
    available_bytes: number;
    swap_used_bytes: number;
    swap_total_bytes: number;
  };
  disks: Array<{
    name: string;
    mount_point: string;
    file_system: string;
    kind: string;
    total_bytes: number;
    available_bytes: number;
    removable: boolean;
    read_only: boolean;
  }>;
  networks: Array<{
    name: string;
    received_per_second: number;
    transmitted_per_second: number;
    total_received_bytes: number;
    total_transmitted_bytes: number;
  }>;
  temperatures: Array<{ label: string; celsius: number; critical_celsius: number | null }>;
  /** Absent on an older backend; empty where the machine reports none. */
  fans?: Array<{ label: string; rpm: number; min_rpm: number | null; max_rpm: number | null }>;
  /** Each cluster's clock while running, null when it sat idle; live only where the platform says. */
  cpu_frequencies?: CpuFrequency[];
  /** The whole machine's draw, and the parts the chip counts; absent where unknown. */
  power_watts?: number | null;
  power_parts?: Array<{ kind: "cpu" | "gpu" | "ane" | "dram"; watts: number }>;
  /** Each GPU the platform reports; absent from an older backend. */
  gpus?: Gpu[];
  processes: Array<{ pid: number; name: string; cpu_percent: number; memory_bytes: number }>;
  sessions: Array<{ session_id: string; cpu_percent: number; memory_bytes: number }>;
}
interface Gpu {
  name: string;
  cores: number | null;
  utilization_percent: number | null;
  memory_used_bytes: number | null;
  memory_total_bytes: number | null;
  mhz: number | null;
  max_mhz: number | null;
  watts: number | null;
  celsius: number | null;
}
interface CpuFrequency {
  label: string;
  kind: "efficiency" | "performance" | null;
  mhz: number | null;
  max_mhz: number | null;
}
const props = defineProps<{ sessions: Session[] }>();
const emit = defineEmits<{ back: []; openSession: [session: Session] }>();
const { t } = useI18n();
const detail = ref<SystemDetail>(),
  error = ref(""),
  unsupported = ref(false);
/** Resource monitoring is turned off: nothing live is read, and the page says so. */
const off = ref(false);
const history = ref({
  cpu: [] as number[],
  memory: [] as number[],
  received: [] as number[],
  transmitted: [] as number[],
  power: [] as number[],
  gpu: [] as number[],
});
let timer: ReturnType<typeof setTimeout> | undefined,
  disposed = false;

async function poll() {
  if (timer) clearTimeout(timer);
  if (disposed) return;
  if (preferences.value.resource_monitoring === false) {
    off.value = true;
    detail.value = undefined;
    return;
  }
  if (document.visibilityState === "visible") {
    try {
      const next = await request<SystemDetail>("/host/system");
      if (disposed) return;
      detail.value = next;
      error.value = "";
      off.value = false;
      const net = next.networks.reduce(
        (sum, item) => ({ rx: sum.rx + item.received_per_second, tx: sum.tx + item.transmitted_per_second }),
        { rx: 0, tx: 0 },
      );
      history.value = {
        cpu: pushSample(history.value.cpu, next.cpu.percent),
        memory: pushSample(
          history.value.memory,
          next.memory.total_bytes ? (next.memory.used_bytes / next.memory.total_bytes) * 100 : 0,
        ),
        received: pushSample(history.value.received, net.rx),
        transmitted: pushSample(history.value.transmitted, net.tx),
        power: pushSample(history.value.power, powerOf(next) ?? 0),
        gpu: pushSample(history.value.gpu, Math.max(0, ...(next.gpus ?? []).map(gpu => gpu.utilization_percent ?? 0))),
      };
    } catch (cause) {
      if (cause instanceof ApiError && [404, 501].includes(cause.status)) {
        unsupported.value = true;
        return;
      }
      // Turned off from another browser: say so, and check again now and then.
      if (cause instanceof ApiError && cause.status === 409) {
        off.value = true;
        detail.value = undefined;
        timer = setTimeout(poll, 30_000);
        return;
      }
      error.value = errorMessage(cause);
    }
  }
  timer = setTimeout(poll, 2000);
}
onMounted(() => {
  void poll();
});
watch(
  () => preferences.value.resource_monitoring,
  () => {
    off.value = false;
    void poll();
  },
);
onBeforeUnmount(() => {
  disposed = true;
  if (timer) clearTimeout(timer);
});

const memoryPercent = computed(() =>
  detail.value?.memory.total_bytes ? (detail.value.memory.used_bytes / detail.value.memory.total_bytes) * 100 : 0,
);
/** The disk that holds `/` (or the first one), which is what "space left" means to most people. */
const mainDisk = computed(
  () =>
    detail.value?.disks.find(disk => disk.mount_point === "/" || /^[A-Z]:\\?$/i.test(disk.mount_point)) ??
    detail.value?.disks[0],
);
const diskPercent = (disk: { total_bytes: number; available_bytes: number }) =>
  disk.total_bytes ? ((disk.total_bytes - disk.available_bytes) / disk.total_bytes) * 100 : 0;
const networkNow = computed(() => ({
  rx: history.value.received.at(-1) ?? 0,
  tx: history.value.transmitted.at(-1) ?? 0,
}));
const loadLevel = computed(() =>
  detail.value?.load ? levelFor((detail.value.load.one / Math.max(1, detail.value.cpu.count)) * 100) : "ok",
);
const sessionRows = computed(() => {
  const cores = detail.value?.cpu.count || 1;
  return (detail.value?.sessions ?? [])
    .map(entry => ({
      ...entry,
      session: props.sessions.find(item => item.id === entry.session_id),
      share: entry.cpu_percent / cores,
    }))
    .sort((a, b) => b.memory_bytes - a.memory_bytes);
});
/** Performance clusters first, numbered only when a chip has more than one of a kind. */
const clocks = computed(() => {
  const all = detail.value?.cpu_frequencies ?? [];
  const rank = (item: CpuFrequency) => (item.kind === "performance" ? 0 : item.kind === "efficiency" ? 1 : 2);
  const name = (item: CpuFrequency) => {
    if (!item.kind) return t("CPU");
    const same = all.filter(other => other.kind === item.kind);
    const base = t(item.kind === "performance" ? "P-cores" : "E-cores");
    return same.length > 1 ? `${base} ${same.indexOf(item) + 1}` : base;
  };
  return [...all]
    .sort((a, b) => rank(a) - rank(b))
    .map(item => `${name(item)} ${item.mhz == null ? t("idle") : gigahertz(item.mhz)}`)
    .join(" · ");
});
/** The whole machine's draw where known, else what the measured parts add up to. */
function powerOf(next: SystemDetail): number | undefined {
  if (next.power_watts != null) return next.power_watts;
  const parts = next.power_parts ?? [];
  return parts.length ? parts.reduce((sum, part) => sum + part.watts, 0) : undefined;
}
const PART_LABELS: Record<string, string> = { cpu: "CPU", gpu: "GPU", ane: "Neural Engine", dram: "Memory" };
const power = computed(() => {
  const next = detail.value;
  const now = next ? powerOf(next) : undefined;
  if (!next || now === undefined) return undefined;
  return {
    now,
    whole: next.power_watts != null,
    // A part drawing next to nothing (an idle neural engine) is left out of the line.
    parts: (next.power_parts ?? [])
      .filter(part => part.watts >= 0.05)
      .map(part => `${t(PART_LABELS[part.kind] ?? part.kind)} ${watts(part.watts)}`),
  };
});
const megahertz = (mhz: number) => (mhz >= 1000 ? gigahertz(mhz) : `${mhz} MHz`);
/** What a GPU tile says under its figure: clock, memory, power, temperature, where known. */
function gpuLine(gpu: Gpu): string[] {
  const memory =
    gpu.memory_used_bytes == null
      ? undefined
      : gpu.memory_total_bytes
        ? `${formatBytes(gpu.memory_used_bytes)} / ${formatBytes(gpu.memory_total_bytes)}`
        : t("Memory {size}", { size: formatBytes(gpu.memory_used_bytes) });
  return [
    gpu.mhz != null ? megahertz(gpu.mhz) : undefined,
    memory,
    gpu.watts != null ? watts(gpu.watts) : undefined,
    gpu.celsius != null ? `${Math.round(gpu.celsius)} °C` : undefined,
  ].filter((item): item is string => !!item);
}
const fanMax = computed(() => Math.max(0, ...(detail.value?.fans ?? []).map(fan => fan.max_rpm ?? 0)) || undefined);
const clockMax = computed(
  () => Math.max(0, ...(detail.value?.cpu_frequencies ?? []).map(item => item.max_mhz ?? 0)) || undefined,
);
const subtitle = computed(() => {
  const host = detail.value?.host;
  if (!host) return undefined;
  return [host.os, host.kernel && !host.os?.includes(host.kernel) ? host.kernel : undefined, host.arch]
    .filter(Boolean)
    .join(" · ");
});
</script>

<template>
  <PageShell :title="detail?.host.hostname || t('This host')" :subtitle="subtitle" @back="emit('back')">
    <div class="system-page">
      <p v-if="unsupported" class="inline-notice">
        {{ t("This backend does not report system details. Update Tendly on the host to see them.") }}
      </p>
      <p v-else-if="error" class="inline-error" role="alert">{{ error }}</p>
      <template v-if="off">
        <p class="inline-notice system-off">
          {{
            t("Resource monitoring is turned off, so nothing is read from this host. What was kept before is below.")
          }}
          <RouterLink :to="{ name: 'settings', params: { section: 'preferences' } }">{{ t("Settings") }}</RouterLink>
        </p>
        <HostHistory :sessions="sessions" :cores="0" off-noted @open-session="emit('openSession', $event)" />
      </template>
      <div v-else-if="!detail && !unsupported" class="pane-empty">
        <p>{{ t("Reading this host…") }}</p>
      </div>
      <template v-if="detail">
        <div class="system-facts">
          <span
            ><Icon name="clock" :size="14" />{{
              t("Up {time}", { time: formatDuration(detail.host.uptime_seconds) })
            }}</span
          >
          <span
            ><Icon name="spark" :size="14" />Tendly {{ detail.host.tendly_version }} ·
            {{ t("running {time}", { time: formatDuration(detail.host.tendly_uptime_seconds) }) }}</span
          >
          <span v-if="detail.cpu.brand"
            ><Icon name="gauge" :size="14" />{{ detail.cpu.brand
            }}<template v-if="detail.cpu.frequency_mhz && !clocks">
              · {{ (detail.cpu.frequency_mhz / 1000).toFixed(1) }} GHz</template
            ></span
          >
          <span v-if="clocks" :title="t('Clock while running, over the last moment')"
            ><Icon name="gauge" :size="14" />{{ clocks }}</span
          >
        </div>

        <div class="system-tiles">
          <article :class="['system-tile', levelFor(detail.cpu.percent)]">
            <header>
              <span>CPU</span><small>{{ t("{count} cores", { count: detail.cpu.count }) }}</small>
            </header>
            <strong>{{ Math.round(detail.cpu.percent) }}<small>%</small></strong>
            <svg class="spark" viewBox="0 0 100 28" preserveAspectRatio="none" aria-hidden="true">
              <path :d="sparkPath(history.cpu, 100)" />
            </svg>
          </article>
          <article :class="['system-tile', levelFor(memoryPercent)]">
            <header>
              <span>{{ t("Memory") }}</span><small>{{ formatBytes(detail.memory.total_bytes) }}</small>
            </header>
            <strong>{{ formatBytes(detail.memory.used_bytes) }}</strong>
            <svg class="spark" viewBox="0 0 100 28" preserveAspectRatio="none" aria-hidden="true">
              <path :d="sparkPath(history.memory, 100)" />
            </svg>
            <footer v-if="detail.memory.swap_total_bytes">
              {{
                t("Swap {used} / {total}", {
                  used: formatBytes(detail.memory.swap_used_bytes),
                  total: formatBytes(detail.memory.swap_total_bytes),
                })
              }}
            </footer>
          </article>
          <article v-if="detail.load" :class="['system-tile', loadLevel]">
            <header>
              <span>{{ t("Load") }}</span><small>1 · 5 · 15 {{ t("min") }}</small>
            </header>
            <strong>{{ detail.load.one.toFixed(2) }}</strong>
            <footer>{{ detail.load.five.toFixed(2) }} · {{ detail.load.fifteen.toFixed(2) }}</footer>
          </article>
          <article
            v-for="gpu in detail.gpus ?? []"
            :key="gpu.name"
            :class="['system-tile', levelFor(gpu.utilization_percent ?? 0)]"
          >
            <header>
              <span>GPU</span
              ><small :title="gpu.name">{{
                gpu.cores ? `${gpu.name} · ${t("{count} cores", { count: gpu.cores })}` : gpu.name
              }}</small>
            </header>
            <strong
              >{{ gpu.utilization_percent == null ? "—" : Math.round(gpu.utilization_percent)
              }}<small v-if="gpu.utilization_percent != null">%</small></strong
            >
            <!-- One line of history for the busiest GPU; with several, a line under each would repeat it. -->
            <svg
              v-if="(detail.gpus ?? []).length === 1"
              class="spark"
              viewBox="0 0 100 28"
              preserveAspectRatio="none"
              aria-hidden="true"
            >
              <path :d="sparkPath(history.gpu, 100)" />
            </svg>
            <footer v-if="gpuLine(gpu).length" class="facts">
              <span v-for="item in gpuLine(gpu)" :key="item">{{ item }}</span>
            </footer>
          </article>
          <article v-if="power" class="system-tile ok">
            <header>
              <span>{{ t("Power") }}</span><small>{{ t(power.whole ? "Whole machine" : "Measured parts") }}</small>
            </header>
            <strong>{{ watts(power.now).replace(" W", "") }}<small>W</small></strong>
            <svg class="spark" viewBox="0 0 100 28" preserveAspectRatio="none" aria-hidden="true">
              <path :d="sparkPath(history.power)" />
            </svg>
            <footer v-if="power.parts.length" class="facts">
              <span v-for="item in power.parts" :key="item">{{ item }}</span>
            </footer>
          </article>
          <article v-if="mainDisk" :class="['system-tile', levelFor(diskPercent(mainDisk))]">
            <header>
              <span>{{ t("Disk") }}</span><small>{{ mainDisk.mount_point }}</small>
            </header>
            <strong>{{ formatBytes(mainDisk.available_bytes) }}</strong>
            <div class="bar" :style="{ '--fill': diskPercent(mainDisk) + '%' }" />
            <footer>{{ t("free of {total}", { total: formatBytes(mainDisk.total_bytes) }) }}</footer>
          </article>
          <article class="system-tile ok">
            <header>
              <span>{{ t("Network") }}</span><small>↓ / ↑</small>
            </header>
            <strong class="dual"
              >{{ formatRate(networkNow.rx) }}<small>↓</small>
              <span>{{ formatRate(networkNow.tx) }}<small>↑</small></span></strong
            >
            <svg class="spark" viewBox="0 0 100 28" preserveAspectRatio="none" aria-hidden="true">
              <path :d="sparkPath(history.received)" />
              <path class="secondary" :d="sparkPath(history.transmitted)" />
            </svg>
          </article>
        </div>

        <HostHistory
          :sessions="sessions"
          :cores="detail.cpu.count"
          :fan-max-rpm="fanMax"
          :clock-max-mhz="clockMax"
          @open-session="emit('openSession', $event)"
        />

        <div class="system-grid">
          <section class="system-card">
            <h2>{{ t("CPU cores") }}</h2>
            <div class="cores">
              <div
                v-for="(core, index) in detail.cpu.cores"
                :key="index"
                :class="['core', levelFor(core)]"
                :title="`#${index} · ${Math.round(core)}%`"
              >
                <i :style="{ '--fill': Math.min(100, core) + '%' }" />
              </div>
            </div>
          </section>

          <section class="system-card">
            <h2>{{ t("Disks") }}</h2>
            <p v-if="!detail.disks.length" class="muted">{{ t("No disks reported.") }}</p>
            <ul class="rows">
              <li v-for="disk in detail.disks" :key="disk.mount_point">
                <div class="row-head">
                  <strong>{{ disk.mount_point }}</strong><small
                    >{{ disk.file_system
                    }}<template v-if="disk.kind !== 'unknown'"> · {{ disk.kind.toUpperCase() }}</template
                    ><template v-if="disk.removable"> · {{ t("removable") }}</template
                    ><template v-if="disk.read_only"> · {{ t("read-only") }}</template></small
                  >
                </div>
                <div :class="['bar', levelFor(diskPercent(disk))]" :style="{ '--fill': diskPercent(disk) + '%' }" />
                <small class="row-foot">{{
                  t("{free} free of {total}", {
                    free: formatBytes(disk.available_bytes),
                    total: formatBytes(disk.total_bytes),
                  })
                }}</small>
              </li>
            </ul>
          </section>

          <section class="system-card">
            <h2>{{ t("Network") }}</h2>
            <p v-if="!detail.networks.length" class="muted">{{ t("No network interfaces reported.") }}</p>
            <table v-else class="table">
              <thead>
                <tr>
                  <th>{{ t("Interface") }}</th>
                  <th>↓</th>
                  <th>↑</th>
                  <th>{{ t("Total") }}</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="net in detail.networks" :key="net.name">
                  <td>{{ net.name }}</td>
                  <td>{{ formatRate(net.received_per_second) }}</td>
                  <td>{{ formatRate(net.transmitted_per_second) }}</td>
                  <td>{{ formatBytes(net.total_received_bytes + net.total_transmitted_bytes) }}</td>
                </tr>
              </tbody>
            </table>
          </section>

          <section v-if="detail.temperatures.length || detail.fans?.length" class="system-card">
            <h2>{{ t(detail.fans?.length ? "Temperatures and fans" : "Temperatures") }}</h2>
            <ul class="temps">
              <li
                v-for="fan in detail.fans"
                :key="fan.label"
                class="ok"
                :title="fan.max_rpm ? t('up to {rpm} rpm', { rpm: fan.max_rpm }) : undefined"
              >
                <span>{{ fan.label }}</span><strong>{{ fan.rpm }} rpm</strong>
              </li>
              <li
                v-for="item in detail.temperatures"
                :key="item.label"
                :class="levelFor(item.critical_celsius ? (item.celsius / item.critical_celsius) * 100 : item.celsius)"
              >
                <span>{{ item.label }}</span><strong>{{ Math.round(item.celsius) }}°C</strong>
              </li>
            </ul>
          </section>

          <section class="system-card">
            <h2>{{ t("Tendly sessions") }}</h2>
            <p v-if="!sessionRows.length" class="muted">{{ t("No session is running.") }}</p>
            <table v-else class="table">
              <thead>
                <tr>
                  <th>{{ t("Session") }}</th>
                  <th>CPU</th>
                  <th>{{ t("Memory") }}</th>
                </tr>
              </thead>
              <tbody>
                <tr
                  v-for="row in sessionRows"
                  :key="row.session_id"
                  :class="{ link: !!row.session }"
                  @click="row.session && emit('openSession', row.session)"
                >
                  <td>
                    <span class="session-cell"
                      ><ProviderIcon v-if="row.session" :provider="row.session.provider" :size="13" /><span>{{
                        row.session?.title ??
                        (row.session ? providerLabel(row.session.provider) : row.session_id.slice(0, 8))
                      }}</span></span
                    >
                  </td>
                  <td>{{ row.share < 0.5 ? "<1" : Math.round(row.share) }}%</td>
                  <td>{{ formatBytes(row.memory_bytes) }}</td>
                </tr>
              </tbody>
            </table>
            <small class="muted">{{ t("Each session counts its client and everything it started.") }}</small>
          </section>

          <section class="system-card">
            <h2>{{ t("Busiest processes") }}</h2>
            <table class="table">
              <thead>
                <tr>
                  <th>{{ t("Process") }}</th>
                  <th>PID</th>
                  <th>CPU</th>
                  <th>{{ t("Memory") }}</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="process in detail.processes" :key="process.pid">
                  <td class="mono">{{ process.name }}</td>
                  <td class="mono">{{ process.pid }}</td>
                  <td>{{ (process.cpu_percent / Math.max(1, detail.cpu.count)).toFixed(1) }}%</td>
                  <td>{{ formatBytes(process.memory_bytes) }}</td>
                </tr>
              </tbody>
            </table>
          </section>
        </div>
      </template>
    </div>
  </PageShell>
</template>

<style scoped>
.system-page {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}
.system-off {
  display: flex;
  flex-wrap: wrap;
  align-items: baseline;
  gap: var(--space-2);
}
.system-off a {
  color: var(--accent);
  font-weight: 550;
  text-decoration: none;
}
.system-off a:hover {
  text-decoration: underline;
}
.system-facts {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2) var(--space-5);
  font-size: var(--text-sm);
  color: var(--ink-soft);
}
.system-facts span {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.system-facts svg {
  color: var(--muted);
}
.system-tiles {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(170px, 1fr));
  gap: var(--space-3);
}
.system-tile {
  --tone: var(--accent);
  position: relative;
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: var(--space-4);
  border-radius: var(--radius-lg);
  background: var(--surface);
  box-shadow: var(--shadow-card);
  overflow: hidden;
  min-height: 128px;
}
.system-tile.warn {
  --tone: var(--warn);
}
.system-tile.danger {
  --tone: var(--danger);
}
.system-tile header {
  display: flex;
  justify-content: space-between;
  gap: var(--space-2);
  font-size: var(--text-sm);
  color: var(--ink-soft);
  font-weight: 550;
}
.system-tile header small {
  font-weight: 400;
  color: var(--muted);
  font-size: var(--text-xs);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.system-tile strong {
  font-size: var(--text-2xl);
  font-weight: 650;
  letter-spacing: -0.5px;
  color: var(--ink);
  font-variant-numeric: tabular-nums;
  line-height: 1.2;
}
.system-tile strong small {
  font-size: var(--text-md);
  font-weight: 500;
  color: var(--muted);
  margin-left: 2px;
}
.system-tile strong.dual {
  font-size: var(--text-xl);
}
.system-tile strong.dual span {
  display: block;
  font-size: var(--text-md);
  color: var(--ink-soft);
  font-weight: 550;
}
.system-tile footer {
  margin-top: auto;
  font-size: var(--text-xs);
  color: var(--muted);
  font-variant-numeric: tabular-nums;
}
/* A list of facts breaks between them, never inside one ("0.1 W"). */
.system-tile footer.facts {
  display: flex;
  flex-wrap: wrap;
  column-gap: 6px;
}
.system-tile footer.facts span {
  white-space: nowrap;
}
.system-tile footer.facts span + span::before {
  content: "·";
  margin-right: 6px;
}
.spark {
  margin-top: auto;
  width: 100%;
  height: 30px;
  overflow: visible;
}
.spark path {
  fill: none;
  stroke: var(--tone);
  stroke-width: 1.6;
  vector-effect: non-scaling-stroke;
  stroke-linejoin: round;
}
.spark path.secondary {
  stroke: var(--violet);
  opacity: 0.7;
}
.bar {
  --fill: 0%;
  --tone: var(--accent);
  position: relative;
  height: 6px;
  border-radius: var(--radius-full);
  background: var(--fill);
  overflow: hidden;
  margin-top: 6px;
}
.bar.warn {
  --tone: var(--warn);
}
.bar.danger {
  --tone: var(--danger);
}
.system-tile .bar {
  --tone: inherit;
}
.bar::after {
  content: "";
  position: absolute;
  inset: 0 auto 0 0;
  width: var(--fill);
  border-radius: inherit;
  background: var(--tone);
  transition: width 0.6s var(--ease-out);
}
.system-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(340px, 1fr));
  gap: var(--space-3);
  align-items: start;
}
.system-card {
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  padding: var(--space-4);
  border-radius: var(--radius-lg);
  background: var(--surface);
  box-shadow: var(--shadow-card);
  min-width: 0;
}
.system-card h2 {
  font-size: var(--text-md);
  font-weight: 600;
  color: var(--ink);
}
.muted {
  font-size: var(--text-sm);
  color: var(--muted);
}
.cores {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(18px, 1fr));
  gap: 4px;
}
.core {
  --tone: var(--accent);
  position: relative;
  height: 40px;
  border-radius: var(--radius-xs);
  background: var(--fill);
  overflow: hidden;
}
.core.warn {
  --tone: var(--warn);
}
.core.danger {
  --tone: var(--danger);
}
.core i {
  position: absolute;
  inset: auto 0 0 0;
  height: var(--fill);
  background: var(--tone);
  opacity: 0.85;
  transition: height 0.6s var(--ease-out);
}
.rows {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}
.row-head {
  display: flex;
  justify-content: space-between;
  align-items: baseline;
  gap: var(--space-2);
}
.row-head strong {
  font-size: var(--text-sm);
  font-weight: 600;
  color: var(--ink);
  font-family: var(--mono);
}
.row-head small,
.row-foot {
  font-size: var(--text-xs);
  color: var(--muted);
}
.row-foot {
  display: block;
  margin-top: 4px;
  font-variant-numeric: tabular-nums;
}
.table {
  width: 100%;
  border-collapse: collapse;
  font-size: var(--text-sm);
}
.table th {
  text-align: left;
  font-weight: 500;
  font-size: var(--text-xs);
  color: var(--muted);
  padding: 0 var(--space-2) 6px;
  border-bottom: 1px solid var(--border);
}
.table td {
  padding: 7px var(--space-2);
  border-bottom: 1px solid var(--border);
  color: var(--ink-soft);
  font-variant-numeric: tabular-nums;
  max-width: 220px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.table tr:last-child td {
  border-bottom: 0;
}
.table th:not(:first-child),
.table td:not(:first-child) {
  text-align: right;
}
.table td:first-child {
  color: var(--ink);
}
.table tr.link {
  cursor: pointer;
}
.table tr.link:hover td {
  background: var(--sunken);
}
.mono {
  font-family: var(--mono);
  font-size: var(--text-xs);
}
.session-cell {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}
.session-cell span {
  overflow: hidden;
  text-overflow: ellipsis;
}
.temps {
  list-style: none;
  margin: 0;
  padding: 0;
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
  gap: var(--space-2);
}
.temps li {
  --tone: var(--ink);
  display: flex;
  justify-content: space-between;
  gap: var(--space-2);
  padding: 8px 10px;
  border-radius: var(--radius-md);
  background: var(--sunken);
  font-size: var(--text-sm);
  color: var(--ink-soft);
}
.temps li.warn {
  --tone: var(--warn-ink);
}
.temps li.danger {
  --tone: var(--danger-ink);
}
.temps li span {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.temps strong {
  color: var(--tone);
  font-variant-numeric: tabular-nums;
}
@media (max-width: 760px) {
  .system-grid {
    grid-template-columns: minmax(0, 1fr);
  }
  .system-tiles {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
  .system-tile {
    min-height: 112px;
    padding: var(--space-3);
  }
  .system-tile strong {
    font-size: var(--text-2xl);
  }
}
@media (prefers-reduced-motion: reduce) {
  .bar::after,
  .core i {
    transition: none;
  }
}
</style>
