import { onUnmounted, ref } from "vue";
import { SIDEBAR_RAIL_MIN_WIDTH } from "./panel-state";

/**
 * A phone-sized viewport: the same width at which the sidebar stops being a
 * column. Below it the shell shows one pane at a time and switches between
 * them from sheets rather than a docking canvas nobody can drag on a phone.
 */
export const MOBILE_QUERY = `(max-width: ${SIDEBAR_RAIL_MIN_WIDTH - 1}px)`;

export function useMobile() {
  const query = typeof window !== "undefined" && window.matchMedia ? window.matchMedia(MOBILE_QUERY) : undefined;
  const mobile = ref(!!query?.matches);
  const change = (event: MediaQueryListEvent) => {
    mobile.value = event.matches;
  };
  query?.addEventListener("change", change);
  onUnmounted(() => query?.removeEventListener("change", change));
  return mobile;
}
