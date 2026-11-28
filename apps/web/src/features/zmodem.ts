/**
 * ZMODEM (`sz` download and `rz` upload) over a raw binary PTY stream.
 *
 * Implements Chuck Forsberg's ZMODEM frame layer (ZHEX, ZBIN16, ZBIN32, ZDLE
 * escaping, CRC-16/XMODEM and CRC-32) and both file-transfer state machines:
 * - `sz` (`ZRQINIT`): receives one or more files into memory (up to a safety
 *   cap) and hands each completed file to `onDownload`.
 * - `rz` (`ZRINIT` while idle): notifies the UI (`awaiting_files`) so a real
 *   user gesture can open the browser's file picker, then streams each file in
 *   flow-controlled `ZCRCW` windows so the PTY input queue never overflows.
 *
 * When idle, chunks without `ZDLE` (`0x18`) pass through untouched with zero
 * copying.
 */

export const ZDLE = 0x18;
const ZPAD = 0x2a;
const ZBIN = 0x41;
const ZHEX = 0x42;
const ZBIN32 = 0x43;

const ZRQINIT = 0;
const ZRINIT = 1;
const ZSINIT = 2;
const ZACK = 3;
const ZFILE = 4;
const ZSKIP = 5;
const ZABORT = 7;
const ZFIN = 8;
const ZRPOS = 9;
const ZDATA = 10;
const ZEOF = 11;
const ZFERR = 12;

const ZCRCE = 0x68;
const ZCRCG = 0x69;
const ZCRCQ = 0x6a;
const ZCRCW = 0x6b;
const ZRUB0 = 0x6c;
const ZRUB1 = 0x6d;

/** `ZF0` capabilities in `p3`: full-duplex + overlap I/O + 32-bit FCS. */
const CANFDX = 0x01;
const CANOVIO = 0x02;
const CANFC32 = 0x20;

/** Cap in-memory `sz` downloads at 512 MiB so a multi-GB file cannot crash the tab. */
export const MAX_ZMODEM_DOWNLOAD_BYTES = 512 * 1024 * 1024;
const SUBPACKET_BYTES = 8192;
const WINDOW_SUBPACKETS = 4;

const ABORT_SEQUENCE = Uint8Array.of(
  0x18,
  0x18,
  0x18,
  0x18,
  0x18,
  0x18,
  0x18,
  0x18,
  0x08,
  0x08,
  0x08,
  0x08,
  0x08,
  0x08,
  0x08,
  0x08,
  0x08,
  0x08,
);

export type ZmodemStatus =
  | { phase: "idle" }
  | { phase: "awaiting_files" }
  | { phase: "downloading"; name: string; transferred: number; total?: number }
  | { phase: "uploading"; name: string; transferred: number; total: number; index: number; count: number };

export interface ZmodemUploadFile {
  name: string;
  size: number;
  lastModified?: number;
  arrayBuffer(): Promise<ArrayBuffer>;
}

export interface ZmodemOptions {
  /** Send raw protocol bytes back to the PTY. */
  send(bytes: Uint8Array): void;
  /** Deliver a completed `sz` file to the browser. */
  onDownload(name: string, bytes: Uint8Array): void;
  /** Report live transfer state to the UI. */
  onStatus?(status: ZmodemStatus): void;
  /** Report a transfer failure or skipped file. */
  onError?(message: string): void;
  maxDownloadBytes?: number;
}

const CRC16_TABLE = (() => {
  const table = new Uint16Array(256);
  for (let i = 0; i < 256; i++) {
    let crc = i << 8;
    for (let bit = 0; bit < 8; bit++) crc = crc & 0x8000 ? ((crc << 1) ^ 0x1021) & 0xffff : (crc << 1) & 0xffff;
    table[i] = crc;
  }
  return table;
})();

const CRC32_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let i = 0; i < 256; i++) {
    let crc = i;
    for (let bit = 0; bit < 8; bit++) crc = crc & 1 ? (crc >>> 1) ^ 0xedb88320 : crc >>> 1;
    table[i] = crc >>> 0;
  }
  return table;
})();

export function crc16Xmodem(bytes: Uint8Array, extraByte?: number): number {
  let crc = 0;
  for (let i = 0; i < bytes.length; i++) crc = ((crc << 8) ^ CRC16_TABLE[((crc >>> 8) ^ bytes[i]!) & 0xff]!) & 0xffff;
  if (extraByte !== undefined) crc = ((crc << 8) ^ CRC16_TABLE[((crc >>> 8) ^ extraByte) & 0xff]!) & 0xffff;
  return crc;
}

