import { computed, ref, watch } from "vue";
import { loadPreferences, preferences, savePreferences } from "./preferences";

/**
 * Light, dark, or whatever the system says, for the interface and, apart
 * from it, for the terminals. Kept by the server with the other preferences,
 * so every device looks the same and an agent can change it; this browser
 * keeps a copy only to paint the right colours before the server answers.
 *
 * "system" leaves <html> without a data-theme, so the stylesheet's
 * prefers-color-scheme block decides; the other two pin it.
 */
export type ThemePreference = "system" | "light" | "dark";
/** "interface" follows the interface theme. */
export type TerminalThemePreference = "interface" | "light" | "dark";
const KEY = "tendly.theme";
const TERMINAL_KEY = "tendly.terminal-theme";

function pinned(value: unknown): "light" | "dark" | undefined {
  return value === "light" || value === "dark" ? value : undefined;
}
function cached(key: string) {
  try {
    return pinned(localStorage.getItem(key));
  } catch {
    return undefined;
  }
}
function cache(key: string, value: "light" | "dark" | undefined) {
  try {
    if (value) localStorage.setItem(key, value);
    else localStorage.removeItem(key);
  } catch {
    /* Private browsing: the server still has it. */
  }
}

const query =
  typeof window !== "undefined" && window.matchMedia ? window.matchMedia("(prefers-color-scheme: dark)") : undefined;
export const themePreference = ref<ThemePreference>(cached(KEY) ?? "system");
export const terminalThemePreference = ref<TerminalThemePreference>(cached(TERMINAL_KEY) ?? "interface");
/** Whether the interface is dark right now, for what CSS cannot reach (the terminal's canvas). */
export const darkScheme = ref(false);
/** Whether terminals are dark right now. */
export const terminalDark = computed(() =>
  terminalThemePreference.value === "interface" ? darkScheme.value : terminalThemePreference.value === "dark",
);

function apply() {
  const preference = themePreference.value;
  if (typeof document !== "undefined") {
    if (preference === "system") delete document.documentElement.dataset.theme;
    else document.documentElement.dataset.theme = preference;
  }
  darkScheme.value = preference === "dark" || (preference === "system" && query?.matches === true);
}
query?.addEventListener("change", apply);
apply();

// Whatever the server says wins: on load, after a save, after an agent's change.
watch(preferences, value => {
  const interfaceScheme = pinned(value.appearance),
    terminalScheme = pinned(value.terminal_appearance);
  themePreference.value = interfaceScheme ?? "system";
  cache(KEY, interfaceScheme);
  terminalThemePreference.value = terminalScheme ?? "interface";
  cache(TERMINAL_KEY, terminalScheme);
  apply();
});

async function save(key: "appearance" | "terminal_appearance", value: "light" | "dark" | undefined) {
  const next = { ...(await loadPreferences()) };
  if (value) next[key] = value;
  else delete next[key];
  try {
    await savePreferences(next);
  } catch {
    /* The change shows here until the next load. */
  }
}

export function setThemePreference(preference: ThemePreference) {
  themePreference.value = preference;
  cache(KEY, pinned(preference));
  apply();
  void save("appearance", pinned(preference));
}

export function setTerminalThemePreference(preference: TerminalThemePreference) {
  terminalThemePreference.value = preference;
  cache(TERMINAL_KEY, pinned(preference));
  void save("terminal_appearance", pinned(preference));
}
