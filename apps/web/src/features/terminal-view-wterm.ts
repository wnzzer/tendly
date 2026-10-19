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
      await view.init();
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
