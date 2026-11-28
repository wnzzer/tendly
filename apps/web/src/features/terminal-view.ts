/**
 * What the terminal pane asks of whichever library draws the terminal.
 *
 * Two libraries can: wterm, the default, lays the rows out as page text the
 * browser scrolls by the pixel; xterm.js, the fallback behind
 * `?terminal=xterm`, paints a canvas. The pane owns the session stream, the
 * on-screen keys and the find bar, and talks to either through this interface,
 * so neither library's details leak into it. The implementations live in
 * `terminal-view-wterm.ts` and `terminal-view-xterm.ts`; this module stays free
 * of both libraries so its helpers can be tested on their own.
 */

import type { TerminalLinkTarget } from "./terminal-links";

/** What a view reports back to the pane. */
export interface TerminalViewEvents {
  /** Text the keyboard typed, before any on-screen modifier is applied. */
  typed(data: string): void;
  /** Bytes for the process exactly as they are: mouse reports, a paste already encoded. */
  send(bytes: Uint8Array): void;
  /** The view scrolled, or new output may have moved how far back it is. */
  scrolled(): void;
  /** The view can now draw and be written to; for wterm, once its emulator has loaded. */
  ready(): void;
  /** The view could not be built; the pane says so and offers to reconnect. */
  failed(): void;
  /**
   * The grid changed, whoever changed it. wterm refits on its own when a web
   * font loads or its element resizes, and can count one column more or fewer
   * than a fit asked for; the process must be told every time, or a program
   * that draws to the full width (Pi) wraps every line and loses its place.
   */
  resized(size: { cols: number; rows: number }): void;
  /** Progress of a find through the history, where the view can find. */
  searched?(progress: FindState): void;
  /** Text the terminal program asked to place on the clipboard via OSC 52. */
  copy?(text: string): void;
  /** Working directory reported by the shell via OSC 7, or `undefined` when cleared. */
  cwdChanged?(cwd: string | undefined): void;
  /** A file path or URL activated in the terminal (e.g. Ctrl/⌘+Click). */
  openLink?(link: TerminalLinkTarget): void;
}

export interface FindState {
  query: string;
  count: number;
  activeIndex: number;
  searching: boolean;
  limited: boolean;
}

export interface TerminalSearch {
  find(query: string): void;
  clear(): void;
  next(): void;
  previous(): void;
}

export interface TerminalView {
  /** Whether it draws and accepts writes yet. */
  readonly ready: boolean;
  /** The grid the process should be told about, once there is one. */
  size(): { cols: number; rows: number } | undefined;
  /** Fit the grid to the host element's current size. */
  fit(): void;
  write(bytes: Uint8Array): void;
  /** Forget everything drawn: the process the view showed is not the one coming. */
  reset(): void;
  /** Paste as the terminal would, honouring bracketed paste where the program asked for it. */
  paste(text: string): void;
  focus(): void;
  /** Whether the program asked for application cursor keys, which change what an arrow sends. */
  applicationCursorKeys(): boolean;
  /** Whether the foreground program currently captures mouse drags. */
  mouseTracking(): boolean;
  hasSelection(): boolean;
  /** Selected plain text in the terminal, or `""` when nothing is selected. */
  getSelection(): string;
  /** Clear any terminal selection. */
  clearSelection(): void;
  /** Select all retained scrollback and screen text. */
  selectAll(): Promise<void> | void;
  /** Clickable file reference or URL at the pointer, if any. */
  linkAt?(clientX: number, clientY: number, target: EventTarget | null): TerminalLinkTarget | undefined;
  /** The screen row under `clientY` and the cursor's row, both from the top of the screen. */
  tappedRow(clientY: number): { row: number; cursor: number } | undefined;
  isScrolledBack(): boolean;
  scrollToBottom(): void;
  applyTheme(theme: TerminalTheme): void;
  /** Find through the whole history; absent where the library cannot. */
  readonly search?: TerminalSearch;
  dispose(): void;
}

