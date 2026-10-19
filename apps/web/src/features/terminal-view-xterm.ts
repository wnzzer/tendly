import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { Terminal } from "@xterm/xterm";
import { swipeAxis } from "./pane-swipe";
import { addSample, flingStep, releaseVelocity, type TouchSample } from "./terminal-scroll";
import { rowAt, type TerminalTheme, type TerminalView, type TerminalViewEvents } from "./terminal-view";

/**
 * Draw with xterm.js, the fallback behind `?terminal=xterm`. It paints a
 * canvas and scrolls a whole row at a time; wterm is the default because it
 * does not. xterm.js is ready as soon as it is built.
 */
export function createXtermView(host: HTMLElement, theme: TerminalTheme, events: TerminalViewEvents): TerminalView {
  const terminal = new Terminal({
    cursorBlink: true, fontSize: 12, lineHeight: 1.22, scrollback: 5000,
    // A wheel notch glides instead of jumping three lines at once; a trackpad
    // already sends fine steps and is followed as it moves.
    smoothScrollDuration: 100,
    // A Nerd Font the viewer already has wins; the bundled symbols fill in the
    // prompt icons for everyone else, phones included.
    fontFamily: '"SFMono-Regular", Consolas, "Liberation Mono", "Symbols Nerd Font Mono", "Tendly Symbols", monospace',
    theme,
  });
  const fit = new FitAddon();
  terminal.loadAddon(fit);
  terminal.open(host);
  useWebgl(terminal);
  const subscriptions = [
    terminal.onData(events.typed),
    terminal.onResize(events.resized),
    terminal.onScroll(events.scrolled),
    terminal.onWriteParsed(events.scrolled),
    terminal.onBinary(data => events.send(Uint8Array.from(data, value => value.charCodeAt(0) & 255))),
  ];
  const touch = touchScrolling(terminal, host, events.scrolled);

  return {
    ready: true,
    size: () => ({ cols: terminal.cols, rows: terminal.rows }),
    fit: () => fit.fit(),
    write: bytes => terminal.write(bytes),
    reset: () => terminal.reset(),
    // terminal.paste honours bracketed paste mode, so a multi-line paste is not run line by line.
    paste: text => terminal.paste(text),
    focus: () => terminal.focus(),
    applicationCursorKeys: () => terminal.modes.applicationCursorKeysMode,
    hasSelection: () => terminal.hasSelection(),
    tappedRow(clientY) {
      const viewport = host.querySelector(".xterm-screen") ?? host;
      const bounds = viewport.getBoundingClientRect();
      const row = rowAt(clientY, bounds.top, bounds.height, terminal.rows);
      return row === undefined ? undefined : { row, cursor: terminal.buffer.active.cursorY };
    },
    isScrolledBack() {
      const buffer = terminal.buffer.active;
      return buffer.type === "normal" && buffer.viewportY < buffer.baseY;
    },
    scrollToBottom() {
      touch.stop();
      terminal.scrollToBottom();
    },
    applyTheme(next) { terminal.options.theme = next; },
    dispose() {
      touch.dispose();
      for (const subscription of subscriptions) {
        try { subscription.dispose(); } catch { /* Best-effort cleanup after partial initialization. */ }
      }
      // A broken renderer should still leave the pane able to retry.
      try { terminal.dispose(); } catch { /* Already gone. */ }
    },
  };
}

/**
 * Draw with the GPU where there is one.
 *
 * xterm.js 6 dropped its canvas renderer, leaving the DOM one, which rebuilds
 * every visible row as spans on each scroll step -- enough to make scrolling a
 * busy client's output stutter. Without WebGL, or once the browser takes the
 * context back (it keeps only so many), the terminal goes on drawing with the
 * DOM, as it did before.
 */
function useWebgl(target: Terminal) {
  try {
    const webgl = new WebglAddon();
    webgl.onContextLoss(() => {
      try { webgl.dispose(); } catch { /* Already gone with the context. */ }
    });
    target.loadAddon(webgl);
  } catch { /* No WebGL here; the DOM renderer stays. */ }
}

/**
 * Scroll with a finger.
 *
 * xterm.js 6 scrolls for a wheel and for nothing else -- its viewport no
 * longer follows touch -- so dragging the terminal on a phone moved nothing,
 * and a conversation the client had pushed into scrollback could not be read
 * again. A vertical drag is turned into what a wheel would have done: the
 * scrollback moves in the ordinary buffer, and a program that owns the screen
 * (the alternate buffer, or one that asked for mouse reports) is handed wheel
 * steps to scroll itself. Sideways drags are left to the pane swipe. A flick
 * keeps coasting after the finger lifts, and the next touch stops it.
 *
 * wterm needs none of this: its rows are page text and the browser scrolls them.
 */
