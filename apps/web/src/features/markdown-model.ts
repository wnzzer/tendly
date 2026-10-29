/**
 * Markdown as the conversation and the file preview show it, and the file
 * references found in prose.
 *
 * A deliberately small parser: blocks and inlines are turned into plain data
 * the renderer draws with Vue text nodes, so no HTML from a reply or a file is
 * ever inserted as markup (an optional, separately sanitised subset aside).
 * File paths in text and links become references the page can open.
 */
import { opensHtmlBlock } from './html-subset';

export type MarkdownInline = { type: 'text' | 'strong' | 'em' | 'code'; text: string } | { type: 'link'; text: string; href: string } | { type: 'image'; alt: string; src: string } | { type: 'file'; text: string; path: string; line?: number; code: boolean };
export type MarkdownBlock = { type: 'paragraph' | 'heading' | 'quote'; text: string; level?: number } | { type: 'code'; text: string; language: string } | { type: 'list'; ordered: boolean; items: string[] } | { type: 'table'; header: string[]; align: Array<'left' | 'center' | 'right' | undefined>; rows: string[][] } | { type: 'rule' } | { type: 'html'; text: string };
/**
 * No HTML or executable URL schemes. The renderer uses Vue text nodes.
 *
 * Images are parsed but only ever loaded from the workspace, through this
 * server; whoever renders them decides that (see workspaceImagePath). A remote
 * image in text a model wrote would otherwise make the browser fetch any URL
 * it named.
 */
export function safeWebUrl(value: string): string | undefined {
  try { const url = new URL(value); return ['https:', 'http:'].includes(url.protocol) && !url.username && !url.password ? url.href : undefined; } catch { return undefined; }
}
export function markdownInline(text: string): MarkdownInline[] {
  const result: MarkdownInline[] = [], pattern = /(`[^`\n]+`|!\[[^\]\n]*\]\([^\s)]+\)|\*\*[^*\n]+\*\*|\*[^*\n]+\*|\[[^\]\n]+\]\([^\s)]+\))/g;
  let cursor = 0;
  for (const match of text.matchAll(pattern)) {
    const start = match.index ?? 0, token = match[0];
    if (start > cursor) result.push(...linkProse(text.slice(cursor, start)));
    if (token.startsWith('`')) { const code = token.slice(1, -1), reference = fileReference(code); result.push(reference ? { type: 'file', text: code, path: reference.path, line: reference.line, code: true } : { type: 'code', text: code }); }
    else if (token.startsWith('**')) result.push({ type: 'strong', text: token.slice(2, -2) });
    else if (token.startsWith('*')) result.push({ type: 'em', text: token.slice(1, -1) });
    else if (token.startsWith('![')) {
      // Only a path inside the workspace becomes an image. A remote one is
      // offered as a link to follow, never fetched on its own.
      const bracket = token.indexOf(']('), alt = token.slice(2, bracket), src = token.slice(bracket + 2, -1), href = safeWebUrl(src);
      result.push(isLocalImageReference(src) ? { type: 'image', alt, src } : href ? { type: 'link', text: alt || href, href } : { type: 'text', text: token });
    }
    else {
      const bracket = token.indexOf(']('), target = token.slice(bracket + 2, -1), href = safeWebUrl(target), reference = href ? undefined : fileReference(target);
      result.push(href ? { type: 'link', text: token.slice(1, bracket), href } : reference ? { type: 'file', text: token.slice(1, bracket), path: reference.path, line: reference.line, code: false } : { type: 'text', text: token });
    }
    cursor = start + token.length;
  }
  if (cursor < text.length) result.push(...linkProse(text.slice(cursor)));
  return result;
}
/** Cells of a `| a | b |` row. A `|` inside backticks or escaped as `\\|` is text. */
function tableCells(line: string): string[] {
  const cells: string[] = []; let cell = '', code = false;
  const body = line.trim().replace(/^\|/, '').replace(/(?<!\\)\|$/, '');
  for (let i = 0; i < body.length; i++) {
    const c = body[i];
    if (c === '\\' && body[i + 1] === '|') { cell += '|'; i++; continue; }
    if (c === '`') code = !code;
    if (c === '|' && !code) { cells.push(cell.trim()); cell = ''; continue; }
    cell += c;
  }
  cells.push(cell.trim());
  return cells;
}
/**
 * One dash a cell is enough, as GFM has it: `|-|-|` is a divider, and agents
 * write it. Without a pipe it takes two, so `a |` over a lone `-` stays a
 * heading's underline.
 */