export function crc32Zmodem(bytes: Uint8Array, extraByte?: number): number {
  let crc = 0xffffffff;
  for (let i = 0; i < bytes.length; i++) crc = (crc >>> 8) ^ CRC32_TABLE[(crc ^ bytes[i]!) & 0xff]!;
  if (extraByte !== undefined) crc = (crc >>> 8) ^ CRC32_TABLE[(crc ^ extraByte) & 0xff]!;
  return (crc ^ 0xffffffff) >>> 0;
}

const HEX = "0123456789abcdef";
function hexValue(byte: number): number {
  if (byte >= 0x30 && byte <= 0x39) return byte - 0x30;
  if (byte >= 0x61 && byte <= 0x66) return byte - 0x61 + 10;
  if (byte >= 0x41 && byte <= 0x46) return byte - 0x41 + 10;
  return -1;
}

function posToBytes(offset: number): [number, number, number, number] {
  const u = offset >>> 0;
  return [u & 0xff, (u >>> 8) & 0xff, (u >>> 16) & 0xff, (u >>> 24) & 0xff];
}

function bytesToPos(p: readonly number[]): number {
  return ((p[0]! | (p[1]! << 8) | (p[2]! << 16)) >>> 0) + p[3]! * 0x1000000;
}

/** Encode a ZMODEM hex header (`**\x18B` + 14 lowercase hex digits + CRLF + optional XON). */
export function encodeHexHeader(type: number, p: readonly [number, number, number, number]): Uint8Array {
  const raw = Uint8Array.of(type & 0xff, p[0] & 0xff, p[1] & 0xff, p[2] & 0xff, p[3] & 0xff);
  const crc = crc16Xmodem(raw);
  const all = [raw[0]!, raw[1]!, raw[2]!, raw[3]!, raw[4]!, (crc >>> 8) & 0xff, crc & 0xff];
  const withXon = type !== ZACK && type !== ZFIN;
  const out = new Uint8Array(4 + 14 + 2 + (withXon ? 1 : 0));
  out[0] = ZPAD;
  out[1] = ZPAD;
  out[2] = ZDLE;
  out[3] = ZHEX;
  let pos = 4;
  for (const b of all) {
    out[pos++] = HEX.charCodeAt((b >>> 4) & 0x0f);
    out[pos++] = HEX.charCodeAt(b & 0x0f);
  }
  out[pos++] = 0x0d;
  out[pos++] = 0x0a;
  if (withXon) out[pos++] = 0x11;
  return out;
}

function needsZdleEscape(byte: number): boolean {
  const low = byte & 0x7f;
  return low < 0x20 || byte === ZDLE || byte === 0x7f || byte === 0xff;
}

function writeZdleByte(target: number[], byte: number) {
  if (byte === 0x7f) {
    target.push(ZDLE, ZRUB0);
  } else if (byte === 0xff) {
    target.push(ZDLE, ZRUB1);
  } else if (needsZdleEscape(byte)) {
    target.push(ZDLE, byte ^ 0x40);
  } else {
    target.push(byte);
  }
}

export function encodeBinHeader(
  type: number,
  p: readonly [number, number, number, number],
  crc32: boolean,
): Uint8Array {
  const raw = Uint8Array.of(type & 0xff, p[0] & 0xff, p[1] & 0xff, p[2] & 0xff, p[3] & 0xff);
  const out: number[] = [ZPAD, ZDLE, crc32 ? ZBIN32 : ZBIN];
  for (const b of raw) writeZdleByte(out, b);
  if (crc32) {
    const crc = crc32Zmodem(raw);
    writeZdleByte(out, crc & 0xff);
    writeZdleByte(out, (crc >>> 8) & 0xff);
    writeZdleByte(out, (crc >>> 16) & 0xff);
    writeZdleByte(out, (crc >>> 24) & 0xff);
  } else {
    const crc = crc16Xmodem(raw);
    writeZdleByte(out, (crc >>> 8) & 0xff);
    writeZdleByte(out, crc & 0xff);
  }
  return Uint8Array.from(out);
}