function touchScrolling(terminal: Terminal, host: HTMLElement, scrolled: () => void) {
  let drag: { x: number; y: number; axis?: "x" | "y"; carry: number; samples: TouchSample[] } | undefined;
  let fling: { frame: number; velocity: number; carry: number; at: number; x: number; y: number } | undefined;

  function stop() {
    if (fling) cancelAnimationFrame(fling.frame);
    fling = undefined;
  }
  function start(event: TouchEvent) {
    stop();
    const touch = event.touches[0];
    drag = event.touches.length === 1 && touch ? { x: touch.clientX, y: touch.clientY, carry: 0, samples: [{ y: touch.clientY, at: event.timeStamp }] } : undefined;
  }
  function move(event: TouchEvent) {
    const current = drag, touch = event.touches[0];
    if (!current || !touch || event.touches.length !== 1) {
      drag = undefined;
      return;
    }
    if (!current.axis) {
      current.axis = swipeAxis(touch.clientX - current.x, touch.clientY - current.y);
      if (current.axis === "x") drag = undefined;
      if (current.axis !== "y") return;
    }
    event.preventDefault();
    current.samples = addSample(current.samples, { y: touch.clientY, at: event.timeStamp });
    // A finger moving up brings later lines into view, as a page does.
    current.carry = scrollByPixels(current.carry + current.y - touch.clientY, touch.clientX, touch.clientY);
    current.y = touch.clientY;
  }
  function end(event: TouchEvent) {
    const current = drag;
    drag = undefined;
    if (current?.axis !== "y" || event.touches.length) return;
    const velocity = releaseVelocity(current.samples, event.timeStamp);
    if (!velocity) return;
    fling = { frame: 0, velocity, carry: current.carry, at: performance.now(), x: current.x, y: current.y };
    fling.frame = requestAnimationFrame(coast);
  }
  function cancel() { drag = undefined; }
  function coast(now: number) {
    const current = fling;
    if (!current) return;
    const step = flingStep(current.velocity, now - current.at);
    current.at = now;
    current.velocity = step.velocity;
    current.carry = scrollByPixels(current.carry + step.distance, current.x, current.y);
    // Coasting into either end of the scrollback stops there rather than pressing on.
    const buffer = terminal.buffer.active;
    const atEnd = buffer.type === "normal" && terminal.modes.mouseTrackingMode === "none" && (current.velocity > 0 ? buffer.viewportY >= buffer.baseY : buffer.viewportY <= 0);
    if (!current.velocity || atEnd) {
      fling = undefined;
      return;
    }
    current.frame = requestAnimationFrame(coast);
  }
  /** Scrolls by whole rows of `pixels` and returns the part of a row left over. */
  function scrollByPixels(pixels: number, clientX: number, clientY: number): number {
    const screen = host.querySelector<HTMLElement>(".xterm-screen");
    const rowHeight = screen ? screen.getBoundingClientRect().height / terminal.rows : 0;
    if (!screen || !(rowHeight > 0)) return 0;
    const lines = Math.trunc(pixels / rowHeight);
    if (!lines) return pixels;
    if (terminal.buffer.active.type === "normal" && terminal.modes.mouseTrackingMode === "none") {
      terminal.scrollLines(lines);
      scrolled();
      return pixels - lines * rowHeight;
    }
    // One wheel notch per line, so xterm.js reports or translates each exactly as it would a real one.
    for (let step = 0; step < Math.abs(lines); step++) {
      screen.dispatchEvent(new WheelEvent("wheel", { deltaY: Math.sign(lines), deltaMode: WheelEvent.DOM_DELTA_LINE, clientX, clientY, bubbles: true, cancelable: true }));
    }
    return pixels - lines * rowHeight;
  }

  host.addEventListener("touchstart", start);
  host.addEventListener("touchmove", move, { passive: false });
  host.addEventListener("touchend", end);
  host.addEventListener("touchcancel", cancel);
  return {
    stop,
    dispose() {
      stop();
      host.removeEventListener("touchstart", start);
      host.removeEventListener("touchmove", move);
      host.removeEventListener("touchend", end);
      host.removeEventListener("touchcancel", cancel);
    },
  };
}
