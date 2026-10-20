/**
 * Every key the app answers, in one place, so the help sheet, the tooltips
 * and the handlers can never disagree. One grammar runs through it:
 *
 * - Inside an area -- a list, a tree, tabs, a menu -- the keys are the usual
 *   ones: arrows move, Enter opens, Space selects, F2 renames, Delete (⌘⌫ on
 *   a Mac) removes after asking, Esc steps back.
 * - mod (⌘ on a Mac, Ctrl elsewhere) does something where you are, and the
 *   same key does the same kind of thing in every area: F finds, Enter
 *   confirms the main action, S saves, K opens the palette.
 * - Ctrl+B, released, then a key goes somewhere or arranges the canvas. A
 *   lower-case letter goes to a place, opening it if need be; the capital is
 *   more of the same: show or hide it, with options, the whole page.
 * - Esc steps back one level and no further: the top overlay, an edit, a
 *   field, the sidebar. The one exception is the agent's reply, which Esc in
 *   the composer interrupts, as in Claude Code's own terminal.
 *
 * Where both layers have a key they share it: K and ":" the palette, "/" and
 * "?" help, "," settings, F find, B the sidebar, E the file panel. A focused
 * terminal keeps every key but Ctrl+B and its own find; a field keeps plain
 * typing, and only chords with mod leave it.
 */
export const isMac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

export interface Chord { key: string; prefix?: boolean; code?: string; mod?: boolean; shift?: boolean; alt?: boolean }
/** `mac` marks a chord that exists only on a Mac: elsewhere ⌘ is Ctrl, and Ctrl+B is the prefix. */
export interface Shortcut { id: string; chord: Chord; label: string; group: "General"; terminal?: boolean; mac?: boolean }

/** The verbs every area shares. */
export const CHORDS = {
  find: { key: "f", mod: true },
  confirm: { key: "Enter", mod: true },
  save: { key: "s", mod: true },
  remove: { key: "Backspace", mod: true },
} as const satisfies Record<string, Chord>;

/**
 * What follows Ctrl+B. Everything done on the canvas has a key here, so the
 * keyboard alone can arrange panes, move between them and their tabs, start
 * sessions and get back to the input. An id shared with SHORTCUTS runs the
 * same action. `action` names the variant a key picks (a direction, a tab);
 * `repeat` keeps the prefix for a moment so the key can be pressed again;
 * `pages` marks the keys that also work on a page, away from the canvas.
 */
