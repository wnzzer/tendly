import { ref, watch } from "vue";

/**
 * How long a session stays in its workspace's list after it was last used
 * before it moves behind the "Earlier" fold; 0 never folds. Like the
 * language, a choice of this browser: a phone wants a shorter list than a desk.
 */
export const RECENT_DAY_CHOICES = [1, 3, 7, 14, 30, 0] as const;
export type RecentDays = (typeof RECENT_DAY_CHOICES)[number];
export const DEFAULT_RECENT_DAYS: RecentDays = 3;

const KEY = "tendly.recent-days";

function read(): RecentDays {
  try {
    const value = Number(localStorage.getItem(KEY) ?? NaN);
    return (RECENT_DAY_CHOICES as readonly number[]).includes(value) ? (value as RecentDays) : DEFAULT_RECENT_DAYS;
  } catch {
    return DEFAULT_RECENT_DAYS;
  }
}

export const recentDays = ref<RecentDays>(read());
watch(recentDays, value => {
  try {
    if (value === DEFAULT_RECENT_DAYS) localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, String(value));
  } catch {
    /* Private browsing: the choice lasts for this page. */
  }
});
export function setRecentDays(value: RecentDays) {
  recentDays.value = value;
}
