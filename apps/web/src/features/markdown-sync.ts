/**
 * Keeping a Markdown source and its preview on the same passage.
 *
 * The two share no geometry: a source line may wrap onto several rows, and a
 * rendered block may be much taller or shorter than the lines it came from.
 * What they share is line numbers. The source side is measured as the top of
 * each line (`offsets`, after wrapping); the preview side as the top of each
 * block that knows its starting line (`anchors`). A position on either side
 * becomes a fractional source line, and that line becomes a position on the
 * other, interpolating between the two anchors around it.
 */
export interface Anchor {
  line: number;
  top: number;
}

function clamp(value: number, low: number, high: number) {
  return Math.min(high, Math.max(low, value));
}

/** The fractional source line at `y` in the editor. */
export function lineAtOffset(offsets: number[], y: number): number {
  if (!offsets.length) return 0;
  let low = 0,
    high = offsets.length - 1;
  if (y <= offsets[0]!) return 0;
  while (low < high) {
    const mid = (low + high + 1) >> 1;
    if (offsets[mid]! <= y) low = mid;
    else high = mid - 1;
  }
  const top = offsets[low]!,
    next = offsets[low + 1];
  return next === undefined || next <= top ? low : low + clamp((y - top) / (next - top), 0, 1);
}

/** Where a fractional source line starts in the editor. */
export function offsetAtLine(offsets: number[], line: number): number {
  if (!offsets.length) return 0;
  const index = clamp(Math.floor(line), 0, offsets.length - 1),
    top = offsets[index]!;
  const next = offsets[index + 1];
  return next === undefined ? top : top + (next - top) * clamp(line - index, 0, 1);
}

/**
 * Anchors in line order with a start and an end, so every line and every
 * position falls between two of them. `lines` is the source's line count and
 * `height` the preview's scroll height.
 */
export function framedAnchors(anchors: Anchor[], lines: number, height: number): Anchor[] {
  const sorted = anchors.filter(anchor => Number.isFinite(anchor.line)).sort((a, b) => a.line - b.line);
  const framed: Anchor[] = [];
  if (!sorted.length || sorted[0]!.line > 0) framed.push({ line: 0, top: 0 });
  for (const anchor of sorted) {
    const last = framed[framed.length - 1];
    // Two blocks cannot start on one line; keep positions moving forward.
    if (last && anchor.line <= last.line) continue;
    framed.push({ line: anchor.line, top: Math.max(anchor.top, last?.top ?? 0) });
  }
  const last = framed[framed.length - 1]!;
  if (lines > last.line) framed.push({ line: lines, top: Math.max(height, last.top) });
  return framed;
}

/** Where a fractional source line is in the preview. */
export function previewTopForLine(anchors: Anchor[], line: number): number {
  if (!anchors.length) return 0;
  let index = 0;
  while (index + 1 < anchors.length && anchors[index + 1]!.line <= line) index++;
  const from = anchors[index]!,
    to = anchors[index + 1];
  if (!to || to.line === from.line) return from.top;
  return from.top + (to.top - from.top) * clamp((line - from.line) / (to.line - from.line), 0, 1);
}

/** The fractional source line at `y` in the preview. */
export function lineForPreviewTop(anchors: Anchor[], y: number): number {
  if (!anchors.length) return 0;
  let index = 0;
  while (index + 1 < anchors.length && anchors[index + 1]!.top <= y) index++;
  const from = anchors[index]!,
    to = anchors[index + 1];
  if (!to || to.top === from.top) return from.line;
  return from.line + (to.line - from.line) * clamp((y - from.top) / (to.top - from.top), 0, 1);
}
