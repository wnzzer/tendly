import { test } from "node:test";
import assert from "node:assert/strict";
import { applyCommand, matchCommands, moveHighlight, slashQuery } from "./slash-commands.ts";

test("a command is only being typed at the start of a message, before its arguments", () => {
  assert.deepEqual(slashQuery("/mod", 4), { term: "mod", length: 4 });
  assert.deepEqual(slashQuery("/", 1), { term: "", length: 1 });
  // Once an argument starts, the user has moved past the command name.
  assert.equal(slashQuery("/model sonnet", 13), undefined);
  assert.equal(slashQuery("/model ", 7), undefined);
  // A slash inside ordinary prose is a path, not a command.
  assert.equal(slashQuery("see /etc/hosts", 14), undefined);
  assert.equal(slashQuery("", 0), undefined);
  assert.equal(slashQuery("/x".padEnd(200, "y"), 120), undefined);
  // The caret, not the whole text, decides what is being completed.
  assert.deepEqual(slashQuery("/model", 3), { term: "mo", length: 3 });
});

test("suggestions come only from what the client advertised, prefix first", () => {
  const commands = ["model", "usage", "compact", "mcp", "permissions"];
  // Prefix matches lead; 'compact' and 'permissions' merely contain an m.
  assert.deepEqual(matchCommands(commands, "m"), ["model", "mcp", "compact", "permissions"]);
  assert.deepEqual(matchCommands(commands, "usa"), ["usage"]);
  assert.deepEqual(matchCommands(commands, "MOD"), ["model"]);
  // An empty term lists what exists; it never invents a command.
  assert.deepEqual(matchCommands(commands, ""), commands);
  assert.deepEqual(matchCommands([], "model"), []);
  assert.deepEqual(matchCommands(commands, "nonexistent"), []);
  assert.equal(
    matchCommands(
      Array.from({ length: 200 }, (_, i) => `c${i}`),
      "c",
    ).length,
    8,
  );
});

test("choosing a command replaces only the typed prefix and keeps the rest", () => {
  assert.deepEqual(applyCommand("/mod", { term: "mod", length: 4 }, "model"), { text: "/model ", caret: 7 });
  // Text after the caret survives the completion.
  assert.deepEqual(applyCommand("/mod rest", { term: "mod", length: 4 }, "model"), { text: "/model  rest", caret: 7 });
});

test("the highlight wraps at both ends and survives an empty list", () => {
  assert.equal(moveHighlight(0, 3, 1), 1);
  assert.equal(moveHighlight(2, 3, 1), 0);
  assert.equal(moveHighlight(0, 3, -1), 2);
  assert.equal(moveHighlight(0, 0, 1), 0);
});

test("a narrowed list restarts its highlight instead of pointing at a removed row", () => {
  const commands = ["deep-research", "design", "dataviz", "usage"];
  const wide = matchCommands(commands, "");
  let index = moveHighlight(0, wide.length, 1);
  index = moveHighlight(index, wide.length, 1);
  assert.equal(wide[index], "dataviz");
  // The composer resets the highlight when the match list changes, so a shorter
  // list never leaves the index past its end.
  const narrow = matchCommands(commands, "usa");
  assert.deepEqual(narrow, ["usage"]);
  assert.equal(index >= narrow.length, true, "the stale index would be out of range");
  assert.equal(narrow[0], "usage");
});

test("a command a client cannot act on is identified rather than sent as prose", async () => {
  const { unsupportedCommand } = await import("./slash-commands.ts");
  // A client that advertises commands handles its own; nothing is intercepted.
  assert.equal(unsupportedCommand("/model", ["model", "usage"]), undefined);
  // A client with no command support would receive this as literal text, and
  // what it names is a control in this composer.
  assert.equal(unsupportedCommand("/model", []), "model");
  assert.equal(unsupportedCommand("  /effort  ", []), "effort");
  // Everything else is the user's own text. Codex publishes no command list, so
  // blocking every slash there made its sessions unable to send one at all.
  for (const text of ["/compact", "/undo", "/vim", "/anything"]) {
    assert.equal(unsupportedCommand(text, []), undefined, text);
  }
  // Ordinary prose, paths and commands with arguments are left alone: only a
  // bare command is unambiguous enough to intercept.
  for (const text of ["hello", "see /etc/hosts", "/model sonnet", "/", "//x", ""]) {
    assert.equal(unsupportedCommand(text, []), undefined, text);
  }
  // A client that has not announced yet — a session opened a moment ago — has an
  // empty list because nothing was said, not because it runs no commands. Its
  // /model must reach the client that does run it.
  assert.equal(unsupportedCommand("/model", [], false), undefined);
});
