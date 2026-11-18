<script setup lang="ts">
/**
 * A menu item with a submenu beside it, for any of the app's menus (a
 * ContextMenu, the "+" menu): the row plus its flyout, both rendered as the
 * menu's own children so they take its item styles.
 *
 * The flyout opens on hover, on the right arrow, or on a click when the row
 * has nothing of its own to do. A row that does (`@activate`, as "New Claude
 * Code session" creating the last-used kind) runs it on a click, and offers
 * the rest beside it. A short delay lets the pointer travel across to the
 * flyout; the left arrow or Esc goes back to the row. Where the menu is a
 * phone's bottom sheet, the flyout opens inline below the row instead.
 */
import { computed, nextTick, onBeforeUnmount, ref } from "vue";
import Icon from "./Icon.vue";

// `@activate` arrives as this prop, so the row can tell whether it has an action of its own.
// `value` is what the row is currently set to, shown before the arrow.
const props = withDefaults(
  defineProps<{ label: string; icon?: string; value?: string; width?: number; onActivate?: () => void }>(),
  { width: 190 },
);
defineOptions({ inheritAttrs: false });

const open = ref(false),
  trigger = ref<HTMLButtonElement>(),
  flyout = ref<HTMLElement>();
const position = ref({ left: 0, top: 0 });
const style = computed(() => ({
  left: position.value.left + "px",
  top: position.value.top + "px",
  minWidth: props.width + "px",
}));
let timer: ReturnType<typeof setTimeout> | undefined;

async function show(focus: boolean) {
  clearTimeout(timer);
  const row = trigger.value?.getBoundingClientRect();
  if (row) {
    const left = row.right + 4 + props.width <= window.innerWidth - 8 ? row.right + 4 : row.left - 4 - props.width;
    position.value = { left: Math.max(4, left), top: Math.max(4, row.top - 5) };
  }
  open.value = true;
  await nextTick();
  // Kept on screen once its real height is known.
  const box = flyout.value?.getBoundingClientRect();
  if (box && box.bottom > window.innerHeight - 8)
    position.value = { ...position.value, top: Math.max(4, window.innerHeight - 8 - box.height) };
  if (focus) flyout.value?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
}
function keep() {
  clearTimeout(timer);
}
function leave() {
  clearTimeout(timer);
  timer = setTimeout(() => {
    open.value = false;
  }, 180);
}
function hide(focusTrigger: boolean) {
  clearTimeout(timer);
  open.value = false;
  if (focusTrigger) trigger.value?.focus();
}
function click() {
  if (props.onActivate) props.onActivate();
  else void show(true);
}
onBeforeUnmount(() => clearTimeout(timer));
</script>

<template>
  <button
    ref="trigger"
    type="button"
    role="menuitem"
    class="menu-flyout-trigger"
    aria-haspopup="menu"
    :aria-expanded="open"
    v-bind="$attrs"
    @click="click"
    @mouseenter="show(false)"
    @mouseleave="leave"
    @keydown.right.prevent="show(true)"
  >
    <slot name="icon"><Icon v-if="icon" :name="icon" :size="14" /></slot
    ><span class="menu-flyout-label">{{ label }}</span
    ><small v-if="value" class="menu-flyout-value">{{ value }}</small
    ><Icon name="chevron" :size="13" class="menu-flyout-chevron" />
  </button>
  <nav
    v-if="open"
    ref="flyout"
    class="menu-flyout"
    role="menu"
    :aria-label="label"
    :style="style"
    @mouseenter="keep"
    @mouseleave="leave"
    @keydown.left.stop.prevent="hide(true)"
    @keydown.esc.stop.prevent="hide(true)"
  >
    <slot />
  </nav>
</template>

<style>
/* A plain menu row wherever the menu has no row style of its own; :where keeps it
   below any menu's own (.context-menu>button, the "+" menu's). */
:where(.menu-flyout-trigger) {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  min-height: 30px;
  padding: 6px 10px;
  border: 0;
  border-radius: var(--radius-sm);
  background: none;
  text-align: left;
  font: inherit;
  font-size: var(--text-sm);
  color: var(--ink-soft);
  white-space: nowrap;
  cursor: pointer;
}
:where(.menu-flyout-trigger):hover {
  background: var(--fill);
  color: var(--ink);
}
/* Size and colour at class strength: the app's own button rule otherwise wins over
   :where and left the row a size larger than the menu's others. A menu with its own
   row style (.context-menu>button) still outranks this. */
.menu-flyout-trigger {
  font-size: var(--text-sm);
  color: var(--ink-soft);
}
.menu-flyout-trigger:hover {
  color: var(--ink);
}
.menu-flyout-trigger[aria-expanded="true"] {
  background: var(--fill);
  color: var(--ink);
}
.menu-flyout-label {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}
.menu-flyout-chevron {
  flex: none;
  margin-left: 8px;
  color: var(--faint) !important;
}
.menu-flyout-value {
  flex: none;
  margin-left: 8px;
  font-size: var(--text-xs);
  color: var(--muted);
}
/* Looks like .context-menu, without its phone bottom-sheet rules (styles.css). */
.menu-flyout {
  position: fixed;
  z-index: 71;
  padding: 5px;
  background: var(--surface);
  border: 1px solid var(--line);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-lg);
}
.menu-flyout > button {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  min-height: 30px;
  padding: 6px 10px;
  border: 0;
  border-radius: var(--radius-sm);
  background: none;
  text-align: left;
  font-size: var(--text-sm);
  color: var(--ink-soft);
  white-space: nowrap;
  cursor: pointer;
}
.menu-flyout > button > svg {
  flex: none;
}
.menu-flyout > button:hover:not(:disabled),
.menu-flyout > button:focus-visible {
  background: var(--fill);
  color: var(--ink);
  outline: none;
}
.menu-flyout > button:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
.menu-flyout > hr {
  border: 0;
  border-top: 1px solid var(--line);
  margin: 4px 6px;
}
.menu-flyout > p {
  margin: 4px 10px 3px;
  max-width: 220px;
  font-size: var(--text-xs);
  line-height: 1.45;
  color: var(--muted);
  white-space: normal;
}
@media (max-width: 760px) {
  .menu-flyout {
    position: static;
    min-width: 0 !important;
    padding: 0 0 0 22px;
    border: 0;
    box-shadow: none;
    background: none;
  }
}
@media (pointer: coarse) {
  .menu-flyout > button {
    min-height: 44px;
  }
}
</style>
