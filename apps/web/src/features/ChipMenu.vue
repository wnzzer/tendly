<script setup lang="ts">
/**
 * A compact labelled chip that opens a panel above itself.
 *
 * This exists because the composer mixed a native `<select>` with hand-built
 * pop-ups: the OS chrome never matched the row around it. Every chip-shaped
 * control in the app is this component, so they open, close and size alike.
 *
 * The panel is whatever the caller slots in — a list of choices, a slider, a
 * form. Dismissal (pointer outside, Escape, choosing something) is handled here
 * once rather than re-implemented per control.
 *
 * On a phone the panel is a bottom sheet instead: a panel pinned above a
 * 44px chip leaves no room for a thumb, and a sheet is where a phone expects
 * a choice to appear.
 */
import { nextTick, onBeforeUnmount, onMounted, ref } from 'vue';
import { chipPanelOffset } from './chip-menu-position';
import Icon from './Icon.vue';
import BottomSheet from './BottomSheet.vue';
import { useMobile } from './mobile';

defineProps<{
  /** The chip's own text. Truncated rather than wrapped: the row is one line. */
  label: string;
  title?: string;
  /** Marks the control as carrying a deliberate, non-default choice. */
  active?: boolean;
  /** A chip whose current setting is worth being uneasy about. */
  tone?: 'danger';
  disabled?: boolean;
  /** What the control is, for a sheet's heading and a settings row. */
  caption?: string;
}>();
const mobile = useMobile();
const opened = ref(false);
const sheet = ref<InstanceType<typeof BottomSheet>>();
const root = ref<HTMLDetailsElement>();
const panel = ref<HTMLElement>();
/** Measured when it opens rather than guessed at from the chip's position: how
 * wide the panel is depends on what the caller slotted into it. */
const position = ref({ left: 0, bottom: 0 });
defineExpose({ close });

/** A label cut short slides sideways under the pointer to show the rest, then back. */
const labelBox = ref<HTMLElement>(), labelScroll = ref(0);
function revealLabel() {
  const box = labelBox.value;
  labelScroll.value = box ? box.scrollWidth - box.clientWidth : 0;
  if (labelScroll.value < 2) labelScroll.value = 0;
}

/**
 * The panel is fixed to the window, above its chip. A row of chips can then
 * scroll sideways at any width without clipping the panel it opens, which an
 * absolutely placed panel inside that row could not avoid. It still stays
 * within its pane: the pane is what the eye reads as the edge.
 */
async function place() {
  const menu = root.value;
  opened.value = !!menu?.open;
  if (!menu?.open || mobile.value) return;
  await nextTick();
  const chip = menu.querySelector('summary')?.getBoundingClientRect(), width = panel.value?.offsetWidth ?? 0;
  if (!chip) return;
  const pane = menu.closest('.dock-pane, .chat-pane')?.getBoundingClientRect();
  const left = Math.max(0, pane?.left ?? 0), right = Math.min(window.innerWidth, pane?.right ?? window.innerWidth);
  // A fixed panel is placed against its containing block: the window,
  // unless an ancestor with a transform, filter or containment captures it.
  const frame = containingBlock(menu);
  const origin = frame ? frame.getBoundingClientRect() : { left: 0, bottom: window.innerHeight };
  const x = chip.left + chipPanelOffset(chip.left - left, width, right - left);
  position.value = { left: x - origin.left - (frame?.clientLeft ?? 0), bottom: origin.bottom - (frame ? frame.offsetHeight - frame.clientHeight - frame.clientTop : 0) - chip.top + 8 };
}
function containingBlock(element: Element): HTMLElement | undefined {
  for (let node = element.parentElement; node; node = node.parentElement) {
    const style = getComputedStyle(node);
    if (style.transform !== 'none' || style.filter !== 'none' || style.perspective !== 'none' || /layout|paint|strict|content/.test(style.contain)) return node;
  }
  return undefined;
}
function close(focus = false) {
  // The sheet slides away first and then closes the menu (`finish`).
  if (mobile.value && opened.value && sheet.value) { sheet.value.close(); return; }
  finish(focus);
}
function finish(focus = false) {
  const menu = root.value;
  if (!menu?.open) return;
  menu.open = false;
  if (focus) menu.querySelector<HTMLElement>('summary')?.focus();
}
function onOutside(event: PointerEvent) {
  const menu = root.value;
  // The sheet lives in the body, outside the menu, and closes itself.
  if (menu?.open && !menu.contains(event.target as Node) && !(event.target as Element).closest?.('.bottom-sheet, .sheet-backdrop')) menu.open = false;
}
/** A disabled chip must not open; `<summary>` has no disabled attribute. */
function guard(event: Event) {
  const menu = root.value;
  if (menu && !menu.open && menu.dataset.locked === 'true') event.preventDefault();
}
// Scrolling anything -- the chip row, the page -- moves the chip; the panel follows.
onMounted(() => { document.addEventListener('pointerdown', onOutside); window.addEventListener('resize', place); document.addEventListener('scroll', place, true); });
onBeforeUnmount(() => { document.removeEventListener('pointerdown', onOutside); window.removeEventListener('resize', place); document.removeEventListener('scroll', place, true); });
</script>

