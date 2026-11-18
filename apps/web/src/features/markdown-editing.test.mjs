import test from "node:test";
import assert from "node:assert/strict";
import { continueList, indentLines } from "./markdown-editing.ts";
import { framedAnchors, lineAtOffset, lineForPreviewTop, offsetAtLine, previewTopForLine } from "./markdown-sync.ts";
import { markdownBlocks, markdownBlocksWithLines } from "./markdown-model.ts";

function apply(text, edit) {
  return text.slice(0, edit.from) + edit.insert + text.slice(edit.to);
}

test("Enter continues bullets, numbers, tasks and quotes", () => {
  const cases = [
    ["- one", "- one\n- "],
    ["  * nested", "  * nested\n  * "],
    ["9. nine", "9. nine\n10. "],
    ["1) first", "1) first\n2) "],
    ["- [x] done", "- [x] done\n- [ ] "],
    ["> quoted", "> quoted\n> "],
  ];
  for (const [text, expected] of cases) {
    const edit = continueList(text, text.length);
    assert.equal(apply(text, edit), expected, text);
    assert.equal(edit.selectionStart, expected.length);
  }
});

test("Enter on an empty item ends the list; plain text and carets in the marker are left alone", () => {
  const text = "intro\n- one\n- ";
  const edit = continueList(text, text.length);
  assert.equal(apply(text, edit), "intro\n- one\n");
  assert.equal(continueList("just prose", 10), undefined);
  assert.equal(continueList("- item", 1), undefined);
});

test("Enter in the middle of an item splits it into two items", () => {
  const text = "- hello world";
  const edit = continueList(text, 8);
  assert.equal(apply(text, edit), "- hello \n- world");
});

test("Tab indents and Shift+Tab outdents every selected line", () => {
  const text = "- a\n- b\n- c";
  const indented = indentLines(text, 0, 7, false);
  assert.equal(apply(text, indented), "  - a\n  - b\n- c");
  const back = "  - a\n  - b";
  assert.equal(apply(back, indentLines(back, 0, back.length, true)), "- a\n- b");
  assert.equal(indentLines("- a", 0, 0, true), undefined);
});

test("Tab on prose with no selection inserts an indent at the caret", () => {
  const edit = indentLines("word", 2, 2, false);
  assert.equal(apply("word", edit), "wo  rd");
  assert.equal(edit.selectionStart, 4);
});

test("blocks know the source line they start on, and the plain API is unchanged", () => {
  const text = "# Title\n\nPara one\nstill one\n\n- a\n- b\n\n```js\nx\n```\n| a | b |\n|---|---|\n| 1 | 2 |";
  assert.deepEqual(
    markdownBlocksWithLines(text).map(entry => [entry.block.type, entry.line]),
    [
      ["heading", 0],
      ["paragraph", 2],
      ["list", 5],
      ["code", 8],
      ["table", 11],
    ],
  );
  assert.deepEqual(
    markdownBlocks(text),
    markdownBlocksWithLines(text).map(entry => entry.block),
  );
});

test("editor offsets map to fractional lines and back, wrapped lines included", () => {
  const offsets = [15, 36, 99, 120]; // line 1 wraps onto three rows
  assert.equal(lineAtOffset(offsets, 0), 0);
  assert.equal(lineAtOffset(offsets, 36), 1);
  assert.equal(lineAtOffset(offsets, 67.5), 1.5);
  assert.equal(lineAtOffset(offsets, 500), 3);
  assert.equal(offsetAtLine(offsets, 1.5), 67.5);
  assert.equal(offsetAtLine(offsets, 9), 120);
});

test("preview anchors interpolate between blocks in both directions", () => {
  const anchors = framedAnchors(
    [
      { line: 2, top: 100 },
      { line: 6, top: 300 },
    ],
    10,
    500,
  );
  assert.deepEqual(anchors, [
    { line: 0, top: 0 },
    { line: 2, top: 100 },
    { line: 6, top: 300 },
    { line: 10, top: 500 },
  ]);
  assert.equal(previewTopForLine(anchors, 4), 200);
  assert.equal(lineForPreviewTop(anchors, 200), 4);
  assert.equal(previewTopForLine(anchors, 1), 50);
  assert.equal(lineForPreviewTop(anchors, 400), 8);
});

test("anchors out of order or on one line still move forward", () => {
  const anchors = framedAnchors(
    [
      { line: 4, top: 80 },
      { line: 1, top: 20 },
      { line: 4, top: 90 },
      { line: 5, top: 70 },
    ],
    6,
    120,
  );
  assert.deepEqual(
    anchors.map(anchor => anchor.line),
    [0, 1, 4, 5, 6],
  );
  for (let index = 1; index < anchors.length; index++) assert.ok(anchors[index].top >= anchors[index - 1].top);
});
