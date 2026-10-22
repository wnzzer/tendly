/**
 * Panes this page has already shown. A pane's section is rebuilt whenever the
 * tree around it changes -- a split wraps it, maximizing lifts it out -- and a
 * rebuilt pane must not play its entrance again: it would fade in from
 * transparent, a flash on the pane just worked in.
 */
const shown = new Set<string>();
/** Whether `id` is appearing for the first time, and so enters with an animation. */
export function firstAppearance(id: string): boolean {
  if (shown.has(id)) return false;
  shown.add(id);
  return true;
}
