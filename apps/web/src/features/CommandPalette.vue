<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue';
import type { ProviderKind } from '@tendly/protocol';
import { fuzzyFilter } from './fuzzy-search';
import Icon from './Icon.vue';
import ProviderIcon from './ProviderIcon.vue';
import { useI18n } from '../i18n';

/**
 * One box for going anywhere and doing anything: a session, a workspace, a
 * page, a command. Empty, it offers what is likely next -- what is waiting on
 * you, what you used recently, the common commands. Typing searches all of it
 * here in the browser.
 */
export interface PaletteItem {
  id: string;
  title: string;
  subtitle?: string;
  group: string;
  icon?: string;
  provider?: ProviderKind;
  hint?: string;
  /** Shown when the query is empty. */
  suggested?: boolean;
  keywords?: string;
  run: () => void;
}
const props = defineProps<{ items: PaletteItem[] }>();
const emit = defineEmits<{ close: [] }>();
const { t } = useI18n();
const query = ref(''), active = ref(0), input = ref<HTMLInputElement>(), list = ref<HTMLElement>();

const results = computed(() => {
  const found = query.value.trim()
    ? fuzzyFilter(props.items, query.value, item => `${item.title} ${item.subtitle ?? ''} ${item.keywords ?? ''}`).slice(0, 40)
    : props.items.filter(item => item.suggested);
  // Keep the groups together, in the order they first appear.
  const order: string[] = [];
  for (const item of found) if (!order.includes(item.group)) order.push(item.group);
  return order.flatMap(group => found.filter(item => item.group === group));
});
watch(query, () => { active.value = 0; });
watch(active, async index => { await nextTick(); list.value?.querySelector<HTMLElement>(`[data-index="${index}"]`)?.scrollIntoView({ block: 'nearest' }); });
onMounted(() => input.value?.focus());

function run(item: PaletteItem | undefined) { if (!item) return; emit('close'); item.run(); }
function onKey(event: KeyboardEvent) {
  if (event.key === 'ArrowDown') { event.preventDefault(); active.value = Math.min(results.value.length - 1, active.value + 1); }
  else if (event.key === 'ArrowUp') { event.preventDefault(); active.value = Math.max(0, active.value - 1); }
  else if (event.key === 'Enter') { event.preventDefault(); run(results.value[active.value]); }
  else if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); emit('close'); }
}
const startsGroup = (index: number) => index === 0 || results.value[index - 1]?.group !== results.value[index]?.group;
</script>

<template>
  <Teleport to="body">
    <div class="palette-backdrop" @mousedown.self="emit('close')">
      <section class="palette" role="dialog" aria-modal="true" :aria-label="t('Search and commands')">
        <label class="palette-input"><Icon name="search" :size="17" /><input ref="input" v-model="query" type="text" role="combobox" aria-autocomplete="list" aria-controls="palette-results" :aria-activedescendant="results.length ? `palette-item-${active}` : undefined" :placeholder="t('Search sessions, workspaces and commands…')" @keydown="onKey" /><kbd>Esc</kbd></label>
        <div id="palette-results" ref="list" class="palette-results" role="listbox">
          <template v-for="(item, index) in results" :key="item.id">
            <p v-if="startsGroup(index)" class="palette-group">{{ item.group }}</p>
            <button :id="`palette-item-${index}`" type="button" role="option" :data-index="index" :aria-selected="index === active" :class="['palette-item', { active: index === active }]" @mousemove="active = index" @click="run(item)">
              <span class="palette-icon"><ProviderIcon v-if="item.provider" :provider="item.provider" :size="16" /><Icon v-else :name="item.icon ?? 'spark'" :size="16" /></span>
              <span class="palette-copy"><strong>{{ item.title }}</strong><small v-if="item.subtitle">{{ item.subtitle }}</small></span>
              <kbd v-if="item.hint">{{ item.hint }}</kbd>
            </button>
          </template>
          <p v-if="!results.length" class="palette-empty">{{ t('Nothing matches “{query}”.', { query }) }}</p>
        </div>
        <footer class="palette-footer"><span><kbd>↑</kbd><kbd>↓</kbd> {{ t('to move') }}</span><span><kbd>Enter</kbd> {{ t('to open') }}</span></footer>
      </section>
    </div>
  </Teleport>
</template>

<style scoped>
.palette-backdrop{position:fixed;inset:0;z-index:150;display:flex;justify-content:center;align-items:flex-start;padding:min(14vh,120px) 16px 16px;background:color-mix(in srgb, var(--shade) 30%, transparent);animation:palette-fade var(--duration-fast) ease-out}
.palette{display:flex;flex-direction:column;width:min(640px,100%);max-height:min(560px,calc(100dvh - 120px));border-radius:var(--radius-xl);background:var(--elevated);box-shadow:var(--shadow-xl),0 0 0 1px color-mix(in srgb, var(--ink) 8%, transparent),inset 0 1px 0 var(--card-highlight);overflow:hidden;animation:palette-in var(--duration-normal) var(--ease-out)}
.palette-input{display:flex;align-items:center;gap:10px;padding:14px 16px;border-bottom:1px solid var(--border);color:var(--muted)}
.palette-input input{flex:1;min-width:0;border:0;outline:0;background:none;font-size:var(--text-lg);color:var(--ink)}
.palette-input input::placeholder{color:var(--muted)}
@media (pointer:coarse){kbd,.palette-footer{display:none}}
kbd{display:inline-flex;align-items:center;justify-content:center;min-width:20px;height:20px;padding:0 5px;border:1px solid var(--line);border-bottom-width:2px;border-radius:var(--radius-xs);background:var(--sunken);color:var(--ink-soft);font:var(--text-xs) var(--font-body)}
.palette-results{flex:1;min-height:0;overflow:auto;padding:6px}
.palette-group{padding:10px 10px 4px;font-size:var(--text-xs);font-weight:600;color:var(--muted)}
.palette-item{display:flex;align-items:center;gap:10px;width:100%;min-height:42px;padding:6px 10px;border:0;border-radius:var(--radius-md);background:none;text-align:left;cursor:pointer;color:var(--ink)}
.palette-item.active{background:var(--selected)}
.palette-icon{display:grid;place-items:center;width:28px;height:28px;flex:none;border-radius:var(--radius-sm);background:var(--fill);color:var(--ink-soft)}
.palette-item.active .palette-icon{background:var(--surface);color:var(--accent)}
.palette-copy{flex:1;min-width:0;display:flex;flex-direction:column}
.palette-copy strong{font-size:var(--text-md);font-weight:500;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.palette-copy small{font-size:var(--text-xs);color:var(--muted);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.palette-empty{padding:24px;text-align:center;color:var(--muted);font-size:var(--text-sm)}
.palette-footer{display:flex;gap:var(--space-4);padding:8px 14px;border-top:1px solid var(--border);font-size:var(--text-xs);color:var(--muted)}
.palette-footer span{display:inline-flex;align-items:center;gap:4px}
@keyframes palette-fade{from{opacity:0}}
@keyframes palette-in{from{opacity:0;transform:translateY(-6px) scale(.98)}}
@media (max-width:760px){.palette-backdrop{padding:12px}.palette{max-height:calc(var(--app-height,100dvh) - 24px)}.palette-input input{font-size:var(--input-text)}.palette-footer{display:none}.palette-item{min-height:48px}}
@media (prefers-reduced-motion:reduce){.palette-backdrop,.palette{animation:none}}
</style>
