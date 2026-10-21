import { StringDecoder } from 'node:string_decoder';
const MAX_NATIVE_LINE = 64 * 1024 * 1024;
const MAX_NATIVE_TOTAL = 2 * 1024 * 1024 * 1024;
export function jsonLines(readable, onMessage, onFailure, { maxLine = MAX_NATIVE_LINE, maxTotal = MAX_NATIVE_TOTAL, lineTimeout = 10_000, skipInvalid = false } = {}) {
  // An incomplete line is kept as the pieces it came in, with their size, so a
  // long one is looked through once as it arrives rather than with every chunk.
  const decoder = new StringDecoder('utf8'); let pieces = [], pending = 0, total = 0, timer, stopped = false;
  const clear = () => { clearTimeout(timer); timer = undefined; };
  const fail = message => { if (!stopped) { stopped = true; clear(); onFailure(message); } };
  const data = chunk => {
    if (stopped) return;
    total += chunk.length; if (total > maxTotal) { fail('Structured stream exceeded its size limit.'); return; }
    let text = decoder.write(chunk), index;
    while ((index = text.indexOf('\n')) !== -1) {
      clear(); const head = text.slice(0,index); text = text.slice(index+1);
      const size = pending + Buffer.byteLength(head), line = pieces.length ? pieces.join('') + head : head;
      pieces = []; pending = 0;
      if (size > maxLine) { fail('Structured message exceeded its size limit.'); return; }
      if (!line.trim()) continue;
      let message;
      try { message = JSON.parse(line); } catch { if (skipInvalid) continue; fail('Invalid structured message received.'); return; }
      try { onMessage(message); } catch { fail('Invalid structured message received.'); return; }
      if (stopped) return;
    }
    if (text) { pieces.push(text); pending += Buffer.byteLength(text); }
    if (pending > maxLine) { fail('Structured message exceeded its size limit.'); return; }
    if (pending && !timer) timer = setTimeout(() => fail('Incomplete structured message timed out.'), lineTimeout);
  };
  const end = () => { const rest = pieces.join('') + decoder.end(); if (rest.trim() && !stopped) fail('Structured stream ended mid-message.'); clear(); };
  readable.on('data',data); readable.on('end',end);
  return () => { stopped = true; clear(); readable.off('data',data); readable.off('end',end); };
}
