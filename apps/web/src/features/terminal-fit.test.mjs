import { test } from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { styleOf } from "../source-text.mjs";

// Compacted, so `.a { b: c }` reads `.a{b:c}` however the file is laid out.
const css = styleOf(new URL("./workflows.css", import.meta.url));
const rule = name => new RegExp(`\\${name}\\s*\\{[^}]*\\}`).exec(css)?.[0] ?? "";

test("the terminal is spaced with margins, because padding lies to the fit addon", () => {
  // The addon sizes the terminal from this element's computed `height`, which
  // under border-box includes its own padding. Padding therefore reads as room
  // for rows that do not fit, and the rows drawn there — the prompt, and the
  // command list a client puts under it — land below the visible area.
  const host = rule(".terminal-host");
  assert.ok(host, "the rule must exist to be constrained");
  assert.doesNotMatch(host, /padding(-top|-bottom)?:\s*(?!0)[^;]*\b\d+px\s+\d+px/, "no vertical padding");
  assert.match(host, /padding:\s*0 /, "horizontal padding only");
  assert.match(host, /margin:/, "vertical space is margin");
  // A finger's keys float over the bottom corner, so the rows stop above them.
  const coarse = /@media\(any-pointer:coarse\)\{\.terminal-host\{[^}]*\}/.exec(css)?.[0] ?? "";
  assert.match(coarse, /margin-bottom:\s*48px/);
  assert.doesNotMatch(coarse, /padding/);
});