export interface TerminalTheme {
  background: string;
  foreground: string;
  cursor: string;
  selectionBackground: string;
  black?: string;
  brightBlack?: string;
  red?: string;
  brightRed?: string;
  green?: string;
  brightGreen?: string;
  yellow?: string;
  brightYellow?: string;
  blue?: string;
  brightBlue?: string;
  magenta?: string;
  brightMagenta?: string;
  cyan?: string;
  brightCyan?: string;
  white?: string;
  brightWhite?: string;
}

/**
 * The pane's palette. Dark keeps each library's own ANSI colours; light
 * replaces them, because the defaults are tuned for a dark background and
 * several wash out on white.
 */
export function terminalTheme(dark: boolean): TerminalTheme {
  return dark
    ? { background: "#17232d", foreground: "#dde6eb", cursor: "#63dac8", selectionBackground: "#2b4a46" }
    : {
        background: "#ffffff",
        foreground: "#243343",
        cursor: "#0a8278",
        selectionBackground: "#ccebe6",
        black: "#243343",
        brightBlack: "#657380",
        red: "#b4374c",
        brightRed: "#c73f55",
        green: "#187953",
        brightGreen: "#098462",
        yellow: "#916100",
        brightYellow: "#9b6800",
        blue: "#3467af",
        brightBlue: "#3375c5",
        magenta: "#7856aa",
        brightMagenta: "#8965bc",
        cyan: "#087e80",
        brightCyan: "#058587",
        white: "#607080",
        brightWhite: "#384658",
      };
}

const ANSI_NAMES = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"] as const;

/**
 * A theme as the CSS variables wterm paints with. A colour the theme leaves
 * out maps to `undefined`, so a variable set by an earlier theme is removed
 * and wterm's own default shows again.
 */
export function themeVariables(theme: TerminalTheme): Record<string, string | undefined> {
  const variables: Record<string, string | undefined> = {
    "--term-fg": theme.foreground,
    "--term-bg": theme.background,
    "--term-cursor": theme.cursor,
  };
  ANSI_NAMES.forEach((name, index) => {
    const bright = `bright${name[0]!.toUpperCase()}${name.slice(1)}` as keyof TerminalTheme;
    variables[`--term-color-${index}`] = theme[name];
    variables[`--term-color-${index + 8}`] = theme[bright];
  });
  return variables;
}

/** The row of an evenly divided grid that `clientY` falls in, or `undefined` outside it. */
export function rowAt(clientY: number, top: number, height: number, rows: number): number | undefined {
  if (height < 1 || rows < 1) return undefined;
  const row = Math.floor((clientY - top) / (height / rows));
  return row >= 0 && row < rows ? row : undefined;
}

/** One buffer holding the parts in order, for a single write instead of many. */
export function concatBytes(parts: readonly Uint8Array[]): Uint8Array {
  if (parts.length === 1) return parts[0]!;
  const joined = new Uint8Array(parts.reduce((size, part) => size + part.byteLength, 0));
  let offset = 0;
  for (const part of parts) {
    joined.set(part, offset);
    offset += part.byteLength;
  }
  return joined;
}

const MAX_OSC52_BASE64 = 1_400_000;

/**
 * Decode an OSC 52 clipboard payload (`<selection>;<base64>`). Clipboard read
 * queries (`?`) and malformed or oversized payloads return `undefined`.
 */
export function decodeOsc52(data: string): string | undefined {
  const sep = data.indexOf(";");
  if (sep < 0) return undefined;
  const encoded = data.slice(sep + 1).replace(/\s+/g, "");
  if (encoded === "?" || encoded.length > MAX_OSC52_BASE64) return undefined;
  if (!encoded) return "";
  if (encoded.length % 4 === 1 || /[^A-Za-z0-9+/=]/.test(encoded)) return undefined;
  try {
    const binary = atob(encoded);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
    return new TextDecoder("utf-8", { fatal: false }).decode(bytes);
  } catch {
    return undefined;
  }
}

/** Local path carried by an OSC 7 working-directory URI (`file://host/path`). */
export function osc7Path(uri: string): string | undefined {
  const trimmed = uri.trim();
  if (!trimmed) return undefined;
  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== "file:") return undefined;
    const decoded = decodeURIComponent(parsed.pathname);
    const normalized = decoded.replace(/^\/(?=[A-Za-z]:\/)/, "");
    return normalized || undefined;
  } catch {
    return undefined;
  }
}
