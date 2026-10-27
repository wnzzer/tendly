<script setup lang="ts">
import { computed } from "vue";
import Icon from "../features/Icon.vue";
import { useI18n } from "../i18n";
import { FIXED_KEYS, KEY_LAYOUTS, isFixedKey, keyLabel, keyTitle, type ArrowKey, type BarSlot, type FixedKeyId, type KeyClient } from "../features/terminal-keys";
const { t } = useI18n();

/**
 * On-screen keys, for a device whose keyboard has no arrows and cannot be
 * relied on to be visible at all. The pane shows the bar only where the
 * pointer is a fingertip, and does the sending; the bar only says which key.
 *
 * Which keys are within reach depends on the program: Claude Code and Codex
 * are driven by Esc, Shift+Tab and Ctrl+C, a shell by Tab, Ctrl+R and Ctrl+D
 * (KEY_LAYOUTS). Everything else folds behind ⋯.
 */
const props = defineProps<{ client: KeyClient }>();
/** One-shot Ctrl: the next character the soft keyboard types becomes a control code. */
const ctrlArmed = defineModel<boolean>("ctrlArmed", { required: true });
/** One-shot Shift: Tab becomes Shift+Tab, an arrow a Shift+arrow, a letter its capital. */
const shiftArmed = defineModel<boolean>("shiftArmed", { required: true });
/** Tap a row to move a list selection onto it; see the pane for why it is opt-in. */
const tapSelect = defineModel<boolean>("tapSelect", { required: true });
/** The less used keys fold behind ⋯ so the everyday ones fit one phone-wide row. */
const moreKeys = defineModel<boolean>("moreKeys", { required: true });
const emit = defineEmits<{ keys: [presses: readonly string[]]; tab: []; arrow: [key: ArrowKey]; enter: []; paste: []; pickFiles: [] }>();

const layout = computed(() => KEY_LAYOUTS[props.client]);
const ARROWS: Record<ArrowKey, string> = { left: "Arrow left", up: "Arrow up", down: "Arrow down", right: "Arrow right" };

function press(slot: BarSlot) {
  if (slot === "ctrl") { ctrlArmed.value = !ctrlArmed.value; return; }
  if (slot === "shift") { shiftArmed.value = !shiftArmed.value; return; }
  if (slot === "enter") { emit("enter"); return; }
  // Tab is the one fixed key Shift changes: armed, it is Shift+Tab.
  if (slot === "tab") { emit("tab"); return; }
  if (isFixedKey(slot)) { emit("keys", FIXED_KEYS[slot].send); return; }
  emit("arrow", slot);
}
const fixed = (slot: BarSlot): FixedKeyId | undefined => isFixedKey(slot) ? slot : undefined;
const title = (slot: BarSlot) => {
  const id = fixed(slot);
  if (id) return t(keyTitle(props.client, id));
  return t(slot === "enter" ? "Enter" : slot === "ctrl" ? "Ctrl" : slot === "shift" ? "Shift" : ARROWS[slot as ArrowKey]);
};
const armed = (slot: BarSlot) => slot === "ctrl" ? ctrlArmed.value : slot === "shift" ? shiftArmed.value : undefined;

/** Keep the caret in the terminal so tapping a key never closes the soft keyboard. */
function keepFocus(event: Event) { event.preventDefault(); }
</script>

<template>
  <div class="terminal-keys" role="group" :aria-label="t('Terminal keys')" :data-client="client">
    <template v-if="moreKeys">
      <div class="terminal-keys-row">
        <button v-for="slot in layout.more" :key="slot" type="button" :class="['key-text', { armed: armed(slot) }]" :aria-pressed="armed(slot)" :aria-label="title(slot)" :title="title(slot)" :data-key="slot" @pointerdown="keepFocus" @click="press(slot)">
          {{ fixed(slot) ? keyLabel(fixed(slot)!, t) : slot === 'ctrl' ? 'Ctrl' : '⇧' }}
        </button>
      </div>
      <div class="terminal-keys-row">
        <button type="button" class="key-labeled" :aria-label="t('Paste')" :title="t('Paste')" @click="moreKeys = false; emit('paste')">
          <Icon name="clipboard" :size="16" /><span>{{ t('Paste') }}</span>
        </button>
        <button type="button" class="key-labeled" :aria-label="t('Photo or file')" :title="t('Photo or file')" @click="emit('pickFiles')">
          <Icon name="upload" :size="16" /><span>{{ t('Photo / file') }}</span>
        </button>
        <button type="button" :class="['key-labeled', { armed: tapSelect }]" :aria-pressed="tapSelect" :aria-label="t('Tap a row to select it')" :title="t('Tap a row to select it')" @pointerdown="keepFocus" @click="tapSelect = !tapSelect">
          <Icon name="locate" :size="16" /><span>{{ t('Select rows') }}</span>
        </button>
      </div>
    </template>
    <div class="terminal-keys-row">
      <template v-for="(slot, index) in layout.main" :key="slot">
        <span v-if="index > 0 && slot === 'left'" class="terminal-keys-sep" aria-hidden="true" />
        <button v-if="fixed(slot)" type="button" :class="['key-text', { armed: armed(slot) }]" :aria-pressed="armed(slot)" :aria-label="title(slot)" :title="title(slot)" :data-key="slot" @pointerdown="keepFocus" @click="press(slot)">{{ keyLabel(fixed(slot)!, t) }}</button>
        <button v-else-if="slot === 'enter'" type="button" :aria-label="title(slot)" :title="title(slot)" data-key="enter" @pointerdown="keepFocus" @click="press(slot)"><Icon name="check" :size="16" /></button>
        <button v-else type="button" :aria-label="title(slot)" :title="title(slot)" :data-key="slot" @pointerdown="keepFocus" @click="press(slot)"><Icon name="chevron" :size="16" :class="`key-${slot}`" /></button>
      </template>
      <span class="terminal-keys-sep" aria-hidden="true" />
      <button type="button" :class="{ armed: moreKeys || tapSelect || ctrlArmed || shiftArmed }" :aria-expanded="moreKeys" :aria-label="t('More keys')" :title="t('More keys')" @pointerdown="keepFocus" @click="moreKeys = !moreKeys"><Icon name="more" :size="16" /></button>
    </div>
  </div>
</template>
