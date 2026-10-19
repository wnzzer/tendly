<script setup lang="ts">
import { computed } from 'vue';
import ModalDialog from './ModalDialog.vue';
import { SHORTCUTS, formatChord } from './shortcuts';
import { useI18n } from '../i18n';

/** Every shortcut, from the same table the keys are handled with. */
const emit = defineEmits<{ close: [] }>();
const { t } = useI18n();
const groups = computed(() => (['General', 'Navigation', 'Sessions', 'Pane actions'] as const).map(group => ({ group, items: SHORTCUTS.filter(item => item.group === group) })));
const chat = [
  { keys: 'Enter', label: 'Send, or steer the running turn' },
  { keys: 'Alt+Enter', label: 'Queue after this turn' },
  { keys: 'Shift+Enter', label: 'New line' },
  { keys: 'Esc', label: 'Close a menu, or cancel the reply' },
];
</script>

<template>
  <ModalDialog :title="t('Keyboard shortcuts')" @close="emit('close')">
    <div class="shortcut-groups">
      <section v-for="entry in groups" :key="entry.group"><h3>{{ t(entry.group) }}</h3><dl><template v-for="item in entry.items" :key="item.id"><dt>{{ t(item.label) }}</dt><dd><kbd>{{ formatChord(item.chord) }}</kbd></dd></template></dl></section>
      <section><h3>{{ t('Conversation') }}</h3><dl><template v-for="item in chat" :key="item.keys"><dt>{{ t(item.label) }}</dt><dd><kbd>{{ item.keys }}</kbd></dd></template></dl></section>
      <p class="shortcut-note">{{ t('Press Ctrl+B, release it, then press a pane key. Esc cancels; Ctrl+B twice passes through to the focused terminal or editor.') }}</p>
    </div>
  </ModalDialog>
</template>

<style scoped>
.shortcut-groups{display:flex;flex-direction:column;gap:var(--space-4)}
h3{font-size:var(--text-xs);font-weight:600;color:var(--muted);margin-bottom:6px}
dl{display:grid;grid-template-columns:minmax(0,1fr) auto;gap:6px var(--space-4);margin:0}
dt{font-size:var(--text-md);color:var(--ink)}
dd{margin:0;text-align:right}
kbd{display:inline-block;padding:1px 7px;border:1px solid var(--line);border-bottom-width:2px;border-radius:var(--radius-xs);background:var(--sunken);font:var(--text-xs) var(--font-body);color:var(--ink-soft)}
.shortcut-note{font-size:var(--text-xs);color:var(--muted)}
</style>