export function encodeSubpacket(payload: Uint8Array, frameEnd: number, crc32: boolean): Uint8Array {
  const out: number[] = [];
  for (let i = 0; i < payload.length; i++) writeZdleByte(out, payload[i]!);
  out.push(ZDLE, frameEnd);
  if (crc32) {
    const crc = crc32Zmodem(payload, frameEnd);
    writeZdleByte(out, crc & 0xff);
    writeZdleByte(out, (crc >>> 8) & 0xff);
    writeZdleByte(out, (crc >>> 16) & 0xff);
    writeZdleByte(out, (crc >>> 24) & 0xff);
  } else {
    const crc = crc16Xmodem(payload, frameEnd);
    writeZdleByte(out, (crc >>> 8) & 0xff);
    writeZdleByte(out, crc & 0xff);
  }
  if (frameEnd === ZCRCW) out.push(0x11);
  return Uint8Array.from(out);
}

interface ParsedHeader {
  format: "hex" | "bin16" | "bin32";
  type: number;
  p: [number, number, number, number];
  consumed: number;
  validCrc: boolean;
}

const LINE_FLOW_BYTES = new Set([0x11, 0x91, 0x13, 0x93]);

/** Read `count` ZDLE-encoded bytes starting at `offset`; returns `null` if incomplete, `"abort"` on 5x CAN. */
function readZdleBytes(
  buf: Uint8Array,
  offset: number,
  count: number,
): { bytes: Uint8Array; next: number } | "abort" | null {
  const out = new Uint8Array(count);
  let pos = offset;
  let written = 0;
  while (written < count) {
    if (pos >= buf.length) return null;
    const b = buf[pos++]!;
    if (LINE_FLOW_BYTES.has(b)) continue;
    if (b !== ZDLE) {
      out[written++] = b;
      continue;
    }
    if (pos >= buf.length) return null;
    const next = buf[pos++]!;
    if (next === ZDLE) {
      let cans = 2;
      while (cans < 5) {
        if (pos >= buf.length) return null;
        if (buf[pos] === ZDLE) {
          cans++;
          pos++;
        } else break;
      }
      if (cans >= 5) return "abort";
      continue;
    }
    if (next === ZRUB0) out[written++] = 0x7f;
    else if (next === ZRUB1) out[written++] = 0xff;
    else if ((next & 0x60) === 0x40) out[written++] = next ^ 0x40;
    else if (LINE_FLOW_BYTES.has(next)) continue;
    else out[written++] = next;
  }
  return { bytes: out, next: pos };
}

/** Try parsing a ZMODEM header starting at `start` (pointing at the first `*`). */
function tryParseHeader(buf: Uint8Array, start: number): ParsedHeader | "abort" | "incomplete" | "invalid" {
  let pos = start;
  while (pos < buf.length && buf[pos] === ZPAD) pos++;
  if (pos === start) return "invalid";
  if (pos >= buf.length) return "incomplete";
  if (buf[pos] !== ZDLE) return "invalid";
  pos++;
  if (pos >= buf.length) return "incomplete";
  const fmt = buf[pos++]!;
  if (fmt === ZHEX) {
    const digits: number[] = [];
    while (digits.length < 14) {
      if (pos >= buf.length) return "incomplete";
      const b = buf[pos++]!;
      if (LINE_FLOW_BYTES.has(b)) continue;
      const v = hexValue(b);
      if (v < 0) return "invalid";
      digits.push(v);
    }
    // Consume trailing CR/LF (including 8th-bit-set 0x8a from lrzsz) and XON.
    while (pos < buf.length) {
      const trail = buf[pos]!;
      if (trail === 0x0d || trail === 0x8d || trail === 0x0a || trail === 0x8a || trail === 0x11 || trail === 0x91) {
        pos++;
      } else {
        break;
      }
    }
    const bytes = new Uint8Array(7);
    for (let i = 0; i < 7; i++) bytes[i] = (digits[i * 2]! << 4) | digits[i * 2 + 1]!;
    const expected = crc16Xmodem(bytes.subarray(0, 5));
    const actual = (bytes[5]! << 8) | bytes[6]!;
    return {
      format: "hex",
      type: bytes[0]!,
      p: [bytes[1]!, bytes[2]!, bytes[3]!, bytes[4]!],
      consumed: pos - start,
      validCrc: expected === actual,
    };
  }
  if (fmt === ZBIN || fmt === ZBIN32) {
    const is32 = fmt === ZBIN32;
    const decoded = readZdleBytes(buf, pos, is32 ? 9 : 7);
    if (decoded === null) return "incomplete";
    if (decoded === "abort") return "abort";
    const head = decoded.bytes.subarray(0, 5);
    const validCrc = is32
      ? crc32Zmodem(head) ===
        ((decoded.bytes[5]! | (decoded.bytes[6]! << 8) | (decoded.bytes[7]! << 16)) >>> 0) +
          decoded.bytes[8]! * 0x1000000
      : crc16Xmodem(head) === ((decoded.bytes[5]! << 8) | decoded.bytes[6]!);
    return {
      format: is32 ? "bin32" : "bin16",
      type: head[0]!,
      p: [head[1]!, head[2]!, head[3]!, head[4]!],
      consumed: decoded.next - start,
      validCrc,
    };
  }
  return "invalid";
}

