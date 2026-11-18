import { readFileSync } from "node:fs";

/**
 * Some tests pin a contract by reading a component's own source: a CSS rule
 * that must exist, an attribute a template must carry. They read it through
 * here, normalised, so that the formatter can lay the file out however it
 * likes without the assertions caring where a line breaks.
 */

/** A file's text, with Windows line endings folded. */
export const sourceText = url => readFileSync(url, "utf8").replace(/\r\n/g, "\n");

/**
 * CSS with every optional space gone: `.a > b { color: red; }` reads
 * `.a>b{color:red}`. A space between selectors stays, as it means something.
 * Each rule ends a line, so a pattern can still stop at the end of a block.
 */
export function compactCss(css) {
  return css
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/\s+/g, " ")
    .replace(/\s*([{};,>])\s*/g, "$1")
    .replace(/:\s+/g, ":")
    .replace(/\(\s+/g, "(")
    .replace(/\s+\)/g, ")")
    .replace(/(@media|@container|@supports) ?([^({]*?) ?\(/g, (_, at, name) => `${at}${name ? ` ${name}` : ""}(`)
    .replace(/;}/g, "}")
    .replace(/}/g, "}\n")
    .trim();
}

/** Every `<style>` block of a single-file component, compacted; a plain stylesheet whole. */
export function styleOf(url) {
  const text = sourceText(url);
  if (!/\.vue$/.test(String(url))) return compactCss(text);
  return [...text.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)].map(match => compactCss(match[1])).join("\n");
}

/**
 * Template or script text on one line: runs of whitespace become one space,
 * and none is left inside a tag's closing `>` or after an opening bracket.
 */
export function compactMarkup(text) {
  return text
    .replace(/\s+/g, " ")
    .replace(/\s+(\/?>)/g, "$1")
    .replace(/([({[]) /g, "$1")
    .replace(/ ([)}\]])/g, "$1");
}

/** A file's text through `compactMarkup`. */
export const markupOf = url => compactMarkup(sourceText(url));
