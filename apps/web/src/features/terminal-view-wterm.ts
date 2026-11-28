import type { WTerm } from "@wterm/dom";
import type { GhosttyCore } from "@wterm/ghostty";
import "@wterm/dom/css";
import { createFrameBatch } from "./frame-batch";
import { quietGhostty } from "./terminal-engine";
import {
  isTerminalLinkModifier,
  parseOsc8Link,
  stitchTuiRows,
  terminalLinkAt,
  type TerminalLinkTarget,
} from "./terminal-links";
import {
  concatBytes,
  osc7Path,
  themeVariables,
  type TerminalTheme,
  type TerminalView,
  type TerminalViewEvents,
} from "./terminal-view";

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
  const output = createFrameBatch<Uint8Array>(
    parts => {
      if (!disposed) term?.write(concatBytes(parts));
    },
    undefined,
    { leading: true },
  );
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
        onClipboardWrite: text => events.copy?.(text),
        onWorkingDirectory: uri => events.cwdChanged?.(osc7Path(uri)),
        onResize: (cols, rows) => events.resized({ cols, rows }),
        onSearchChange: state => events.searched?.(state),
      });
      // wterm focuses its input during init. A background pane must not steal
      // the user's focus or emit a pane-selection event while mounting. Focus
      // goes back to what wterm took it from -- not to what had it when init
      // began, which the person may have left for this very terminal since
      // (focusing the host is asking for it) or for another one.
      let wanted = host.contains(document.activeElement),
        stolenFrom: Element | null = null;
      const quietFocus = (event: FocusEvent) => {
        event.stopPropagation();
        if (event.target === host) {
          wanted = true;
          return;
        }
        const from =
          event.relatedTarget instanceof Element && !host.contains(event.relatedTarget) ? event.relatedTarget : null;
        if (from) stolenFrom = from;
        // Focus passed on inside a terminal that was asked for it is its own, not taken.
        if (!wanted && event.target instanceof Element && (from || !event.relatedTarget))
          takenFrom.set(event.target, from);
      };
      host.addEventListener("focusin", quietFocus);
      initializing++;
      try {
        await view.init();
      } finally {
        host.removeEventListener("focusin", quietFocus);
        const active = document.activeElement;
        if (!wanted && active instanceof HTMLElement && host.contains(active)) {
          let from = stolenFrom as Element | null;
          for (let hops = 0; from && takenFrom.has(from) && hops < 16; hops++) from = takenFrom.get(from) ?? null;
          // Traced back to this terminal itself: it was asked for before its init began, and keeps the keys.
          if (from && host.contains(from)) {
            /* stays */
          } else if (from instanceof HTMLElement && from.isConnected) from.focus({ preventScroll: true });
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
      installLinks();
      events.ready();
    } catch {
      if (!disposed) events.failed();
    }
  }

  let cleanupLinks = () => {};
  function resolveHit(
    clientX: number,
    clientY: number,
    target: EventTarget | null,
  ): { row: number; col: number } | undefined {
    if (!term || !core) return undefined;
    const element =
      target instanceof Element
        ? target
        : ((target as { parentElement?: Element | null } | null)?.parentElement ?? null);
    if (!element) return undefined;
    const internal = term as unknown as {
      renderer?: { positionAt(el: Element, x: number, w: number): { row: number; col: number } | null };
      _charWidth?: number;
    };
    if (internal.renderer && internal._charWidth && internal._charWidth > 0) {
      const hit = internal.renderer.positionAt(element, clientX, internal._charWidth);
      if (hit) return hit;
    }
    const rowEl = element.closest(".term-row");
    if (!rowEl) return undefined;
    const cols = core.getCols();
    const rect = rowEl.getBoundingClientRect();
    if (cols <= 0 || rect.width <= 0 || rect.height <= 0) return undefined;
    const col = Math.floor(((clientX - rect.left) / rect.width) * cols);
    if (col < 0 || col >= cols) return undefined;
    if (!rowEl.classList.contains("term-scrollback-row")) {
      const liveRows = [...surface.querySelectorAll(".term-row:not(.term-scrollback-row)")];
      const liveIdx = liveRows.indexOf(rowEl);
      if (liveIdx >= 0) return { row: core.getScrollbackCount() + liveIdx, col };
    }
    const grid = surface.querySelector(".term-grid");
    if (!grid) return undefined;
    const row = Math.round((rect.top - grid.getBoundingClientRect().top) / rect.height);
    return row >= 0 ? { row, col } : undefined;
  }

  function linkAt(clientX: number, clientY: number, target: EventTarget | null): TerminalLinkTarget | undefined {
    if (!core) return undefined;
    const hit = resolveHit(clientX, clientY, target);
    if (!hit) return undefined;
    const oscLink = osc8LinkAt(core, hit.row, hit.col);
    if (oscLink) return oscLink;
    const stitched = tuiStitchedLineAt(core, hit.row, hit.col);
    const stitchedHit = stitched ? terminalLinkAt(stitched.text, stitched.offset) : undefined;
    if (stitchedHit) return stitchedHit;
    const line = logicalLineAt(core, hit.row, hit.col);
    return line ? terminalLinkAt(line.text, line.offset) : undefined;
  }

  function installLinks() {
    let hovered = false;
    let suppressClick = false;
    let activeDragCleanup: (() => void) | undefined;
    const doc = surface.ownerDocument;
    const clearHover = () => {
      if (!hovered) return;
      hovered = false;
      surface.classList.remove("has-hover-link");
      surface.removeAttribute("title");
    };
    const onMouseMove = (event: MouseEvent) => {
      if (!isTerminalLinkModifier(event) || (event.target instanceof Element && event.target.closest("a.term-link"))) {
        clearHover();
        return;
      }
      const found = linkAt(event.clientX, event.clientY, event.target);
      if (!found) {
        clearHover();
        return;
      }
      hovered = true;
      surface.classList.add("has-hover-link");
      surface.title = found.text;
    };
    const onMouseDown = (event: MouseEvent) => {
      if (event.button === 2) {
        // Prevent right-click mousedown from focusing `.terminal-host` (tabindex=-1)
        // or wterm's hidden textarea, either of which wipes out the DOM selection
        // before `contextmenu` fires. `contextmenu` still fires normally.
        event.preventDefault();
        event.stopPropagation();
        return;
      }
      if (event.button !== 0) return;
      if (isTerminalLinkModifier(event) && !(event.target instanceof Element && event.target.closest("a.term-link"))) {
        if (linkAt(event.clientX, event.clientY, event.target)) {
          event.preventDefault();
          event.stopPropagation();
          return;
        }
      }
      if (event.altKey || event.ctrlKey || event.metaKey || !core || !term) return;
      const tracking = core.mouseTracking();
      // When a TUI (e.g. agy, Claude Code, tmux) enables mouse tracking, wterm
      // otherwise swallows every drag so text cannot be selected, and Shift+drag
      // fails in Chromium while the hidden textarea has focus. Handle drag-to-select
      // directly and forward non-drag clicks to the TUI.
      if (!tracking && !event.shiftKey) return;

      activeDragCleanup?.();
      const startX = event.clientX;
      const startY = event.clientY;
      const shift = event.shiftKey;
      if (event.detail >= 2) {
        const hit = resolveHit(startX, startY, event.target);
        if (hit) {
          event.preventDefault();
          event.stopPropagation();
          const selected = event.detail === 2 ? term.selectWord(hit) : term.selectLine(hit.row);
          if (selected) suppressClick = true;
        }
        return;
      }

      const startCaret = caretPointAt(doc, surface, startX, startY);
      if (!startCaret) return;
      event.preventDefault();
      event.stopPropagation();
      let dragged = false;
      if (shift) {
        doc.getSelection()?.setBaseAndExtent(startCaret.node, startCaret.offset, startCaret.node, startCaret.offset);
      }
      const onWinMove = (moveEvent: MouseEvent) => {
        if (!(moveEvent.buttons & 1)) {
          finishDrag(moveEvent);
          return;
        }
        if (!dragged && Math.hypot(moveEvent.clientX - startX, moveEvent.clientY - startY) < 4) return;
        dragged = true;
        const endCaret = caretPointAt(doc, surface, moveEvent.clientX, moveEvent.clientY);
        if (endCaret) {
          doc.getSelection()?.setBaseAndExtent(startCaret.node, startCaret.offset, endCaret.node, endCaret.offset);
        }
      };
      const finishDrag = (_upEvent: MouseEvent) => {
        activeDragCleanup?.();
        if (dragged) {
          suppressClick = true;
          return;
        }
        term?.clearSelection();
        doc.getSelection()?.removeAllRanges();
        term?.focus();
        if (!shift && core && core.mouseTracking()) {
          sendMouseClick(core, surface, startX, startY, events.send, events.typed);
        }
      };
      const win = doc.defaultView;
      win?.addEventListener("mousemove", onWinMove, true);
      win?.addEventListener("mouseup", finishDrag, true);
      activeDragCleanup = () => {
        activeDragCleanup = undefined;
        win?.removeEventListener("mousemove", onWinMove, true);
        win?.removeEventListener("mouseup", finishDrag, true);
      };
    };
    const onClick = (event: MouseEvent) => {
      if (suppressClick) {
        suppressClick = false;
        event.preventDefault();
        event.stopPropagation();
        return;
      }
      if (
        event.button !== 0 ||
        !isTerminalLinkModifier(event) ||
        (event.target instanceof Element && event.target.closest("a.term-link"))
      )
        return;
      const found = linkAt(event.clientX, event.clientY, event.target);
      if (!found) return;
      event.preventDefault();
      event.stopPropagation();
      clearHover();
      events.openLink?.(found);
    };
    const onKeyUp = (event: KeyboardEvent) => {
      if (!isTerminalLinkModifier(event)) clearHover();
    };
    surface.addEventListener("mousemove", onMouseMove);
    surface.addEventListener("mouseleave", clearHover);
    surface.addEventListener("mousedown", onMouseDown, true);
    surface.addEventListener("click", onClick, true);
    doc.addEventListener("keyup", onKeyUp);
    doc.defaultView?.addEventListener("blur", clearHover);
    cleanupLinks = () => {
      activeDragCleanup?.();
      clearHover();
      surface.removeEventListener("mousemove", onMouseMove);
      surface.removeEventListener("mouseleave", clearHover);
      surface.removeEventListener("mousedown", onMouseDown, true);
      surface.removeEventListener("click", onClick, true);
      doc.removeEventListener("keyup", onKeyUp);
      doc.defaultView?.removeEventListener("blur", clearHover);
    };
  }

  function applyTheme(next: TerminalTheme) {
    for (const [name, value] of Object.entries(themeVariables(next))) {
      if (value) surface.style.setProperty(name, value);
      else surface.style.removeProperty(name);
    }
  }

  return {
    get ready() {
      return !!term;
    },
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
    mouseTracking: () => !!core?.mouseTracking(),
    hasSelection: () => !!term?.getSelectionText(),
    getSelection: () => term?.getSelectionText() ?? "",
    clearSelection: () => term?.clearSelection(),
    async selectAll() {
      await term?.selectAll();
    },
    linkAt,
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
      cleanupLinks();
      surface.removeEventListener("scroll", scrolled);
      // A broken renderer should still leave the pane able to retry.
      try {
        term?.destroy();
        core?.dispose();
      } catch {
        /* Already gone. */
      }
      term = undefined;
      core = undefined;
    },
  };
}

