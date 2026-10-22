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
 * On a phone the panel is a card as wide as the pane, right above the composer
 * (or below the top bar, for a control that lives there). Only one is ever
 * open and nothing dims behind it: a stack of sheets, each over the last, is
 * where a tap meant for one used to land on another.
 */
import { nextTick, onBeforeUnmount, onMounted, ref } from 'vue';
import { chipPanelOffset } from './chip-menu-position';
import Icon from './Icon.vue';
import { useMobile } from './mobile';

const props = defineProps<{
  /** The chip's own text. Truncated rather than wrapped: the row is one line. */
  label: string;
  title?: string;
  /** Marks the control as carrying a deliberate, non-default choice. */
  active?: boolean;
  /** A chip whose current setting is worth being uneasy about. */
  tone?: 'danger';
  disabled?: boolean;
  /** What the control is, when the label alone says only what it is set to. */
  caption?: string;
  /** Which way a phone's card opens: up from the composer, or down from the top bar. */
  placement?: 'up' | 'down';
}>();
const mobile = useMobile();
const root = ref<HTMLDetailsElement>();
const panel = ref<HTMLElement>();
/** Measured when it opens rather than guessed at from the chip's position: how
 * wide the panel is depends on what the caller slotted into it. */
const position = ref({ left: 0, bottom: 0 });
/** A phone's card: the pane's width, and the room between the composer and the top bar. */
const card = ref<Record<string, string>>();
defineExpose({ close, toggle });

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
  if (!menu?.open) return;
  await nextTick();
  const pane = menu.closest('.dock-pane, .chat-pane')?.getBoundingClientRect();
  const left = Math.max(0, pane?.left ?? 0), right = Math.min(window.innerWidth, pane?.right ?? window.innerWidth);
  // A fixed panel is placed against its containing block: the window,
  // unless an ancestor with a transform, filter or containment captures it.
  const frame = containingBlock(menu);
  const origin = frame ? frame.getBoundingClientRect() : { left: 0, top: 0, bottom: window.innerHeight };
  if (mobile.value) { placeCard(menu, left, right, frame, origin); return; }
  card.value = undefined;
  const chip = menu.querySelector('summary')?.getBoundingClientRect(), width = panel.value?.offsetWidth ?? 0;
  if (!chip) return;
  const x = chip.left + chipPanelOffset(chip.left - left, width, right - left);
  position.value = { left: x - origin.left - (frame?.clientLeft ?? 0), bottom: origin.bottom - (frame ? frame.offsetHeight - frame.clientHeight - frame.clientTop : 0) - chip.top + 8 };
}
function placeCard(menu: HTMLElement, left: number, right: number, frame: HTMLElement | undefined, origin: { left: number; top: number; bottom: number }) {
  const inset = 10, gap = 8;
  const height = window.visualViewport?.height ?? window.innerHeight;
  const top = (document.querySelector('.topbar')?.getBoundingClientRect().bottom ?? 0) + gap;
  const composer = (menu.closest('.chat-composer') ?? menu).getBoundingClientRect();
  const geometry: Record<string, string> = { left: `${left + inset - origin.left - (frame?.clientLeft ?? 0)}px`, width: `${Math.max(0, right - left - inset * 2)}px`, minWidth: '0', maxWidth: 'none' };
  if (props.placement === 'down') {
    geometry.top = `${top - origin.top - (frame?.clientTop ?? 0)}px`; geometry.bottom = 'auto';
    geometry.maxHeight = `${Math.max(160, Math.min(composer.top, height) - top - gap)}px`;
  } else {
    geometry.bottom = `${origin.bottom - (frame ? frame.offsetHeight - frame.clientHeight - frame.clientTop : 0) - composer.top + gap}px`; geometry.top = 'auto';
    geometry.maxHeight = `${Math.max(160, composer.top - top - gap)}px`;
  }
  card.value = geometry;
}
function containingBlock(element: Element): HTMLElement | undefined {
  for (let node = element.parentElement; node; node = node.parentElement) {
    const style = getComputedStyle(node);
    // A frosted backdrop captures it as well.
    const backdrop = style.backdropFilter || style.getPropertyValue('-webkit-backdrop-filter');
    if (style.transform !== 'none' || style.filter !== 'none' || style.perspective !== 'none' || (backdrop && backdrop !== 'none') || /layout|paint|strict|content/.test(style.contain)) return node;
  }
  return undefined;
}
function close(focus = false) {
  const menu = root.value;
  if (!menu?.open) return;
  menu.open = false;
  // A phone has no focus ring to return to, and focusing the chip would
  // only scroll the page under the keyboard.
  if (focus && !mobile.value) menu.querySelector<HTMLElement>('summary')?.focus();
}
/** Opened from somewhere else: the top bar on a phone, for a chip it does not show. */
function toggle() {
  const menu = root.value;
  if (menu && menu.dataset.locked !== 'true') menu.open = !menu.open;
}
function onOutside(event: PointerEvent) {
  const menu = root.value;
  if (menu?.open && !menu.contains(event.target as Node)) menu.open = false;
}
/** A disabled chip must not open; `<summary>` has no disabled attribute. */
function guard(event: Event) {
  const menu = root.value;
  if (menu && !menu.open && menu.dataset.locked === 'true') event.preventDefault();
}
// Scrolling anything -- the chip row, the page -- moves the chip; the panel follows.
// A phone keyboard coming or going moves the composer, and the card with it.
onMounted(() => { document.addEventListener('pointerdown', onOutside); window.addEventListener('resize', place); window.visualViewport?.addEventListener('resize', place); document.addEventListener('scroll', place, true); });
onBeforeUnmount(() => { document.removeEventListener('pointerdown', onOutside); window.removeEventListener('resize', place); window.visualViewport?.removeEventListener('resize', place); document.removeEventListener('scroll', place, true); });
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
    <div ref="panel" :class="['chip-menu-panel', { 'is-card': mobile, 'is-down': mobile && placement === 'down' }]" :style="card ?? { '--chip-left': position.left + 'px', '--chip-bottom': position.bottom + 'px' }"><slot :close="close" /></div>
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
.chip-menu-caret{transform:rotate(90deg);flex:0 0 auto;opacity:.55}
.chip-menu[open] .chip-menu-caret{transform:rotate(-90deg)}
.chip-menu-panel{position:fixed;bottom:var(--chip-bottom);left:var(--chip-left);z-index:60;background:var(--surface);border:1px solid var(--border);border-radius:var(--radius-lg);box-shadow:var(--shadow-lg),inset 0 1px 0 var(--card-highlight)}
.chip-menu-panel.is-card{overflow-y:auto;overscroll-behavior:contain;padding:6px;border-radius:var(--radius-xl);box-shadow:var(--shadow-lg),inset 0 1px 0 var(--card-highlight);animation:chip-card-in .18s cubic-bezier(.2,.8,.2,1)}
@keyframes chip-card-in{from{opacity:0;transform:translateY(6px)}}
.chip-menu-panel.is-card.is-down{animation-name:chip-card-down}
@keyframes chip-card-down{from{opacity:0;transform:translateY(-6px)}}
@media(prefers-reduced-motion:reduce){.chip-menu-panel.is-card{animation:none}}
/* Density follows the pointing device, not the pane width: a narrow pane on a
   desktop is still a mouse. */
@media(pointer:coarse){.chip-menu>summary{min-height:44px;border-radius:var(--radius-md);padding:0 10px;font-size:var(--text-xs)}}
</style>
