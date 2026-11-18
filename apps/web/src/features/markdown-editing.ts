/**
 * The two keystrokes that make Markdown bearable in a plain text box.
 *
 * Enter on a list item starts the next item (numbered lists count on, task
 * items start unticked, quotes stay quoted); Enter on an empty item ends the
 * list instead. Tab and Shift+Tab indent and outdent whole lines. Everything
 * else is left to the textarea. Each returns the replacement to make and the
 * selection after it, or undefined to let the key do what it normally does.
 */
export interface TextEdit {
  from: number;
  to: number;
  insert: string;
  selectionStart: number;
  selectionEnd: number;
}

const ITEM = /^(\s*)(?:([-*+])|(\d+)([.)]))(\s+)(\[[ xX]\]\s+)?/;
const QUOTE = /^(\s*>\s?)/;
const INDENT = "  ";

function lineBounds(text: string, at: number): [number, number] {
  const start = text.lastIndexOf("\n", at - 1) + 1,
    end = text.indexOf("\n", at);
  return [start, end < 0 ? text.length : end];
}

export function continueList(text: string, caret: number): TextEdit | undefined {
  const [start, end] = lineBounds(text, caret),
    line = text.slice(start, end);
  const item = line.match(ITEM),
    quote = item ? undefined : line.match(QUOTE);
  const marker = item?.[0] ?? quote?.[0];
  if (!marker || caret < start + marker.length) return undefined;
  // An empty item is how a list ends: the marker goes, the line stays.
  if (!line.slice(marker.length).trim() && caret === end)
    return { from: start, to: end, insert: "", selectionStart: start, selectionEnd: start };
  let next = marker;
  if (item) {
    const [, indent, bullet, number, delimiter, space, task] = item;
    next = indent + (bullet ?? `${Number(number) + 1}${delimiter}`) + space + (task ? "[ ] " : "");
  }
  const insert = "\n" + next,
    at = caret + insert.length;
  return { from: caret, to: caret, insert, selectionStart: at, selectionEnd: at };
}

/**
 * Indent or outdent every line the selection touches. With no selection on a
 * line that is not a list item, Tab just inserts the indent at the caret.
 */
export function indentLines(
  text: string,
  selectionStart: number,
  selectionEnd: number,
  outdent: boolean,
): TextEdit | undefined {
  const [first] = lineBounds(text, selectionStart);
  // A selection ending at the very start of a line does not include that line.
  const endAt = selectionEnd > selectionStart && text[selectionEnd - 1] === "\n" ? selectionEnd - 1 : selectionEnd;
  const [, last] = lineBounds(text, endAt);
  const region = text.slice(first, last),
    lines = region.split("\n");
  if (!outdent && selectionStart === selectionEnd && !ITEM.test(lines[0] ?? "")) {
    const at = selectionStart + INDENT.length;
    return { from: selectionStart, to: selectionStart, insert: INDENT, selectionStart: at, selectionEnd: at };
  }
  let shiftFirst = 0,
    shiftTotal = 0;
  const changed = lines.map((line, index) => {
    if (!outdent) {
      shiftTotal += INDENT.length;
      if (index === 0) shiftFirst = INDENT.length;
      return INDENT + line;
    }
    const removed = line.startsWith("\t") ? 1 : (line.match(/^ {1,2}/)?.[0].length ?? 0);
    shiftTotal -= removed;
    if (index === 0) shiftFirst = -removed;
    return line.slice(removed);
  });
  if (!shiftTotal) return undefined;
  const insert = changed.join("\n");
  return {
    from: first,
    to: last,
    insert,
    selectionStart: Math.max(first, selectionStart + shiftFirst),
    selectionEnd:
      selectionEnd === selectionStart ? Math.max(first, selectionStart + shiftFirst) : selectionEnd + shiftTotal,
  };
}
