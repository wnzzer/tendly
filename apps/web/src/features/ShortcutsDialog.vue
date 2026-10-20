<script setup lang="ts">
import ModalDialog from './ModalDialog.vue';
import { AREA_KEYS, KEY_AREAS, PREFIX_COMMANDS, PREFIX_GROUPS, formatChord, isMac, platformShortcuts, type Chord } from './shortcuts';
import { useI18n } from '../i18n';

/** Every key, from the same tables the keys are handled with, under the grammar they share. */
const emit = defineEmits<{ close: [] }>();
const { t } = useI18n();
const general = platformShortcuts();
const prefixGroups = PREFIX_GROUPS.map(group => ({ group, items: PREFIX_COMMANDS.filter(item => item.group === group) }));
const areaGroups = KEY_AREAS.map(area => ({ area, items: AREA_KEYS.filter(item => item.area === area) }));
const shown = (key: Chord | string) => typeof key === 'string' ? key : formatChord(key);
const rules = [
  { keys: '↑ ↓ Enter Esc', text: 'Inside a list, a tree or a menu: the usual keys.' },
  { keys: isMac ? '⌘' : 'Ctrl', text: 'Does something where you are: the same key does the same kind of thing in every area.' },
  { keys: 'Ctrl+B', text: 'Goes somewhere or arranges the canvas. A letter goes to a place; its capital shows or hides it, or opens the whole page.' },
  { keys: 'Esc', text: 'Steps back one level and no further.' },
];
</script>

<template>
  <ModalDialog :title="t('Keyboard shortcuts')" wide @close="emit('close')">
    <dl class="shortcut-rules"><template v-for="rule in rules" :key="rule.keys"><dt><kbd>{{ rule.keys }}</kbd></dt><dd>{{ t(rule.text) }}</dd></template></dl>
    <div class="shortcut-groups">
      <section><h3>{{ t('General') }}</h3><dl><template v-for="item in general" :key="item.id"><dt>{{ t(item.label) }}</dt><dd><kbd>{{ formatChord(item.chord) }}</kbd></dd></template></dl></section>
      <section v-for="entry in prefixGroups" :key="entry.group"><h3>Ctrl+B · {{ t(entry.group) }}</h3><dl><template v-for="item in entry.items" :key="item.id"><dt>{{ t(item.label) }}</dt><dd><kbd v-for="key in (item.display ? [item.display] : item.keys)" :key="key">{{ key === ' ' ? 'Space' : key }}</kbd></dd></template></dl></section>
      <section v-for="entry in areaGroups" :key="entry.area"><h3>{{ t(entry.area) }}</h3><dl><template v-for="item in entry.items" :key="item.label"><dt>{{ t(item.label) }}</dt><dd><kbd v-for="key in item.keys" :key="shown(key)">{{ shown(key) }}</kbd></dd></template></dl></section>
    </div>
    <p class="shortcut-note">{{ t('Press Ctrl+B, release it, then press a pane key. Esc cancels; Ctrl+B twice passes through to the focused terminal or editor.') }}</p>
  </ModalDialog>
</template>

<style scoped>
.shortcut-rules{display:grid;grid-template-columns:max-content minmax(0,1fr);gap:8px var(--space-4);margin:0 0 var(--space-5);padding:12px 14px;border:1px solid var(--border);border-radius:var(--radius-md);background:var(--sunken)}
.shortcut-rules dt{margin:0}
.shortcut-rules dd{margin:0;font-size:var(--text-sm);line-height:20px;color:var(--ink-soft)}
.shortcut-groups{columns:2 300px;column-gap:var(--space-6)}
.shortcut-groups>section{break-inside:avoid;margin-bottom:var(--space-5)}
h3{font-size:var(--text-xs);font-weight:600;color:var(--muted);margin-bottom:6px}
.shortcut-groups dl{display:grid;grid-template-columns:minmax(0,1fr) auto;gap:6px var(--space-4);margin:0}
.shortcut-groups dt{font-size:var(--text-md);color:var(--ink)}
.shortcut-groups dd{display:flex;justify-content:flex-end;gap:4px;margin:0}
kbd{display:inline-block;padding:1px 7px;border:1px solid var(--line);border-bottom-width:2px;border-radius:var(--radius-xs);background:var(--sunken);font:var(--text-xs) var(--font-body);color:var(--ink-soft);white-space:nowrap}
.shortcut-note{margin:0;font-size:var(--text-xs);color:var(--muted)}
</style>
