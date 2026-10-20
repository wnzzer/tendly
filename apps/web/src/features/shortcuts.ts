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
/** `mac` marks a chord that exists only on a Mac: elsewhere ⌘ is Ctrl, and Ctrl+B is the prefix. */
export interface Shortcut { id: string; chord: Chord; label: string; group: "General" | "Navigation" | "Sessions"; terminal?: boolean; mac?: boolean }

/**
 * What follows Ctrl+B. Everything done on the canvas has a key here, so the
 * keyboard alone can arrange panes, move between them and their tabs, start
 * sessions and get back to the input. An id shared with SHORTCUTS runs the
 * same action. `action` names the variant a key picks (a direction, a tab);
 * `repeat` keeps the prefix for a moment so the key can be pressed again.
 */
export type PrefixGroup = "Panes" | "Layout" | "Tabs" | "Create" | "Views";
export interface PrefixCommand { id: string; keys: readonly string[]; label: string; group: PrefixGroup; display?: string; repeat?: boolean; action?: (key: string) => string }
const DIRECTIONS: Record<string, "left" | "right" | "up" | "down"> = { ArrowLeft: "left", h: "left", H: "left", ArrowRight: "right", l: "right", L: "right", ArrowUp: "up", k: "up", K: "up", ArrowDown: "down", j: "down", J: "down" };
export const PREFIX_COMMANDS: readonly PrefixCommand[] = [
  { id: "split-horizontal", keys: ["%", "|"], label: "Split side by side", group: "Panes" },
  { id: "split-vertical", keys: ['"', "-"], label: "Split top and bottom", group: "Panes" },
  { id: "focus", keys: ["ArrowLeft", "ArrowDown", "ArrowUp", "ArrowRight", "h", "j", "k", "l"], display: "←↓↑→ hjkl", label: "Move to the pane in that direction", group: "Panes", action: key => `focus-${DIRECTIONS[key]}` },
  { id: "next-pane", keys: ["o"], label: "Next pane", group: "Panes" },
  { id: "last-pane", keys: [";"], label: "The pane used before", group: "Panes" },
  { id: "maximize-pane", keys: ["z"], label: "Maximize or restore pane", group: "Panes" },
  { id: "resize", keys: ["H", "J", "K", "L"], display: "H J K L", label: "Move the divider, again and again", group: "Layout", repeat: true, action: key => `resize-${DIRECTIONS[key]}` },
  { id: "swap-previous", keys: ["{"], label: "Swap with the previous pane", group: "Layout" },
  { id: "swap-next", keys: ["}"], label: "Swap with the next pane", group: "Layout" },
  { id: "next-layout", keys: [" "], display: "Space", label: "Next layout preset", group: "Layout" },
  { id: "close-pane", keys: ["x"], label: "Close pane", group: "Panes" },
  { id: "next-tab", keys: ["n"], label: "Next tab", group: "Tabs" },
  { id: "previous-tab", keys: ["p"], label: "Previous tab", group: "Tabs" },
  { id: "tab", keys: ["1", "2", "3", "4", "5", "6", "7", "8", "9"], display: "1–9", label: "Tab by its number", group: "Tabs", action: key => `tab-${key}` },
  { id: "rename-session", keys: [","], label: "Rename session", group: "Tabs" },
  { id: "new-here", keys: ["c"], label: "New session here", group: "Create" },
  { id: "new-session", keys: ["C"], label: "New session…", group: "Create" },
  { id: "new-terminal", keys: ["t"], label: "New terminal here", group: "Create" },
  { id: "open-changes", keys: ["g"], label: "Git Changes", group: "Create" },
  { id: "focus-input", keys: ["i"], label: "Back to the input", group: "Views" },
  { id: "choose-view", keys: ["w"], label: "Go to an open view", group: "Views" },
  { id: "choose-session", keys: ["s"], label: "Find a session", group: "Views" },
  { id: "find-file", keys: ["f"], label: "Find a file", group: "Views" },
  { id: "sidebar", keys: ["b"], label: "Show or hide the sidebar", group: "Views" },
  { id: "files", keys: ["e"], label: "Show or hide the file panel", group: "Views" },
  { id: "palette", keys: [":"], label: "Search and commands", group: "Views" },
  { id: "help", keys: ["?"], label: "Keyboard shortcuts", group: "Views" },
];
export const PREFIX_GROUPS: readonly PrefixGroup[] = ["Panes", "Layout", "Tabs", "Create", "Views"];

