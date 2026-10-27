/**
 * Key sequences for the on-screen controls a touch device needs.
 *
 * A phone has no arrow keys and no Enter, which is exactly what a full-screen
 * client command such as `/config` asks for. These produce the bytes a physical
 * key would, so the client cannot tell the difference.
 */
export type ArrowKey = "up" | "down" | "left" | "right";

/**
 * Arrow keys have two encodings, and which one is correct is the terminal's
 * state rather than a preference. A full-screen program usually switches the
 * terminal into application cursor mode (DECCKM), after which it expects `ESC O
 * A`; outside that mode the same key is `ESC [ A`. Sending the wrong one is
 * silently ignored or, worse, printed, so the current mode decides.
 */
export function arrowSequence(key: ArrowKey, applicationCursorKeys: boolean, modifiers: KeyModifiers = {}): string {
  const final = { up: "A", down: "B", right: "C", left: "D" }[key];
  // A modified arrow has one encoding in either mode: `CSI 1 ; m X`, where m is
  // 1 plus Shift (1) and Ctrl (4) -- what xterm sends for Ctrl+Left to jump a word.
  const code = 1 + (modifiers.shift ? 1 : 0) + (modifiers.ctrl ? 4 : 0);
  if (code > 1) return `\x1b[1;${code}${final}`;
  return applicationCursorKeys ? `\x1bO${final}` : `\x1b[${final}`;
}

/** The one-shot modifiers armed on the bar for the next key. */
export interface KeyModifiers { shift?: boolean; ctrl?: boolean }

/** What the Enter key sends: a carriage return, not a newline. */
export const ENTER_SEQUENCE = "\r";

/**
 * Escape, which a phone keyboard does not have at all and which is how a
 * running agent is interrupted or a menu dismissed.
 */
export const ESCAPE_SEQUENCE = "\x1b";

/** Tab, and Shift+Tab (CBT), which agent clients use to cycle their modes. */
export const TAB_SEQUENCE = "\t";
export const BACKTAB_SEQUENCE = "\x1b[Z";

/**
 * Tab with the bar's Shift armed is Shift+Tab, which agent clients use to cycle
 * their modes. Shift is a modifier rather than a key of its own so the bar does
 * not carry two Tab buttons that look alike.
 */
export function tabSequence(modifiers: KeyModifiers = {}): string {
  return modifiers.shift ? BACKTAB_SEQUENCE : TAB_SEQUENCE;
}

/** A character typed on the soft keyboard while the bar's Shift is armed. */
export function shiftSequence(data: string): string {
  return data.length === 1 ? data.toUpperCase() : data;
}

/**
 * What a key typed while Ctrl is held sends.
 *
 * A phone keyboard has no Ctrl, so the bar offers a one-shot modifier and the
 * next character typed on the soft keyboard is turned into its control code:
 * the letter's low five bits, which is what a terminal has always sent. Only a
 * single character has a control form; anything else -- a word from an input
 * method, a paste -- passes through unchanged rather than being mangled.
 */
export function ctrlSequence(data: string): string {
  if (data.length !== 1) return data;
  if (data === " " || data === "@" || data === "2") return "\x00";
  if (data === "?") return "\x7f";
  const code = data.toUpperCase().charCodeAt(0);
  // `A`..`Z` and the five punctuation keys after them: [ \ ] ^ _
  if (code >= 0x41 && code <= 0x5f) return String.fromCharCode(code & 0x1f);
  return data;
}

/**
 * How many arrow presses move a list selection from one row to another.
 *
 * This is the tap-to-select gesture, and it rests on an assumption worth
 * stating: that the program keeps the cursor on the row it has highlighted.
 * List menus generally do, because that is how a screen reader follows them.
 * A program that parks its cursor elsewhere will move by the wrong amount, so
 * the gesture is only offered where a full-screen program is actually drawing.
 *
 * The count is bounded because a mistaken row — or a tap in a program that
 * does not behave this way — should cost a few keystrokes, not hundreds.
 */
export function selectionPresses(cursorRow: number, targetRow: number, limit = 40): { key: ArrowKey; count: number } | undefined {
  if (!Number.isInteger(cursorRow) || !Number.isInteger(targetRow)) return undefined;
  const distance = targetRow - cursorRow;
  if (distance === 0) return undefined;
  const count = Math.min(Math.abs(distance), limit);
  return { key: distance < 0 ? "up" : "down", count };
}

/** The bytes for `count` presses of one arrow key. */
export function repeatArrow(key: ArrowKey, count: number, applicationCursorKeys: boolean): string {
  return arrowSequence(key, applicationCursorKeys).repeat(Math.max(0, count));
}

/**
 * Whether a pointer gesture was a tap rather than a drag or a text selection.
 *
 * Selecting text and scrolling both begin as a touch on the same surface, so a
 * gesture only counts as a tap when the finger stayed put, lifted quickly, and
 * left nothing selected behind it.
 */
export function isTap(gesture: { movedPx: number; elapsedMs: number; hasSelection: boolean }): boolean {
  return !gesture.hasSelection && gesture.movedPx <= 10 && gesture.elapsedMs <= 500;
}

