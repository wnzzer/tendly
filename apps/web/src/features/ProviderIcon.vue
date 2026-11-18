<script setup lang="ts">
import type { ProviderKind } from "@tendly/protocol";
import { computed } from "vue";
import { providerColor } from "./icon-palette";
import { agentClient } from "./clients";
const props = withDefaults(defineProps<{ provider: ProviderKind; size?: number }>(), { size: 18 });
// An agent client draws its own mark (clients.ts); anything else is the terminal prompt.
const client = computed(() => agentClient(props.provider));
</script>
<template>
  <svg
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    fill="currentColor"
    aria-hidden="true"
    class="provider-glyph"
    :style="{
      '--icon-semantic-color': providerColor(provider),
      color: 'var(--icon-color, var(--icon-semantic-color))',
    }"
  >
    <path v-if="client" :d="client.glyph.path" :fill-rule="client.glyph.evenOdd ? 'evenodd' : undefined" />
    <g v-else fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
      <path d="m5 6 6 6-6 6M14 18h6" />
    </g>
  </svg>
</template>
<style scoped>
.provider-glyph {
  flex: none;
  vertical-align: middle;
  color: var(--icon-color, var(--icon-semantic-color));
}
</style>
