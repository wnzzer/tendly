/**
 * How the file pane lays a file out, as plain functions: which view of a
 * Markdown file is on screen, the views on offer, and where the band behind
 * the caret's line goes.
 */

export type MarkdownMode = "preview" | "split" | "source";

/** A remembered mode, or side by side when there is none or it is not one. */
export function parseMarkdownMode(value: string | null | undefined): MarkdownMode {
  return value === "preview" || value === "source" || value === "split" ? value : "split";
}

/**
 * What is on screen. Only Markdown has a rendered view, and two columns that
 * do not fit the pane become the rendered view alone.
 */
export function visibleMode(isMarkdown: boolean, mode: MarkdownMode, canSplit: boolean): MarkdownMode {
  if (!isMarkdown) return "source";
  return mode === "split" && !canSplit ? "preview" : mode;
}

/** The views offered: three where two columns fit, else reading and editing. */
export function modeChoices(canSplit: boolean): { mode: MarkdownMode; label: string }[] {
  return canSplit
    ? [
        { mode: "preview", label: "Preview" },
        { mode: "split", label: "Split" },
        { mode: "source", label: "Source" },
      ]
    : [
        { mode: "preview", label: "Preview" },
        { mode: "source", label: "Edit" },
      ];
}

/** The zero-based line `offset` falls on in `text`. */
export function lineOfOffset(text: string, offset: number): number {
  let line = 0;
  for (let at = text.indexOf("\n"); at !== -1 && at < offset; at = text.indexOf("\n", at + 1)) line++;
  return line;
}

/** Each line's height and the space above the first, in an editor of unwrapped lines. */
export const LINE_HEIGHT = 20,
  EDITOR_TOP = 12;

/**
 * Where the band behind the caret's line goes. Unwrapped lines are all one
 * height; wrapped ones are where measuring found them, and a line not
 * measured yet gets no band rather than a wrong one.
 */
export function caretBand(
  line: number,
  wrapped: boolean,
  rows: { tops: readonly number[]; heights: readonly number[] },
): { top: number; height: number } | undefined {
  if (line < 0) return undefined;
  if (!wrapped) return { top: EDITOR_TOP + line * LINE_HEIGHT, height: LINE_HEIGHT };
  const top = rows.tops[line];
  return top === undefined ? undefined : { top, height: rows.heights[line] ?? LINE_HEIGHT };
}
