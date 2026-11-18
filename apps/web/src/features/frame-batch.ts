/**
 * Items that arrive faster than a screen can show them -- a reply streams
 * dozens of pieces a second -- applied together once per frame, so the view
 * is rebuilt and re-rendered once per frame rather than once per piece.
 * `flush` applies what is waiting at once, for an arrival that must not be
 * reordered behind the queue (a snapshot replaces what the queue builds on).
 *
 * With `leading`, the first item after a quiet spell is applied at once and
 * only what follows it waits for a frame. A terminal echoing a keystroke uses
 * this: the echo shows without a frame's delay, and a burst of output behind
 * it is still gathered. The window stays open while items keep arriving and
 * closes on the first frame that finds nothing waiting.
 */
export function createFrameBatch<T>(
  apply: (items: T[]) => void,
  schedule: (run: () => void) => unknown = defaultSchedule,
  options: { leading?: boolean } = {},
) {
  let queue: T[] = [],
    pending = false;
  const open = () => {
    pending = true;
    schedule(flush);
  };
  const flush = () => {
    pending = false;
    if (!queue.length) return;
    const items = queue;
    queue = [];
    apply(items);
    if (options.leading) open();
  };
  return {
    push(item: T) {
      if (options.leading && !pending) {
        apply([item]);
        open();
        return;
      }
      queue.push(item);
      if (!pending) open();
    },
    flush,
    /** Drops what is waiting: the view it was for has gone. */
    clear() {
      queue = [];
    },
    get size() {
      return queue.length;
    },
  };
}

/** The next frame; a hidden page has none, so a short timer stands in. */
function defaultSchedule(run: () => void) {
  if ((typeof document !== "undefined" && document.hidden) || typeof requestAnimationFrame !== "function")
    return setTimeout(run, 100);
  return requestAnimationFrame(() => run());
}
