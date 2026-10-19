/**
 * Keyboard shortcuts for the whole app, in one table, so the help sheet and
 * the handler can never disagree about what a key does.
 *
 * "mod" is ⌘ on a Mac and Ctrl elsewhere. A focused terminal keeps ordinary keys:
 * Ctrl+K and the rest mean something to the shell or the CLI inside it,
 * Ctrl+B is handled separately as the workspace prefix; pressing it twice passes it through. Typing
 * in a field keeps plain keys; only chords with mod reach the app there.
 */
export interface Chord { key: string; prefix?: boolean; code?: string; mod?: boolean; shift?: boolean; alt?: boolean }
export interface Shortcut { id: string; chord: Chord; label: string; group: "General" | "Navigation" | "Sessions" | "Pane actions"; terminal?: boolean }

export const PANE_SHORTCUTS: readonly Shortcut[] = [
  { id: "split-horizontal", chord: { key: "%", prefix: true }, label: "Split side by side", group: "Pane actions" },
  { id: "split-vertical", chord: { key: '\"', prefix: true }, label: "Split top and bottom", group: "Pane actions" },
  ...(["Left", "Right", "Up", "Down"] as const).map(direction => ({ id: `focus-${direction.toLowerCase()}`, chord: { key: `Arrow${direction}`, prefix: true }, label: `Focus pane ${direction.toLowerCase()}`, group: "Pane actions" as const })),
  { id: "next-pane", chord: { key: "o", prefix: true }, label: "Next pane", group: "Pane actions" },
  { id: "maximize-pane", chord: { key: "z", prefix: true }, label: "Maximize or restore pane", group: "Pane actions" },
  { id: "pane-palette", chord: { key: "w", prefix: true }, label: "Search and commands", group: "Pane actions" },
  { id: "pane-help", chord: { key: "?", prefix: true }, label: "Keyboard shortcuts", group: "Pane actions" },
];

/** Prefix parsing is independent of the DOM, including terminal and editor targets. */
export function prefixKey(event: Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey" | "repeat" | "isComposing">, active: boolean): string | undefined {
  if (event.isComposing || event.key === "Process" || event.key === "Dead") return active ? "cancel-pass" : undefined;
  if (["Control", "Shift", "Alt", "Meta"].includes(event.key)) return undefined;
  const prefix = event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey && event.key.toLowerCase() === "b";
  if (event.repeat) return active || prefix ? "ignore" : undefined;
  if (prefix) return active ? "pass" : "prefix";
  if (!active) return undefined;
  if (event.key === "Escape") return "cancel";
  if (event.ctrlKey || event.metaKey || event.altKey) return "cancel-pass";
  return PANE_SHORTCUTS.find(item => item.chord.key === event.key)?.id ?? "cancel";
}

export const SHORTCUTS: readonly Shortcut[] = [
  ...PANE_SHORTCUTS,
  { id: "palette", chord: { key: "k", mod: true }, label: "Search and commands", group: "General" },
  { id: "help", chord: { key: "/", mod: true }, label: "Keyboard shortcuts", group: "General" },
  { id: "settings", chord: { key: ",", mod: true }, label: "Settings", group: "General" },
  { id: "sidebar", chord: { key: "b", mod: true }, label: "Show or hide the sidebar", group: "Navigation" },
  { id: "files", chord: { key: "e", mod: true, shift: true }, label: "Show or hide the file panel", group: "Navigation" },
  { id: "sessions", chord: { key: "s", mod: true, shift: true }, label: "All sessions", group: "Navigation" },
  { id: "system", chord: { key: "m", mod: true, shift: true }, label: "System", group: "Navigation" },
  { id: "canvas", chord: { key: "h", mod: true, shift: true }, label: "Back to the workspace canvas", group: "Navigation" },
  { id: "new-session", chord: { key: "o", mod: true, shift: true }, label: "New session…", group: "Sessions" },
];

export const isMac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

/** ⌘⇧O on a Mac, Ctrl+Shift+O elsewhere. */
export function formatChord(chord: Chord, mac = isMac): string {
  if (chord.prefix) return `Ctrl+B → ${chord.key.replace("Arrow", "")}`;
  const symbols: Record<string, string> = { ArrowLeft: "←", ArrowRight: "→", Enter: "Enter" };
  const key = symbols[chord.key] ?? (chord.key.length === 1 ? chord.key.toUpperCase() : chord.key);
  const parts = [chord.mod ? (mac ? "⌘" : "Ctrl") : "", chord.alt ? (mac ? "⌥" : "Alt") : "", chord.shift ? (mac ? "⇧" : "Shift") : "", key].filter(Boolean);
  return mac ? parts.join("") : parts.join("+");
}

export function matches(event: Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey"> & { code?: string }, chord: Chord, mac = isMac): boolean {
  if (chord.prefix) return false;
  const mod = mac ? event.metaKey : event.ctrlKey;
  if (mac ? event.ctrlKey : event.metaKey) return false;
  if (Boolean(chord.mod) !== mod || Boolean(chord.shift) !== event.shiftKey || Boolean(chord.alt) !== event.altKey) return false;
  // Shift changes the character ("/" becomes "?"), so compare case-insensitively and accept both.
  if (chord.code && event.code) return event.code === chord.code;
  return event.key.toLowerCase() === chord.key.toLowerCase() || (chord.key === "/" && event.key === "?");
}

/** Only explicitly reserved app chords leave a terminal; fields keep plain typing. */
export function keyBelongsToTarget(target: EventTarget | null, chord: Chord, terminal = false): boolean {
  const element = target instanceof Element ? target : null;
  if (element?.closest(".xterm, .terminal-host")) return !terminal;
  const field = element?.closest("input, textarea, select, [contenteditable='true']");
  return Boolean(field) && !chord.mod;
}

export function shortcutFor(event: KeyboardEvent, list: readonly Shortcut[] = SHORTCUTS): Shortcut | undefined {
  if (event.isComposing || event.key === "Process" || event.key === "Dead") return undefined;
  const hit = list.find(item => matches(event, item.chord));
  return hit && !keyBelongsToTarget(event.target, hit.chord, hit.terminal) ? hit : undefined;
}
