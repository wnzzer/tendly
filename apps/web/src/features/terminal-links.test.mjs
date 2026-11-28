import { test } from "node:test";
import assert from "node:assert/strict";
import {
  findTerminalLinks,
  isTerminalLinkModifier,
  parseOsc8Link,
  resolveWithTerminalCwd,
  stitchTuiRows,
  terminalLinkAt,
} from "./terminal-links.ts";

test("compiler, TypeScript, Python and git diff lines yield file references with line numbers", () => {
  const rust = "  --> crates/server/src/pty_routes.rs:149:5:";
  const [rustLink] = findTerminalLinks(rust);
  assert.deepEqual(rustLink, {
    kind: "file",
    text: "crates/server/src/pty_routes.rs:149:5",
    path: "crates/server/src/pty_routes.rs",
    line: 149,
    start: rust.indexOf("crates/"),
    end: rust.length - 1,
  });

  const tsLine = "src/App.vue(652,9): error TS2345: Argument of type";
  const [tsLink] = findTerminalLinks(tsLine);
  assert.deepEqual(tsLink, {
    kind: "file",
    text: "src/App.vue(652,9)",
    path: "src/App.vue",
    line: 652,
    start: 0,
    end: "src/App.vue(652,9)".length,
  });

  const pyLine = '  File "/srv/app/worker.py", line 87, in run';
  const [pyLink] = findTerminalLinks(pyLine);
  assert.equal(pyLink?.kind, "file");
  assert.equal(pyLink?.path, "/srv/app/worker.py");
  assert.equal(pyLink?.line, 87);

  const diffLine = "+++ b/apps/web/src/components/TerminalPane.vue";
  const [diffLink] = findTerminalLinks(diffLine);
  assert.equal(diffLink?.kind, "file");
  assert.equal(diffLink?.path, "apps/web/src/components/TerminalPane.vue");
});

test("web URLs and bare filenames are found at the clicked character offset while versions and abbreviations are ignored", () => {
  const line = "Server at http://localhost:5173/ (see README.md, v24.21.0, e.g. Cargo.toml:12)";
  const links = findTerminalLinks(line);
  assert.deepEqual(
    links.map(item => (item.kind === "url" ? item.href : `${item.path}:${item.line ?? ""}`)),
    ["http://localhost:5173/", "README.md:", "Cargo.toml:12"],
  );
  const hit = terminalLinkAt(line, line.indexOf("Cargo.toml") + 3);
  assert.equal(hit?.kind, "file");
  if (hit?.kind === "file") {
    assert.equal(hit.path, "Cargo.toml");
    assert.equal(hit.line, 12);
  }
  assert.equal(terminalLinkAt(line, line.indexOf("v24.21.0") + 2), undefined);
});

test("link activation modifier uses ⌘ on macOS and Ctrl elsewhere without Alt", () => {
  assert.equal(isTerminalLinkModifier({ metaKey: true, ctrlKey: false, altKey: false }, true), true);
  assert.equal(isTerminalLinkModifier({ metaKey: false, ctrlKey: true, altKey: false }, true), false);
  assert.equal(isTerminalLinkModifier({ metaKey: false, ctrlKey: true, altKey: false }, false), true);
  assert.equal(isTerminalLinkModifier({ metaKey: false, ctrlKey: true, altKey: true }, false), false);
});

test("working directory from OSC 7 resolves subdirectory and external paths without doubling repo prefixes", () => {
  assert.deepEqual(resolveWithTerminalCwd("src/App.vue", "/repo/apps/web", "/repo"), {
    path: "/repo/apps/web/src/App.vue",
    fallbackPath: "src/App.vue",
  });
  // `git status` inside `apps/web` still prints repo-relative `apps/web/src/App.vue`.
  assert.deepEqual(resolveWithTerminalCwd("apps/web/src/App.vue", "/repo/apps/web", "/repo"), {
    path: "apps/web/src/App.vue",
  });
  assert.deepEqual(resolveWithTerminalCwd("./local.txt", "/repo/apps/web", "/repo"), {
    path: "/repo/apps/web/local.txt",
  });
  assert.deepEqual(resolveWithTerminalCwd("nginx.conf", "/etc/nginx", "/repo"), {
    path: "/etc/nginx/nginx.conf",
  });
});

test("OSC 8 file:// hyperlinks resolve paths, line anchors and extensionless host files", () => {
  assert.deepEqual(parseOsc8Link("file:///home/wnzzer/agentdock/apps/web/src/App.vue#L42-L50", "App.vue"), {
    kind: "file",
    text: "App.vue",
    path: "/home/wnzzer/agentdock/apps/web/src/App.vue",
    line: 42,
    start: 0,
    end: 7,
  });
  assert.deepEqual(parseOsc8Link("file:///etc/hosts#L2", "[hosts link]"), {
    kind: "file",
    text: "[hosts link]",
    path: "/etc/hosts",
    line: 2,
    start: 0,
    end: 12,
  });
});

test("TUI box-wrapped paths stitch across adjacent bordered rows", () => {
  const rows = [
    "│ Box wrapped: /home/wnzzer/agentdock/apps/web/src/components/Term │",
    "│ inalPane.vue:10                                                  │",
  ];
  const topHit = stitchTuiRows(rows, 0, rows[0].indexOf("components/Term") + 3);
  assert.ok(topHit);
  const topLink = terminalLinkAt(topHit.text, topHit.offset);
  assert.equal(topLink?.kind, "file");
  if (topLink?.kind === "file") {
    assert.equal(topLink.path, "/home/wnzzer/agentdock/apps/web/src/components/TerminalPane.vue");
    assert.equal(topLink.line, 10);
  }

  const bottomHit = stitchTuiRows(rows, 1, rows[1].indexOf("inalPane.vue") + 2);
  assert.ok(bottomHit);
  const bottomLink = terminalLinkAt(bottomHit.text, bottomHit.offset);
  assert.equal(bottomLink?.kind, "file");
  if (bottomLink?.kind === "file") {
    assert.equal(bottomLink.path, "/home/wnzzer/agentdock/apps/web/src/components/TerminalPane.vue");
    assert.equal(bottomLink.line, 10);
  }
});