const MAX_WRAPPED_SPAN = 16;

function logicalLineAt(core: GhosttyCore, row: number, col: number): { text: string; offset: number } | undefined {
  const history = core.getScrollbackCount();
  const total = history + core.getRows();
  const cols = core.getCols();
  if (row < 0 || row >= total || col < 0 || col >= cols) return undefined;
  const meta = (r: number) =>
    r < history
      ? core.getScrollbackLineLen(history - r - 1) <= cols
        ? core.getScrollbackRowMetadata?.(history - r - 1)
        : null
      : core.getRowMetadata?.(r - history);
  const wraps = (r: number) => r >= 0 && r + 1 < total && !!meta(r)?.wrapsToNext && !!meta(r + 1)?.continuesPrevious;

  let first = row;
  while (first > 0 && row - first < MAX_WRAPPED_SPAN && wraps(first - 1)) first--;
  let last = row;
  while (last + 1 < total && last - row < MAX_WRAPPED_SPAN && wraps(last)) last++;

  let text = "";
  let hitOffset = -1;
  let lastCharStart = 0;
  for (let r = first; r <= last; r++) {
    const lineLen = r < history ? Math.min(cols, core.getScrollbackLineLen(history - r - 1)) : cols;
    for (let c = 0; c < cols; c++) {
      const cell =
        r < history
          ? c < lineLen
            ? core.getScrollbackCell(history - r - 1, c)
            : undefined
          : core.getCell(r - history, c);
      if (cell?.width === 0) {
        if (r === row && c === col) hitOffset = lastCharStart;
        continue;
      }
      if (cell?.spacerHead) {
        if (r === row && c === col) hitOffset = text.length;
        continue;
      }
      if (r === row && c === col) hitOffset = text.length;
      lastCharStart = text.length;
      text += cell?.chars ?? String.fromCodePoint(cell?.char || 32);
    }
  }
  const trimmed = text.replace(/\s+$/, "");
  if (hitOffset < 0 || hitOffset >= trimmed.length) return undefined;
  return { text: trimmed, offset: hitOffset };
}