const TABLE_DIVIDER = /^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$/;
const isTableDivider = (line: string) => TABLE_DIVIDER.test(line) && (line.includes('|') || /-{2,}/.test(line));
const RULE = /^ {0,3}([-*_])(?:[ \t]*\1){2,}[ \t]*$/;
/** `===` or `---` under a paragraph: the line above is a heading. */
const UNDERLINE = /^ {0,3}(=+|-+)[ \t]*$/;
/**
 * The same blocks, each with the 0-based source line it starts on. The file
 * pane uses it to keep its source and preview aligned; messages do not need it.
 *
 * `html` reads HTML blocks as such (see html-subset.ts): a file's README is
 * written in it. A message keeps every `<` as the text it is.
 */
export function markdownBlocksWithLines(text: string, options: { html?: boolean } = {}): Array<{ block: MarkdownBlock; line: number }> {
  const lines = text.replace(/\r\n/g, '\n').split('\n'), blocks: Array<{ block: MarkdownBlock; line: number }> = [];
  for (let i = 0; i < lines.length;) {
    const line = lines[i];
    if (!line.trim()) { i++; continue; }
    const start = i;
    const fence = line.match(/^\s*```([^`]*)$/);
    if (fence) { const body: string[] = []; i++; while (i < lines.length && !/^\s*```\s*$/.test(lines[i])) body.push(lines[i++]); if (i < lines.length) i++; blocks.push({ line: start, block: { type: 'code', language: fence[1].trim(), text: body.join('\n') } }); continue; }
    const heading = line.match(/^(#{1,6})\s+(.*?)(?:\s+#+)?\s*$/);
    if (heading) { blocks.push({ line: start, block: { type: 'heading', level: heading[1].length, text: heading[2] } }); i++; continue; }
    if (options.html && opensHtmlBlock(line)) {
      // A comment runs to its close; any other block to the next blank line.
      const comment = /^\s*<!--/.test(line), body: string[] = [];
      if (comment) { while (i < lines.length) { body.push(lines[i]); if (lines[i++].includes('-->')) break; } }
      else while (i < lines.length && lines[i].trim()) body.push(lines[i++]);
      blocks.push({ line: start, block: { type: 'html', text: body.join('\n') } }); continue;
    }
    if (RULE.test(line)) { blocks.push({ line: start, block: { type: 'rule' } }); i++; continue; }
    // Consecutive quoted lines are one quote, as they read.
    if (/^\s*>/.test(line)) {
      const body: string[] = [];
      while (i < lines.length && /^\s*>/.test(lines[i])) body.push(lines[i++].replace(/^\s*>\s?/, ''));
      blocks.push({ line: start, block: { type: 'quote', text: body.join('\n') } }); continue;
    }
    // A GFM table: a row, a divider of dashes, then rows until one without a pipe.
    if (line.includes('|') && i + 1 < lines.length && isTableDivider(lines[i + 1])) {
      const header = tableCells(line), divider = tableCells(lines[i + 1]);
      if (divider.length === header.length) {
        const align = divider.map(cell => cell.startsWith(':') && cell.endsWith(':') ? 'center' as const : cell.endsWith(':') ? 'right' as const : cell.startsWith(':') ? 'left' as const : undefined);
        const rows: string[][] = []; i += 2;
        while (i < lines.length && lines[i].trim() && lines[i].includes('|')) { const cells = tableCells(lines[i++]); rows.push(header.map((_, index) => cells[index] ?? '')); }
        blocks.push({ line: start, block: { type: 'table', header, align, rows } }); continue;
      }
    }
    const list = line.match(/^\s*(?:([-*+])|(\d+)\.)\s+(.*)$/);
    if (list) {
      const ordered = !!list[2], items: string[] = [];
      while (i < lines.length) { const item = lines[i].match(/^\s*(?:([-*+])|(\d+)\.)\s+(.*)$/); if (!item || !!item[2] !== ordered) break; items.push(item[3]); i++; }
      blocks.push({ line: start, block: { type: 'list', ordered, items } }); continue;
    }
    const body = [line]; i++;
    let level: number | undefined;
    while (i < lines.length && lines[i].trim()) {
      const next = lines[i], underline = next.match(UNDERLINE);
      if (underline) { level = underline[1][0] === '=' ? 1 : 2; i++; break; }
      if (/^(?:\s*```|#{1,6}\s|\s*>|\s*(?:[-*+]|\d+\.)\s)/.test(next) || RULE.test(next) || (next.includes('|') && isTableDivider(lines[i + 1] ?? '')) || (options.html && opensHtmlBlock(next))) break;
      body.push(lines[i++]);
    }
    blocks.push({ line: start, block: level ? { type: 'heading', level, text: body.join(' ') } : { type: 'paragraph', text: body.join('\n') } });
  }
  return blocks;
}
export function markdownBlocks(text: string, options: { html?: boolean } = {}): MarkdownBlock[] {
  return markdownBlocksWithLines(text, options).map(entry => entry.block);
}

