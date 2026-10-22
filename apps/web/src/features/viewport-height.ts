import { onMounted, onUnmounted } from "vue";

/**
 * How tall the page actually is, once a phone keyboard is in the way.
 *
 * `100dvh` accounts for a browser's own chrome and nothing else: an on-screen
 * keyboard covers the bottom of the page without the page ever being told, so
 * whatever lives down there — a terminal's prompt, and the command list a
 * client draws under it — is behind the keyboard rather than above it.
 *
 * The visual viewport does know, so it is what the shell is sized from while a
 * keyboard is open, and `dvh` is left alone the rest of the time.
 */
export interface Visual { height: number; scale: number; offsetTop?: number }

/** A keyboard, as distinct from the several other things that move this number. */
const KEYBOARD = 80;

export function shellHeight(visual: Visual | undefined, innerHeight: number): number | undefined {
  // Pinching zooms the visual viewport without hiding anything. Resizing the
  // page to a pinch would fight the gesture rather than help it.
  if (!visual || visual.scale > 1.01) return undefined;
  const hidden = innerHeight - visual.height;
  // A collapsing URL bar moves this by tens of pixels and is already `dvh`'s
  // job. Only a keyboard takes this much, and only then is this worth doing.
  return hidden >= KEYBOARD ? Math.round(visual.height) : undefined;
}

/**
 * Where the visible part of the page starts, while the shell is fitted to it.
 *
 * Under a keyboard iOS does not resize the page: it pans what is visible down
 * the page to the caret. A shell sized to the visible height but left at the
 * top of the page then ends that far above the keyboard, with empty page
 * below it, and the composer floats halfway up the screen.
 */
export function shellTop(visual: Visual | undefined, height: number | undefined): number {
  return height && visual?.offsetTop ? Math.max(0, Math.round(visual.offsetTop)) : 0;
}

/**
 * Give way to a phone keyboard, for as long as the calling component is
 * mounted. Whatever lives at the bottom of the page goes under a keyboard with
 * it: in a terminal that is the prompt, and the command list a client draws
 * underneath it, which was the whole of what a slash did.
 */
export function useKeyboardFit() {
  function fitToKeyboard() {
    const visual = window.visualViewport ?? undefined, root = document.documentElement;
    const height = shellHeight(visual, window.innerHeight), top = shellTop(visual, height);
    root.style.setProperty("--app-height", height ? `${height}px` : "");
    root.style.setProperty("--app-top", top ? `${top}px` : "");
    // What sits at the very bottom only needs the home indicator's inset while no keyboard covers it.
    root.toggleAttribute("data-keyboard", !!height);
  }
  onMounted(() => {
    // The visual viewport also scrolls under a keyboard, which is the other way
    // its height stops describing what can be seen.
    window.visualViewport?.addEventListener("resize", fitToKeyboard);
    window.visualViewport?.addEventListener("scroll", fitToKeyboard);
    fitToKeyboard();
  });
  onUnmounted(() => { window.visualViewport?.removeEventListener("resize", fitToKeyboard); window.visualViewport?.removeEventListener("scroll", fitToKeyboard); });
}
