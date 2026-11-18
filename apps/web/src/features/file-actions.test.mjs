import { readFileSync } from "node:fs";
import { test } from "node:test";
import assert from "node:assert/strict";
import { nameTaken, newFilePath, renamedPath } from "./file-actions.ts";
import { markupOf } from "../source-text.mjs";

test("a typed name becomes the path the person meant, or no request at all", () => {
  assert.equal(newFilePath("", "notes.md"), "notes.md");
  assert.equal(newFilePath("docs", "notes.md"), "docs/notes.md");
  // Typing a path is how a folder gets made along the way.
  assert.equal(newFilePath("docs", "guide/start.md"), "docs/guide/start.md");
  // Habits from the shell and the clipboard: a leading slash, a trailing one,
  // stray spaces, a doubled separator. None of them change what was meant.
  assert.equal(newFilePath("docs", "/notes.md"), "docs/notes.md");
  assert.equal(newFilePath("docs/", "notes.md"), "docs/notes.md");
  assert.equal(newFilePath("docs", "  notes.md  "), "docs/notes.md");
  assert.equal(newFilePath("docs", "guide//start.md"), "docs/guide/start.md");
  // Nothing typed is not a file named nothing; it is no request.
  assert.equal(newFilePath("docs", "   "), "");
  assert.equal(newFilePath("", "/"), "");
});

test("renaming changes the last name and stays in the folder it was offered in", () => {
  assert.equal(renamedPath("docs/hello.md", "readme.md"), "docs/readme.md");
  assert.equal(renamedPath("hello.md", "readme.md"), "readme.md");
  assert.equal(renamedPath("docs/guide", " notes "), "docs/notes");
  // A path typed into a rename would move the file somewhere the tree never
  // offered, so it is refused rather than quietly obeyed.
  assert.equal(renamedPath("docs/hello.md", "guide/readme.md"), "");
  assert.equal(renamedPath("docs/hello.md", "../escape.md"), "");
  assert.equal(renamedPath("docs/hello.md", "  "), "");
  // The unchanged name composes to the path it already has, which the caller
  // reads as nothing to do.
  assert.equal(renamedPath("docs/hello.md", "hello.md"), "docs/hello.md");
});

test("a folder the tree has not loaded claims to know nothing about its names", () => {
  const loaded = {
    loaded: true,
    loading: false,
    error: "",
    entries: [{ path: "docs/taken.md", name: "taken.md", kind: "file", size: 0 }],
  };
  const directories = new Map([
    ["docs", loaded],
    ["", { ...loaded, entries: [{ path: "root.md", name: "root.md", kind: "file", size: 0 }] }],
  ]);
  assert.equal(nameTaken(directories, "docs/taken.md"), true);
  assert.equal(nameTaken(directories, "root.md"), true);
  assert.equal(nameTaken(directories, "docs/free.md"), false);
  // Unknown or unloaded folders answer false, leaving the refusal to the host
  // rather than inventing one from what the tree happens to have seen.
  assert.equal(nameTaken(directories, "unseen/taken.md"), false);
  directories.set("half", {
    loaded: false,
    loading: true,
    error: "",
    entries: [{ path: "half/taken.md", name: "taken.md", kind: "file", size: 0 }],
  });
  assert.equal(nameTaken(directories, "half/taken.md"), false);
});

test("the tree makes folders as well as files, and only a file opens once made", () => {
  const source = markupOf(new URL("./FileExplorer.vue", import.meta.url));
  assert.match(source, /t\(["']New folder["']\)/, "a header and menu action makes a folder");
  assert.match(source, /t\(["']New folder here["']\)/, "a folder row offers a folder inside it");
  assert.match(source, /draft\.directory \? \{path, directory: true\} : \{path\}/);
  assert.match(source, /if \(!draft\.directory\) emit\("open", entry\)/);
});
