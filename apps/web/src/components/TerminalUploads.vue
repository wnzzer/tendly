<script setup lang="ts">
import Icon from "../features/Icon.vue";
import { useI18n } from "../i18n";
import type { Upload } from "../features/terminal-uploads";
const { t } = useI18n();

/** A chip per file on its way into the terminal: progress while it uploads, retry or dismiss if it failed. */
defineProps<{ uploads: Upload[] }>();
const emit = defineEmits<{ retry: [entry: Upload]; dismiss: [id: number] }>();
</script>

<template>
  <div class="terminal-uploads" role="status" aria-live="polite">
    <div v-for="entry in uploads" :key="entry.id" :class="['terminal-upload', { failed: entry.failed }]">
      <img v-if="entry.preview" :src="entry.preview" alt="" />
      <Icon v-else name="file" :size="16" />
      <span class="terminal-upload-text">
        <strong :title="entry.name">{{ entry.name }}</strong>
        <small>{{ entry.failed ?? t("Uploading {percent}%", { percent: Math.round(entry.progress * 100) }) }}</small>
      </span>
      <button v-if="entry.failed && entry.retryable" type="button" class="text-button" @click="emit('retry', entry)">
        {{ t("Retry") }}
      </button>
      <button
        type="button"
        class="icon-button"
        :aria-label="t(entry.failed ? 'Dismiss' : 'Cancel upload')"
        @click="emit('dismiss', entry.id)"
      >
        <Icon name="close" :size="13" />
      </button>
    </div>
  </div>
</template>