/** The command a key runs after the prefix, with its variant: `focus-left`, `tab-3`. */
export function prefixCommand(key: string): { command: PrefixCommand; id: string } | undefined {
  const command = PREFIX_COMMANDS.find(item => item.keys.includes(key));
  return command && { command, id: command.action ? command.action(key) : command.id };
}

/**
 * Prefix parsing is independent of the DOM, including terminal and editor
 * targets. `repeating` is the moment after a repeatable command, when only
 * its keys are taken and anything else is typed as usual.
 */
export function prefixKey(event: Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey" | "repeat" | "isComposing">, active: boolean, repeating = false): string | undefined {
  if (event.isComposing || event.key === "Process" || event.key === "Dead") return active ? "cancel-pass" : undefined;
  if (["Control", "Shift", "Alt", "Meta"].includes(event.key)) return undefined;
  const prefix = event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey && event.key.toLowerCase() === "b";
  if (repeating && !prefix) {
    if (event.key === "Escape") return "cancel";
    const hit = !event.ctrlKey && !event.metaKey && !event.altKey ? prefixCommand(event.key) : undefined;
    return hit?.command.repeat ? hit.id : "cancel-pass";
  }
  if (event.repeat) return active || prefix ? "ignore" : undefined;
  if (prefix) return active ? "pass" : "prefix";
  if (!active) return undefined;
  if (event.key === "Escape") return "cancel";
  if (event.ctrlKey || event.metaKey || event.altKey) return "cancel-pass";
  return prefixCommand(event.key)?.id ?? "cancel";
}

/** "Ctrl+B z", for a menu or a tooltip. */
export function formatPrefix(command: PrefixCommand): string {
  return `Ctrl+B ${command.display ?? command.keys[0]}`;
}

export const SHORTCUTS: readonly Shortcut[] = [
  { id: "palette", chord: { key: "k", mod: true }, label: "Search and commands", group: "General" },
  { id: "help", chord: { key: "/", mod: true }, label: "Keyboard shortcuts", group: "General" },
  { id: "settings", chord: { key: ",", mod: true }, label: "Settings", group: "General" },
  { id: "sidebar", chord: { key: "b", mod: true }, label: "Show or hide the sidebar", group: "Navigation", mac: true },
  { id: "files", chord: { key: "e", mod: true, shift: true }, label: "Show or hide the file panel", group: "Navigation" },
  { id: "sessions", chord: { key: "s", mod: true, shift: true }, label: "All sessions", group: "Navigation" },
  { id: "system", chord: { key: "m", mod: true, shift: true }, label: "System", group: "Navigation" },
  { id: "canvas", chord: { key: "h", mod: true, shift: true }, label: "Back to the workspace canvas", group: "Navigation" },
  { id: "new-session", chord: { key: "o", mod: true, shift: true }, label: "New session…", group: "Sessions" },
];

/** The chords this platform has. */
export function platformShortcuts(mac = isMac): Shortcut[] { return SHORTCUTS.filter(item => !item.mac || mac); }

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

export function shortcutFor(event: KeyboardEvent, list: readonly Shortcut[] = platformShortcuts()): Shortcut | undefined {
  if (event.isComposing || event.key === "Process" || event.key === "Dead") return undefined;
  const hit = list.find(item => matches(event, item.chord));
  return hit && !keyBelongsToTarget(event.target, hit.chord, hit.terminal) ? hit : undefined;
}
