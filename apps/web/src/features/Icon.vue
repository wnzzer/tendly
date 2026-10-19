<script setup lang="ts">
import { iconColor } from "./icon-palette";
withDefaults(defineProps<{ name: string; size?: number }>(), { size: 18 });
const paths: Record<string, string> = {
  grid: "M3 3h7v7H3z M14 3h7v7h-7z M3 14h7v7H3z M14 14h7v7h-7z",
  plus: "M12 5v14 M5 12h14",
  close: "M6 6l12 12 M18 6 6 18",
  closeOthers: "M9 4h6v16H9z M2 9l4 6 M6 9l-4 6 M18 9l4 6 M22 9l-4 6",
  closeLeft: "M15 4h6v16h-6z M3 8l7 8 M10 8l-7 8",
  closeRight: "M3 4h6v16H3z M14 8l7 8 M21 8l-7 8",
  closeAll: "M8 3h13v13 M3 8h13v13H3z M7 12l5 5 M12 12l-5 5",
  folder: "M3 7V5h6l2 3h10v12H3z",
  file: "M5 3h9l5 5v13H5z M14 3v6h5 M8 13h8 M8 17h6",
  git: "M6 3v12a5 5 0 0 0 10 0V8 M3 6a3 3 0 1 0 6 0a3 3 0 1 0-6 0 M13 5a3 3 0 1 0 6 0a3 3 0 1 0-6 0",
  spark: "m12 3 2.5 6.5L21 12l-6.5 2.5L12 21l-2.5-6.5L3 12l6.5-2.5z",
  terminal: "m5 7 5 5-5 5 M13 17h6",
  monitor: "M4 5h16v10H4z M9 19h6 M12 15v4",
  settings: "M4 7h16 M4 17h16 M8 4v6 M16 14v6",
  account: "M8 7a4 4 0 1 0 8 0a4 4 0 1 0-8 0 M4 21v-2a8 8 0 0 1 16 0v2",
  refresh: "M20 7v5h-5 M20 12a8 8 0 1 0-2 5 M20 7l-2-2",
  chevron: "m9 5 7 7-7 7",
  back: "m14 5-7 7 7 7",
  menu: "M4 6h16 M4 12h16 M4 18h16",
  save: "M4 3h14l3 3v15H3V3z M7 3v6h10V3 M7 21v-8h10v8",
  edit: "M4 20h4L19 9a2.1 2.1 0 0 0-3-3L5 17v3 M14 7l3 3",
  play: "m7 4 14 8-14 8z",
  stop: "M5 5h14v14H5z",
  search: "M3 10a7 7 0 1 0 14 0a7 7 0 1 0-14 0 M15 15l6 6",
  arrow: "M5 12h14 m-6-6 6 6-6 6",
  download: "M12 3v12 m-5-5 5 5 5-5 M4 16v5h16v-5",
  upload: "M12 16V4 m-5 5 5-5 5 5 M4 16v5h16v-5",
  sync: "M20 8a8 8 0 0 0-14.6-2.5 M4 3v4h4 M4 16a8 8 0 0 0 14.6 2.5 M20 21v-4h-4",
  clipboard: "M9 4h6v3H9z M9 5H5v16h14V5h-4 M8 12h8 M8 16h5",
  check: "m5 12 4 4L19 6",
  clock: "M2 12a10 10 0 1 0 20 0a10 10 0 1 0-20 0 M12 6v6l4 2",
  // A dial with its needle part-way round: how deeply this is being thought
  // about, which is a level rather than a duration.
  gauge: "M3 17a9 9 0 1 1 18 0 M12 17l4.5-5.5",
  wallet: "M19 7V4a1 1 0 0 0-1-1H5a2 2 0 0 0 0 4h15a1 1 0 0 1 1 1v4h-3a2 2 0 0 0 0 4h3a1 1 0 0 0 1-1v-2a1 1 0 0 0-1-1 M3 5v14a2 2 0 0 0 2 2h15a1 1 0 0 0 1-1v-4",
  chart: "M4 20V10 M10 20V4 M16 20v-7 M3 20h18",
  // What stands between a tool and the work. The composer's approval chip.
  globe: "M2 12a10 10 0 1 0 20 0a10 10 0 1 0-20 0 M2 12h20 M12 2a15 15 0 0 1 0 20 M12 2a15 15 0 0 0 0 20",
  shield: "M12 3l7 3v6c0 4.2-2.9 7.4-7 9-4.1-1.6-7-4.8-7-9V6z",
  image: "M3 3h18v18H3z m0 13 5-5 5 6 3-3 5 6 M14 7h.01",
  info: "M2 12a10 10 0 1 0 20 0a10 10 0 1 0-20 0 M12 11v6 M12 7h.01",
  locate: "M12 2v3 M12 19v3 M2 12h3 M19 12h3 M5 12a7 7 0 1 0 14 0a7 7 0 1 0-14 0 M9 12a3 3 0 1 0 6 0a3 3 0 1 0-6 0",
  archive: "M3 3h18v5H3z M5 8v13h14V8 M10 12h4",
  restore: "M4 8V3 M4 8h5 M4 8a9 9 0 1 1-1 8 M12 7v5l3 2",
  layout: "M3 4h18v16H3z M3 9h18 M10 9v11",
  splitHorizontal: "M3 4h18v16H3z M12 4v16",
  splitVertical: "M3 4h18v16H3z M3 12h18",
  maximize: "M8 3H3v5 M16 3h5v5 M21 16v5h-5 M8 21H3v-5",
  minimize: "M3 8h5V3 M16 3v5h5 M21 16h-5v5 M8 21v-5H3",
  more: "M4 12h.01 M12 12h.01 M20 12h.01",
  panelLeft: "M3 4h18v16H3z M9 4v16",
  panelRight: "M3 4h18v16H3z M15 4v16",
};
</script>
<template><svg :width="size" :height="size" viewBox="0 0 24 24" fill="none" stroke="currentColor" :stroke-width="name === 'more' ? 3.5 : 1.65" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" class="app-icon" :data-icon="name" :style="{ '--icon-semantic-color': iconColor(name), color: 'var(--icon-color, var(--icon-semantic-color))' }"><path :d="paths[name] || paths.grid" /></svg></template>
<style>
.app-icon,.provider-glyph{color:var(--icon-color,var(--icon-semantic-color));flex-shrink:0;vertical-align:middle}
/* A tab's media palette and a solid/destructive control's foreground are
   intentional overrides. Inherit their color rather than painting teal on teal. */
:where(.tab-icon,.primary-button,.primary,.chat-primary,.chat-send,.account-primary,.danger,.danger-text,.account-danger,.danger-button) :is(.app-icon,.provider-glyph){--icon-color:currentColor}
</style>
