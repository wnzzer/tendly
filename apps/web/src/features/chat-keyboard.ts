export interface ChatKeyEvent {
  key: string;
  shiftKey?: boolean;
  repeat?: boolean;
}

/**
 * A key pressed while a suggestion list (slash commands, `@` files) is open
 * under the caret. The list owns the arrows, Tab and Enter, so a highlighted
 * entry is completed instead of being sent as literal text; Shift+Enter and a
 * held Enter are left to the composer.
 */
export function suggestionKey(event: ChatKeyEvent): "next" | "previous" | "choose" | "dismiss" | undefined {
  if (event.key === "ArrowDown") return "next";
  if (event.key === "ArrowUp") return "previous";
  if (event.key === "Tab" || (event.key === "Enter" && !event.shiftKey && !event.repeat)) return "choose";
  if (event.key === "Escape") return "dismiss";
  return undefined;
}
