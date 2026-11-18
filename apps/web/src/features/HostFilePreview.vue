<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from "vue";
import ModalDialog from "./ModalDialog.vue";
import { ApiError, errorMessage, json, request } from "./api";
import Icon from "./Icon.vue";
import MediaPreview from "./MediaPreview.vue";
import { mediaKind, type MediaKind } from "./media-kind";
import { MAX_HIGHLIGHT_BYTES, escapeHtml, languageFor, loadHighlighter } from "./highlighting";
import { useI18n } from "../i18n";

/**
 * A file an agent mentioned that lives outside every workspace: shown
 * read-only, at the line it named. Editing belongs to a workspace; the way
 * there is adding the folder as one.
 */
const props = defineProps<{ path: string; line?: number }>();
const emit = defineEmits<{ close: []; addWorkspace: [folder: string] }>();
const { t } = useI18n();
const content = ref<string>(),
  error = ref(""),
  copied = ref(false),
  html = ref<string[]>();
/** Refused only for lying outside the folders Tendly may browse: something the person can open, for now. */
const grantable = ref(false),
  granting = ref(false);
const body = ref<HTMLElement>();
const name = computed(() => props.path.split("/").pop() || props.path);
const folder = computed(() => props.path.slice(0, props.path.lastIndexOf("/")) || "/");
const lines = computed(() => (content.value ?? "").split("\n"));
/** Images, video, audio and PDFs are shown as themselves; so is a file the host says is not text. */
const kind = ref<MediaKind>(mediaKind(props.path));
const rawUrl = computed(() => `/api/host/raw?${new URLSearchParams({ path: props.path })}`);
const media = ref(false);

async function load() {
  error.value = "";
  grantable.value = false;
  media.value = false;
  try {
    if (kind.value !== "text") {
      // One byte is enough to learn whether it may be read, before an <img>
      // or <video> fails with nothing to say why.
      const probe = await fetch(rawUrl.value, { headers: { Range: "bytes=0-0" } });
      if (!probe.ok) {
        const body = await probe.json().catch(() => ({}));
        throw new ApiError(body.error ?? probe.statusText, probe.status);
      }
      media.value = true;
      return;
    }
    const file = await request<{ content: string }>(`/host/file?${new URLSearchParams({ path: props.path })}`);
    content.value = file.content;
    const language = languageFor(props.path);
    if (language && file.content.length <= MAX_HIGHLIGHT_BYTES) {
      const highlight = await loadHighlighter(language);
      // Highlighted per line so the numbered rows and the target line stay aligned.
      if (highlight) html.value = file.content.split("\n").map(line => highlight(line, language) || escapeHtml(line));
    }
    await nextTick();
    if (props.line) body.value?.querySelector(`[data-line="${props.line}"]`)?.scrollIntoView({ block: "center" });
  } catch (cause) {
    if (cause instanceof ApiError && cause.status === 415) {
      kind.value = "binary";
      media.value = true;
      return;
    }
    grantable.value = cause instanceof ApiError && cause.status === 403 && /outside the folders/.test(cause.message);
    error.value = errorMessage(cause);
  }
}
onMounted(load);
/** A read-only grant for this file, or its folder, kept until Tendly restarts. */
async function grant(directory: boolean) {
  granting.value = true;
  try {
    await request("/host/grants", json("POST", { path: props.path, directory }));
    await load();
  } catch (cause) {
    grantable.value = false;
    error.value = errorMessage(cause);
  } finally {
    granting.value = false;
  }
}
async function copyPath() {
  try {
    await navigator.clipboard.writeText(props.path);
    copied.value = true;
    setTimeout(() => {
      copied.value = false;
    }, 1500);
  } catch {
    /* Clipboard refused; the path is on screen. */
  }
}
</script>

