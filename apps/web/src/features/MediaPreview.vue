<script setup lang="ts">
import { computed, ref, watch } from "vue";
import Icon from "./Icon.vue";
import { extensionOf, type MediaKind } from "./media-kind";
import { openLightbox } from "./image-lightbox";
import { useI18n } from "../i18n";
/**
 * A file shown as what it is rather than as text: an image, a video, audio, a
 * PDF, or -- for a format nothing here can show -- a way to download it. The
 * file pane and the preview of a file outside the workspaces both use this,
 * so they support the same formats.
 */
const props = defineProps<{ url: string; kind: Exclude<MediaKind, "text">; name: string }>();
const { t } = useI18n();
const failed = ref(false);
watch(
  () => props.url,
  () => {
    failed.value = false;
  },
);
const source = computed(() =>
  props.kind === "converted" ? `${props.url}${props.url.includes("?") ? "&" : "?"}as=png` : props.url,
);
const extension = computed(() => extensionOf(props.name).toUpperCase());
</script>
<template>
  <div v-if="kind === 'image' || kind === 'converted'" class="media-preview image-preview">
    <img
      v-if="!failed"
      :src="source"
      :alt="name"
      :title="t('Click to enlarge')"
      @click="openLightbox(source, name)"
      @error="failed = true"
    />
    <template v-else
      ><p>{{ t("Unable to preview this image. It may be unavailable or unsupported.") }}</p>
      <a class="secondary-button" :href="url" :download="name">{{ t("Download file") }}</a></template
    >
  </div>
  <div v-else-if="kind === 'video'" class="media-preview">
    <video v-if="!failed" :src="url" controls preload="metadata" @error="failed = true" /><template v-else
      ><p>{{ t("This browser cannot play this file. Use Download to open it locally.") }}</p>
      <a class="secondary-button" :href="url" :download="name">{{ t("Download file") }}</a></template
    >
  </div>
  <div v-else-if="kind === 'audio'" class="media-preview">
    <Icon name="file" :size="28" /><audio
      v-if="!failed"
      :src="url"
      controls
      preload="metadata"
      @error="failed = true"
    /><template v-else
      ><p>{{ t("This browser cannot play this file.") }}</p>
      <a class="secondary-button" :href="url" :download="name">{{ t("Download file") }}</a></template
    >
  </div>
  <iframe v-else-if="kind === 'pdf'" :src="url" class="pdf-preview" :title="t('PDF preview: {path}', { path: name })" />
  <div v-else class="pane-empty">
    <Icon name="file" :size="28" />
    <p>{{ t("{format} files cannot be previewed in the browser.", { format: extension || t("These") }) }}</p>
    <a class="secondary-button" :href="url" :download="name">{{ t("Download file") }}</a>
  </div>
</template>
<style scoped>
.image-preview img {
  cursor: zoom-in;
}
</style>
