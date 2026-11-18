import { test } from "node:test";
import assert from "node:assert/strict";
import { languageFor, escapeHtml, MAX_HIGHLIGHT_BYTES, wrapsProse } from "./highlighting.ts";

test("a language is chosen from the file name, never guessed", () => {
  assert.equal(languageFor("apps/web/src/features/chat-model.ts"), "typescript");
  assert.equal(languageFor("crates/server/src/main.rs"), "rust");
  assert.equal(languageFor("Cargo.toml"), "ini");
  assert.equal(languageFor("packages/native-bridge/chat.mjs"), "javascript");
  assert.equal(languageFor("Dockerfile"), "dockerfile");
  assert.equal(languageFor("README.md"), "markdown");
  // Unknown stays unknown: colouring a file by the wrong grammar is worse than
  // leaving it plain, because it reads as meaning.
  assert.equal(languageFor("notes.whatever"), undefined);
  assert.equal(languageFor("LICENSE"), undefined);
});

test("a dotfile is not mistaken for an extension", () => {
  // ".gitignore" has no extension; the part after the dot is the whole name.
  assert.equal(languageFor(".gitignore"), undefined);
  assert.equal(languageFor(".eslintrc.json"), "json");
});

test("escaping closes the markup the highlighter writes into", () => {
  assert.equal(escapeHtml("<script>a && b</script>"), "&lt;script&gt;a &amp;&amp; b&lt;/script&gt;");
  // Quotes are left alone deliberately: this only ever lands in text content,
  // never in an attribute, and escaping them would show entities to the reader.
  assert.equal(escapeHtml('say "hi"'), 'say "hi"');
});

test("the size ceiling is low enough to keep typing responsive", () => {
  assert.ok(MAX_HIGHLIGHT_BYTES <= 1024 * 1024);
  assert.ok(MAX_HIGHLIGHT_BYTES >= 64 * 1024, "but not so low that ordinary source files miss out");
});

test("markdown element styles are shared, not scoped to the conversation", async () => {
  const { readFile } = await import("node:fs/promises");
  const shared = await readFile(new URL("./workflows.css", import.meta.url), "utf8");
  const pane = await readFile(new URL("./ChatSessionPane.vue", import.meta.url), "utf8");
  // Extracting the renderer without its styles left the file pane rendering the
  // same markup unstyled: code blocks with no frame, links with no colour.
  for (const selector of [
    ".chat-markdown a",
    ".chat-markdown code",
    ".chat-markdown .chat-code",
    ".chat-markdown pre",
  ]) {
    assert.ok(shared.includes(selector), `${selector} must live in the shared sheet`);
  }
  assert.doesNotMatch(
    pane,
    /\.chat-markdown :deep\(/,
    "a scoped copy would apply to one pane and silently not the other",
  );
});

test("a file pane and a message render markdown through one component", async () => {
  const { readFile } = await import("node:fs/promises");
  for (const file of ["FilePane.vue", "ChatSessionPane.vue"]) {
    const source = await readFile(new URL(`./${file}`, import.meta.url), "utf8");
    if (!source.includes("MarkdownContent")) continue;
    assert.match(
      source,
      /import \{ MarkdownContent \} from ["']\.\/MarkdownContent["']/,
      `${file} uses the component but does not import it — Vue resolves it to nothing at runtime`,
    );
  }
});

test("prose wraps and code does not, because only one of them has a column to keep", () => {
  // Markdown, plain text and anything unrecognised are read and written as
  // prose: a Chinese paragraph is one long line, and unwrapped it is written
  // through a horizontal scrollbar.
  assert.equal(wrapsProse(languageFor("chapters/first.md")), true);
  assert.equal(wrapsProse(languageFor("notes.txt")), true);
  assert.equal(wrapsProse(languageFor("LICENSE")), true);
  // Code keeps its columns, so it scrolls sideways instead.
  assert.equal(wrapsProse(languageFor("src/main.rs")), false);
  assert.equal(wrapsProse(languageFor("app.ts")), false);
  assert.equal(wrapsProse(languageFor("Dockerfile")), false);
});

test("a code fence names its language the ways people write it", async () => {
  const { fenceLanguage } = await import("./highlighting.ts");
  assert.equal(fenceLanguage("ts"), "typescript");
  assert.equal(fenceLanguage("Rust"), "rust");
  assert.equal(fenceLanguage("shell"), "bash");
  assert.equal(fenceLanguage("html"), "xml");
  assert.equal(fenceLanguage("js {1,3}"), "javascript");
  assert.equal(fenceLanguage("text"), undefined);
  assert.equal(fenceLanguage(""), undefined);
});
