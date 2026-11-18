import { test } from "node:test";
import assert from "node:assert/strict";
import { createFileSearch, highlight, SEARCH_DEBOUNCE_MS } from "./file-search.ts";

const settle = (ms = SEARCH_DEBOUNCE_MS + 30) => new Promise(resolve => setTimeout(resolve, ms));
const results = files => ({ files, truncated: false, available: true });

test("typing produces one query, not one per keystroke", async () => {
  const queries = [];
  const search = createFileSearch(async (_w, q) => {
    queries.push(q);
    return results([{ path: q }]);
  });
  let applied;
  for (const q of ["c", "ch", "cha", "chat"])
    search.search("w1", q, r => {
      applied = r;
    });
  await settle();
  assert.deepEqual(queries, ["chat"], "only the final query should reach the index");
  assert.deepEqual(applied.files, [{ path: "chat" }]);
});

test("a slow answer for an old query cannot overwrite the current one", async () => {
  const search = createFileSearch(async (_w, q) => {
    // The first query resolves last, which is the ordering that corrupts a view.
    await new Promise(resolve => setTimeout(resolve, q === "slow" ? 120 : 5));
    return results([{ path: q }]);
  });
  const seen = [];
  search.search("w1", "slow", r => seen.push(r?.files[0]?.path ?? null));
  // Past the debounce so 'slow' is genuinely in flight, but well before its
  // 120ms reply — otherwise this would pass without a race ever occurring.
  await settle();
  search.search("w1", "fast", r => seen.push(r?.files[0]?.path ?? null));
  await settle(SEARCH_DEBOUNCE_MS + 250);
  assert.deepEqual(seen, ["fast"], `a superseded result was applied: ${JSON.stringify(seen)}`);
});

test("an empty query clears immediately without asking the index", async () => {
  let calls = 0;
  const search = createFileSearch(async () => {
    calls += 1;
    return results([]);
  });
  let applied = "untouched";
  search.search("w1", "   ", r => {
    applied = r;
  });
  assert.equal(applied, undefined, "clearing must not wait for a debounce");
  await settle();
  assert.equal(calls, 0);
});

test("a failed query leaves the previous view rather than claiming no matches", async () => {
  const search = createFileSearch(async () => {
    throw new Error("offline");
  });
  let applied = "untouched";
  search.search("w1", "chat", r => {
    applied = r;
  });
  await settle();
  // undefined means "no workspace results to show", which the caller renders as
  // its own fallback; an empty files array would read as a confident zero.
  assert.equal(applied, undefined);
});

test("highlighting marks the characters the index actually matched", () => {
  const parts = highlight("apps/web/src/features/chat-model.ts", "chat-model.ts", [22, 23, 24, 25]);
  assert.deepEqual(parts.map(p => p.text).join(""), "chat-model.ts", "no character may be lost");
  assert.deepEqual(
    parts.filter(p => p.match).map(p => p.text),
    ["chat"],
  );
});

test("indices outside the name are ignored rather than trusted", () => {
  // Offsets pointing into the directory part, or past the end, must not shift
  // the highlight onto characters the index never matched.
  const parts = highlight("src/app.ts", "app.ts", [0, 1, 2, 999]);
  assert.equal(parts.filter(p => p.match).length, 0);
  assert.equal(parts.map(p => p.text).join(""), "app.ts");
});

test("scrollable panes get a scrollbar that reserves space, not an overlay", async () => {
  const { readFile } = await import("node:fs/promises");
  const css = await readFile(new URL("../styles.css", import.meta.url), "utf8");
  // Chrome ignores ::-webkit-scrollbar entirely once `scrollbar-width` is set,
  // and macOS still overlays at `thin` — so the standard property must stay
  // scoped to engines without the pseudo-elements, or every scrollbar goes back
  // to having no width to press on.
  assert.match(css, /::-webkit-scrollbar \{[^}]*width: 11px/);
  const standalone = css.replace(/@supports[^{]*\{[\s\S]*?\n\}/g, "");
  assert.doesNotMatch(
    standalone,
    /^\s*\*\s*\{[^}]*scrollbar-width/m,
    "scrollbar-width outside the @supports guard disables the pseudo-elements in Chrome",
  );
  assert.match(css, /@supports \(scrollbar-width: thin\) and \(not selector\(::-webkit-scrollbar\)\)/);

  // The thumb's padding is drawn as a transparent border so the hit area stays
  // the full track width; a real margin would shrink what can be grabbed.
  assert.match(
    css,
    /::-webkit-scrollbar-thumb \{[\s\S]*?border: 3px solid transparent;[\s\S]*?background-clip: content-box/,
  );
});
