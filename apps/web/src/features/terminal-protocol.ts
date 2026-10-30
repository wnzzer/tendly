import { routedUrl } from "./api";
/**
 * The terminal pane's side of the PTY stream protocol, as plain functions.
 *
 * A framed socket prefixes every output frame with its sequence number, an
 * unsigned 64-bit big-endian integer, so a reconnect can ask to resume after
 * the last frame the view drew. The server opens with a `stream` hello saying
 * which process it is attached to and whether it resumed; a server that does
 * not number its frames sends neither and replays everything instead. Text
 * frames are JSON control messages: `stream`, `exit`, `gap` and `error`.
 */

/** Where a reconnect picks up: the process the view was showing and the last frame it drew. */
export interface ResumePoint { runtime: string; seq: number }

export interface ControlMessage { type: string; message?: string; runtime?: unknown; resumed?: unknown }

/** The sequence number and output of a framed binary frame, or `undefined` for one too short to carry them. */
export function readFramed(data: ArrayBuffer): { seq: number; bytes: Uint8Array } | undefined {
  if (data.byteLength < 8) return undefined;
  return { seq: Number(new DataView(data).getBigUint64(0)), bytes: new Uint8Array(data, 8) };
}

/** A control message, or `undefined` for text that is not one. */
export function parseControl(text: string): ControlMessage | undefined {
  try {
    const parsed: unknown = JSON.parse(text);
    if (!parsed || typeof parsed !== "object" || typeof (parsed as { type?: unknown }).type !== "string") return undefined;
    return parsed as ControlMessage;
  } catch {
    return undefined;
  }
}

/**
 * What a `stream` hello means for the view. The screen is kept only when the
 * server resumed the very process the view was showing; anything else -- a
 * fresh start, a restarted process, a server that could not resume -- is
 * redrawn from what it replays.
 */
export function afterHello(hello: ControlMessage, previous: ResumePoint | undefined): { reset: boolean; resume: ResumePoint | undefined } {
  const runtime = typeof hello.runtime === "string" ? hello.runtime : undefined;
  const resumed = hello.resumed === true;
  return {
    reset: !resumed || runtime === undefined || runtime !== previous?.runtime,
    resume: runtime === undefined ? undefined : { runtime, seq: resumed && previous ? previous.seq : 0 },
  };
}

/** The socket address for a session, carrying the view's size and, on a reconnect, where to resume. */
export function streamUrl(pageUrl: string, sessionId: string, size: { cols: number; rows: number } | undefined, resume: ResumePoint | undefined): string {
  const url = new URL(routedUrl(`/api/sessions/${encodeURIComponent(sessionId)}/pty/ws`), pageUrl);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  if (size) {
    url.searchParams.set("cols", String(size.cols));
    url.searchParams.set("rows", String(size.rows));
  }
  url.searchParams.set("framed", "true");
  if (resume?.runtime) {
    url.searchParams.set("runtime", resume.runtime);
    url.searchParams.set("after", String(resume.seq));
  }
  return url.toString();
}
