<script setup lang="ts">
/**
 * A menu opened at the pointer by a right-click.
 *
 * Teleported to the body so no overflow clipping can cut it off, with a
 * backdrop so any press elsewhere -- another right-click included -- closes it.
 * Items are plain <button>s and <hr>s in the default slot, with
 * .context-menu-label headings and .context-menu-note text; an item closes the
 * menu itself by calling the owner's close. Every right-click and long-press
 * menu in the app is this one, so they share keys, look and, on a phone, the
 * bottom sheet they become (styles.css), titled by `label`.
 */
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from "vue";

const props = withDefaults(defineProps<{ x: number; y: number; label: string; width?: number }>(), { width: 200 });
const emit = defineEmits<{ close: [] }>();
const menu = ref<HTMLElement>();
const height = ref(0);
const style = computed(() => ({
  left: Math.max(4, Math.min(props.x, window.innerWidth - props.width - 8)) + "px",
  top: Math.max(4, Math.min(props.y, window.innerHeight - height.value - 8)) + "px",
  minWidth: props.width + "px",
}));
function items() { return Array.from(menu.value?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? []); }
function move(event: KeyboardEvent) {
  const list = items();
  if (event.key === "Home" || event.key === "End") { event.preventDefault(); (event.key === "Home" ? list[0] : list[list.length - 1])?.focus(); return; }
  if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
  event.preventDefault();
  const index = list.indexOf(document.activeElement as HTMLButtonElement);
  list[(index + (event.key === "ArrowDown" ? 1 : -1) + list.length) % list.length]?.focus();
}
onMounted(async () => { await nextTick(); height.value = menu.value?.offsetHeight ?? 0; items()[0]?.focus({ preventScroll: true }); });
/** Closing hands the focus back to whatever had it, unless an item sent it elsewhere. */
const returnTo = typeof document === "undefined" ? null : document.activeElement as HTMLElement | null;
onBeforeUnmount(() => {
  const now = document.activeElement;
  if (!now || now === document.body || menu.value?.contains(now)) void nextTick(() => { if (returnTo?.isConnected) returnTo.focus({ preventScroll: true }); });
});
</script>

<template>
  <Teleport to="body">
    <div class="context-menu-backdrop" @pointerdown="emit('close')" @contextmenu.prevent="emit('close')">
      <nav ref="menu" class="context-menu" :style="style" role="menu" :aria-label="label" @pointerdown.stop @contextmenu.prevent.stop @keydown.esc.stop.prevent="emit('close')" @keydown="move"><div class="context-menu-title" aria-hidden="true">{{ label }}</div><slot /></nav>
    </div>
  </Teleport>
</template>

<style>
.context-menu-backdrop{position:fixed;inset:0;z-index:70}
.context-menu{position:fixed;max-height:calc(100vh - 16px);overflow:auto;padding:5px;background:var(--surface);border:1px solid var(--line);border-radius:var(--radius-md);box-shadow:var(--shadow-lg)}
.context-menu>button{display:flex;align-items:center;gap:8px;width:100%;min-height:30px;padding:6px 10px;border:0;border-radius:var(--radius-sm);background:none;text-align:left;font-size:var(--text-sm);color:var(--ink-soft);white-space:nowrap;cursor:pointer}
.context-menu>button>svg{flex:none;color:var(--violet)}
.context-menu>button:hover:not(:disabled),.context-menu>button:focus-visible{background:var(--fill);color:var(--ink);outline:none}
.context-menu>button:disabled{opacity:.4;cursor:not-allowed}
.context-menu>button.danger{color:var(--danger)}.context-menu>button.danger:hover:not(:disabled){background:var(--danger-soft);color:var(--danger)}
.context-menu>hr{border:0;border-top:1px solid var(--line);margin:4px 6px}
.context-menu>.context-menu-note{max-width:240px;margin:0;padding:6px 10px;font-size:var(--text-xs);line-height:1.6;color:var(--ink-soft);white-space:normal}
.context-menu>.context-menu-title{display:none}
@media(max-width:760px){.context-menu>.context-menu-title{display:block;overflow:hidden;padding:4px 12px 8px;font-size:var(--text-sm);font-weight:600;color:var(--muted);text-overflow:ellipsis;white-space:nowrap}.context-menu>.context-menu-note{max-width:none;font-size:var(--text-sm)}}
.context-menu>.context-menu-label{padding:5px 10px 3px;font-size:var(--text-xs);font-weight:600;color:var(--faint)}
@media(pointer:coarse){.context-menu>button{min-height:44px}}
</style>
