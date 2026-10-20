<script setup lang="ts">
import { onBeforeUnmount, onMounted, watch } from 'vue';
import { closeLightbox, lightbox } from './image-lightbox';
import { useI18n } from '../i18n';

/** Any image clicked in a message or a rendered file, at full size. */
const { t } = useI18n();
/** Esc closes the picture and nothing else: taken first, so the conversation
 * under it does not read it as interrupting the reply. Focus goes back where it was. */
let returnTo: HTMLElement | null = null;
watch(lightbox, (shown, was) => { if (shown && !was) returnTo = document.activeElement as HTMLElement | null; else if (!shown && was) { returnTo?.focus?.({ preventScroll: true }); returnTo = null; } });
const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape' && lightbox.value) { event.preventDefault(); event.stopPropagation(); closeLightbox(); } };
onMounted(() => window.addEventListener('keydown', onKey, true));
onBeforeUnmount(() => window.removeEventListener('keydown', onKey, true));
</script>

<template>
  <Teleport to="body">
    <Transition name="lightbox">
      <div v-if="lightbox" class="lightbox" role="dialog" :aria-label="lightbox.alt || t('Image preview')" @click.self="closeLightbox">
        <figure><img :src="lightbox.src" :alt="lightbox.alt" /><figcaption v-if="lightbox.alt">{{ lightbox.alt }}</figcaption></figure>
        <div class="lightbox-actions"><a :href="lightbox.src" target="_blank" rel="noopener">{{ t('Open original') }}</a><button type="button" :aria-label="t('Close')" @click="closeLightbox">×</button></div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.lightbox{position:fixed;inset:0;z-index:1000;display:grid;place-items:center;padding:48px 24px;background:color-mix(in srgb, var(--shade) 80%, transparent);backdrop-filter:blur(4px);cursor:zoom-out}
.lightbox figure{margin:0;max-width:100%;max-height:100%;display:flex;flex-direction:column;align-items:center;gap:10px;cursor:default}
.lightbox img{max-width:min(1600px,100%);max-height:calc(100dvh - 140px);object-fit:contain;border-radius:var(--radius-md);background:repeating-conic-gradient(var(--fill) 0 25%,var(--surface) 0 50%) 0 0/16px 16px;box-shadow:0 20px 60px #0008}
.lightbox figcaption{color:var(--fill-hover);font-size:var(--text-sm)}
.lightbox-actions{position:absolute;top:14px;right:16px;display:flex;align-items:center;gap:10px}
.lightbox-actions a{color:var(--fill-hover);font-size:var(--text-sm);text-decoration:none;padding:6px 10px;border:1px solid color-mix(in srgb, var(--surface) 25%, transparent);border-radius:var(--radius-md)}
.lightbox-actions button{width:36px;height:36px;border:0;border-radius:50%;background:color-mix(in srgb, var(--surface) 15%, transparent);color:var(--on-accent);font-size:var(--text-2xl);line-height:1;cursor:pointer}
.lightbox-enter-active,.lightbox-leave-active{transition:opacity .18s ease}
.lightbox-enter-from,.lightbox-leave-to{opacity:0}
</style>