<template>
  <ModalDialog :title="name" wide @close="emit('close')">
    <div class="host-file">
      <header>
        <code :title="path">{{ path }}</code>
        <span class="host-file-badge">{{ t("Read-only · outside every workspace") }}</span>
        <button type="button" class="small-button" @click="copyPath">{{ t(copied ? "Copied" : "Copy path") }}</button>
        <button type="button" class="small-button" @click="emit('addWorkspace', folder)">
          {{ t("Add folder as workspace") }}
        </button>
      </header>
      <div v-if="grantable" class="host-grant" role="alertdialog">
        <span class="host-grant-icon"><Icon name="shield" :size="18" /></span>
        <div>
          <strong>{{ t("This file is outside the folders Tendly may browse") }}</strong>
          <p>
            {{
              t(
                "You can allow read-only access for now. It ends when Tendly restarts, and it opens nothing to editing, browsing or agents.",
              )
            }}
          </p>
          <div class="host-grant-actions">
            <button type="button" class="primary-button" :disabled="granting" @click="grant(false)">
              {{ t("Allow reading this file") }}
            </button>
            <button type="button" class="secondary-button" :disabled="granting" @click="grant(true)">
              {{ t("Allow reading the folder {folder}", { folder }) }}
            </button>
          </div>
        </div>
      </div>
      <p v-else-if="error" class="inline-error" role="alert">{{ error }}</p>
      <div v-else-if="media && kind !== 'text'" class="host-file-media">
        <MediaPreview :url="rawUrl" :kind="kind" :name="name" />
      </div>
      <p v-else-if="content === undefined" class="host-file-quiet">{{ t("Loading…") }}</p>
      <div v-else ref="body" class="host-file-body">
        <div
          v-for="(text, index) in lines"
          :key="index"
          :data-line="index + 1"
          :class="['host-file-line', { target: index + 1 === line }]"
        >
          <span>{{ index + 1 }}</span><code v-if="html" v-html="html[index]" /><code v-else>{{ text }}</code>
        </div>
      </div>
    </div>
  </ModalDialog>
</template>

<style scoped>
.host-file {
  display: flex;
  flex-direction: column;
  gap: 10px;
  min-height: 0;
  height: min(620px, calc(100vh - 200px));
}
.host-file header {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.host-file header code {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--text-sm);
  color: var(--ink-soft);
}
.host-file-badge {
  flex-shrink: 0;
  font-size: var(--text-xs);
  padding: 2px 8px;
  border-radius: var(--radius-sm);
  background: var(--fill);
  color: var(--muted);
}
.host-file-media {
  display: flex;
  flex: 1;
  min-height: 0;
  overflow: hidden;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
}
.host-file-quiet {
  font-size: var(--text-sm);
  color: var(--muted);
}
.host-file-body {
  flex: 1;
  min-height: 0;
  overflow: auto;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  background: var(--surface);
  padding: 10px 0;
  font: var(--text-md)/20px var(--mono);
  tab-size: 4;
}
.host-file-line {
  display: flex;
  min-width: max-content;
}
.host-file-line > span {
  position: sticky;
  left: 0;
  flex: none;
  min-width: 52px;
  padding: 0 12px 0 8px;
  margin-right: 12px;
  text-align: right;
  font-size: var(--text-sm);
  color: var(--code-gutter);
  background: var(--surface);
  border-right: 1px solid var(--fill);
  user-select: none;
  font-variant-numeric: tabular-nums;
}
.host-file-line > code {
  white-space: pre;
  padding: 0 16px 0 0;
  color: var(--syn-text);
  background: none;
  font: inherit;
}
.host-file-line.target {
  background: var(--warn-line);
}
.host-file-line.target > span {
  color: var(--ink);
  font-weight: 600;
  background: var(--warn-line);
}
.host-grant {
  display: flex;
  gap: 12px;
  padding: 14px 16px;
  border-radius: var(--radius-lg);
  background: var(--warn-soft);
  box-shadow: inset 3px 0 0 var(--warn);
}
.host-grant-icon {
  display: grid;
  place-items: center;
  width: 32px;
  height: 32px;
  flex: none;
  border-radius: var(--radius-md);
  background: var(--surface);
  color: var(--warn-ink);
}
.host-grant strong {
  display: block;
  font-size: var(--text-md);
  color: var(--ink);
}
.host-grant p {
  margin-top: 4px;
  font-size: var(--text-sm);
  line-height: 1.6;
  color: var(--ink-soft);
}
.host-grant-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 10px;
}
</style>
