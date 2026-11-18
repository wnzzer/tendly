import { test } from "node:test";
import assert from "node:assert/strict";
import { createRowHeights, estimateRowHeight } from "./row-heights.ts";

const reply = text => ({ type: "message", id: "a", role: "assistant", text });

test("a long reply with code is estimated tall, a short one short, a folded call small", () => {
  const code = "```rust\n" + Array.from({ length: 30 }, (_, i) => `let v${i} = ${i};`).join("\n") + "\n```";
  const long = estimateRowHeight(
    reply(`## Turn 1\n\nHere is what changed:\n\n- one\n- two\n\n${code}\n\n| a | b |\n| - | - |\n| 1 | 2 |\n\nDone.`),
    800,
  );
  assert.ok(long > 800 && long < 1400, `${long}`);
  assert.ok(estimateRowHeight(reply("Sure."), 800) < 100);
  assert.equal(estimateRowHeight({ type: "tool_run", id: "t", tools: [{}, {}, {}] }, 800), 46);
  // A narrower view wraps a long paragraph onto more lines.
  const paragraph = reply("word ".repeat(400));
  assert.ok(estimateRowHeight(paragraph, 360) > estimateRowHeight(paragraph, 800));
});

test("measured heights are kept by session, row and width, and bounded", () => {
  const heights = createRowHeights(2);
  heights.remember("s", "message:a", 800, 640);
  assert.equal(heights.get("s", "message:a", 810), 640, "a few pixels wider is the same layout");
  assert.equal(heights.get("s", "message:a", 400), undefined, "a much narrower one is not");
  assert.equal(heights.get("other", "message:a", 800), undefined);
  heights.remember("s", "b", 800, 1);
  heights.remember("s", "c", 800, 1);
  assert.equal(heights.get("s", "message:a", 800), undefined, "the oldest goes first");
  heights.remember("s", "zero", 800, 0);
  assert.equal(heights.get("s", "zero", 800), undefined);
});
