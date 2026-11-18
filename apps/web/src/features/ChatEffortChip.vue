<script setup lang="ts">
/**
 * The composer's thinking-depth chip: a slider over the levels the model in
 * use offers, with "leave it to the client" as its first stop.
 *
 * It only says which level was picked; sending it is the session's.
 */
import { computed, ref, watch } from "vue";
import ChipMenu from "./ChipMenu.vue";
import Icon from "./Icon.vue";
import { useMobile } from "./mobile";
import {
  effortFill as effortFillFor,
  effortLabel,
  effortStops as effortStopsFor,
  effortVivid as effortVividAt,
  restingStop,
} from "./reasoning-effort";
import { useI18n } from "../i18n";

const props = defineProps<{
  /** The levels on offer, as the model or the endpoint names them. */
  choices: string[];
  /** The level in effect: what the client reports, or what was last chosen. */
  live: string;
  /** The endpoint is configured by hand, so whether a level takes is up to it. */
  manual: boolean;
  /** The chip opens: no turn or approval is in the way. */
  configurable: boolean;
  /** The slider moves: a level can be sent now. */
  adjustable: boolean;
}>();
/** A level let go on; "" is the client's own default. */
const emit = defineEmits<{ pick: [stop: string] }>();
const { t } = useI18n();
const mobile = useMobile();

const effortStops = computed(() => effortStopsFor(props.choices));
/**
 * A drag is one gesture, and the request belongs at the end of it.
 *
 * Every step used to send its own change, and the chip disables itself while
 * one is in flight -- so the slider went disabled under the finger and the drag
 * ended at the second stop, every time. The thumb follows the pointer locally
 * now, and the level is sent when it is let go.
 */
const heldEffort = ref<number>();
const effortIndex = computed(() => heldEffort.value ?? restingStop(effortStops.value, props.live));
/** What the slider is pointing at: where it is being held, or where it rests. */
const effortStop = computed(() =>
  heldEffort.value === undefined ? props.live : (effortStops.value[heldEffort.value] ?? ""),
);
const effortName = computed(() => (effortStop.value ? effortLabel(effortStop.value) : t("Thinking depth")));
/** Inside the panel there is room to say what the chip cannot: that nothing has
 * been chosen and the client is deciding. */
const effortSetting = computed(() => (effortStop.value ? effortLabel(effortStop.value) : t("Client default")));
const effortFill = computed(() => effortFillFor(effortStops.value, effortIndex.value));
function dragEffort(event: Event) {
  heldEffort.value = Number((event.target as HTMLInputElement).value);
}
function commitEffort(event: Event) {
  const stop = effortStops.value[Number((event.target as HTMLInputElement).value)];
  heldEffort.value = undefined;
  if (stop === undefined) return;
  emit("pick", stop);
}
// Letting go without changing anything raises no `change` event, so the client
// reporting its level is the other thing that ends a hold.
watch(
  () => props.live,
  () => {
    heldEffort.value = undefined;
  },
);
const effortVivid = computed(() => effortVividAt(effortStops.value, effortIndex.value, effortStop.value));
</script>

<template>
  <ChipMenu
    class="chat-effort"
    :caption="t('Thinking depth')"
    :class="{ vivid: effortVivid }"
    :label="mobile && !effortStop ? '' : effortName"
    :active="!!live"
    :disabled="!configurable"
    :title="
      configurable ? t('Thinking depth') : t('Wait for the current turn and approvals to finish before changing this.')
    "
    ><template #mark><Icon :name="effortVivid ? 'spark' : 'gauge'" :size="12" /></template>
    <div class="chat-effort-body">
      <p v-if="manual" class="chat-effort-note">
        {{ t("Actual effort support depends on the endpoint and model.") }}
      </p>
      <header>
        <strong>{{ t("Thinking depth") }}</strong><em>{{ effortSetting }}</em>
      </header>
      <div class="chat-effort-slider" :style="{ '--effort-fill': effortFill }">
        <span class="chat-effort-track"
          ><i
            v-for="(stop, index) in effortStops"
            :key="stop || 'default'"
            :style="{
              left: effortStops.length < 2 ? '50%' : (index / (effortStops.length - 1)) * 100 + '%',
            }" /></span
        ><span class="chat-effort-fill" /><span class="chat-effort-thumb" /><input
          type="range"
          min="0"
          :max="effortStops.length - 1"
          step="1"
          :value="effortIndex"
          :disabled="!adjustable"
          :aria-label="t('Thinking depth')"
          :aria-valuetext="effortName"
          @input="dragEffort"
          @change="commitEffort"
        />
      </div>
      <footer>
        <span>{{ t("Faster") }}</span><span>{{ t("Smarter") }}</span>
      </footer>
    </div></ChipMenu
  >
</template>

<style scoped>
/* The pane's defaults for its controls (ChatSessionPane). Its scoped rules do
   not reach into this component, so they are repeated here, first, where the
   pane has them. */
.chat-pane button:disabled {
  cursor: not-allowed;
  opacity: 0.5;
}
.chat-pane button:focus-visible,
.chat-pane summary:focus-visible,
.chat-pane select:focus-visible,
.chat-pane input:focus-visible,
.chat-pane textarea:focus-visible {
  outline: 2px solid var(--focus);
  outline-offset: 3px;
}
@media (prefers-reduced-motion: reduce) {
  .chat-pane * {
    scroll-behavior: auto;
  }
}
.chat-effort {
  max-width: 132px;
}
.chat-effort :deep(.chip-menu-panel) {
  padding: 13px 14px 10px;
}
.chat-effort-note {
  font-size: var(--text-xs);
  color: var(--muted);
  line-height: 1.5;
  margin: 0 0 10px;
}
.chat-effort-body {
  width: 218px;
  max-width: 74cqw;
}