const IMAGE_EXTENSION = /\.(png|apng|jpe?g|jpe|jfif|gif|webp|svg|avif|bmp|ico|tiff?|tga|qoi)$/i;
/**
 * A workspace-relative path for an image reference, or undefined when it is
 * not one: a URL with a scheme, an absolute path, or one that climbs out of
 * the workspace. `base` is the directory of the file it appears in, so
 * `![](img/a.png)` in docs/readme.md means docs/img/a.png, as on GitHub.
 */
export function workspaceImagePath(src: string, base = ''): string | undefined {
  let value = src.trim();
  try { value = decodeURIComponent(value); } catch { return undefined; }
  value = value.split(/[?#]/)[0];
  if (!value || /^[a-z][a-z0-9+.-]*:/i.test(value) || value.startsWith('//') || !IMAGE_EXTENSION.test(value)) return undefined;
  const parts: string[] = [];
  for (const part of (value.startsWith('/') ? value.slice(1) : `${base ? base + '/' : ''}${value}`).split('/')) {
    if (!part || part === '.') continue;
    if (part === '..') { if (!parts.length) return undefined; parts.pop(); continue; }
    parts.push(part);
  }
  return parts.length ? parts.join('/') : undefined;
}
/** A reference to an image on disk rather than on the web. Where it points is
 * settled when rendering, against the file it appears in. */
export function isLocalImageReference(src: string): boolean {
  const value = src.trim().split(/[?#]/)[0];
  return !!value && !/^[a-z][a-z0-9+.-]*:/i.test(value) && !value.startsWith('//') && IMAGE_EXTENSION.test(value);
}
/** Image files a message says it attached, from composeMessage's header. */
export function attachedImagePaths(text: string): string[] {
  const match = text.match(/^Attached files? in this workspace:\n((?:- .+\n?)+)/);
  if (!match) return [];
  return match[1].split('\n').map(line => line.replace(/^- /, '').trim()).filter(path => IMAGE_EXTENSION.test(path) && workspaceImagePath(path) === path);
}

/** A place in a file a message points at. */
export interface FileReference { path: string; line?: number }
const FILE_PATTERN = /^(?:file:\/\/)?((?:\/?[A-Za-z]:[\\/]|\/|\.{1,2}[\\/])?(?:[\w.@+-]+[\\/])*[\w@+-][\w.@+-]*\.[A-Za-z][A-Za-z0-9]{0,9})(?:(?::|#L)(\d{1,7})(?::\d{1,5}|-L?\d{1,7})?)?$/;
/**
 * A file path, as agents write them: `src/main.rs`, `main.rs:42`,
 * `src/main.rs:42:7`, `main.rs#L42`, an absolute path, or a `file://` URL.
 * A bare name counts only with an extension that starts with a letter, so a
 * version (`1.2.3`) or a number is not taken for a file. Agents on a Windows
 * host write `C:\repo\a.ts` or `src\a.ts`; those come back with `/` separators,
 * which the server accepts there too, so one form flows through the UI.
 */
export function fileReference(value: string): FileReference | undefined {
  const match = value.trim().match(FILE_PATTERN);
  if (!match) return undefined;
  let path = match[1].replace(/\\/g, '/').replace(/^\/(?=[A-Za-z]:\/)/, '');
  try { path = decodeURIComponent(path); } catch { return undefined; }
  if (path.includes('//') || /^\d/.test(path.split('/').pop() ?? '')) return undefined;
  return match[2] ? { path, line: Number(match[2]) } : { path };
}
/** Plain prose is scanned only for paths with a directory in them, and URLs;
 * a bare "main.rs" in a sentence stays a word. */
const PROSE_REFERENCES = /(https?:\/\/[^\s<>()"'`]+[^\s<>()"'`.,;:!?。，；：！？)])|((?:\/|\.{1,2}\/)?(?:[\w.@+-]+\/)+[\w@+-][\w.@+-]*\.[A-Za-z][A-Za-z0-9]{0,9}(?::\d{1,7}(?::\d{1,5})?|#L\d{1,7})?)/g;
export function linkProse(text: string): MarkdownInline[] {
  const parts: MarkdownInline[] = [];
  let cursor = 0;
  for (const match of text.matchAll(PROSE_REFERENCES)) {
    const start = match.index ?? 0, token = match[0];
    const reference = match[2] ? fileReference(token) : undefined;
    const href = match[1] ? safeWebUrl(token) : undefined;
    if (!reference && !href) continue;
    if (start > cursor) parts.push({ type: 'text', text: text.slice(cursor, start) });
    parts.push(reference ? { type: 'file', text: token, path: reference.path, line: reference.line, code: false } : { type: 'link', text: token, href: href! });
    cursor = start + token.length;
  }
  if (cursor < text.length) parts.push({ type: 'text', text: text.slice(cursor) });
  return parts.length ? parts : [{ type: 'text', text }];
}
