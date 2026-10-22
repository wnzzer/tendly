<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import Icon from "./Icon.vue";
import { useI18n } from "../i18n";
import { backLabel } from "./page-back";

/**
 * A page over the canvas: a title, a way back, and a body that scrolls on its
 * own. Esc goes back, unless something inside (a menu, a dialog, a field's own
 * Esc) has already used it.
 */
// `namedAbove`: on a phone the top bar already shows this title, so the page
// does not say it a second time.
defineProps<{ title: string; subtitle?: string; namedAbove?: boolean }>();
const emit = defineEmits<{ back: [] }>();
const { t } = useI18n();
const page = ref<HTMLElement>();
const router = useRouter();
// The history entry is already this page's when it mounts; `back` names the one before.
const previous = typeof window === "undefined" ? undefined : window.history.state?.back;
const back = t(backLabel(typeof previous === "string" && router ? router.resolve(previous).name : undefined));
function onKey(event: KeyboardEvent) {
  if (event.key !== "Escape" || event.defaultPrevented) return;
  // An open menu closes first. Other open <details> -- a tool call expanded in a
  // conversation behind the page, an "Advanced" section -- are not in the way.
  if (document.querySelector(".modal-backdrop, .bottom-sheet-backdrop, details[open]:is(.chip-menu, .session-menu, .new-session-more, .dock-layout-menu, .dock-add-menu, .account-more, .chat-menu)")) return;
  event.preventDefault();
  // From a field, Esc only leaves it; the next one goes back.
  if (event.target instanceof Element && event.target.closest("input, textarea, select, [contenteditable='true']")) { page.value?.focus({ preventScroll: true }); return; }
  emit("back");
}
onMounted(() => { document.addEventListener("keydown", onKey); page.value?.focus({ preventScroll: true }); });
onBeforeUnmount(() => document.removeEventListener("keydown", onKey));
</script>

<template>
  <section ref="page" class="page-shell" tabindex="-1" :aria-label="title">
    <header class="page-header">
      <button type="button" class="page-back" :title="back + ' · Esc'" @click="emit('back')"><Icon name="back" :size="16" /><span>{{ back }}</span></button>
      <div class="page-heading" :class="{ 'named-above': namedAbove }"><h1>{{ title }}</h1><p v-if="subtitle">{{ subtitle }}</p></div>
      <div class="page-actions"><slot name="actions" /></div>
    </header>
    <div class="page-body"><slot /></div>
  </section>
</template>

<style scoped>
.page-shell:focus,.page-shell:focus-visible{outline:none}
.page-shell{position:absolute;inset:0;z-index:20;display:flex;flex-direction:column;min-height:0;background:var(--backdrop-glow) no-repeat,var(--bg);outline:0;animation:page-in var(--duration-normal) var(--ease-out)}
.page-header{display:flex;align-items:center;gap:var(--space-4);flex-wrap:wrap;padding:var(--space-4) var(--space-6) var(--space-3);max-width:1180px;width:100%;margin:0 auto}
.page-back{display:inline-flex;align-items:center;gap:6px;min-height:32px;padding:0 10px 0 6px;border:0;border-radius:var(--radius-md);background:none;color:var(--ink-soft);font-size:var(--text-sm);cursor:pointer}
.page-back:hover{background:var(--fill);color:var(--ink)}
.page-heading{flex:1;min-width:0}
.page-heading h1{font-size:var(--text-2xl);font-weight:650;letter-spacing:-.3px;color:var(--ink)}
.page-heading p{margin-top:2px;font-size:var(--text-sm);color:var(--muted)}
.page-actions{display:flex;align-items:center;gap:var(--space-2)}
.page-body{flex:1;min-height:0;overflow:auto;padding:var(--space-2) var(--space-6) var(--space-6)}
.page-body>:deep(*){max-width:1180px;margin-inline:auto}
@keyframes page-in{from{opacity:0;transform:translateY(6px)}to{opacity:1;transform:none}}
@media (max-width:760px){
  .page-header{padding:var(--space-3) var(--space-3) var(--space-2);gap:var(--space-2)}
  .page-back span{display:none}
  .page-heading{flex-basis:calc(100% - 48px)}
  .page-heading h1{font-size:var(--text-xl)}
  .page-heading.named-above h1{display:none}
  .page-heading.named-above{flex-basis:0}
  .page-body{padding:var(--space-2) var(--space-3) var(--space-4)}
}
@media (prefers-reduced-motion:reduce){.page-shell{animation:none}}
</style>