<template>
  <details ref="root" class="chip-menu" :class="{ 'is-active': active, 'is-disabled': disabled, 'is-danger': tone === 'danger' }" :data-locked="disabled ? 'true' : 'false'" @keydown.esc.stop.prevent="close(true)" @toggle="place">
    <summary :title="title ?? label" :aria-disabled="disabled ? 'true' : undefined" @click="guard" @mouseenter="revealLabel" @mouseleave="labelScroll = 0">
      <slot name="mark" />
      <span v-if="caption" class="chip-menu-caption">{{ caption }}</span>
      <span ref="labelBox" :class="['chip-menu-label', { scrolling: labelScroll > 0 }]" :style="labelScroll ? { '--chip-scroll': -labelScroll + 'px', '--chip-scroll-time': Math.max(1.6, labelScroll / 30) + 's' } : undefined" :data-repeats="label === caption ? 'true' : undefined"><span>{{ label }}</span></span>
      <slot name="badge" />
      <Icon class="chip-menu-caret" name="chevron" :size="12" />
    </summary>
    <div v-if="!mobile" ref="panel" class="chip-menu-panel" :style="{ '--chip-left': position.left + 'px', '--chip-bottom': position.bottom + 'px' }"><slot :close="close" /></div>
    <BottomSheet v-else-if="opened" ref="sheet" :sheet-class="['chip-menu-sheet', $attrs.class]" :title="caption ?? title ?? label" @close="finish()"><slot :close="close" /></BottomSheet>
  </details>
</template>

<style scoped>
.chip-menu{position:relative;flex:0 1 auto;min-width:0}
/* The border is always there and usually invisible, so a chip that lights up
   does not shift the row by a pixel as it does. */
.chip-menu>summary{list-style:none;display:flex;align-items:center;gap:5px;min-height:28px;max-width:100%;padding:0 8px;border:1px solid transparent;border-radius:var(--radius-sm);background:var(--fill);font-size:var(--text-xs);color:var(--ink-soft);cursor:pointer;white-space:nowrap}
.chip-menu>summary::-webkit-details-marker{display:none}
.chip-menu>summary:hover{background:var(--fill-hover)}
.chip-menu[open]>summary{background:var(--border);color:var(--ink)}
.chip-menu.is-disabled>summary{cursor:not-allowed;opacity:.55}
/* A set control is legible as set from across the row, not only once read:
   colour alone is a weak signal among four chips that otherwise match. */
.chip-menu.is-active>summary{color:var(--accent-ink);font-weight:600;background:var(--fill);border-color:var(--line)}
.chip-menu.is-danger>summary,.chip-menu.is-danger.is-active>summary{color:#a85c4e;background:var(--danger-soft);border-color:#e6b5ad}
.chip-menu.is-danger[open]>summary{background:var(--danger-soft);color:#8f4b3e}
/* The mark names the control; the label says what it is set to. Together they
   are what tells four chips apart at a glance. */
.chip-menu>summary>svg{flex:0 0 auto;opacity:.8}
.chip-menu.is-active>summary>svg,.chip-menu.is-danger>summary>svg{opacity:1}
.chip-menu-label{overflow:hidden;text-overflow:ellipsis}
.chip-menu-label.scrolling{text-overflow:clip;-webkit-mask-image:linear-gradient(90deg,transparent,currentColor 6px,currentColor calc(100% - 6px),transparent);mask-image:linear-gradient(90deg,transparent,currentColor 6px,currentColor calc(100% - 6px),transparent)}
.chip-menu-label.scrolling>span{display:inline-block;animation:chip-label-scroll var(--chip-scroll-time) ease-in-out .3s infinite alternate}
@keyframes chip-label-scroll{0%,12%{transform:none}88%,100%{transform:translateX(var(--chip-scroll))}}
@media(prefers-reduced-motion:reduce){.chip-menu-label.scrolling>span{animation:none}}
.chip-menu-caption{display:none}
/* Inside a settings sheet (see .chip-rows) each chip is a full-width row:
   what it is on the left, what it is set to on the right. */
.chip-rows .chip-menu{width:100%}
.chip-rows .chip-menu>summary{min-height:54px;gap:12px;padding:0 12px;border-radius:var(--radius-lg);background:none;border-color:transparent;font-size:var(--text-lg);color:var(--ink)}
.chip-rows .chip-menu>summary:active,.chip-rows .chip-menu[open]>summary{background:var(--selected)}
.chip-rows .chip-menu>summary>svg:first-child{width:18px;height:18px}
.chip-rows .chip-menu-caption{display:block;flex:1;min-width:0;font-weight:400;overflow:hidden;text-overflow:ellipsis}
.chip-rows .chip-menu-label{flex:0 1 auto;max-width:55%;color:var(--ink-soft);font-size:var(--text-base);font-weight:400}
.chip-rows .chip-menu.is-active .chip-menu-label{color:var(--teal-deep);font-weight:600}
.chip-rows .chip-menu.is-danger .chip-menu-label{color:#a85c4e;font-weight:600}
.chip-rows .chip-menu.is-active>summary,.chip-rows .chip-menu.is-danger>summary{background:none;border-color:transparent}
.chip-rows .chip-menu-label[data-repeats="true"]{display:none}
.chip-rows .chip-menu-caret,.chip-rows .chip-menu[open] .chip-menu-caret{transform:none}
.chip-menu-caret{transform:rotate(90deg);flex:0 0 auto;opacity:.55}
.chip-menu[open] .chip-menu-caret{transform:rotate(-90deg)}
.chip-menu-panel{position:fixed;bottom:var(--chip-bottom);left:var(--chip-left);z-index:60;background:var(--surface);border:1px solid var(--border);border-radius:var(--radius-lg);box-shadow:var(--shadow-lg),inset 0 1px 0 var(--card-highlight)}
/* Density follows the pointing device, not the pane width: a narrow pane on a
   desktop is still a mouse. */
@media(pointer:coarse){.chip-menu>summary{min-height:44px;border-radius:var(--radius-md);padding:0 10px;font-size:var(--text-xs)}}
</style>
