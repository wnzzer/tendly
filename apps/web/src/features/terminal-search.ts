import { isMac, matches, type Chord } from "./shortcuts";

/**
 * Finding in the terminal's whole history.
 *
 * A focused terminal keeps its keys, Ctrl+F among them: the shell moves the
 * cursor with it. ⌘ is not a terminal key, so a Mac finds with ⌘F; elsewhere
 * finding takes Ctrl+Shift+F, as other terminals do.
 */
export function terminalFindChord(mac = isMac): Chord {
  return mac ? { key: "f", mod: true } : { key: "f", mod: true, shift: true };
}

/**
 * Copying and pasting off a Mac, where Ctrl+C and Ctrl+V are the shell's:
 * Ctrl+Shift+C and V, as other terminals take them, or Ctrl+Insert and
 * Shift+Insert. On a Mac ⌘C and ⌘V already do it.
 */
export function terminalClipboardKey(event: Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey">, mac = isMac): "copy" | "paste" | undefined {
  if (mac || event.altKey || event.metaKey) return undefined;
  if (matches(event, { key: "c", mod: true, shift: true }, false) || (event.key === "Insert" && event.ctrlKey && !event.shiftKey)) return "copy";
  if (matches(event, { key: "v", mod: true, shift: true }, false) || (event.key === "Insert" && event.shiftKey && !event.ctrlKey)) return "paste";
  return undefined;
}

export function isTerminalFind(event: Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey">, mac = isMac): boolean {
  return matches(event, terminalFindChord(mac), mac);
}

/** What wterm reports while it searches, as far as the count needs it. */
export interface FindProgress { count: number; activeIndex: number; searching: boolean; limited: boolean }

/** "3 / 12", with "+" past the match limit, "…" while the first matches are still coming. */
export function findCount(query: string, progress: FindProgress | undefined, noMatches: string): string {
  if (!query.trim() || !progress) return "";
  if (!progress.count) return progress.searching ? "…" : noMatches;
  const total = `${progress.count}${progress.limited || progress.searching ? "+" : ""}`;
  return progress.activeIndex >= 0 ? `${progress.activeIndex + 1} / ${total}` : total;
}