.chat-effort-body header {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 8px;
  margin-bottom: 11px;
}
.chat-effort-body header strong {
  font-size: var(--text-sm);
  font-weight: 600;
  color: var(--ink);
}
.chat-effort-body header em {
  font-style: normal;
  font-size: var(--text-sm);
  font-weight: 600;
  color: var(--teal);
}
.chat-effort.vivid .chat-effort-body header em {
  background: linear-gradient(92deg, var(--accent-ink), #4f7fd8);
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
}
.chat-effort-body footer {
  display: flex;
  justify-content: space-between;
  margin-top: 7px;
  font-size: var(--text-xs);
  color: var(--muted);
}
/* A real range input stays on top, invisible: dragging, clicking the track and
   arrow keys keep working while the pill below is free to be styled. */
.chat-effort-slider {
  position: relative;
  height: 26px;
  display: flex;
  align-items: center;
}
.chat-effort-track {
  position: absolute;
  inset: 0;
  border-radius: var(--radius-lg);
  background: var(--fill-hover);
}
.chat-effort-track i {
  position: absolute;
  top: 50%;
  width: 4px;
  height: 4px;
  margin: -2px 0 0 -2px;
  border-radius: 50%;
  background: var(--line-strong);
}
.chat-effort-track i:first-child {
  margin-left: 5px;
}
.chat-effort-track i:last-child {
  margin-left: -9px;
}
.chat-effort-fill {
  position: absolute;
  left: 0;
  top: 0;
  bottom: 0;
  border-radius: var(--radius-lg);
  width: calc(11px + (100% - 22px) * var(--effort-fill));
  min-width: 26px;
  background: var(--teal);
  transition:
    width 0.16s ease,
    background 0.16s ease;
}
.chat-effort-thumb {
  position: absolute;
  top: 50%;
  left: calc(11px + (100% - 22px) * var(--effort-fill));
  width: 22px;
  height: 22px;
  margin: -11px 0 0 -11px;
  border-radius: 50%;
  background: var(--surface);
  box-shadow: var(--shadow-md);
  transition: left 0.16s ease;
}
.chat-effort-slider input {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  margin: 0;
  opacity: 0;
  cursor: grab;
  -webkit-appearance: none;
  appearance: none;
}
.chat-effort-slider input:active {
  cursor: grabbing;
}
.chat-effort-slider input:disabled {
  cursor: not-allowed;
}
.chat-effort-slider input:disabled ~ .chat-effort-fill,
.chat-effort-slider input:disabled ~ .chat-effort-thumb {
  opacity: 0.45;
}
.chat-effort-slider input:focus-visible ~ .chat-effort-thumb {
  outline: 2px solid var(--focus);
  outline-offset: 2px;
}
/* The top levels cost real time and money, so they read as charged rather than
   as one more step: a gradient, a lift, and a slow pulse. */
.chat-effort.vivid .chat-effort-fill {
  background: linear-gradient(92deg, var(--accent-ink), #3f9fd0 62%, #6b6ee0);
  box-shadow: 0 0 16px #4f8fd855;
}
.chat-effort.vivid .chat-effort-thumb {
  box-shadow:
    0 1px 6px #243b4c33,
    0 0 0 3px #6b6ee02e;
  animation: effort-pulse 2.1s ease-in-out infinite;
}
@media (prefers-reduced-motion: reduce) {
  .chat-effort-fill,
  .chat-effort-thumb {
    transition: none;
  }
  .chat-effort.vivid .chat-effort-thumb {
    animation: none;
  }
}
@media (pointer: coarse) {
  /* A finger needs a taller pill to drag along. */
  .chat-effort-body {
    width: 236px;
  }
  .chat-effort-slider {
    height: 34px;
  }
  .chat-effort-track,
  .chat-effort-fill {
    border-radius: var(--radius-xl);
  }
  .chat-effort-fill {
    width: calc(14px + (100% - 28px) * var(--effort-fill));
    min-width: 34px;
  }
  .chat-effort-thumb {
    left: calc(14px + (100% - 28px) * var(--effort-fill));
    width: 28px;
    height: 28px;
    margin: -14px 0 0 -14px;
  }
}
.chat-composer.is-phone .chat-effort :deep(.chip-menu-label) {
  max-width: 4.5em;
}
/* Depth: the slider at full width, sized for a thumb. */
.chat-composer.is-phone .chat-effort-body {
  width: auto;
  max-width: none;
  padding: 8px 12px 10px;
}
.chat-composer.is-phone .chat-effort-body header strong,
.chat-composer.is-phone .chat-effort-body header em {
  font-size: var(--text-lg);
}
.chat-composer.is-phone .chat-effort-body footer {
  font-size: var(--text-sm);
}
@keyframes effort-pulse {
  0%,
  100% {
    box-shadow:
      0 1px 6px #243b4c33,
      0 0 0 3px #6b6ee02e;
  }
  50% {
    box-shadow:
      0 1px 6px #243b4c33,
      0 0 0 7px #6b6ee014;
  }
}
</style>
