<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import type { Session } from '@tendly/protocol';
import { ApiError, request } from './api';
import { preferences } from './preferences';
import { useI18n } from '../i18n';

/**
 * CPU and memory of the host, in the status bar. It opens the system page,
 * where the rest of the machine (disks, network, load, sessions) is.
 *
 * It polls only while the page is visible, and stops for good on a backend
 * that has no such route rather than asking again every few seconds. With
 * resource monitoring turned off it is hidden and asks nothing; it comes back
 * as soon as monitoring is turned on again.
 */
interface HostUsage { cpu_percent: number; cpu_count: number; memory_used_bytes: number; memory_total_bytes: number }
defineProps<{ sessions: Session[] }>();
const emit = defineEmits<{ open: [] }>();
const { t } = useI18n();
const usage = ref<HostUsage>(), unsupported = ref(false);
let timer: ReturnType<typeof setTimeout> | undefined, disposed = false;
const monitoring = computed(() => preferences.value.resource_monitoring !== false);

async function poll() {
  if (timer) clearTimeout(timer);
  if (disposed || unsupported.value || !monitoring.value) { usage.value = undefined; return; }
  if (document.visibilityState === 'visible') {
    try { usage.value = await request<HostUsage>('/host/resources'); }
    catch (cause) {
      if (cause instanceof ApiError && [404, 501].includes(cause.status)) { unsupported.value = true; return; }
      // Turned off from another browser, or signed out: hide, and check again now and then.
      if (cause instanceof ApiError && [401, 409].includes(cause.status)) { usage.value = undefined; timer = setTimeout(poll, 30_000); return; }
    }
  }
  timer = setTimeout(poll, 4000);
}
onMounted(() => { void poll(); });
watch(monitoring, () => { void poll(); });
onBeforeUnmount(() => { disposed = true; if (timer) clearTimeout(timer); });

const memoryPercent = computed(() => usage.value && usage.value.memory_total_bytes ? usage.value.memory_used_bytes / usage.value.memory_total_bytes * 100 : 0);
const level = (percent: number) => percent >= 90 ? 'danger' : percent >= 70 ? 'warn' : 'ok';
function bytes(value: number) {
  const gb = value / 1024 ** 3;
  return gb >= 1 ? `${gb.toFixed(gb >= 10 ? 0 : 1)} GB` : `${Math.round(value / 1024 ** 2)} MB`;
}
</script>

<template>
  <div v-if="usage&&!unsupported" class="host-usage">
    <button type="button" class="host-usage-chip" :title="t('System resources')" @click="emit('open')">
      <span :class="['host-meter', level(usage.cpu_percent)]"><b>CPU</b><i :style="{ '--fill': Math.min(100, usage.cpu_percent) + '%' }" />{{ Math.round(usage.cpu_percent) }}%</span>
      <span :class="['host-meter', level(memoryPercent)]"><b>{{ t('Mem') }}</b><i :style="{ '--fill': memoryPercent + '%' }" />{{ bytes(usage.memory_used_bytes) }}</span>
    </button>
  </div>
</template>

<style scoped>
.host-usage{position:relative;display:flex}
.host-usage-chip{display:flex;align-items:center;gap:12px;padding:0 4px;border:0;background:none;color:inherit;font:inherit;cursor:pointer;border-radius:var(--radius-sm)}
.host-usage-chip:hover{background:var(--fill)}
.host-meter{display:flex;align-items:center;gap:5px;font-variant-numeric:tabular-nums;color:var(--ink-soft)}
.host-meter b{font-weight:600;color:var(--muted)}
.host-meter i{--bar:var(--teal);position:relative;width:34px;height:4px;border-radius:var(--radius-xs);background:var(--fill);overflow:hidden}
.host-meter i::after{content:"";position:absolute;inset:0 auto 0 0;width:var(--fill);background:var(--bar);border-radius:var(--radius-xs);transition:width .6s ease}
.host-meter.warn i{--bar:var(--warn)}.host-meter.danger i{--bar:var(--danger)}
.host-meter.danger{color:var(--danger)}
@media(max-width:520px){.host-usage-chip{gap:8px}.host-meter i{display:none}.host-meter{white-space:nowrap}}
@media(prefers-reduced-motion:reduce){.host-meter i::after{transition:none}}
</style>
