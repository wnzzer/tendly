import { test } from "node:test";
import assert from "node:assert/strict";
import {
  attachmentError,
  canSendMessage,
  composeMessage,
  formatBytes,
  MAX_ATTACHMENT_BYTES,
  MAX_ATTACHMENTS_PER_MESSAGE,
} from "./attachment-state.ts";

const file = (path, name = "photo.jpg", bytes = 1024) => ({ path, name, bytes });

test("attachment paths travel with the message so the agent knows what to open", () => {
  assert.equal(composeMessage("  look at this  ", []), "look at this");
  assert.equal(
    composeMessage("what is wrong here?", [file(".tendly-files/2026-09-14/photo-abc.jpg")]),
    "Attached file in this workspace:\n- .tendly-files/2026-09-14/photo-abc.jpg\n\nwhat is wrong here?",
  );
  // Attachments alone are a valid message; the header stands on its own.
  assert.equal(
    composeMessage("   ", [file("a.png"), file("b.png")]),
    "Attached files in this workspace:\n- a.png\n- b.png",
  );
  // No attachments means no header at all, not an empty one.
  assert.equal(composeMessage("hello", []).includes("Attached"), false);
});

test("a message needs text or an attachment, and never both empty", () => {
  assert.equal(canSendMessage("", []), false);
  assert.equal(canSendMessage("   \n  ", []), false);
  assert.equal(canSendMessage("hi", []), true);
  assert.equal(canSendMessage("", [file("a.png")]), true);
});

test("oversized, empty and excess picks are refused before anything uploads", () => {
  assert.equal(attachmentError({ name: "ok.png", size: 1024 }, 0), undefined);
  assert.equal(attachmentError({ name: "ok.png", size: MAX_ATTACHMENT_BYTES }, 0), undefined);
  assert.match(attachmentError({ name: "big.mov", size: MAX_ATTACHMENT_BYTES + 1 }, 0), /50 MiB/);
  assert.match(attachmentError({ name: "empty", size: 0 }, 0), /empty/);
  assert.match(attachmentError({ name: "ok.png", size: 10 }, MAX_ATTACHMENTS_PER_MESSAGE), /at most/);
  // The count check comes first, so a full list reports the real reason.
  assert.match(
    attachmentError({ name: "big", size: MAX_ATTACHMENT_BYTES + 1 }, MAX_ATTACHMENTS_PER_MESSAGE),
    /at most/,
  );
});

test("sizes read naturally without overstating what was attached", () => {
  assert.equal(formatBytes(0), "0 B");
  assert.equal(formatBytes(900), "900 B");
  assert.equal(formatBytes(2048), "2 KB");
  assert.equal(formatBytes(1536), "1.5 KB");
  assert.equal(formatBytes(1024 * 1024), "1 MB");
  assert.equal(formatBytes(5.5 * 1024 * 1024), "5.5 MB");
  assert.equal(formatBytes(-1), "0 B");
  assert.equal(formatBytes(Number.NaN), "0 B");
});
