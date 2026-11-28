import { fileReference, safeWebUrl } from "./markdown-model";
import { isMac } from "./shortcuts";

/**
 * Clickable file paths and web URLs in terminal output.
 *
 * Compiler diagnostics, test runners, `git status`, `git diff`, `ripgrep` and
 * `ls` print paths in a handful of shapes: `src/a.ts:42:7`, `src/a.ts(42,7)`,
 * `File "a.py", line 42`, `+++ b/src/a.ts`, or bare names like `Cargo.toml`.
 * Modifier-clicking (⌘ on macOS, Ctrl elsewhere) opens file paths inside
 * AgentDock and web URLs in a new browser tab, while leaving ordinary clicks
 * to the terminal and its TUI programs.
 */
export type TerminalLinkTarget =
  | { kind: "file"; text: string; path: string; line?: number; start: number; end: number }
  | { kind: "url"; text: string; href: string; start: number; end: number };

export function isTerminalLinkModifier(
  event: Pick<MouseEvent | KeyboardEvent, "ctrlKey" | "metaKey" | "altKey">,
  mac = isMac,
): boolean {
  return !event.altKey && (mac ? event.metaKey : event.ctrlKey);
}

const TOKEN_PATTERN = /[^\s'"`<>[\]{}()│|]+/g;
const TRAILING_PUNCTUATION = /[.,;:!?]+$/;
const GIT_DIFF_LINE = /^(?:diff --git |--- |\+\+\+ )/;
const PROSE_ABBREVIATIONS = new Set(["e.g", "i.e", "etc", "vs"]);

/** Parse a single candidate token (plus optional trailing `(line,col)` in `line`) into a link target. */
function parseToken(
  raw: string,
  tokenStart: number,
  fullLine: string,
  gitDiffLine: boolean,
): TerminalLinkTarget | undefined {
  let start = tokenStart;
  let end = tokenStart + raw.length;
  let cleaned = raw;

  // Strip a trailing colon or punctuation that compilers append (`a.rs:10:5:`).
  const trailing = cleaned.match(TRAILING_PUNCTUATION);
  if (trailing) {
    cleaned = cleaned.slice(0, -trailing[0].length);
    end -= trailing[0].length;
  }
  if (!cleaned) return undefined;

  const url = safeWebUrl(cleaned);
  if (url) {
    return { kind: "url", text: cleaned, href: url, start, end };
  }

  // TypeScript / MSBuild style `src/App.vue(42,10)` where `(` broke TOKEN_PATTERN.
  let parenLine: number | undefined;
  const after = fullLine.slice(end);
  const parenMatch = after.match(/^\((\d{1,7})(?:,\d{1,5})?\)/);
  if (parenMatch) {
    parenLine = Number(parenMatch[1]);
    end += parenMatch[0].length;
    cleaned = fullLine.slice(start, end);
  }

  // Python traceback style `File "path/to/file.py", line 42`.
  if (parenLine === undefined) {
    const pyMatch = fullLine.slice(end).match(/^["']?,\s*line\s+(\d{1,7})\b/);
    if (pyMatch) parenLine = Number(pyMatch[1]);
  }

  const candidate = parenMatch ? cleaned.slice(0, cleaned.length - parenMatch[0].length) : cleaned;
  if (PROSE_ABBREVIATIONS.has(candidate.toLowerCase())) return undefined;

  // `git diff` prefixes paths with `a/` and `b/`.
  let normalized = candidate;
  if (gitDiffLine && (normalized.startsWith("a/") || normalized.startsWith("b/")) && normalized.length > 2) {
    normalized = normalized.slice(2);
  }

  const ref = fileReference(normalized);
  if (!ref) return undefined;
  const line = ref.line ?? parenLine;
  return {
    kind: "file",
    text: fullLine.slice(start, end),
    path: ref.path,
    ...(line !== undefined ? { line } : {}),
    start,
    end,
  };
}

/** Every clickable link or file reference on one logical terminal line. */
export function findTerminalLinks(line: string): TerminalLinkTarget[] {
  if (!line || line.length > 4096) return [];
  const links: TerminalLinkTarget[] = [];
  const gitDiff = GIT_DIFF_LINE.test(line);
  for (const match of line.matchAll(TOKEN_PATTERN)) {
    const start = match.index ?? 0;
    const raw = match[0];
    if (links.length && start < links[links.length - 1]!.end) continue;
    const parsed = parseToken(raw, start, line, gitDiff);
    if (parsed) links.push(parsed);
  }
  return links;
}

/** The link covering character `offset` in `line`, if any. */
export function terminalLinkAt(line: string, offset: number): TerminalLinkTarget | undefined {
  if (offset < 0 || offset >= line.length) return undefined;
  return findTerminalLinks(line).find(link => offset >= link.start && offset < link.end);
}

/**
 * Parse an OSC 8 cell hyperlink URI (`file:///path#L10-L20`, `file:///etc/hosts`,
 * or `https://...`), even when its visible label is shortened or arbitrary text
 * or the target file has no extension.
 */
export function parseOsc8Link(uri: string, label?: string): TerminalLinkTarget | undefined {
  const trimmed = uri.trim();
  if (!trimmed) return undefined;
  const web = safeWebUrl(trimmed);
  if (web) {
    const text = label?.trim() || web;
    return { kind: "url", text, href: web, start: 0, end: text.length };
  }
  if (!/^file:\/\//i.test(trimmed)) return undefined;
  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== "file:") return undefined;
    let decodedPath = decodeURIComponent(parsed.pathname).replace(/^\/(?=[A-Za-z]:\/)/, "");
    let line: number | undefined;
    const hashMatch = parsed.hash.match(/^#L?(\d{1,7})(?:[-:]L?\d{1,7})?$/i);
    if (hashMatch) line = Number(hashMatch[1]);
    const colonMatch = decodedPath.match(/^(.*?):(\d{1,7})(?::\d{1,5})?$/);
    if (colonMatch && colonMatch[1]) {
      decodedPath = colonMatch[1];
      if (line === undefined) line = Number(colonMatch[2]);
    }
    if (!decodedPath || decodedPath.endsWith("/")) return undefined;
    const display = label?.trim() || (line !== undefined ? `${decodedPath}:${line}` : decodedPath);
    return {
      kind: "file",
      text: display,
      path: decodedPath,
      ...(line !== undefined ? { line } : {}),
      start: 0,
      end: display.length,
    };
  } catch {
    return undefined;
  }
}

const BOX_LEFT = /^[│┃║╎╏▏▎▍▌▋▊▉█▕\s]+/;
const BOX_RIGHT = /[│┃║╎╏▏▎▍▌▋▊▉█▕\s]+$/;
const PATH_TAIL = /(?:[A-Za-z]:[\\/]|\/|\.{1,2}[\\/]|[\w@+-]+[\\/])[\w.@+\\/-]*$/;
const PATH_HEAD = /^[\w.@+-]+(?:[\\/][\w.@+-]+)*(?:(?::|#L)\d{1,7}(?::\d{1,5}|-L?\d{1,7})?|\(\d{1,7}(?:,\d{1,5})?\))?/;

function stripBoxRow(row: string): { core: string; start: number; end: number } {
  const left = row.match(BOX_LEFT)?.[0].length ?? 0;
  const rest = row.slice(left);
  const rightLen = rest.match(BOX_RIGHT)?.[0].length ?? 0;
  const end = row.length - rightLen;
  return end > left ? { core: row.slice(left, end), start: left, end } : { core: "", start: left, end: left };
}

/**
 * Stitch adjacent TUI rows when a full-screen program (like `agy` or BubbleTea)
 * wraps a long file path inside a bordered or indented box without setting VT
 * soft-wrap flags.
 */
export function stitchTuiRows(
  rows: readonly string[],
  targetIdx: number,
  charOffsetInTarget: number,
): { text: string; offset: number } | undefined {
  if (targetIdx < 0 || targetIdx >= rows.length) return undefined;
  const stripped = rows.map(stripBoxRow);
  const canJoin = (upperIdx: number) => {
    const upper = stripped[upperIdx]!;
    const lower = stripped[upperIdx + 1]!;
    return !!upper.core && !!lower.core && PATH_TAIL.test(upper.core) && PATH_HEAD.test(lower.core);
  };

  let first = targetIdx;
  while (first > 0 && canJoin(first - 1)) first--;
  let last = targetIdx;
  while (last + 1 < rows.length && canJoin(last)) last++;
  if (first === last) return undefined;

  let text = "";
  let mappedOffset = -1;
  for (let i = first; i <= last; i++) {
    const seg = stripped[i]!;
    if (i === targetIdx) {
      if (charOffsetInTarget < seg.start || charOffsetInTarget >= seg.end) return undefined;
      mappedOffset = text.length + (charOffsetInTarget - seg.start);
    }
    text += seg.core;
  }
  return mappedOffset >= 0 ? { text, offset: mappedOffset } : undefined;
}

/** Normalize `.` and `..` segments in a slash-separated absolute path. */
export function resolvePosixPath(base: string, relative: string): string {
  const cleanBase = base.replace(/\\/g, "/").replace(/\/+$/, "");
  const drive = cleanBase.match(/^[A-Za-z]:/)?.[0] ?? "";
  const body = drive ? cleanBase.slice(drive.length) : cleanBase;
  const stack = body.split("/").filter(Boolean);
  for (const part of relative.replace(/\\/g, "/").split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") stack.pop();
    else stack.push(part);
  }
  return `${drive}/${stack.join("/")}`;
}

/**
 * Combine a clicked `path` with the shell's reported `cwd` (from OSC 7) when
 * helpful. Explicit `./` or `../` always resolves against `cwd`; when `cwd` is
 * outside every workspace, bare/relative paths also resolve against `cwd` so
 * `HostFilePreview` can open them; inside a workspace, `candidate` and
 * `fallback` let the caller prefer whichever file actually exists.
 */
export function resolveWithTerminalCwd(
  path: string,
  cwd: string | undefined,
  workspaceRoot?: string,
): { path: string; fallbackPath?: string } {
  if (!cwd || /^([A-Za-z]:)?\//.test(path)) return { path };
  const joined = resolvePosixPath(cwd, path);
  if (path.startsWith("./") || path.startsWith("../")) return { path: joined };
  if (!workspaceRoot) return { path: joined };
  const normRoot = workspaceRoot.replace(/\\/g, "/").replace(/\/+$/, "");
  const normCwd = cwd.replace(/\\/g, "/").replace(/\/+$/, "");
  if (normCwd === normRoot) return { path };
  if (normCwd.startsWith(normRoot + "/")) {
    const sub = normCwd.slice(normRoot.length + 1);
    // `git status` / `git diff` run inside `apps/web` still prints `apps/web/src/App.vue`.
    if (path === sub || path.startsWith(sub + "/")) return { path };
    return { path: joined, fallbackPath: path };
  }
  return { path: joined };
}
