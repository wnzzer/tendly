import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

// Colours, type sizes and radii come from the tokens in styles.css. These
// counts are a ratchet: they may go down as the last literals move over, and
// a new hard-coded value has to be a deliberate change here, not a drift.
const LIMITS = { hex: 89, fontPx: 3, radiusPx: 0 };

// fileURLToPath, not .pathname: on Windows the latter keeps a leading slash before the drive.
const src = fileURLToPath(new URL("..", import.meta.url));
const files = [];
(function walk(dir) {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) walk(path);
    else if (/\.(vue|css)$/.test(name)) files.push(path);
  }
})(src);

function styles(path) {
  const source = readFileSync(path, "utf8");
  if (path.endsWith(".vue"))
    return [...source.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)].map(match => match[1]).join("\n");
  // The token definitions themselves (the light :root block and the dark
  // overrides) are the one place literals belong.
  return path.endsWith("styles.css") ? source.replace(/:root[^{]*\{[^}]*\}/g, "") : source;
}

test("component styles read design tokens instead of literal colours, sizes and radii", () => {
  let hex = 0,
    fontPx = 0,
    radiusPx = 0;
  for (const path of files) {
    const css = styles(path);
    hex += css.match(/#[0-9a-fA-F]{3,8}\b/g)?.length ?? 0;
    fontPx += css.match(/font-size\s*:\s*[0-9.]+px/g)?.length ?? 0;
    radiusPx += [...css.matchAll(/border-radius\s*:\s*([^;}]+)/g)].filter(match =>
      /\b(?:[1-9]|1\d|2[0-4])(?:\.\d+)?px/.test(match[1]),
    ).length;
  }
  assert.ok(hex <= LIMITS.hex, `${hex} literal colours (limit ${LIMITS.hex}); use a token from styles.css`);
  assert.ok(fontPx <= LIMITS.fontPx, `${fontPx} px font sizes (limit ${LIMITS.fontPx}); use --text-*`);
  assert.ok(radiusPx <= LIMITS.radiusPx, `${radiusPx} px radii (limit ${LIMITS.radiusPx}); use --radius-*`);
});