function readRowCells(core: GhosttyCore, row: number): { text: string; offsets: number[] } {
  const history = core.getScrollbackCount();
  const cols = core.getCols();
  const lineLen = row < history ? Math.min(cols, core.getScrollbackLineLen(history - row - 1)) : cols;
  let text = "";
  let lastStart = 0;
  const offsets = new Array<number>(cols);
  for (let c = 0; c < cols; c++) {
    const cell =
      row < history
        ? c < lineLen
          ? core.getScrollbackCell(history - row - 1, c)
          : undefined
        : core.getCell(row - history, c);
    if (cell?.width === 0) {
      offsets[c] = lastStart;
      continue;
    }
    if (cell?.spacerHead) {
      offsets[c] = text.length;
      continue;
    }
    offsets[c] = text.length;
    lastStart = text.length;
    text += cell?.chars ?? String.fromCodePoint(cell?.char || 32);
  }
  return { text, offsets };
}

function osc8LinkAt(core: GhosttyCore, row: number, col: number): TerminalLinkTarget | undefined {
  const history = core.getScrollbackCount();
  const total = history + core.getRows();
  const cols = core.getCols();
  if (row < 0 || row >= total || col < 0 || col >= cols) return undefined;
  const getCell = (c: number) =>
    row < history
      ? c < core.getScrollbackLineLen(history - row - 1)
        ? core.getScrollbackCell(history - row - 1, c)
        : undefined
      : core.getCell(row - history, c);
  const hitCell = getCell(col);
  const uri = hitCell?.linkUri;
  if (!uri) return undefined;
  let label = "";
  let startCol = col;
  while (startCol > 0 && getCell(startCol - 1)?.linkUri === uri) startCol--;
  for (let c = startCol; c < cols && getCell(c)?.linkUri === uri; c++) {
    const cell = getCell(c);
    if (cell?.width === 0 || cell?.spacerHead) continue;
    label += cell?.chars ?? String.fromCodePoint(cell?.char || 32);
  }
  return parseOsc8Link(uri, label);
}

