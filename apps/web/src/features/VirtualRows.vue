<script setup lang="ts" generic="T">
import { computed, watch } from "vue";
import { measureElement, useVirtualizer } from "@tanstack/vue-virtual";

/**
 * A conversation as a window onto its rows, like OpenClaw's: only those near
 * the view are in the page, between two spacers sized from the heights the
 * others were measured at (or estimated, until seen). Rows keep their height
 * by key, so rows put in front shift nothing.
 *
 * Its own component so that scrolling, which moves the window every frame,
 * re-renders this and not the pane around it: the composer, menus and docks
 * are none of a scroll's business.
 */
const props = defineProps<{
  rows: readonly T[];
  keys: readonly string[];
  scrollElement: HTMLElement | undefined;
  /** Where the rows begin inside the scrolled element. */
  scrollMargin: number;
  /** A row's height before it has been measured. */
  estimate: (index: number) => number;
  /** Told each measured height, to be remembered across views. */
  measured?: (index: number, height: number) => void;
  /** What else a row's content depends on, beyond the row itself. */
  memo: readonly unknown[];
}>();
const emit = defineEmits<{
  /** The rows' total height changed: a view following the newest output follows it. */
  resize: [];
  /** Other rows came into the window. */
  range: [];
}>();

const virtualizer = useVirtualizer(
  computed(() => ({
    count: props.rows.length,
    getScrollElement: () => props.scrollElement ?? null,
    estimateSize: props.estimate,
    measureElement: (
      element: Element,
      entry: ResizeObserverEntry | undefined,
      instance: Parameters<typeof measureElement>[2],
    ) => {
      const height = measureElement(element, entry, instance);
      props.measured?.(Number((element as HTMLElement).dataset.index), height);
      return height;
    },
    overscan: 6,
    getItemKey: (index: number) => props.keys[index] ?? index,
    scrollMargin: props.scrollMargin,
    // A conversation opens at its newest rows: before the scrolled element has
    // been measured (and when rendered on the server) the window is the last ones.
    initialRect: { width: 0, height: 1200 },
    initialOffset: Number.MAX_SAFE_INTEGER,
  })),
);

// A row measured above the view would have the view moved by what it grew, to
// keep what is read in place. iOS WebKit stops a coasting scroll that is moved,
// so the virtualizer saves those moves for when it stops and makes them all at
// once: a flick through older rows jumped by everything they grew, mid-coast.
// Nothing is read in place mid-scroll anyway: on iOS, rows measured then move nothing.
const iosWebKit =
  typeof navigator !== "undefined" &&
  (/iP(hone|od|ad)/.test(navigator.userAgent) || (navigator.platform === "MacIntel" && navigator.maxTouchPoints > 0));
if (iosWebKit)
  virtualizer.value.shouldAdjustScrollPositionOnItemSizeChange = (item, _delta, instance) => {
    if (instance.isScrolling) return false;
    // Otherwise the virtualizer's own rule: a row seen for the first time when it
    // starts above the view, one seen before when it lies wholly above it.
    const offset = (instance.scrollOffset ?? 0) + instance.scrollAdjustments;
    return instance.itemSizeCache.has(item.key)
      ? item.end <= offset && instance.scrollDirection !== "backward"
      : item.start < offset;
  };

const visible = computed(() =>
  virtualizer.value
    .getVirtualItems()
    .map(virtual => ({ key: String(virtual.key), index: virtual.index, row: props.rows[virtual.index]! })),
);
const padding = computed(() => {
  const items = virtualizer.value.getVirtualItems(),
    first = items[0],
    last = items[items.length - 1];
  if (!first || !last) return { top: 0, bottom: 0 };
  return {
    top: first.start - props.scrollMargin,
    bottom: Math.max(0, virtualizer.value.getTotalSize() - (last.end - props.scrollMargin)),
  };
});
// While scrolling, nothing under a resting pointer is hovered: rows passing
// beneath it would otherwise restyle, and animate their hover controls, each frame.
const scrolling = computed(() => virtualizer.value.isScrolling);
const measure = (element: unknown) => {
  if (element instanceof Element) virtualizer.value.measureElement(element);
};

watch(
  () => virtualizer.value.getTotalSize(),
  () => emit("resize"),
);
watch(
  () => visible.value.map(row => row.key).join("\n"),
  () => emit("range"),
);

defineExpose({ virtualizer });
</script>

<template>
  <div
    :class="['chat-rows', { 'is-scrolling': scrolling }]"
    :style="{ paddingTop: padding.top + 'px', paddingBottom: padding.bottom + 'px' }"
  >
    <div
      v-for="{ key, index, row } in visible"
      :key="key"
      :data-index="index"
      :ref="measure"
      class="chat-row"
      v-memo="[row, index, ...memo]"
    >
      <slot :row="row" :index="index" />
    </div>
  </div>
</template>

<style scoped>
/* One row: its own margins stay inside it (flow-root), so its measured height
   is the room it takes; contained, laying it out stays inside it. */
.chat-row {
  display: flow-root;
  contain: layout paint style;
}
.chat-rows.is-scrolling {
  pointer-events: none;
}
</style>
