import { test } from "node:test";
import assert from "node:assert/strict";
import {
  caretBand,
  EDITOR_TOP,
  LINE_HEIGHT,
  lineOfOffset,
  modeChoices,
  parseMarkdownMode,
  visibleMode,
} from "./file-view.ts";

test("a remembered markdown mode is used only when it is one", () => {
  for (const mode of ["preview", "split", "source"]) assert.equal(parseMarkdownMode(mode), mode);
  for (const value of [null, undefined, "", "Split", "edit"]) assert.equal(parseMarkdownMode(value), "split");
});

test("only markdown has a rendered view, and side by side needs the room", () => {
  assert.equal(visibleMode(false, "preview", true), "source");
  assert.equal(visibleMode(true, "split", true), "split");
  assert.equal(visibleMode(true, "split", false), "preview", "two columns that do not fit become the rendered view");
  assert.equal(visibleMode(true, "source", false), "source");
  assert.deepEqual(
    modeChoices(true).map(choice => choice.mode),
    ["preview", "split", "source"],
  );
  assert.deepEqual(
    modeChoices(false).map(choice => choice.label),
    ["Preview", "Edit"],
  );
});

test("the caret line counts the line breaks before it", () => {
  const text = "one\ntwo\nthree";
  assert.equal(lineOfOffset(text, 0), 0);
  assert.equal(lineOfOffset(text, 3), 0, "just before the first break");
  assert.equal(lineOfOffset(text, 4), 1);
  assert.equal(lineOfOffset(text, text.length), 2);
  assert.equal(lineOfOffset("", 0), 0);
});

test("the band sits on the caret line, measured where lines wrap", () => {
  const rows = { tops: [12, 32, 92], heights: [20, 60, 20] };
  assert.deepEqual(caretBand(2, false, rows), { top: EDITOR_TOP + 2 * LINE_HEIGHT, height: LINE_HEIGHT });
  assert.deepEqual(caretBand(1, true, rows), { top: 32, height: 60 });
  assert.equal(caretBand(5, true, rows), undefined, "not measured yet");
  assert.equal(caretBand(-1, false, rows), undefined, "the editor does not have the caret");
});
