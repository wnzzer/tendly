import test from "node:test";
import assert from "node:assert/strict";
import { imageCaveat, isImage, pasteText, shellPath, uploadName, uploadProblem } from "./terminal-uploads.ts";

test("a pasted path is escaped the way a terminal escapes a dropped file", () => {
  assert.equal(
    shellPath("/srv/ws/.tendly-files/pastes/2026-10-07/shot-a1b2.png"),
    "/srv/ws/.tendly-files/pastes/2026-10-07/shot-a1b2.png",
  );
  assert.equal(shellPath("/home/me/My Projects/a (1).png"), "/home/me/My\\ Projects/a\\ \\(1\\).png");
  assert.equal(shellPath("/x/it's $HOME;&.txt"), String.raw`/x/it\'s\ \$HOME\;\&.txt`);
  assert.equal(
    pasteText(["/a b.png", "/c.pdf"]),
    "/a\\ b.png /c.pdf ",
    "space-separated, ending in a space to keep typing",
  );
  assert.equal(pasteText([]), "");
});

test("clipboard screenshots get a dated name; real files keep theirs", () => {
  const now = new Date(2026, 9, 7, 10, 32, 5);
  assert.equal(uploadName({ name: "image.png", type: "image/png" }, now), "paste-20261007-103205.png");
  assert.equal(uploadName({ name: "", type: "image/jpeg" }, now), "paste-20261007-103205.jpg");
  assert.equal(uploadName({ name: "blob", type: "image/webp" }, now), "paste-20261007-103205.webp");
  assert.equal(uploadName({ name: "design v2.png", type: "image/png" }, now), "design v2.png");
  assert.equal(uploadName({ name: "report.pdf", type: "application/pdf" }, now), "report.pdf");
});

test("what cannot be uploaded is said before sending, and HEIC carries a caveat", () => {
  assert.ok(uploadProblem({ size: 0 }));
  assert.ok(uploadProblem({ size: 51 * 1024 * 1024 }));
  assert.equal(uploadProblem({ size: 3 * 1024 * 1024 }), undefined);
  assert.ok(imageCaveat("IMG_0001.HEIC"));
  assert.equal(imageCaveat("photo.jpg"), undefined);
  assert.ok(isImage("a.png") && isImage("scan.tiff") && isImage("x", "image/png"));
  assert.ok(!isImage("notes.pdf"));
});

test("a batch pastes the paths that made it, in order, and keeps a failed one to retry", async () => {
  const { useTerminalUploads } = await import("./terminal-uploads.ts");
  const pasted = [],
    warnings = [];
  const uploads = useTerminalUploads({
    sessionId: () => "s1",
    canUpload: () => true,
    focus: () => {},
    paste: text => pasted.push(text),
    translate: text => text,
    warn: message => warnings.push(message),
    upload: (_session, file) => ({
      promise:
        file.name === "bad.txt"
          ? Promise.reject(new Error("Disk full"))
          : Promise.resolve({ name: file.name, absolute: `/up/${file.name}` }),
      abort() {},
    }),
  });
  await uploads.sendFiles([new File(["a"], "a.txt"), new File(["b"], "bad.txt"), new File(["c"], "c.txt")]);
  assert.deepEqual(pasted, [pasteText(["/up/a.txt", "/up/c.txt"])]);
  assert.equal(uploads.uploads.value.length, 1, "only the failed chip stays");
  assert.equal(uploads.uploads.value[0].failed, "Disk full");
  assert.equal(uploads.uploads.value[0].retryable, true);
});

test("nothing uploads before a terminal is connected to type the paths into", async () => {
  const { useTerminalUploads } = await import("./terminal-uploads.ts");
  const warnings = [];
  const uploads = useTerminalUploads({
    sessionId: () => "s1",
    canUpload: () => false,
    focus() {},
    paste() {
      throw new Error("no paste");
    },
    translate: text => text,
    warn: message => warnings.push(message),
    upload() {
      throw new Error("no upload");
    },
  });
  await uploads.sendFiles([new File(["a"], "a.txt")]);
  assert.deepEqual(warnings, ["Connect the terminal before adding files."]);
  assert.equal(uploads.uploads.value.length, 0);
});
