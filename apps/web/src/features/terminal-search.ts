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
 * Copying and pasting in a terminal.
 *
 * Off a Mac, Ctrl+C interrupts the foreground process unless text is selected,
 * in which case copying (and clearing the selection so a second Ctrl+C still
 * interrupts) matches modern terminals; Ctrl+Shift+C, Ctrl+Insert, Ctrl+Shift+V
 * and Shift+Insert always copy or paste. On a Mac ⌘C copies when the terminal
 * holds a selection, including canvas/WebGL selections the browser cannot see.
 */
export function terminalClipboardKey(
  event: Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey">,
  mac = isMac,
  hasSelection = false,
): "copy" | "paste" | undefined {
  if (event.altKey) return undefined;
  if (mac) {
    if (hasSelection && !event.shiftKey && !event.ctrlKey && matches(event, { key: "c", mod: true }, true))
      return "copy";
    return undefined;
  }
  if (event.metaKey) return undefined;
  if (hasSelection && !event.shiftKey && matches(event, { key: "c", mod: true }, false)) return "copy";
  if (
    matches(event, { key: "c", mod: true, shift: true }, false) ||
    (event.key === "Insert" && event.ctrlKey && !event.shiftKey)
  )
    return "copy";
  if (
    matches(event, { key: "v", mod: true, shift: true }, false) ||
    (event.key === "Insert" && event.shiftKey && !event.ctrlKey)
  )
    return "paste";
  return undefined;
}

/** Plain Ctrl+C off a Mac clears the copied selection so pressing Ctrl+C again interrupts the shell. */
export function clearsSelectionOnCopy(
  event: Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey">,
  mac = isMac,
): boolean {
  return !mac && !event.altKey && !event.metaKey && !event.shiftKey && matches(event, { key: "c", mod: true }, false);
}

export function isTerminalFind(
  event: Pick<KeyboardEvent, "key" | "metaKey" | "ctrlKey" | "shiftKey" | "altKey">,
  mac = isMac,
): boolean {
  return matches(event, terminalFindChord(mac), mac);
}

/** What wterm reports while it searches, as far as the count needs it. */
export interface FindProgress {
  count: number;
  activeIndex: number;
  searching: boolean;
  limited: boolean;
}

/** "3 / 12", with "+" past the match limit, "…" while the first matches are still coming. */
export function findCount(query: string, progress: FindProgress | undefined, noMatches: string): string {
  if (!query.trim() || !progress) return "";
  if (!progress.count) return progress.searching ? "…" : noMatches;
  const total = `${progress.count}${progress.limited || progress.searching ? "+" : ""}`;
  return progress.activeIndex >= 0 ? `${progress.activeIndex + 1} / ${total}` : total;
}
