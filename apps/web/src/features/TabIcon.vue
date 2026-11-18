<script setup lang="ts">
import { computed } from "vue";
import type { ProviderKind } from "@tendly/protocol";
import type { LeafPaneKind } from "@tendly/protocol/layout";
import { resolveTabIcon } from "./tab-icons";
import Icon from "./Icon.vue";
import ProviderIcon from "./ProviderIcon.vue";

const props = withDefaults(
  defineProps<{
    kind: LeafPaneKind;
    metadata?: Record<string, unknown>;
    sessionProviders?: Record<string, ProviderKind>;
    size?: number;
  }>(),
  { size: 15 },
);
const appearance = computed(() => resolveTabIcon(props.kind, props.metadata, props.sessionProviders));
</script>
<template>
  <span
    class="tab-icon"
    :style="{
      color: appearance.color,
      '--tab-tint': appearance.background,
      width: `${size + 6}px`,
      height: `${size + 6}px`,
    }"
    :data-tab-icon="appearance.type"
    :data-icon-provider="appearance.provider"
    aria-hidden="true"
    ><ProviderIcon v-if="appearance.provider" :provider="appearance.provider" :size="size" /><Icon
      v-else
      :name="appearance.icon"
      :size="size"
  /></span>
</template>
<style scoped>
.tab-icon {
  background-color: color-mix(in srgb, var(--tab-tint) var(--tint-mix), var(--surface));
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  border-radius: var(--radius-sm);
  line-height: 1;
  vertical-align: middle;
  opacity: 1;
}
.tab-icon > svg {
  display: block;
  flex-shrink: 0;
}
</style>
