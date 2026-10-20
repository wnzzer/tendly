<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import type { ModelCatalog } from '@tendly/protocol';
import Icon from './Icon.vue';
import { effortLabel } from './reasoning-effort';
import { useI18n } from '../i18n';

type ModelEntry = ModelCatalog['models'][number];
const props = defineProps<{
  modelValue: string;
  models?: ModelEntry[];
  disabled?: boolean;
  placeholder?: string;
}>();
const emit = defineEmits<{ 'update:modelValue': [value: string] }>();
const { t } = useI18n();
const root = ref<HTMLElement>();
const open = ref(false);
const query = ref(props.modelValue);
const activeIndex = ref(0);
const options = computed(() => {
  const unique = [...new Map((props.models ?? []).map(entry => [entry.id, entry])).values()];
  const term = query.value.trim().toLocaleLowerCase();
  if (!term) return unique;
  return unique.filter(entry => `${entry.id} ${entry.name}`.toLocaleLowerCase().includes(term));
});
function sync(value: string) { query.value = value; activeIndex.value = 0; emit('update:modelValue', value); }
function choose(entry: ModelEntry) { sync(entry.id); open.value = false; }
function onOutside(event: PointerEvent) { if (root.value && !root.value.contains(event.target as Node)) open.value = false; }
function onKeydown(event: KeyboardEvent) {
  // Closing the list is this Esc's whole job; the dialog around it stays.
  if (event.key === 'Escape') { if (open.value) { event.preventDefault(); event.stopPropagation(); open.value = false; } return; }
  if (event.key === 'ArrowDown') {
    event.preventDefault();
    if (!open.value) { open.value = true; activeIndex.value = 0; }
    else if (options.value.length) activeIndex.value = (activeIndex.value + 1) % options.value.length;
  } else if (event.key === 'ArrowUp' && open.value && options.value.length) {
    event.preventDefault(); activeIndex.value = (activeIndex.value - 1 + options.value.length) % options.value.length;
  } else if (event.key === 'Enter' && open.value && options.value[activeIndex.value]) {
    event.preventDefault(); choose(options.value[activeIndex.value]);
  }
}
onMounted(() => document.addEventListener('pointerdown', onOutside));
onBeforeUnmount(() => document.removeEventListener('pointerdown', onOutside));
</script>

<template>
  <div ref="root" class="model-picker" @keydown="onKeydown">
    <div class="model-picker-input">
      <input :value="modelValue" :disabled="disabled" :placeholder="placeholder" autocomplete="off" spellcheck="false" role="combobox" :aria-expanded="open" :aria-controls="open ? 'model-picker-options' : undefined" @focus="query=modelValue;open=true" @input="sync(($event.target as HTMLInputElement).value)" />
      <button type="button" class="model-picker-toggle" :disabled="disabled" :aria-label="t(open ? 'Close model choices' : 'Open model choices')" @click="query=modelValue;open=!open"><Icon name="chevron" :size="13" /></button>
    </div>
    <div v-if="open" id="model-picker-options" class="model-picker-menu" role="listbox">
      <button v-for="(entry,index) in options" :key="entry.id" type="button" role="option" :aria-selected="entry.id===modelValue" :class="{active:index===activeIndex}" @mousedown.prevent="choose(entry)">
        <span class="model-picker-copy"><strong>{{ entry.name }}</strong><code>{{ entry.id }}</code></span>
        <span v-if="entry.efforts?.length" class="model-picker-efforts">{{ entry.efforts.map(effortLabel).join(' · ') }}</span>
      </button>
      <p v-if="!options.length">{{ t('No matching models. You can still enter a model ID.') }}</p>
    </div>
  </div>
</template>

<style scoped>
.model-picker{position:relative;min-width:0}.model-picker-input{display:flex;align-items:center;min-width:0}.model-picker-input>input{min-width:0;flex:1;padding-right:34px}.model-picker-toggle{display:grid;place-items:center;width:30px;height:30px;margin-left:-34px;margin-right:4px;border:0;border-radius:var(--radius-sm);background:transparent;color:var(--ink-soft);cursor:pointer}.model-picker-toggle svg{transform:rotate(90deg)}.model-picker:has(.model-picker-menu) .model-picker-toggle svg{transform:rotate(270deg)}.model-picker-menu{position:absolute;z-index:30;left:0;right:0;top:calc(100% + 5px);max-height:260px;overflow:auto;padding:5px;border:1px solid var(--border);border-radius:var(--radius-md);background:var(--surface);box-shadow:var(--shadow-lg)}.model-picker-menu button{display:flex;align-items:center;gap:8px;width:100%;min-height:42px;padding:7px 9px;border:0;border-radius:var(--radius-sm);background:none;text-align:left;cursor:pointer;color:var(--ink)}.model-picker-menu button:hover,.model-picker-menu button[aria-selected=true],.model-picker-menu button.active{background:var(--teal-soft)}.model-picker-copy{display:flex;flex-direction:column;min-width:0;flex:1}.model-picker-copy strong{font-size:var(--text-sm);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.model-picker-copy code{font-size:var(--text-xs);color:var(--muted);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.model-picker-efforts{font-size:var(--text-xs);color:var(--teal);white-space:nowrap}.model-picker-menu p{padding:10px;margin:0;font-size:var(--text-xs);color:var(--muted)}
</style>