export type PrefixGroup = "Panes" | "Layout" | "Tabs" | "Create" | "Go to" | "App";
export interface PrefixCommand { id: string; keys: readonly string[]; label: string; group: PrefixGroup; display?: string; repeat?: boolean; pages?: boolean; action?: (key: string) => string }
const DIRECTIONS: Record<string, "left" | "right" | "up" | "down"> = { ArrowLeft: "left", h: "left", H: "left", ArrowRight: "right", l: "right", L: "right", ArrowUp: "up", k: "up", K: "up", ArrowDown: "down", j: "down", J: "down" };
export const PREFIX_COMMANDS: readonly PrefixCommand[] = [
  { id: "split-horizontal", keys: ["%", "|"], label: "Split side by side", group: "Panes" },
  { id: "split-vertical", keys: ['"', "-"], label: "Split top and bottom", group: "Panes" },
  { id: "focus", keys: ["ArrowLeft", "ArrowDown", "ArrowUp", "ArrowRight", "h", "j", "k", "l"], display: "←↓↑→ hjkl", label: "Move to the pane in that direction", group: "Panes", action: key => `focus-${DIRECTIONS[key]}` },
  { id: "next-pane", keys: ["o"], label: "Next pane", group: "Panes" },
  { id: "last-pane", keys: [";"], label: "The pane used before", group: "Panes" },
  { id: "maximize-pane", keys: ["z"], label: "Maximize or restore pane", group: "Panes" },
  { id: "close-pane", keys: ["x"], label: "Close pane", group: "Panes" },
  { id: "resize", keys: ["H", "J", "K", "L"], display: "H J K L", label: "Move the divider, again and again", group: "Layout", repeat: true, action: key => `resize-${DIRECTIONS[key]}` },
  { id: "swap-previous", keys: ["{"], label: "Swap with the previous pane", group: "Layout" },
  { id: "swap-next", keys: ["}"], label: "Swap with the next pane", group: "Layout" },
  { id: "next-layout", keys: [" "], display: "Space", label: "Next layout preset", group: "Layout" },
  { id: "next-tab", keys: ["n"], label: "Next tab", group: "Tabs" },
  { id: "previous-tab", keys: ["p"], label: "Previous tab", group: "Tabs" },
  { id: "tab", keys: ["1", "2", "3", "4", "5", "6", "7", "8", "9"], display: "1–9", label: "Tab by its number", group: "Tabs", action: key => `tab-${key}` },
  { id: "new-here", keys: ["c"], label: "New session here", group: "Create" },
  { id: "new-session", keys: ["C"], label: "New session…", group: "Create", pages: true },
  { id: "new-terminal", keys: ["t"], label: "New terminal here", group: "Create" },
  { id: "open-changes", keys: ["g"], label: "Git Changes", group: "Create" },
  { id: "focus-input", keys: ["i"], label: "Back to the input", group: "Go to" },
  { id: "choose-view", keys: ["w"], label: "Go to an open view", group: "Go to" },
  { id: "choose-session", keys: ["s"], label: "Find a session", group: "Go to", pages: true },
  { id: "find-file", keys: ["f"], label: "Find a file", group: "Go to" },
  { id: "focus-sidebar", keys: ["b"], label: "Go to the sidebar", group: "Go to", pages: true },
  { id: "focus-files", keys: ["e"], label: "Go to the file panel", group: "Go to" },
  { id: "sidebar", keys: ["B"], label: "Show or hide the sidebar", group: "App", pages: true },
  { id: "files", keys: ["E"], label: "Show or hide the file panel", group: "App" },
  { id: "canvas", keys: ["W"], label: "Back to the workspace canvas", group: "App", pages: true },
  { id: "sessions", keys: ["S"], label: "All sessions", group: "App", pages: true },
  { id: "usage", keys: ["U"], label: "Usage", group: "App", pages: true },
  { id: "system", keys: ["M"], label: "System", group: "App", pages: true },
  { id: "settings", keys: [","], label: "Settings", group: "App", pages: true },
  { id: "palette", keys: [":"], label: "Search and commands", group: "App", pages: true },
  { id: "help", keys: ["?"], label: "Keyboard shortcuts", group: "App", pages: true },
];
export const PREFIX_GROUPS: readonly PrefixGroup[] = ["Panes", "Layout", "Tabs", "Create", "Go to", "App"];

/**
 * The keys each area answers by itself, for the help sheet. The handlers
 * live with their areas; the shared verbs come from CHORDS, so a key shown
 * here is the key that works.
 */
