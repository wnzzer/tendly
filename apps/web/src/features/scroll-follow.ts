/**
 * Whether a conversation keeps following its newest output after a scroll.
 *
 * Nearness to the bottom alone cannot say: a reply streaming in pulls the view
 * back down every frame, so a person scrolling up in small steps (a trackpad,
 * a gentle wheel) never got far enough to stop following. Moving up because
 * the person scrolled stops following at once; coming back near the bottom
 * resumes it. A move up nobody asked for -- content above or below shrinking
 * -- changes nothing.
 */
export function followsAfterScroll(state: { following: boolean; movedUp: boolean; byPerson: boolean; gap: number }): boolean {
  // Upward, by the person, stops following; by anything else, changes nothing --
  // not even near the bottom, where a wheel's own scroll arriving late (a busy
  // page) would otherwise read as coming back down.
  if (state.movedUp) return state.byPerson ? false : state.following;
  if (state.gap < 90) return true;
  return state.following;
}

/** Keys that move a scrolled view toward older content. */
export const UPWARD_KEYS = new Set(['ArrowUp', 'PageUp', 'Home']);
/** How long after a person's wheel, touch, key or pointer a move counts as theirs. */
export const PERSON_WINDOW_MS = 600;