function parseZfileInfo(payload: Uint8Array): { name: string; size?: number } {
  let nul = payload.indexOf(0);
  if (nul < 0) nul = payload.length;
  const rawName = new TextDecoder("utf-8", { fatal: false }).decode(payload.subarray(0, nul));
  const cleanName = rawName.replace(/\\/g, "/").split("/").filter(Boolean).pop() || "download.bin";
  const metaText = new TextDecoder("ascii", { fatal: false }).decode(
    payload.subarray(Math.min(payload.length, nul + 1)),
  );
  const sizeField = metaText.trim().split(/\s+/)[0];
  const parsedSize = sizeField && /^\d+$/.test(sizeField) ? Number(sizeField) : undefined;
  return { name: cleanName, size: Number.isSafeInteger(parsedSize) ? parsedSize : undefined };
}

function sanitizeUploadName(name: string): string {
  return (
    name
      .replace(/\\/g, "/")
      .split("/")
      .filter(Boolean)
      .pop()
      ?.replace(/[\x00-\x1f]/g, "_") || "upload.bin"
  );
}

export function createZmodem(options: ZmodemOptions) {
  const maxDownload = options.maxDownloadBytes ?? MAX_ZMODEM_DOWNLOAD_BYTES;
  let mode: "idle" | "rx_header" | "rx_subpacket" | "tx_wait_files" | "tx_wait_header" | "rx_oo" = "idle";
  let rxSubpacketKind: "zsinit" | "zfile" | "zdata" = "zfile";
  let rxSubpacketCrc32 = false;
  let buffer: Uint8Array = new Uint8Array(0);

  // Active `sz` download file state.
  let rxFile:
    | {
        name: string;
        size?: number;
        chunks: Uint8Array[];
        received: number;
      }
    | undefined;
  let subpacketAccum: number[] = [];
  let subpacketEscaped = false;
  let subpacketFrameEnd = 0;
  let subpacketCrcNeed = 0;
  let subpacketCrcBuf: number[] = [];
  let canRun = 0;

  // Active `rz` upload queue state.
  let peerCrc32 = true;
  let txQueue: ZmodemUploadFile[] = [];
  let txIndex = 0;
  let txCurrent: { name: string; bytes: Uint8Array; mtime: number } | undefined;
  let txStage: "none" | "file_sent" | "window_sent" | "eof_sent" | "fin_sent" = "none";
  let txOffset = 0;

  function setStatus(status: ZmodemStatus) {
    options.onStatus?.(status);
  }

  function resetState() {
    mode = "idle";
    buffer = new Uint8Array(0);
    rxFile = undefined;
    subpacketAccum = [];
    subpacketEscaped = false;
    subpacketFrameEnd = 0;
    subpacketCrcNeed = 0;
    subpacketCrcBuf = [];
    canRun = 0;
    txQueue = [];
    txIndex = 0;
    txCurrent = undefined;
    txStage = "none";
    txOffset = 0;
    setStatus({ phase: "idle" });
  }

  function sendZrinit() {
    options.send(encodeHexHeader(ZRINIT, [0, 0, 0, CANFDX | CANOVIO | CANFC32]));
  }

  function appendBuffer(chunk: Uint8Array) {
    if (!buffer.length) {
      buffer = chunk.slice();
      return;
    }
    const next = new Uint8Array(buffer.length + chunk.length);
    next.set(buffer, 0);
    next.set(chunk, buffer.length);
    buffer = next;
  }

  function beginSubpacket(kind: "zsinit" | "zfile" | "zdata", crc32: boolean) {
    mode = "rx_subpacket";
    rxSubpacketKind = kind;
    rxSubpacketCrc32 = crc32;
    subpacketAccum = [];
    subpacketEscaped = false;
    subpacketFrameEnd = 0;
    subpacketCrcNeed = 0;
    subpacketCrcBuf = [];
  }

  function finishDownload() {
    if (!rxFile) return;
    const total = rxFile.received;
    const out = new Uint8Array(total);
    let offset = 0;
    for (const part of rxFile.chunks) {
      out.set(part, offset);
      offset += part.length;
    }
    const name = rxFile.name;
    rxFile = undefined;
    options.onDownload(name, out);
  }

  async function startNextUploadFile() {
    if (txIndex >= txQueue.length) {
      txCurrent = undefined;
      txStage = "fin_sent";
      mode = "tx_wait_header";
      options.send(encodeHexHeader(ZFIN, [0, 0, 0, 0]));
      return;
    }
    const file = txQueue[txIndex]!;
    const name = sanitizeUploadName(file.name);
    setStatus({
      phase: "uploading",
      name,
      transferred: 0,
      total: file.size,
      index: txIndex + 1,
      count: txQueue.length,
    });
    try {
      const bytes = new Uint8Array(await file.arrayBuffer());
      if (mode !== "tx_wait_header" && mode !== "tx_wait_files") return;
      const mtime = Math.floor((file.lastModified ?? Date.now()) / 1000);
      txCurrent = { name, bytes, mtime };
      txOffset = 0;
      txStage = "file_sent";
      mode = "tx_wait_header";
      const remainingFiles = txQueue.length - txIndex;
      let remainingBytes = bytes.byteLength;
      for (let i = txIndex + 1; i < txQueue.length; i++) remainingBytes += txQueue[i]!.size;
      const info = new TextEncoder().encode(
        `${name}\0${bytes.byteLength} ${mtime.toString(8)} 100644 0 ${remainingFiles} ${remainingBytes}\0`,
      );
      options.send(encodeBinHeader(ZFILE, [0, 0, 0, 0], peerCrc32));
      options.send(encodeSubpacket(info, ZCRCW, peerCrc32));
    } catch (cause) {
      options.onError?.(cause instanceof Error ? cause.message : "Failed to read selected file.");
      cancel();
    }
  }

  function sendUploadWindow() {
    if (!txCurrent) return;
    const { name, bytes } = txCurrent;
    options.send(encodeBinHeader(ZDATA, posToBytes(txOffset), peerCrc32));
    if (bytes.byteLength === 0 || txOffset >= bytes.byteLength) {
      options.send(encodeSubpacket(new Uint8Array(0), ZCRCE, peerCrc32));
      txOffset = bytes.byteLength;
      txStage = "eof_sent";
      options.send(encodeHexHeader(ZEOF, posToBytes(txOffset)));
      setStatus({
        phase: "uploading",
        name,
        transferred: txOffset,
        total: bytes.byteLength,
        index: txIndex + 1,
        count: txQueue.length,
      });
      return;
    }
    let packets = 0;
    while (txOffset < bytes.byteLength && packets < WINDOW_SUBPACKETS) {
      const end = Math.min(bytes.byteLength, txOffset + SUBPACKET_BYTES);
      const slice = bytes.subarray(txOffset, end);
      txOffset = end;
      packets++;
      const atEof = txOffset >= bytes.byteLength;
      const frameEnd = atEof ? ZCRCE : packets === WINDOW_SUBPACKETS ? ZCRCW : ZCRCG;
      options.send(encodeSubpacket(slice, frameEnd, peerCrc32));
      if (atEof) {
        txStage = "eof_sent";
        options.send(encodeHexHeader(ZEOF, posToBytes(txOffset)));
        break;
      }
      if (frameEnd === ZCRCW) {
        txStage = "window_sent";
        break;
      }
    }
    setStatus({
      phase: "uploading",
      name,
      transferred: txOffset,
      total: bytes.byteLength,
      index: txIndex + 1,
      count: txQueue.length,
    });
  }

  function handleRxHeader(header: ParsedHeader) {
    if (!header.validCrc) {
      if (rxFile) options.send(encodeHexHeader(ZRPOS, posToBytes(rxFile.received)));
      return;
    }
    const crc32 = header.format === "bin32";
    switch (header.type) {
      case ZRQINIT:
        sendZrinit();
        break;
      case ZSINIT:
        beginSubpacket("zsinit", crc32);
        break;
      case ZFILE:
        beginSubpacket("zfile", crc32);
        break;
      case ZDATA: {
        const pos = bytesToPos(header.p);
        if (!rxFile) {
          options.send(encodeHexHeader(ZSKIP, [0, 0, 0, 0]));
          break;
        }
        if (pos !== rxFile.received) {
          options.send(encodeHexHeader(ZRPOS, posToBytes(rxFile.received)));
          break;
        }
        beginSubpacket("zdata", crc32);
        break;
      }
      case ZEOF: {
        const pos = bytesToPos(header.p);
        if (!rxFile) {
          sendZrinit();
          break;
        }
        if (pos !== rxFile.received) {
          options.send(encodeHexHeader(ZRPOS, posToBytes(rxFile.received)));
          break;
        }
        finishDownload();
        sendZrinit();
        break;
      }
      case ZFIN:
        options.send(encodeHexHeader(ZFIN, [0, 0, 0, 0]));
        mode = "rx_oo";
        break;
      case ZABORT:
      case ZFERR:
        options.onError?.("ZMODEM transfer was aborted by the remote program.");
        resetState();
        break;
      default:
        break;
    }
  }

  function handleTxHeader(header: ParsedHeader) {
    if (!header.validCrc) return;
    switch (header.type) {
      case ZRINIT:
        peerCrc32 = (header.p[3]! & CANFC32) !== 0;
        if (txStage === "eof_sent") {
          txIndex++;
          void startNextUploadFile();
        } else if (txStage === "file_sent" && txCurrent) {
          // Remote repeated ZRINIT before seeing ZFILE; wait or re-offer on next cycle.
        }
        break;
      case ZRPOS: {
        if (!txCurrent) break;
        const requested = bytesToPos(header.p);
        txOffset = Math.min(Math.max(0, requested), txCurrent.bytes.byteLength);
        sendUploadWindow();
        break;
      }
      case ZACK:
        if (txStage === "window_sent" && txCurrent) {
          sendUploadWindow();
        }
        break;
      case ZSKIP:
        txIndex++;
        void startNextUploadFile();
        break;
      case ZFIN:
        options.send(Uint8Array.of(0x4f, 0x4f));
        resetState();
        break;
      case ZABORT:
      case ZFERR:
        options.onError?.("ZMODEM upload was rejected by the remote program.");
        resetState();
        break;
      default:
        break;
    }
  }

  function consumeSubpacketBuffer(): boolean {
    let pos = 0;
    while (pos < buffer.length) {
      const b = buffer[pos++]!;
      if (b === ZDLE) {
        canRun++;
        if (canRun >= 5) {
          buffer = buffer.subarray(pos);
          options.onError?.("ZMODEM transfer cancelled.");
          resetState();
          return true;
        }
        if (subpacketEscaped) continue;
        subpacketEscaped = true;
        continue;
      }
      canRun = 0;
      if (LINE_FLOW_BYTES.has(b) && !subpacketEscaped) continue;
      if (subpacketCrcNeed > 0) {
        let decoded = b;
        if (subpacketEscaped) {
          subpacketEscaped = false;
          if (b === ZRUB0) decoded = 0x7f;
          else if (b === ZRUB1) decoded = 0xff;
          else if ((b & 0x60) === 0x40) decoded = b ^ 0x40;
        }
        subpacketCrcBuf.push(decoded);
        if (subpacketCrcBuf.length < subpacketCrcNeed) continue;
        // Consume optional trailing XON after ZCRCW.
        if (subpacketFrameEnd === ZCRCW && pos < buffer.length && (buffer[pos] === 0x11 || buffer[pos] === 0x91)) {
          pos++;
        }
        buffer = buffer.subarray(pos);
        const payload = Uint8Array.from(subpacketAccum);
        const valid = rxSubpacketCrc32
          ? crc32Zmodem(payload, subpacketFrameEnd) ===
            ((subpacketCrcBuf[0]! | (subpacketCrcBuf[1]! << 8) | (subpacketCrcBuf[2]! << 16)) >>> 0) +
              subpacketCrcBuf[3]! * 0x1000000
          : crc16Xmodem(payload, subpacketFrameEnd) === ((subpacketCrcBuf[0]! << 8) | subpacketCrcBuf[1]!);
        const frameEnd = subpacketFrameEnd;
        subpacketAccum = [];
        subpacketFrameEnd = 0;
        subpacketCrcNeed = 0;
        subpacketCrcBuf = [];
        onSubpacketComplete(payload, frameEnd, valid);
        return true;
      }
      if (subpacketEscaped) {
        subpacketEscaped = false;
        if (b === ZCRCE || b === ZCRCG || b === ZCRCQ || b === ZCRCW) {
          subpacketFrameEnd = b;
          subpacketCrcNeed = rxSubpacketCrc32 ? 4 : 2;
          subpacketCrcBuf = [];
          continue;
        }
        if (b === ZRUB0) subpacketAccum.push(0x7f);
        else if (b === ZRUB1) subpacketAccum.push(0xff);
        else if ((b & 0x60) === 0x40) subpacketAccum.push(b ^ 0x40);
        else subpacketAccum.push(b);
      } else {
        subpacketAccum.push(b);
      }
    }
    buffer = new Uint8Array(0);
    return false;
  }

  function onSubpacketComplete(payload: Uint8Array, frameEnd: number, validCrc: boolean) {
    if (rxSubpacketKind === "zsinit") {
      mode = "rx_header";
      if (validCrc) options.send(encodeHexHeader(ZACK, [0, 0, 0, 0]));
      return;
    }
    if (rxSubpacketKind === "zfile") {
      mode = "rx_header";
      if (!validCrc) {
        sendZrinit();
        return;
      }
      const info = parseZfileInfo(payload);
      if (info.size !== undefined && info.size > maxDownload) {
        options.send(encodeHexHeader(ZSKIP, [0, 0, 0, 0]));
        options.onError?.(`Skipped ${info.name}: file is larger than the 512 MiB browser download limit.`);
        return;
      }
      rxFile = { name: info.name, size: info.size, chunks: [], received: 0 };
      setStatus({ phase: "downloading", name: info.name, transferred: 0, total: info.size });
      options.send(encodeHexHeader(ZRPOS, [0, 0, 0, 0]));
      return;
    }
    // `zdata`
    if (!rxFile) {
      mode = "rx_header";
      return;
    }
    if (!validCrc) {
      mode = "rx_header";
      options.send(encodeHexHeader(ZRPOS, posToBytes(rxFile.received)));
      return;
    }
    if (rxFile.received + payload.length > maxDownload) {
      rxFile = undefined;
      mode = "rx_header";
      options.onError?.("Download exceeded the 512 MiB browser memory limit and was cancelled.");
      cancel();
      return;
    }
    if (payload.length) {
      rxFile.chunks.push(payload);
      rxFile.received += payload.length;
      setStatus({ phase: "downloading", name: rxFile.name, transferred: rxFile.received, total: rxFile.size });
    }
    if (frameEnd === ZCRCQ || frameEnd === ZCRCW) {
      options.send(encodeHexHeader(ZACK, posToBytes(rxFile.received)));
    }
    if (frameEnd === ZCRCE || frameEnd === ZCRCW) {
      mode = "rx_header";
    } else {
      // ZCRCG or ZCRCQ: another data subpacket follows immediately without a header.
      mode = "rx_subpacket";
    }
  }

  function processActiveBuffer() {
    while (buffer.length > 0 && mode !== "idle") {
      if (mode === "rx_oo") {
        // Consume trailing "OO" (or anything up to 2 bytes) and return to terminal.
        if (buffer[0] === 0x4f) {
          if (buffer.length < 2) return;
          buffer = buffer.subarray(buffer[1] === 0x4f ? 2 : 1);
        }
        const leftover = buffer;
        resetState();
        buffer = leftover;
        return;
      }
      if (mode === "rx_subpacket") {
        if (!consumeSubpacketBuffer()) return;
        continue;
      }
      // Searching for a header (`*` + ZDLE + A/B/C) or 5x CAN abort.
      let fiveCan = 0;
      for (let i = 0; i < buffer.length; i++) {
        if (buffer[i] === ZDLE) {
          if (++fiveCan >= 5) {
            buffer = buffer.subarray(i + 1);
            options.onError?.("ZMODEM transfer cancelled.");
            resetState();
            return;
          }
        } else if (!LINE_FLOW_BYTES.has(buffer[i]!)) {
          fiveCan = 0;
        }
      }
      const pad = buffer.indexOf(ZPAD);
      if (pad < 0) {
        // Keep trailing ZDLE bytes in case a 5x CAN split across chunks.
        let tailCan = 0;
        while (tailCan < buffer.length && buffer[buffer.length - 1 - tailCan] === ZDLE) tailCan++;
        buffer = tailCan ? buffer.subarray(buffer.length - tailCan) : new Uint8Array(0);
        return;
      }
      if (pad > 0) buffer = buffer.subarray(pad);
      const parsed = tryParseHeader(buffer, 0);
      if (parsed === "incomplete") return;
      if (parsed === "abort") {
        options.onError?.("ZMODEM transfer cancelled.");
        resetState();
        return;
      }
      if (parsed === "invalid") {
        buffer = buffer.subarray(1);
        continue;
      }
      buffer = buffer.subarray(parsed.consumed);
      if (mode === "rx_header") {
        handleRxHeader(parsed);
      } else if (mode === "tx_wait_files") {
        if (parsed.validCrc && parsed.type === ZRINIT) {
          peerCrc32 = (parsed.p[3]! & CANFC32) !== 0;
        } else if (parsed.type === ZABORT || parsed.type === ZFERR) {
          resetState();
          return;
        }
      } else if (mode === "tx_wait_header") {
        handleTxHeader(parsed);
      }
    }
  }

  /**
   * Inspect an incoming PTY chunk. Returns the bytes that belong to the visual
   * terminal (`bytes` itself when no ZMODEM frame is active or starting).
   */
  function consume(bytes: Uint8Array, replaying = false): Uint8Array {
    if (replaying) return bytes;
    if (mode === "idle") {
      if (!buffer.length && !bytes.includes(ZDLE) && bytes[bytes.length - 1] !== ZPAD) {
        return bytes;
      }
      const combined = buffer.length
        ? (() => {
            const joined = new Uint8Array(buffer.length + bytes.length);
            joined.set(buffer, 0);
            joined.set(bytes, buffer.length);
            buffer = new Uint8Array(0);
            return joined;
          })()
        : bytes;

      // Scan for `**\x18B00` (sz ZRQINIT) or `**\x18B01` (rz ZRINIT) with valid CRC-16.
      let searchFrom = 0;
      while (searchFrom < combined.length) {
        const star = combined.indexOf(ZPAD, searchFrom);
        if (star < 0) return combined;
        // Require `**` for initial hex trigger so random `*` in output costs nothing.
        if (star + 1 >= combined.length) {
          buffer = combined.subarray(star);
          return combined.subarray(0, star);
        }
        if (combined[star + 1] !== ZPAD) {
          searchFrom = star + 1;
          continue;
        }
        const parsed = tryParseHeader(combined, star);
        if (parsed === "incomplete") {
          // Hold at most 24 bytes at the end of a frame while waiting for the rest of the header.
          if (combined.length - star <= 24) {
            buffer = combined.subarray(star);
            return combined.subarray(0, star);
          }
          searchFrom = star + 1;
          continue;
        }
        if (parsed !== "invalid" && parsed !== "abort" && parsed.format === "hex" && parsed.validCrc) {
          if (parsed.type === ZRQINIT) {
            const before = combined.subarray(0, star);
            mode = "rx_header";
            buffer = combined.subarray(star + parsed.consumed);
            sendZrinit();
            processActiveBuffer();
            return before;
          }
          if (parsed.type === ZRINIT) {
            const before = combined.subarray(0, star);
            peerCrc32 = (parsed.p[3]! & CANFC32) !== 0;
            mode = "tx_wait_files";
            buffer = combined.subarray(star + parsed.consumed);
            setStatus({ phase: "awaiting_files" });
            processActiveBuffer();
            return before;
          }
        }
        searchFrom = star + 1;
      }
      return combined;
    }

    appendBuffer(bytes);
    processActiveBuffer();
    if ((mode as string) === "idle" && buffer.length > 0) {
      const rest = buffer;
      buffer = new Uint8Array(0);
      return rest;
    }
    return new Uint8Array(0);
  }

  /** Provide the files the user picked for an active `rz` request. */
  function sendFiles(files: readonly ZmodemUploadFile[]) {
    if (mode !== "tx_wait_files") return;
    if (!files.length) {
      cancel();
      return;
    }
    txQueue = [...files];
    txIndex = 0;
    mode = "tx_wait_header";
    void startNextUploadFile();
  }

  /** Abort an active `sz` or `rz` session and release the terminal. */
  function cancel() {
    if (mode === "idle") return;
    options.send(ABORT_SEQUENCE);
    resetState();
  }

  return {
    get active() {
      return mode !== "idle";
    },
    consume,
    sendFiles,
    cancel,
    reset: resetState,
  };
}