function tuiStitchedLineAt(core: GhosttyCore, row: number, col: number): { text: string; offset: number } | undefined {
  const total = core.getScrollbackCount() + core.getRows();
  const cols = core.getCols();
  if (row < 0 || row >= total || col < 0 || col >= cols) return undefined;
  const first = Math.max(0, row - 2);
  const last = Math.min(total - 1, row + 2);
  const rows: string[] = [];
  let charCol = 0;
  for (let r = first; r <= last; r++) {
    const built = readRowCells(core, r);
    rows.push(built.text);
    if (r === row) charCol = built.offsets[col] ?? 0;
  }
  return stitchTuiRows(rows, row - first, charCol);
}

function caretPointAt(
  doc: Document,
  surface: HTMLElement,
  clientX: number,
  clientY: number,
): { node: Node; offset: number } | undefined {
  const anyDoc = doc as Document & {
    caretRangeFromPoint?: (x: number, y: number) => Range | null;
    caretPositionFromPoint?: (x: number, y: number) => { offsetNode: Node; offset: number } | null;
  };
  if (typeof anyDoc.caretRangeFromPoint === "function") {
    const range = anyDoc.caretRangeFromPoint(clientX, clientY);
    if (range && surface.contains(range.startContainer)) {
      return { node: range.startContainer, offset: range.startOffset };
    }
  }
  if (typeof anyDoc.caretPositionFromPoint === "function") {
    const pos = anyDoc.caretPositionFromPoint(clientX, clientY);
    if (pos && surface.contains(pos.offsetNode)) {
      return { node: pos.offsetNode, offset: pos.offset };
    }
  }
  return undefined;
}

