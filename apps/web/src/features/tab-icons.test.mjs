import { test } from "node:test";
import assert from "node:assert/strict";
import { TAB_ICON_PALETTE, resolveTabIcon, tabIconProvider } from "./tab-icons.ts";
import { ICON_PALETTE, iconColor, providerColor } from "./icon-palette.ts";

test("actual session provider wins over stale tab metadata, including legacy metadata without provider", () => {
  assert.equal(tabIconProvider({ session_id: "one", provider: "codex" }, { one: "claude_code" }), "claude_code");
  assert.equal(resolveTabIcon("agent_chat", { session_id: "two" }, { two: "codex" }).type, "codex");
  assert.equal(resolveTabIcon("terminal", { session_id: "one" }, { one: "claude_code" }).provider, "claude_code");
});

test("legal metadata remains a fallback while unknown providers never masquerade as a particular client", () => {
  assert.equal(tabIconProvider({ provider: "claude_code" }), "claude_code");
  assert.equal(tabIconProvider({ session_id: "missing", provider: "codex" }, {}), "codex");
  assert.equal(resolveTabIcon("agent_chat", { provider: "unknown", title: "Claude Code" }).type, "agent");
  assert.equal(resolveTabIcon("agent_chat").provider, undefined);
  assert.equal(resolveTabIcon("terminal").type, "terminal");
  assert.equal(tabIconProvider({ session_id: "toString" }, {}), undefined);
});

test("Git and files retain their own icon identity even if stale provider metadata is present", () => {
  assert.equal(resolveTabIcon("git_diff", { provider: "codex" }).type, "git");
  assert.equal(resolveTabIcon("editor", { provider: "claude_code", path: "src/App.vue" }).type, "text");
  assert.equal(resolveTabIcon("file_preview", { path: "notes.md" }).icon, "file");
});

test("file extensions distinguish image, video, audio, PDF and text without depending on translated titles", () => {
  for (const [path, type] of [
    ["media/截图 #1.PNG", "image"],
    ["clips/demo.WEBM", "video"],
    ["audio/sample.flac", "audio"],
    ["reports/中文.PDF", "pdf"],
    ["src/test.rs", "text"],
  ])
    assert.equal(resolveTabIcon("file_preview", { path }).type, type);
  assert.equal(resolveTabIcon("file_preview").type, "image");
  assert.equal(resolveTabIcon("editor").type, "text");
});

test("every icon has an explicit semantic foreground and a light background with readable contrast", () => {
  function luminance(hex) {
    const channels = hex
      .slice(1)
      .match(/../g)
      .map(value => parseInt(value, 16) / 255)
      .map(value => (value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4));
    return channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722;
  }
  for (const [type, palette] of Object.entries(TAB_ICON_PALETTE)) {
    assert.match(palette.color, /^#[0-9A-F]{6}$/);
    assert.ok(luminance(palette.background) > 0.75, `${type} stays light`);
    assert.ok((luminance(palette.background) + 0.05) / (luminance(palette.color) + 0.05) >= 3, `${type} icon contrast`);
  }
});

test("key icons share a semantic palette across tabs, navigation and provider badges", () => {
  // Functional icons take the colour of the text beside them; only stop keeps a hue.
  for (const name of [
    "folder",
    "archive",
    "git",
    "file",
    "terminal",
    "account",
    "settings",
    "grid",
    "plus",
    "locate",
    "more",
    "toString",
  ])
    assert.equal(iconColor(name), "currentColor", name);
  assert.equal(iconColor("stop"), ICON_PALETTE.rose);
  for (const provider of ["claude_code", "codex", "terminal"])
    assert.equal(providerColor(provider), TAB_ICON_PALETTE[provider].color);
});

test("resolved appearance is a separate value, not a mutable palette entry", () => {
  const icon = resolveTabIcon("git_diff");
  icon.color = "#000000";
  assert.equal(resolveTabIcon("git_diff").color, TAB_ICON_PALETTE.git.color);
});
