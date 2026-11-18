/**
 * What a page's back button returns to, by name. Back is a step back in the
 * app's own history (leavePage in App.vue): to the page this one was opened
 * from, or to the workspace when there was none -- a link opened directly, a
 * reload. "Back to workspace" on every page said where it went only half the time.
 */
const DESTINATIONS: Record<string, string> = {
  canvas: "Back to workspace",
  session: "Back to workspace",
  sessions: "Back to all sessions",
  settings: "Back to settings",
  usage: "Back to usage",
  system: "Back to system",
};

/** `previous` is the route name the history steps back to, or none. */
export function backLabel(previous: string | symbol | null | undefined): string {
  if (previous === undefined || previous === null) return DESTINATIONS.canvas;
  return typeof previous === "string" ? (DESTINATIONS[previous] ?? "Back") : "Back";
}
