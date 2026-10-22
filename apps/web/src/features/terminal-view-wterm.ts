import type { WTerm } from "@wterm/dom";
import type { GhosttyCore } from "@wterm/ghostty";
import "@wterm/dom/css";
import { createFrameBatch } from "./frame-batch";
import { quietGhostty } from "./terminal-engine";
import { concatBytes, themeVariables, type TerminalTheme, type TerminalView, type TerminalViewEvents } from "./terminal-view";

/**
 * Draw with wterm: Ghostty's emulator, compiled to WebAssembly, with the rows
 * laid out as page text in an element the browser scrolls. Scrolling back then
 * moves by the pixel on the compositor, with the platform's own momentum,
 * instead of repainting a canvas a whole row at a time.
 *
 * The emulator loads asynchronously. Until it is up the view is not ready:
 * output that arrives meanwhile is kept and written once it is, and `ready`
 * is reported then.
 */
/**
 * Focus taken by a terminal's init, and from what, while any init is under
 * way. Terminals loading together take focus from one another; giving it
 * back to one that only had it by taking it would keep the steal, so the
 * chain is followed back to what the person or the page last focused.
 */
const takenFrom = new Map<Element, Element | null>();
let initializing = 0;

export function createWtermView(host: HTMLElement, theme: TerminalTheme, events: TerminalViewEvents): TerminalView {
  const surface = document.createElement("div");
  surface.className = "wterm-surface";
  host.replaceChildren(surface);
  let term: WTerm | undefined;
  let core: GhosttyCore | undefined;
  let early: Uint8Array[] = [];
  let disposed = false;
  /*
   * A busy client's output arrives as thousands of small frames, and handing
   * wterm each one on its own costs more in per-write bookkeeping than the
   * parsing does. The first frame after a quiet spell is written at once, so a
   * keystroke echoes without delay; the rest of a burst goes in once a frame.
   */
  const output = createFrameBatch<Uint8Array>(parts => {
    if (!disposed) term?.write(concatBytes(parts));
  }, undefined, { leading: true });
  const scrolled = () => events.scrolled();

  applyTheme(theme);
  quietGhostty();
  void load();

  async function load() {
    try {
      const [{ WTerm }, { GhosttyCore }, { default: wasmPath }] = await Promise.all([
        import("@wterm/dom"),
        import("@wterm/ghostty"),
        import("@wterm/ghostty/ghostty-vt.wasm?url"),
      ]);
      // Ghostty budgets history in bytes; this keeps several thousand wide rows.
      const loaded = await GhosttyCore.load({ wasmPath, scrollbackLimit: 8_000_000 });
      const view = new WTerm(surface, {
        core: loaded,
        cursorBlink: true,
        onData: events.typed,
        onBinary: events.send,
        onResize: (cols, rows) => events.resized({ cols, rows }),
        onSearchChange: state => events.searched?.(state),
      });
      // wterm focuses its input during init. A background pane must not steal
      // the user's focus or emit a pane-selection event while mounting. Focus
      // goes back to what wterm took it from -- not to what had it when init
      // began, which the person may have left for this very terminal since
      // (focusing the host is asking for it) or for another one.
      let wanted = host.contains(document.activeElement), stolenFrom: Element | null = null;
      const quietFocus = (event: FocusEvent) => {
        event.stopPropagation();
        if (event.target === host) { wanted = true; return; }
        const from = event.relatedTarget instanceof Element && !host.contains(event.relatedTarget) ? event.relatedTarget : null;
        if (from) stolenFrom = from;
        // Focus passed on inside a terminal that was asked for it is its own, not taken.
        if (!wanted && event.target instanceof Element && (from || !event.relatedTarget)) takenFrom.set(event.target, from);
      };
      host.addEventListener("focusin", quietFocus);
      initializing++;
      try { await view.init(); }
      finally {
        host.removeEventListener("focusin", quietFocus);
        const active = document.activeElement;
        if (!wanted && active instanceof HTMLElement && host.contains(active)) {
          let from = stolenFrom as Element | null;
          for (let hops = 0; from && takenFrom.has(from) && hops < 16; hops++) from = takenFrom.get(from) ?? null;
          // Traced back to this terminal itself: it was asked for before its init began, and keeps the keys.
          if (from && host.contains(from)) { /* stays */ }
          else if (from instanceof HTMLElement && from.isConnected) from.focus({ preventScroll: true });
          else active.blur();
        }
        if (--initializing === 0) takenFrom.clear();
      }
      if (disposed) {
        view.destroy();
        loaded.dispose();
        return;
      }
      term = view;
      core = loaded;
      for (const bytes of early) view.write(bytes);
      early = [];
      surface.addEventListener("scroll", scrolled, { passive: true });
      events.ready();
    } catch {
      if (!disposed) events.failed();
    }
  }

  function applyTheme(next: TerminalTheme) {
    for (const [name, value] of Object.entries(themeVariables(next))) {
      if (value) surface.style.setProperty(name, value);
      else surface.style.removeProperty(name);
    }
  }

  return {
    get ready() { return !!term; },
    size: () => (term ? { cols: term.cols, rows: term.rows } : undefined),
    fit: () => term?.fit(),
    write(bytes) {
      if (term) output.push(bytes);
      // The frame's buffer is only borrowed; keep a copy until there is a terminal to write it to.
      else early.push(bytes.slice());
    },
    reset() {
      if (term) {
        output.clear();
        term.write("\x1bc");
      } else {
        early = [];
      }
    },
    paste(text) {
      if (!term) return;
      // Line breaks go in as Enter, and wrapped as a bracketed paste when the program asked for one.
      const body = text.replace(/\r?\n/g, "\r");
      events.send(new TextEncoder().encode(core?.bracketedPaste() ? `\x1b[200~${body}\x1b[201~` : body));
    },
    focus: () => term?.focus(),
    applicationCursorKeys: () => !!core?.cursorKeysApp(),
    hasSelection: () => !!term?.getSelectionText(),
    tappedRow(clientY) {
      if (!core) return undefined;
      // wterm keeps the live screen's rows apart from the history rows above them.
      const rows = [...surface.querySelectorAll(".term-row:not(.term-scrollback-row)")];
      const row = rows.findIndex(element => {
        const bounds = element.getBoundingClientRect();
        return clientY >= bounds.top && clientY < bounds.bottom;
      });
      return row >= 0 ? { row, cursor: core.getCursor().row } : undefined;
    },
    isScrolledBack: () => !!term && surface.scrollHeight - surface.scrollTop - surface.clientHeight > 5,
    scrollToBottom() {
      if (term) surface.scrollTop = surface.scrollHeight;
    },
    applyTheme,
    search: {
      find: query => term?.search(query),
      clear: () => term?.clearSearch(),
      next: () => term?.findNext(),
      previous: () => term?.findPrevious(),
    },
    dispose() {
      disposed = true;
      output.clear();
      early = [];
      surface.removeEventListener("scroll", scrolled);
      // A broken renderer should still leave the pane able to retry.
      try {
        term?.destroy();
        core?.dispose();
      } catch { /* Already gone. */ }
      term = undefined;
      core = undefined;
    },
  };
}
