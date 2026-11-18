/**
 * The HTML a README is written in, read as data.
 *
 * A repository's docs open with `<p align="center"><img …></p>`: shown as
 * source, the first screen of a README was a wall of tags. This reads such a
 * block into a small tree of the elements and attributes allowed below and
 * nothing else; the renderer builds Vue nodes from the tree, so no markup the
 * file wrote ever reaches innerHTML. An element outside the list keeps its
 * text; one whose content is not text (a script, a style) is dropped whole.
 * Which images may load stays the renderer's decision, as it is for markdown.
 */
export type HtmlNode = string | { tag: string; attrs: Record<string, string>; children: HtmlNode[] };

const ALLOWED = new Set([
  "p",
  "div",
  "span",
  "center",
  "a",
  "img",
  "picture",
  "source",
  "br",
  "hr",
  "h1",
  "h2",
  "h3",
  "h4",
  "h5",
  "h6",
  "details",
  "summary",
  "kbd",
  "sub",
  "sup",
  "b",
  "strong",
  "i",
  "em",
  "code",
  "pre",
  "ul",
  "ol",
  "li",
  "blockquote",
  "small",
  "del",
  "s",
  "ins",
  "mark",
  "table",
  "thead",
  "tbody",
  "tr",
  "th",
  "td",
]);
/** Dropped with everything inside them: their content is not prose. */
const OPAQUE = new Set([
  "script",
  "style",
  "iframe",
  "object",
  "embed",
  "template",
  "noscript",
  "textarea",
  "select",
  "title",
  "head",
  "svg",
  "math",
  "video",
  "audio",
  "canvas",
  "form",
  "frameset",
  "frame",
]);
const VOID = new Set([
  "img",
  "source",
  "br",
  "hr",
  "input",
  "meta",
  "link",
  "wbr",
  "col",
  "area",
  "base",
  "param",
  "track",
  "embed",
]);
/** Per element, the attributes kept; anything else, every on* handler and style included, is not. */
const ATTRIBUTES: Record<string, string[]> = {
  a: ["href", "title"],
  img: ["src", "alt", "title", "width", "height"],
  source: ["srcset", "media"],
  p: ["align"],
  div: ["align"],
  h1: ["align"],
  h2: ["align"],
  h3: ["align"],
  h4: ["align"],
  h5: ["align"],
  h6: ["align"],
  td: ["align", "valign", "width", "colspan", "rowspan"],
  th: ["align", "valign", "width", "colspan", "rowspan"],
  details: ["open"],
};

const ENTITIES: Record<string, string> = {
  amp: "&",
  lt: "<",
  gt: ">",
  quot: '"',
  apos: "'",
  nbsp: " ",
  copy: "©",
  reg: "®",
  mdash: "—",
  ndash: "–",
  hellip: "…",
  middot: "·",
  rarr: "→",
  larr: "←",
};
export function decodeEntities(text: string): string {
  return text.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (whole, name: string) => {
    if (name[0] === "#") {
      const code = name[1] === "x" || name[1] === "X" ? parseInt(name.slice(2), 16) : parseInt(name.slice(1), 10);
      return code > 0 && code <= 0x10ffff && !(code >= 0xd800 && code <= 0xdfff) ? String.fromCodePoint(code) : whole;
    }
    return ENTITIES[name.toLowerCase()] ?? whole;
  });
}

function attributes(source: string): Record<string, string> {
  const found: Record<string, string> = {};
  for (const match of source.matchAll(/([^\s"'>\/=]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+)))?/g)) {
    const name = match[1].toLowerCase();
    if (!(name in found)) found[name] = decodeEntities(match[2] ?? match[3] ?? match[4] ?? "");
  }
  return found;
}

/** Elements that may appear inside a line of prose: `press <kbd>Ctrl</kbd>`. */
const INLINE = new Set([
  "a",
  "img",
  "br",
  "kbd",
  "sub",
  "sup",
  "b",
  "strong",
  "i",
  "em",
  "del",
  "s",
  "ins",
  "mark",
  "small",
  "span",
  "code",
]);
const INLINE_TAG = /<\/?(?:a|img|br|kbd|sub|sup|b|strong|i|em|del|s|ins|mark|small|span|code)(?=[\s>\/])/i;
/** Whether a run of prose has any inline element worth reading as one. */
export function hasInlineHtml(text: string): boolean {
  return INLINE_TAG.test(text);
}

/**
 * Reads `html` into allowed elements and text. Unclosed elements close at the end.
 *
 * `inline` reads a line of prose instead: only inline elements are elements,
 * and any other `<…>` -- `Vec<T>`, `<https://…>` -- stays the text it was.
 */
export function parseHtml(html: string, inline = false): HtmlNode[] {
  const root: HtmlNode[] = [];
  const stack: Array<{ tag: string; children: HtmlNode[] }> = [{ tag: "", children: root }];
  const push = (node: HtmlNode) => {
    const children = stack[stack.length - 1].children,
      last = children[children.length - 1];
    if (typeof node === "string" && typeof last === "string") children[children.length - 1] = last + node;
    else children.push(node);
  };
  const tags = /<!--[\s\S]*?(?:-->|$)|<(\/?)([A-Za-z][\w-]*)((?:[^>"']|"[^"]*"|'[^']*')*)>/g;
  let cursor = 0,
    skipping: string | undefined;
  for (const match of html.matchAll(tags)) {
    const start = match.index ?? 0;
    if (!skipping && start > cursor)
      push(inline ? html.slice(cursor, start) : decodeEntities(html.slice(cursor, start)));
    cursor = start + match[0].length;
    if (inline && !(match[2] && INLINE.has(match[2].toLowerCase()))) {
      push(match[0]);
      continue;
    }
    if (!match[2]) continue; // a comment
    const closing = match[1] === "/",
      tag = match[2].toLowerCase();
    if (skipping) {
      if (closing && tag === skipping) skipping = undefined;
      continue;
    }
    if (OPAQUE.has(tag)) {
      if (!closing && !VOID.has(tag) && !/\/\s*$/.test(match[3])) skipping = tag;
      continue;
    }
    if (closing) {
      const depth = stack.map(entry => entry.tag).lastIndexOf(tag);
      if (depth > 0) stack.length = depth;
      continue;
    }
    if (!ALLOWED.has(tag)) continue;
    const kept = ATTRIBUTES[tag] ?? [],
      found = attributes(match[3]),
      attrs: Record<string, string> = {};
    for (const name of kept) if (name in found) attrs[name] = found[name];
    const node = { tag, attrs, children: [] as HtmlNode[] };
    push(node);
    if (!VOID.has(tag) && !/\/\s*$/.test(match[3])) stack.push(node);
  }
  if (!skipping && cursor < html.length) push(inline ? html.slice(cursor) : decodeEntities(html.slice(cursor)));
  return root;
}

/** Whether a line opens an HTML block (CommonMark's kinds 2, 6 and 7, near enough). */
export function opensHtmlBlock(line: string): boolean {
  const match = line.match(/^ {0,3}<(?:(!--)|\/?([A-Za-z][\w-]*)(?=[\s>\/]|$))/);
  if (!match) return false;
  return !!match[1] || ALLOWED.has(match[2].toLowerCase()) || OPAQUE.has(match[2].toLowerCase());
}
