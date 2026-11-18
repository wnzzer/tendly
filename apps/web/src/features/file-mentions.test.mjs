import { test } from "node:test";
import assert from "node:assert/strict";
import { mentionQuery, applyMention } from "./file-mentions.ts";

test("a mention is recognised wherever a word can start", () => {
  assert.deepEqual(mentionQuery("@chat", 5), { term: "chat", start: 0, end: 5 });
  // Mid-sentence is the ordinary case: a file is referenced while writing.
  assert.deepEqual(mentionQuery("compare @src/a", 14), { term: "src/a", start: 8, end: 14 });
  assert.equal(mentionQuery("@src/features/chat-model.ts", 27).term, "src/features/chat-model.ts");
});

test("an @ that continues a word is not a mention", () => {
  // Otherwise every email address in a message opens a file menu.
  assert.equal(mentionQuery("mail me at someone@example.com", 30), undefined);
  assert.equal(mentionQuery("user@host", 9), undefined);
});

test("whitespace ends a mention, and prose never becomes one", () => {
  assert.equal(mentionQuery("@chat model please", 18), undefined);
  assert.equal(mentionQuery("no mention here", 15), undefined);
  assert.equal(mentionQuery(`@${"a".repeat(200)}`, 201), undefined, "an overlong run is not a path");
});

test("completing replaces only the typed fragment, leaving the rest of the sentence", () => {
  const text = "compare @src/a with the other one";
  const query = mentionQuery(text, 14);
  assert.deepEqual(query, { term: "src/a", start: 8, end: 14 });
  const next = applyMention(text, query, "apps/web/src/a.ts");
  // One space, not two: the sentence already supplied the one that follows.
  assert.equal(next.text, "compare @apps/web/src/a.ts with the other one");
  assert.equal(next.caret, "compare @apps/web/src/a.ts".length);
});

test("completing at the end of a message supplies the space itself", () => {
  const next = applyMention("look at @chat", mentionQuery("look at @chat", 13), "src/chat.ts");
  assert.equal(next.text, "look at @src/chat.ts ");
  assert.equal(next.caret, next.text.length);
});

test("a path with spaces is quoted so it stays one argument", () => {
  const next = applyMention("@my", { term: "my", start: 0, end: 3 }, "docs/my notes.md");
  assert.equal(next.text, '@"docs/my notes.md" ');
  // Without quoting the agent would read a path ending at the first space.
  assert.ok(next.text.includes('"docs/my notes.md"'));
});
