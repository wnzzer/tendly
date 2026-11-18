/**
 * Scrolling the terminal with a finger, the way a page scrolls.
 *
 * Dragging alone moves the view exactly as far as the finger travels, which
 * makes reading back through a long answer a long series of swipes. A flick
 * keeps going after the finger lifts and slows down on its own, so one quick
 * swipe covers a screenful or more and a touch stops it.
 */

/** Only the finger's last stretch of travel decides how hard it was flicked. */
export const FLING_WINDOW_MS = 100;
/** Slower than this at release (px/ms) is a drag that ended, not a flick. */
export const FLING_MIN_VELOCITY = 0.25;
/** Below this (px/ms) a coasting view has come to rest. */
export const FLING_STOP_VELOCITY = 0.02;
/** How quickly a flick slows: the time constant of its decay, as on iOS. */
export const FLING_TIME_CONSTANT_MS = 325;

export type TouchSample = { y: number; at: number };

/**
 * Keeps the samples a release velocity is read from, dropping the ones too old
 * to matter so a long drag does not grow the list without bound.
 */
export function addSample(samples: TouchSample[], sample: TouchSample): TouchSample[] {
  const recent = samples.filter(entry => sample.at - entry.at <= FLING_WINDOW_MS);
  recent.push(sample);
  return recent;
}

/**
 * How fast the content was moving when the finger lifted, in px/ms, positive
 * when the finger was moving up (bringing later lines into view). A finger
 * that stopped before lifting has no velocity left, however fast it went first.
 */
export function releaseVelocity(samples: TouchSample[], now: number): number {
  const recent = samples.filter(entry => now - entry.at <= FLING_WINDOW_MS);
  const first = recent[0],
    last = recent[recent.length - 1];
  if (!first || !last || last.at <= first.at) return 0;
  const velocity = (first.y - last.y) / (last.at - first.at);
  return Math.abs(velocity) >= FLING_MIN_VELOCITY ? velocity : 0;
}

/** One frame of a coasting flick: how far it moves and how fast it goes next. */
export function flingStep(velocity: number, elapsedMs: number): { distance: number; velocity: number } {
  const dt = Math.max(0, Math.min(elapsedMs, 64));
  const next = velocity * Math.exp(-dt / FLING_TIME_CONSTANT_MS);
  return { distance: ((velocity + next) / 2) * dt, velocity: Math.abs(next) < FLING_STOP_VELOCITY ? 0 : next };
}
