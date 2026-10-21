<script setup lang="ts">
/**
 * A sheet that rises from the bottom of a phone screen.
 *
 * Menus and pickers anchored to a small control work with a mouse; with a
 * thumb they open off-screen or under the finger. A sheet is where a phone
 * puts that kind of choice: full width, within reach, dismissed by a tap
 * outside, Escape, or dragging it down by its handle.
 */
import { nextTick, onMounted, onUnmounted, ref } from "vue";
import { useI18n } from "../i18n";
const { t } = useI18n();
/** `sheetClass` lets a sheet carry its opener's classes, so content styled
 * under that opener (`.chat-model nav button`) looks the same in the sheet. */
defineProps<{ title?: string; sheetClass?: unknown }>();
const emit = defineEmits<{ close: []; opened: [] }>();
const sheet = ref<HTMLElement>();
const shown = ref(false);
const offset = ref(0);
// Nested sheets can be teleported before their parent in DOM order.
// Give each mounted sheet a higher layer than the sheets already open.
const layer = ref(90);
let startY: number | undefined;
let previousFocus: HTMLElement | null = null;

function close() { shown.value = false; }
function afterLeave() { emit("close"); }
function key(event: KeyboardEvent) { if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); close(); } }
function dragStart(event: PointerEvent) {
  startY = event.clientY; offset.value = 0;
  (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
}
function dragMove(event: PointerEvent) { if (startY !== undefined) offset.value = Math.max(0, event.clientY - startY); }
function dragEnd() {
  if (startY === undefined) return;
  startY = undefined;
  // Past a third of its height, or a clear flick, the sheet goes.
  if (offset.value > Math.min(120, (sheet.value?.offsetHeight ?? 300) / 3)) close();
  offset.value = 0;
}
onMounted(() => {
  layer.value = Math.max(90, ...Array.from(document.querySelectorAll<HTMLElement>(".bottom-sheet"), element => Number(getComputedStyle(element).zIndex) || 90)) + 2;
  previousFocus = document.activeElement as HTMLElement | null;
  shown.value = true;
  void nextTick(() => emit("opened"));
  requestAnimationFrame(() => sheet.value?.focus({ preventScroll: true }));
});
onUnmounted(() => previousFocus?.focus?.({ preventScroll: true }));
defineExpose({ close });
</script>

<template>
  <Teleport to="body">
    <Transition name="sheet-fade"><div v-if="shown" class="sheet-backdrop" :style="{ zIndex: layer }" @click="close" /></Transition>
    <Transition name="sheet-rise" @after-leave="afterLeave">
      <section v-if="shown" ref="sheet" :class="['bottom-sheet', sheetClass]" role="dialog" aria-modal="true" :aria-label="title" tabindex="-1" :style="{ zIndex: layer + 1, ...(offset ? { transform: `translateY(${offset}px)`, transition: 'none' } : {}) }" @keydown="key">
        <div class="sheet-grip" :aria-label="t('Drag down to close')" @pointerdown="dragStart" @pointermove="dragMove" @pointerup="dragEnd" @pointercancel="dragEnd"><i /></div>
        <header v-if="title || $slots.header" class="sheet-header"><slot name="header"><h2>{{ title }}</h2></slot></header>
        <div class="sheet-body"><slot :close="close" /></div>
      </section>
    </Transition>
  </Teleport>
</template>

<style>
.sheet-backdrop{position:fixed;inset:0;z-index:90;background:var(--overlay)}
.bottom-sheet{position:fixed;left:0;right:0;bottom:0;z-index:91;display:flex;flex-direction:column;max-height:min(82dvh,calc(var(--app-height,100dvh) - 40px));background:var(--surface);border-radius:var(--radius-xl) var(--radius-xl) 0 0;box-shadow:0 -8px 40px #1b2a3624;padding-bottom:env(safe-area-inset-bottom);outline:0;transition:transform .26s cubic-bezier(.2,.8,.2,1)}
.sheet-grip{display:flex;justify-content:center;padding:9px 0 6px;flex:none;touch-action:none;cursor:grab}
.sheet-grip>i{display:block;width:38px;height:5px;border-radius:var(--radius-xs);background:var(--line)}
.sheet-header{flex:none;padding:2px 20px 10px}
.sheet-header h2{font-size:var(--text-lg);font-weight:600;color:var(--ink);margin:0}
.sheet-body{flex:1;min-height:0;overflow-y:auto;overscroll-behavior:contain;padding:0 10px 12px}
.sheet-fade-enter-active,.sheet-fade-leave-active{transition:opacity .22s ease}
.sheet-fade-enter-from,.sheet-fade-leave-to{opacity:0}
.sheet-rise-enter-active,.sheet-rise-leave-active{transition:transform .26s cubic-bezier(.2,.8,.2,1)}
.sheet-rise-enter-from,.sheet-rise-leave-to{transform:translateY(100%)}
/* Rows a sheet is usually made of: one choice per line, thumb-high. */
.sheet-row{display:flex;align-items:center;gap:12px;width:100%;min-height:52px;padding:8px 12px;border:0;border-radius:var(--radius-lg);background:none;color:var(--ink);font:inherit;font-size:var(--text-lg);text-align:left;cursor:pointer}
.sheet-row:active,.sheet-row.current{background:var(--teal-soft)}
.sheet-row.current{color:var(--teal-deep);font-weight:600}
.sheet-row>svg{flex:none;color:var(--ink-soft)}
.sheet-row-copy{flex:1;min-width:0;display:flex;flex-direction:column;gap:2px}
.sheet-row-copy>strong{font-weight:inherit;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.sheet-row-copy>small{font-size:var(--text-sm);color:var(--ink-soft);font-weight:400;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.sheet-section{margin:14px 12px 6px;font-size:var(--text-sm);font-weight:600;letter-spacing:.3px;color:var(--ink-soft)}
.sheet-divider{height:1px;margin:8px 12px;background:var(--border);border:0}
</style>