function sendMouseClick(
  core: GhosttyCore,
  surface: HTMLElement,
  clientX: number,
  clientY: number,
  sendBytes: (bytes: Uint8Array) => void,
  sendText: (data: string) => void,
) {
  const viewportRow = surface.querySelector(".term-row:not(.term-scrollback-row)");
  const rect = (viewportRow ?? surface).getBoundingClientRect();
  const cols = core.getCols();
  const rows = core.getRows();
  if (cols <= 0 || rows <= 0 || rect.width <= 0) return;
  const rowHeight = viewportRow ? rect.height : rect.height / rows;
  if (rowHeight <= 0) return;
  const encoding = core.mouseEncoding?.() ?? (core.mouseSgr() ? "sgr" : null);
  if (!encoding) return;
  const pixels = encoding === "sgr-pixels";
  const gridHeight = rowHeight * rows;
  const x = Math.max(
    1,
    Math.min(
      pixels ? Math.ceil(rect.width) : cols,
      Math.floor((clientX - rect.left) / (pixels ? 1 : rect.width / cols)) + 1,
    ),
  );
  const y = Math.max(
    1,
    Math.min(pixels ? Math.ceil(gridHeight) : rows, Math.floor((clientY - rect.top) / (pixels ? 1 : rowHeight)) + 1),
  );
  if (encoding === "x10") {
    if (x <= 223 && y <= 223) {
      sendBytes(Uint8Array.of(0x1b, 0x5b, 0x4d, 32, x + 32, y + 32));
      sendBytes(Uint8Array.of(0x1b, 0x5b, 0x4d, 35, x + 32, y + 32));
    }
  } else if (encoding === "utf8") {
    if (x <= 2015 && y <= 2015) {
      sendText(`\x1b[M${String.fromCodePoint(32, x + 32, y + 32)}\x1b[M${String.fromCodePoint(35, x + 32, y + 32)}`);
    }
  } else if (encoding === "urxvt") {
    sendText(`\x1b[32;${x};${y}M\x1b[35;${x};${y}M`);
  } else {
    sendText(`\x1b[<0;${x};${y}M\x1b[<0;${x};${y}m`);
  }
}