/**
 * Which program a terminal runs, for the keys its bar puts within reach.
 * Claude Code and Codex share most of theirs; a shell wants other ones.
 */
export type KeyClient = "claude_code" | "codex" | "shell";
export function keyClient(provider?: string | null): KeyClient {
  return provider === "claude_code" || provider === "codex" ? provider : "shell";
}

/**
 * A key on the bar that sends fixed bytes. `send` holds one write per key
 * press: Esc twice is two presses, because a single write of two escapes reads
 * to a client as Alt+Esc rather than as Esc pressed twice.
 */
export interface FixedKey { label: string; title: string; send: string[]; translated?: boolean }

/**
 * Every fixed key the bar can show. The control codes are what a physical
 * keyboard sends for Ctrl and the letter; the titles say what the clients do
 * with them, as each client's own `?` shortcut list describes it.
 */
export const FIXED_KEYS = {
  esc: { label: "Esc", title: "Escape · interrupt or dismiss", send: [ESCAPE_SEQUENCE] },
  escEsc: { label: "Esc×2", title: "Escape twice", send: [ESCAPE_SEQUENCE, ESCAPE_SEQUENCE] },
  shiftTab: { label: "⇧Tab", title: "Shift+Tab · change mode", send: [BACKTAB_SEQUENCE] },
  tab: { label: "Tab", title: "Tab", send: [TAB_SEQUENCE] },
  slash: { label: "/", title: "Slash · command menu", send: ["/"] },
  newline: { label: "⏎ New line", title: "New line without sending (Ctrl+J)", send: ["\n"], translated: true },
  ctrlA: { label: "^A", title: "Ctrl+A · start of line", send: ["\x01"] },
  ctrlC: { label: "^C", title: "Ctrl+C · cancel", send: ["\x03"] },
  ctrlD: { label: "^D", title: "Ctrl+D · end of input", send: ["\x04"] },
  ctrlE: { label: "^E", title: "Ctrl+E · end of line", send: ["\x05"] },
  ctrlL: { label: "^L", title: "Ctrl+L · clear the screen", send: ["\x0c"] },
  ctrlO: { label: "^O", title: "Ctrl+O · verbose output", send: ["\x0f"] },
  ctrlR: { label: "^R", title: "Ctrl+R · search history", send: ["\x12"] },
  ctrlT: { label: "^T", title: "Ctrl+T", send: ["\x14"] },
  ctrlZ: { label: "^Z", title: "Ctrl+Z · suspend", send: ["\x1a"] },
} as const satisfies Record<string, FixedKey>;
export type FixedKeyId = keyof typeof FIXED_KEYS;

/** What a client does with a key, where that differs from the key's own title. */
const CLIENT_TITLES: Partial<Record<KeyClient, Partial<Record<FixedKeyId, string>>>> = {
  claude_code: { escEsc: "Escape twice · clear the input", ctrlT: "Ctrl+T · show or hide tasks" },
  codex: { escEsc: "Escape twice · edit the last message", ctrlT: "Ctrl+T · open the transcript", tab: "Tab · queue the message" },
};
export function keyTitle(client: KeyClient, id: FixedKeyId): string {
  return CLIENT_TITLES[client]?.[id] ?? FIXED_KEYS[id].title;
}

/** A place on the bar: a fixed key, an arrow, Enter, or a one-shot modifier. */
export type BarSlot = FixedKeyId | ArrowKey | "enter" | "ctrl" | "shift";

/**
 * What each client's bar holds. The first row fits a phone held upright and
 * has the keys used many times a session; the rest fold behind ⋯. Ctrl and
 * Shift stay as one-shot modifiers there, for any combination not listed.
 */
export const KEY_LAYOUTS: Record<KeyClient, { main: BarSlot[]; more: BarSlot[] }> = {
  claude_code: {
    main: ["esc", "shiftTab", "ctrlC", "slash", "left", "up", "down", "right", "enter"],
    more: ["newline", "escEsc", "ctrlO", "ctrlT", "tab", "ctrl", "shift"],
  },
  codex: {
    main: ["esc", "shiftTab", "ctrlC", "slash", "left", "up", "down", "right", "enter"],
    more: ["newline", "escEsc", "ctrlR", "ctrlT", "tab", "ctrl", "shift"],
  },
  shell: {
    main: ["tab", "ctrlC", "ctrlR", "ctrlD", "left", "up", "down", "right", "enter"],
    more: ["esc", "ctrlL", "ctrlZ", "ctrlA", "ctrlE", "ctrl", "shift"],
  },
};
/** A key's label: a key's own name stays as printed on keyboards; words are translated. */
export function keyLabel(id: FixedKeyId, t: (text: string) => string): string {
  const key: FixedKey = FIXED_KEYS[id];
  return key.translated ? t(key.label) : key.label;
}
export function isFixedKey(slot: BarSlot): slot is FixedKeyId {
  return slot in FIXED_KEYS;
}
