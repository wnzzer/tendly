import test from "node:test";
import assert from "node:assert/strict";
import { messageQueue, queuedContent } from "./message-queue.ts";

test("queued messages go out in order and each session keeps its own", () => {
  messageQueue.push("a", { id: "1", text: "first" });
  messageQueue.push("a", { id: "2", text: "second" });
  messageQueue.push("b", { id: "3", text: "elsewhere" });
  assert.equal(messageQueue.remove("a", "2").text, "second");
  assert.equal(messageQueue.shift("a").text, "first");
  assert.equal(messageQueue.shift("a"), undefined);
  assert.equal(messageQueue.get("b").length, 1);
});

test("a message waiting mid-turn keeps its attachments, and may be only attachments", () => {
  const file = { path: ".tendly-files/shot.png", name: "shot.png", bytes: 10 };
  assert.equal(queuedContent({ id: "a", text: " look at this " }), "look at this");
  assert.equal(
    queuedContent({ id: "b", text: "look", attachments: [file] }),
    "Attached file in this workspace:\n- .tendly-files/shot.png\n\nlook",
  );
  assert.equal(
    queuedContent({ id: "c", text: "", attachments: [file] }),
    "Attached file in this workspace:\n- .tendly-files/shot.png",
  );
});