export type KeyArea = "Everywhere" | "Lists" | "Conversation" | "Terminal" | "Files" | "Git";
export interface AreaKey { area: KeyArea; keys: readonly (Chord | string)[]; label: string }
export const AREA_KEYS: readonly AreaKey[] = [
  { area: "Everywhere", keys: ["Esc"], label: "Back one step: a menu, an edit, a field, the sidebar" },
  { area: "Everywhere", keys: [CHORDS.find], label: "Find in this area" },
  { area: "Everywhere", keys: [CHORDS.confirm], label: "Its main action: send, commit, accept, save the form" },
  { area: "Lists", keys: ["↑", "↓"], label: "Move" },
  { area: "Lists", keys: ["←", "→"], label: "Fold or unfold" },
  { area: "Lists", keys: ["Enter"], label: "Open" },
  { area: "Lists", keys: ["Space"], label: "Select, or stage a change" },
  { area: "Lists", keys: ["Shift+↑", "Shift+↓"], label: "Select a range" },
  { area: "Lists", keys: ["F2"], label: "Rename" },
  { area: "Lists", keys: ["Delete", CHORDS.remove], label: "Delete, archive or discard, after asking" },
  { area: "Lists", keys: ["Home", "End"], label: "First or last" },
  { area: "Conversation", keys: ["Enter"], label: "Send, or steer the running turn" },
  { area: "Conversation", keys: [{ key: "Enter", alt: true }], label: "Queue after this turn" },
  { area: "Conversation", keys: [{ key: "Enter", shift: true }], label: "New line" },
  { area: "Conversation", keys: ["Esc"], label: "Interrupt the reply, from the composer" },
  { area: "Conversation", keys: [CHORDS.confirm], label: "Accept the request waiting on you" },
  { area: "Conversation", keys: [CHORDS.remove], label: "Decline it, with the composer empty" },
  { area: "Conversation", keys: ["/"], label: "Commands, typed anywhere in the conversation" },
  // ⌘ is not a terminal key, so a Mac uses it; elsewhere Ctrl belongs to the shell and the app's keys add Shift, as other terminals do.
  { area: "Terminal", keys: [isMac ? { key: "f", mod: true } : { key: "f", mod: true, shift: true }], label: "Find in the history" },
  { area: "Terminal", keys: isMac ? [{ key: "c", mod: true }] : [{ key: "c", mod: true, shift: true }, "Ctrl+Insert"], label: "Copy the selection" },
  { area: "Terminal", keys: isMac ? [{ key: "v", mod: true }] : [{ key: "v", mod: true, shift: true }, "Shift+Insert"], label: "Paste" },
  { area: "Terminal", keys: ["Ctrl+B Ctrl+B"], label: "Send Ctrl+B to the terminal" },
  { area: "Files", keys: [CHORDS.save], label: "Save" },
  { area: "Git", keys: [CHORDS.confirm], label: "Commit" },
];
export const KEY_AREAS: readonly KeyArea[] = ["Everywhere", "Lists", "Conversation", "Terminal", "Files", "Git"];

/** F2, with no modifier: rename what is focused. */
export function isRename(event: Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey">): boolean {
  return event.key === "F2" && !event.metaKey && !event.ctrlKey && !event.shiftKey && !event.altKey;
}
/** Delete, or ⌘⌫ on a Mac, whose keyboards have no Delete: remove what is focused. */
export function isRemove(event: Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey">, mac = isMac): boolean {
  if (event.key === "Delete" && !event.metaKey && !event.ctrlKey && !event.shiftKey && !event.altKey) return true;
  return matches(event, CHORDS.remove, mac);
}

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

/**
 * The app's own chords: few, and none a browser keeps for itself. Going to a
 * page or a panel is Ctrl+B's; ⌘⇧ letters collided with the browser's.
 */
export const SHORTCUTS: readonly Shortcut[] = [
  { id: "palette", chord: { key: "k", mod: true }, label: "Search and commands", group: "General" },
  { id: "help", chord: { key: "/", mod: true }, label: "Keyboard shortcuts", group: "General" },
  { id: "settings", chord: { key: ",", mod: true }, label: "Settings", group: "General" },
  { id: "sidebar", chord: { key: "b", mod: true }, label: "Show or hide the sidebar", group: "General", mac: true },
];

/** The chords this platform has. */
export function platformShortcuts(mac = isMac): Shortcut[] { return SHORTCUTS.filter(item => !item.mac || mac); }


/** ⌘⇧O on a Mac, Ctrl+Shift+O elsewhere. */
export function formatChord(chord: Chord, mac = isMac): string {
  if (chord.prefix) return `Ctrl+B → ${chord.key.replace("Arrow", "")}`;
  const symbols: Record<string, string> = { ArrowLeft: "←", ArrowRight: "→", Enter: mac ? "↵" : "Enter", Backspace: mac ? "⌫" : "Backspace" };
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
