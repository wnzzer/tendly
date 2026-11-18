import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { TAB_ICON_PALETTE } from "./tab-icons.ts";
import { AGENT_CLIENTS, clientInfo, clientTone } from "./clients.ts";
import { compactCss } from "../source-text.mjs";

const css = file => {
  const source = readFileSync(new URL(file, import.meta.url), "utf8");
  const style = source.match(/<style scoped>([\s\S]*?)<\/style>/)?.[1];
  assert.ok(style, `${file} has scoped styles`);
  return compactCss(style);
};

test("the existing workspace palette remains the source for new conversation and account surfaces", () => {
  const shell = readFileSync(new URL("../styles.css", import.meta.url), "utf8");
  for (const token of [
    "--accent: #0f7f70",
    "--accent-soft: #e3f1ec",
    "--violet: #7760b5",
    "--border: #e6e2da",
    "--muted: #817b72",
    "--surface: #fcfbf9",
    "--elevated: #ffffff",
    "--teal: var(--accent)",
    "--teal-soft: var(--accent-soft)",
  ])
    assert.ok(shell.includes(token), token);
  for (const file of ["./ChatSessionPane.vue", "./AccountsDialog.vue", "./ChatPreview.vue"]) {
    const style = css(file);
    for (const token of ["var(--teal)", "var(--teal-soft)", "var(--border)", "var(--muted)", "var(--surface)"])
      assert.ok(style.includes(token), `${file} reuses ${token}`);
    assert.doesNotMatch(
      style,
      /#(?:fcfbf7|fffefb|f2f3eb|728b5f|66864f|748762)\b/i,
      `${file} does not introduce a separate cream/olive theme`,
    );
  }
});

test("provider orange and purple stay consistent with the existing colored tabs", () => {
  // The provider colours are tokens, so a theme can retint them; the light
  // values must stay the ones the coloured tabs use.
  const shell = readFileSync(new URL("../styles.css", import.meta.url), "utf8").toLowerCase();
  for (const [provider, token] of AGENT_CLIENTS.map(id => [id, "--" + clientInfo(id).colorToken])) {
    assert.ok(
      shell.includes(`${token}: ${TAB_ICON_PALETTE[provider].color.toLowerCase()};`),
      `${token} matches the ${provider} tab`,
    );
    assert.ok(
      shell.includes(`${token}-soft: ${TAB_ICON_PALETTE[provider].background.toLowerCase()};`),
      `${token}-soft matches the ${provider} tab`,
    );
  }
  // Each surface tints a mark through the client's own token (clients.ts), set inline.
  assert.deepEqual(clientTone("claude_code"), {
    "--client-ink": "var(--claude)",
    "--client-soft": "var(--claude-soft)",
  });
  assert.deepEqual(clientTone("codex"), { "--client-ink": "var(--codex)", "--client-soft": "var(--codex-soft)" });
  for (const file of [
    "./ChatSessionPane.vue",
    "./AccountsDialog.vue",
    "./ChatPreview.vue",
    "./PreferencesPanel.vue",
    "./AgentClientsPanel.vue",
  ]) {
    const style = css(file);
    for (const token of ["var(--client-ink)", "var(--client-soft)"])
      assert.ok(style.includes(token), `${file} uses ${token}`);
  }
});

test("chat retains mobile sizing and semantic warning/error colors within the original light palette", () => {
  const style = css("./ChatSessionPane.vue");
  assert.match(style, /\.chat-pane\{[^}]*background:var\(--surface\)/);
  // The send button is the one accent-filled control in the composer; the
  // composer itself is a glass surface over the pane (styles.css --glass-surface).
  assert.match(style, /\.chat-send\{[^}]*background:var\(--accent\)/);
  assert.match(style, /\.chat-composer\{[^}]*background:var\(--glass-surface\)/);
  assert.ok(style.includes("min-height:44px"));
  assert.ok(style.includes("font-size:var(--input-text)"));
  // 16px is the floor that stops iOS zooming a focused field, whatever the text scale.
  assert.ok(readFileSync(new URL("../styles.css", import.meta.url), "utf8").includes("--input-text: max(16px,"));
  assert.ok(style.includes("safe-area-inset-bottom"));
  assert.ok(style.includes("#fffaee"));
  // The semantic error colour is unchanged; it is now named rather than repeated.
  assert.ok(style.includes("var(--danger-ink)"));
});

// An @import after any other rule is dropped by the build without an error,
// and with it every style the imported sheet carried.
test("the shell stylesheet keeps its imports before every other rule", () => {
  const shell = readFileSync(new URL("../styles.css", import.meta.url), "utf8")
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .trim();
  const firstImport = shell.indexOf("@import"),
    firstRule = shell.search(/[^;]\s*\{/);
  assert.ok(
    firstImport === 0 && (firstRule === -1 || firstRule > shell.lastIndexOf("@import")),
    "every @import comes first",
  );
});
