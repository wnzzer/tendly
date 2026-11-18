/**
 * Syntax highlighting for the file pane.
 *
 * Loaded on demand, like the terminal, so opening a workspace never pays for a
 * highlighter that only a text file needs. Only the languages registered below
 * are bundled; an unknown extension renders as plain text rather than guessing,
 * because a wrong guess colours a file misleadingly rather than not at all.
 */
export type Highlighter = (source: string, language: string) => string;

/** Extension → highlight.js language id. */
const LANGUAGES: Record<string, string> = {
  ts: "typescript",
  tsx: "typescript",
  mts: "typescript",
  cts: "typescript",
  js: "javascript",
  jsx: "javascript",
  mjs: "javascript",
  cjs: "javascript",
  rs: "rust",
  py: "python",
  go: "go",
  rb: "ruby",
  php: "php",
  java: "java",
  kt: "kotlin",
  swift: "swift",
  c: "c",
  h: "c",
  cpp: "cpp",
  cc: "cpp",
  hpp: "cpp",
  cs: "csharp",
  sh: "bash",
  bash: "bash",
  zsh: "bash",
  fish: "bash",
  json: "json",
  jsonc: "json",
  yml: "yaml",
  yaml: "yaml",
  toml: "ini",
  ini: "ini",
  css: "css",
  scss: "scss",
  less: "less",
  html: "xml",
  xml: "xml",
  svg: "xml",
  vue: "xml",
  sql: "sql",
  md: "markdown",
  markdown: "markdown",
  dockerfile: "dockerfile",
  makefile: "makefile",
  diff: "diff",
  patch: "diff",
};

/** Files with no extension whose name names the language. */
const BY_NAME: Record<string, string> = {
  dockerfile: "dockerfile",
  makefile: "makefile",
  gemfile: "ruby",
  rakefile: "ruby",
};

export function languageFor(path: string): string | undefined {
  const name = (path.split("/").pop() ?? "").toLowerCase();
  const extension = name.includes(".") ? name.slice(name.lastIndexOf(".") + 1) : "";
  return LANGUAGES[extension] ?? BY_NAME[name];
}

/** Names a code fence uses that are neither an extension nor a grammar id. */
const FENCE_NAMES: Record<string, string> = {
  shell: "bash",
  console: "bash",
  shellsession: "bash",
  html: "xml",
  svelte: "xml",
  golang: "go",
  py: "python",
  rs: "rust",
  "c++": "cpp",
  "c#": "csharp",
  yml: "yaml",
  docker: "dockerfile",
  make: "makefile",
};

/** The grammar for a code fence's ```lang, or undefined to leave it plain. */
export function fenceLanguage(name: string): string | undefined {
  const value = name
    .trim()
    .split(/[\s{,]/)[0]
    .toLowerCase();
  if (!value) return undefined;
  const language = FENCE_NAMES[value] ?? (SUPPORTED.has(value) ? value : LANGUAGES[value]);
  return language && SUPPORTED.has(language) ? language : undefined;
}

/**
 * Whether this kind of file is prose, and so should wrap.
 *
 * A column that has to line up is why a code editor scrolls sideways rather
 * than wrapping, and prose has no such column. A paragraph of Chinese is one
 * long line by nature: left unwrapped it runs off the right edge and is written
 * through a horizontal scrollbar.
 */
export function wrapsProse(language: string | undefined): boolean {
  return !language || language === "markdown" || language === "plaintext";
}

/**
 * Highlighting a very large file costs more than it gives: the pass is
 * synchronous and blocks typing. Past this the pane stays plain and says so,
 * which is better than an editor that stutters on every keystroke.
 */
export const MAX_HIGHLIGHT_BYTES = 512 * 1024;

/**
 * Each grammar is its own chunk, fetched only for the file being opened.
 *
 * Loading them together cost 55 KB for any text file at all; a TypeScript file
 * now pulls its own 3 KB. The map is explicit because a computed import path
 * cannot be code-split — the bundler has to see every specifier.
 */
const GRAMMARS: Record<string, () => Promise<{ default: unknown }>> = {
  typescript: () => import("highlight.js/lib/languages/typescript"),
  javascript: () => import("highlight.js/lib/languages/javascript"),
  rust: () => import("highlight.js/lib/languages/rust"),
  python: () => import("highlight.js/lib/languages/python"),
  go: () => import("highlight.js/lib/languages/go"),
  ruby: () => import("highlight.js/lib/languages/ruby"),
  php: () => import("highlight.js/lib/languages/php"),
  java: () => import("highlight.js/lib/languages/java"),
  kotlin: () => import("highlight.js/lib/languages/kotlin"),
  swift: () => import("highlight.js/lib/languages/swift"),
  c: () => import("highlight.js/lib/languages/c"),
  cpp: () => import("highlight.js/lib/languages/cpp"),
  csharp: () => import("highlight.js/lib/languages/csharp"),
  bash: () => import("highlight.js/lib/languages/bash"),
  json: () => import("highlight.js/lib/languages/json"),
  yaml: () => import("highlight.js/lib/languages/yaml"),
  ini: () => import("highlight.js/lib/languages/ini"),
  css: () => import("highlight.js/lib/languages/css"),
  scss: () => import("highlight.js/lib/languages/scss"),
  less: () => import("highlight.js/lib/languages/less"),
  xml: () => import("highlight.js/lib/languages/xml"),
  sql: () => import("highlight.js/lib/languages/sql"),
  markdown: () => import("highlight.js/lib/languages/markdown"),
  dockerfile: () => import("highlight.js/lib/languages/dockerfile"),
  makefile: () => import("highlight.js/lib/languages/makefile"),
  diff: () => import("highlight.js/lib/languages/diff"),
};

/** Every language this build can colour; anything else stays plain text. */
export const SUPPORTED = new Set(Object.keys(GRAMMARS));

let core: Promise<typeof import("highlight.js/lib/core").default> | undefined;
const registered = new Map<string, Promise<void>>();

export async function loadHighlighter(language: string): Promise<Highlighter | undefined> {
  const grammar = GRAMMARS[language];
  if (!grammar) return undefined;
  core ??= import("highlight.js/lib/core").then(module => module.default);
  const hljs = await core;
  // Registering twice is harmless, but sharing the promise means a pane that
  // reopens the same file does not re-fetch the grammar.
  registered.set(
    language,
    registered.get(language) ??
      grammar().then(module => {
        hljs.registerLanguage(language, module.default as never);
      }),
  );
  await registered.get(language);
  return (source, lang) => {
    // `ignoreIllegals` keeps a file the grammar dislikes rendering as text
    // rather than throwing: half-written files are normal in an editor.
    try {
      return hljs.highlight(source, { language: lang, ignoreIllegals: true }).value;
    } catch {
      return escapeHtml(source);
    }
  };
}

export function escapeHtml(value: string): string {
  return value.replace(/[&<>]/g, character => (character === "&" ? "&amp;" : character === "<" ? "&lt;" : "&gt;"));
}
