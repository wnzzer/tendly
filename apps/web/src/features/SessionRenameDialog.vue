<script setup lang="ts">
import { ref } from 'vue';
import type { Session } from '@tendly/protocol';
import ModalDialog from './ModalDialog.vue';
import { useI18n } from '../i18n';

const props = defineProps<{ session: Session; busy?: boolean }>();
const emit = defineEmits<{ close: []; save: [title: string] }>();
const { t } = useI18n();
const title = ref(props.session.title);
function submit() {
  const value = title.value.trim();
  if (value) emit('save', value);
}
</script>

<template>
  <ModalDialog :title="t('Rename session')" :closable="!busy" @close="!busy && emit('close')">
    <form class="session-rename-dialog" @submit.prevent="submit">
      <p class="form-description">{{ t('Rename this session without changing its Session ID, history or running process.') }}</p>
      <label>{{ t('Session title') }}<input v-model="title" autofocus required maxlength="200" :disabled="busy" /></label>
      <div class="dialog-actions"><button type="button" class="secondary-button" :disabled="busy" @click="emit('close')">{{ t('Cancel') }}</button><button class="primary-button" :disabled="busy||!title.trim()">{{ busy ? t('Saving…') : t('Save session name') }}</button></div>
    </form>
  </ModalDialog>
</template>

<style scoped>
.session-rename-dialog label{display:flex;flex-direction:column;gap:7px;font-size:var(--text-sm);color:var(--ink-soft)}.session-rename-dialog input{min-height:38px;width:100%;padding:8px 12px;border:1px solid var(--line);border-radius:var(--radius-md);background:var(--surface);font-size:var(--text-md);color:var(--ink)}.dialog-actions{display:flex;justify-content:flex-end;gap:8px;margin-top:18px}
@media(pointer:coarse){.session-rename-dialog input{min-height:44px;font-size:var(--input-text)}}
</style>
