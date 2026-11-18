import { test } from "node:test";
import assert from "node:assert/strict";
import { afterHello, parseControl, readFramed, streamUrl } from "./terminal-protocol.ts";

test("a framed frame carries its sequence number ahead of the output", () => {
  const buffer = new ArrayBuffer(11);
  new DataView(buffer).setBigUint64(0, 42n);
  new Uint8Array(buffer, 8).set([104, 105, 33]);
  const frame = readFramed(buffer);
  assert.equal(frame.seq, 42);
  assert.deepEqual([...frame.bytes], [104, 105, 33]);
  // Too short to hold a sequence number: nothing to draw, nothing to resume from.
  assert.equal(readFramed(new ArrayBuffer(7)), undefined);
  // A frame of only a header is valid and draws nothing.
  assert.equal(readFramed(new ArrayBuffer(8)).bytes.byteLength, 0);
});

test("only JSON objects with a string type are control messages", () => {
  assert.deepEqual(parseControl('{"type":"gap","message":"lost"}'), { type: "gap", message: "lost" });
  for (const text of ["not json", "null", '"stream"', "[]", '{"type":3}', "{}"])
    assert.equal(parseControl(text), undefined, text);
});

test("the screen survives a reconnect only when the same process resumed", () => {
  const previous = { runtime: "r1", seq: 9 };
  assert.deepEqual(afterHello({ type: "stream", runtime: "r1", resumed: true }, previous), {
    reset: false,
    resume: { runtime: "r1", seq: 9 },
  });
  // The server could not resume: redraw from its replay, counting from the start.
  assert.deepEqual(afterHello({ type: "stream", runtime: "r1", resumed: false }, previous), {
    reset: true,
    resume: { runtime: "r1", seq: 0 },
  });
  // A different process: whatever was on screen belonged to the old one.
  assert.deepEqual(afterHello({ type: "stream", runtime: "r2", resumed: true }, previous), {
    reset: true,
    resume: { runtime: "r2", seq: 9 },
  });
  // The first connection has nothing to resume.
  assert.deepEqual(afterHello({ type: "stream", runtime: "r1", resumed: true }, undefined), {
    reset: true,
    resume: { runtime: "r1", seq: 0 },
  });
  // A hello without a runtime cannot be resumed later.
  assert.deepEqual(afterHello({ type: "stream" }, previous), { reset: true, resume: undefined });
});

test("the socket address asks for framing and carries the size and the resume point", () => {
  const plain = new URL(streamUrl("http://host:28789/workspaces", "a b", { cols: 80, rows: 24 }, undefined));
  assert.equal(plain.protocol, "ws:");
  assert.equal(plain.pathname, "/api/sessions/a%20b/pty/ws");
  assert.equal(plain.searchParams.get("cols"), "80");
  assert.equal(plain.searchParams.get("rows"), "24");
  assert.equal(plain.searchParams.get("framed"), "true");
  assert.equal(plain.searchParams.has("runtime"), false);
  const resumed = new URL(streamUrl("https://host/", "id", undefined, { runtime: "r1", seq: 12 }));
  assert.equal(resumed.protocol, "wss:");
  assert.equal(resumed.searchParams.has("cols"), false, "no size before the view has one");
  assert.equal(resumed.searchParams.get("runtime"), "r1");
  assert.equal(resumed.searchParams.get("after"), "12");
});
