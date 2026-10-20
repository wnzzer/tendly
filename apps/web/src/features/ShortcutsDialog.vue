<script setup lang="ts">
import { computed } from 'vue';
import ModalDialog from './ModalDialog.vue';
import { PREFIX_COMMANDS, PREFIX_GROUPS, formatChord, platformShortcuts } from './shortcuts';
import { useI18n } from '../i18n';

/** Every shortcut, from the same tables the keys are handled with. */
const emit = defineEmits<{ close: [] }>();
const { t } = useI18n();
const groups = computed(() => (['General', 'Navigation', 'Sessions'] as const).map(group => ({ group, items: platformShortcuts().filter(item => item.group === group) })));
const prefixGroups = PREFIX_GROUPS.map(group => ({ group, items: PREFIX_COMMANDS.filter(item => item.group === group) }));
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
      <section v-for="entry in prefixGroups" :key="entry.group"><h3>Ctrl+B · {{ t(entry.group) }}</h3><dl><template v-for="item in entry.items" :key="item.id"><dt>{{ t(item.label) }}</dt><dd><kbd v-for="key in (item.display ? [item.display] : item.keys)" :key="key">{{ key === ' ' ? 'Space' : key }}</kbd></dd></template></dl></section>
    </div>
  </ModalDialog>
</template>

<style scoped>
.shortcut-groups{display:flex;flex-direction:column;gap:var(--space-4)}
h3{font-size:var(--text-xs);font-weight:600;color:var(--muted);margin-bottom:6px}
dl{display:grid;grid-template-columns:minmax(0,1fr) auto;gap:6px var(--space-4);margin:0}
dt{font-size:var(--text-md);color:var(--ink)}
dd{display:flex;justify-content:flex-end;gap:4px;margin:0}
kbd{display:inline-block;padding:1px 7px;border:1px solid var(--line);border-bottom-width:2px;border-radius:var(--radius-xs);background:var(--sunken);font:var(--text-xs) var(--font-body);color:var(--ink-soft)}
.shortcut-note{font-size:var(--text-xs);color:var(--muted)}
</style>
