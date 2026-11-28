import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createZmodem, encodeHexHeader } from "./zmodem.ts";

test("ordinary terminal output passes through untouched without copying, and replay ignores stale headers", () => {
  const plain = new TextEncoder().encode("hello world\r\n");
  const zm = createZmodem({
    send() {
      assert.fail("idle terminal text must never trigger ZMODEM output");
    },
    onDownload() {
      assert.fail("no download expected");
    },
  });
  assert.equal(zm.consume(plain), plain, "fast path returns the exact same Uint8Array");

  const staleHeader = encodeHexHeader(0, [0, 0, 0, 0]);
  assert.equal(zm.consume(staleHeader, true), staleHeader, "history replay does not start a transfer");
  assert.equal(zm.active, false);
});

test("split initial ZRQINIT header across chunks starts ZMODEM cleanly and preserves preceding text", () => {
  const sent = [];
  const zm = createZmodem({
    send: bytes => sent.push(bytes),
    onDownload() {},
  });
  const prefix = new TextEncoder().encode("rz\r");
  const header = encodeHexHeader(0, [0, 0, 0, 0]);
  const full = new Uint8Array(prefix.length + header.length);
  full.set(prefix, 0);
  full.set(header, prefix.length);

  const part1 = zm.consume(full.subarray(0, 6)); // "rz\r**\x18"
  const part2 = zm.consume(full.subarray(6));
  const shown = new TextDecoder().decode(new Uint8Array([...part1, ...part2]));
  assert.equal(shown, "rz\r");
  assert.equal(zm.active, true);
  assert.equal(sent.length, 1, "replied with ZRINIT");
});

const hasLrzsz = existsSync("/usr/bin/sz") && existsSync("/usr/bin/rz");

test(
  "real /usr/bin/sz downloads binary files containing control and ZDLE bytes byte-for-byte",
  { skip: !hasLrzsz },
  async () => {
    const dir = mkdtempSync(join(tmpdir(), "agentdock-sz-"));
    try {
      // Include every tricky byte: NUL, XON, XOFF, CR, LF, 0x8a, ZDLE (0x18), DEL (0x7f), 0xff.
      const sampleA = new Uint8Array(20_000);
      for (let i = 0; i < sampleA.length; i++) sampleA[i] = (i * 73 + (i >> 4)) & 0xff;
      sampleA.set([0x00, 0x10, 0x11, 0x13, 0x18, 0x18, 0x0d, 0x8a, 0x7f, 0xff], 128);
      const sampleB = new TextEncoder().encode("second file via multi-file sz\n中文内容测试\n");

      const fileA = join(dir, "binary-a.bin");
      const fileB = join(dir, "notes-b.txt");
      writeFileSync(fileA, sampleA);
      writeFileSync(fileB, sampleB);

      const downloads = [];
      const statuses = [];
      await new Promise((resolve, reject) => {
        const proc = spawn("/usr/bin/sz", ["--zmodem", "--quiet", fileA, fileB], {
          stdio: ["pipe", "pipe", "pipe"],
        });
        const zm = createZmodem({
          send: bytes => proc.stdin.write(bytes),
          onDownload: (name, bytes) => downloads.push({ name, bytes }),
          onStatus: s => statuses.push(s.phase),
          onError: err => reject(new Error(err)),
        });
        proc.stdout.on("data", chunk => {
          // Feed in small irregular slices to exercise streaming boundary conditions.
          for (let pos = 0; pos < chunk.length; pos += 97) {
            zm.consume(new Uint8Array(chunk.buffer, chunk.byteOffset + pos, Math.min(97, chunk.length - pos)));
          }
        });
        proc.on("error", reject);
        proc.on("close", code => {
          if (code === 0) resolve();
          else reject(new Error(`sz exited with code ${code}`));
        });
      });

      assert.equal(downloads.length, 2);
      assert.equal(downloads[0].name, "binary-a.bin");
      assert.deepEqual(downloads[0].bytes, sampleA);
      assert.equal(downloads[1].name, "notes-b.txt");
      assert.deepEqual(downloads[1].bytes, sampleB);
      assert.ok(statuses.includes("downloading"));
      assert.equal(statuses[statuses.length - 1], "idle");
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  },
);

test(
  "real /usr/bin/rz receives multi-window binary uploads from createZmodem byte-for-byte",
  { skip: !hasLrzsz },
  async () => {
    const dir = mkdtempSync(join(tmpdir(), "agentdock-rz-"));
    try {
      // > 32 KiB so multiple ZCRCW flow-controlled windows and ZACK handshakes run.
      const payload = new Uint8Array(45_000);
      for (let i = 0; i < payload.length; i++) payload[i] = (i * 131 + 17) & 0xff;
      payload.set([0x18, 0x18, 0x18, 0x11, 0x13, 0x7f, 0xff, 0x00, 0x0d, 0x8d], 500);

      let prompted = false;
      await new Promise((resolve, reject) => {
        const proc = spawn("/usr/bin/rz", ["--zmodem", "--quiet", "--overwrite"], {
          cwd: dir,
          stdio: ["pipe", "pipe", "pipe"],
        });
        const zm = createZmodem({
          send: bytes => proc.stdin.write(bytes),
          onDownload() {},
          onStatus(status) {
            if (status.phase === "awaiting_files" && !prompted) {
              prompted = true;
              zm.sendFiles([
                {
                  name: "uploaded-test.bin",
                  size: payload.byteLength,
                  lastModified: 1_700_000_000_000,
                  async arrayBuffer() {
                    return payload.buffer.slice(payload.byteOffset, payload.byteOffset + payload.byteLength);
                  },
                },
              ]);
            }
          },
          onError: err => reject(new Error(err)),
        });
        proc.stdout.on("data", chunk => {
          zm.consume(new Uint8Array(chunk.buffer, chunk.byteOffset, chunk.byteLength));
        });
        proc.on("error", reject);
        proc.on("close", code => {
          if (code === 0) resolve();
          else reject(new Error(`rz exited with code ${code}`));
        });
      });

      assert.equal(prompted, true);
      const saved = new Uint8Array(readFileSync(join(dir, "uploaded-test.bin")));
      assert.deepEqual(saved, payload);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  },
);
